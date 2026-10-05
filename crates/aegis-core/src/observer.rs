//! Read-only AI observation, decision validation, paper simulation, and journal.
//!
//! This module never sends broker orders. Paper fills are estimates, not
//! evidence of live execution quality or strategy profitability.

use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{
    live_market::MarketSample,
    market::{Candle, Timeframe},
};

const MAX_PROMPT_CHARS: usize = 512;
const MAX_PROMPT_BYTES: usize = 10_000;
const MAX_JOURNAL_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SAMPLE_AGE_MS: u64 = 30_000;
const REQUIRED_STRATEGIES: [&str; 4] = ["density_bounce", "structural", "breakout", "liquidity_sweep"];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ObserverConfig {
    /// At 100, every required strategy rule must be confirmed by price data.
    /// At 0, the model may choose freely, subject to the same safety checks.
    pub strictness: u8,
    pub strategies: Vec<StrategyConfig>,
    /// Percent of equity at risk per paper position; capped at 1%.
    pub risk_pct: f64,
    pub initial_equity: f64,
    pub max_spread: f64,
    /// Round-trip commission per ounce, charged once across entry and exit.
    pub commission_per_oz: f64,
    /// Slippage estimate per side, charged at entry and exit.
    pub slippage: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StrategyConfig {
    pub id: String,
    pub enabled: bool,
    pub prompt: String,
    pub timeframes: Vec<Timeframe>,
}

impl Default for ObserverConfig {
    fn default() -> Self {
        let timeframes = Timeframe::ALL.to_vec();
        Self {
            strictness: 100,
            strategies: vec![
                StrategyConfig {
                    id: "density_bounce".into(),
                    enabled: true,
                    prompt: "Ищи отбой цены от подтверждённого уровня. При наличии стакана учитывай реальную плотность; без стакана используй только явно названный ценовой прокси из прошлых экстремумов, не называй его плотностью. Вход только после закрытого бара с отбоем; иначе жди.".into(),
                    timeframes: timeframes.clone(),
                },
                StrategyConfig {
                    id: "structural".into(),
                    enabled: true,
                    prompt: "Ищи структурный разворот: вынос прошлого swing-экстремума, возврат за него и подтверждённый сдвиг закрытия. Не считай один прокол разворотом; при отсутствии всех признаков жди.".into(),
                    timeframes: timeframes.clone(),
                },
                StrategyConfig {
                    id: "breakout".into(),
                    enabled: true,
                    prompt: "Ищи пробой уровня закрытием за пределами прошлого диапазона и удержание цены за уровнем. Не входи на одном касании или незакрытой свече; иначе жди.".into(),
                    timeframes: timeframes.clone(),
                },
                StrategyConfig {
                    id: "liquidity_sweep".into(),
                    enabled: true,
                    prompt: "Ищи снятие ликвидности за прошлым swing high/low и возврат закрытием обратно за уровень. Не утверждай наличие видимой ликвидности без соответствующих данных; при отсутствии рейда и возврата жди.".into(),
                    timeframes,
                },
            ],
            risk_pct: 0.5,
            initial_equity: 10_000.0,
            max_spread: 1.0,
            commission_per_oz: 0.0,
            slippage: 0.0,
        }
    }
}

impl ObserverConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.strictness > 100 {
            return Err("strictness must be between 0 and 100".into());
        }
        if !finite_positive(self.initial_equity) {
            return Err("initial_equity must be finite and positive".into());
        }
        if !finite_positive(self.risk_pct) || self.risk_pct > 1.0 {
            return Err("risk_pct must be positive and no greater than 1%".into());
        }
        if !finite_positive(self.max_spread) {
            return Err("max_spread must be finite and positive".into());
        }
        if !finite_nonnegative(self.commission_per_oz) || !finite_nonnegative(self.slippage) {
            return Err("commission and slippage must be finite and nonnegative".into());
        }
        if self.strategies.len() != REQUIRED_STRATEGIES.len() {
            return Err("exactly the four supported strategies must be configured".into());
        }
        let mut prompt_bytes = 0usize;
        for required in &REQUIRED_STRATEGIES {
            let strategy = self
                .strategies
                .iter()
                .find(|strategy| strategy.id == *required)
                .ok_or_else(|| format!("missing strategy {required}"))?;
            if strategy.id.trim() != *required {
                return Err(format!("unknown strategy id {}", strategy.id));
            }
            let prompt_chars = strategy.prompt.chars().count();
            if strategy.prompt.trim().is_empty() || prompt_chars > MAX_PROMPT_CHARS {
                return Err(format!(
                    "{required} prompt must contain 1..={MAX_PROMPT_CHARS} characters"
                ));
            }
            prompt_bytes = prompt_bytes.saturating_add(strategy.prompt.len());
            if strategy.timeframes.is_empty() || strategy.timeframes.len() > Timeframe::ALL.len() {
                return Err(format!("{required} must select between 1 and 6 timeframes"));
            }
            for (tf_index, timeframe) in strategy.timeframes.iter().enumerate() {
                if !Timeframe::ALL.contains(timeframe) || strategy.timeframes[..tf_index].contains(timeframe) {
                    return Err(format!("{required} has an invalid or duplicate timeframe"));
                }
            }
        }
        if prompt_bytes > MAX_PROMPT_BYTES {
            return Err(format!("strategy prompts exceed {MAX_PROMPT_BYTES} total bytes"));
        }
        if self
            .strategies
            .iter()
            .any(|s| !REQUIRED_STRATEGIES.contains(&s.id.as_str()))
        {
            return Err("unknown strategy configured".into());
        }
        Ok(())
    }

    pub fn strategy(&self, id: &str) -> Option<&StrategyConfig> {
        self.strategies.iter().find(|strategy| strategy.id == id)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FrameSummary {
    pub timeframe: Timeframe,
    pub closed_at_ms: u64,
    pub last_closed: Candle,
    /// Recent closed bars, including `last_closed`, in chronological order.
    pub closed_candles: Vec<Candle>,
    /// Prior 20-bar extremes, deliberately excluding `last_closed`.
    pub support: f64,
    pub resistance: f64,
    pub atr: f64,
    pub trend: String,
}

/// Summarize only bars that have closed by `now_ms`; future and forming bars
/// are ignored. The resulting levels exclude the latest closed candle.
pub fn summarize(timeframe: Timeframe, bars: &[Candle], now_ms: u64) -> Result<FrameSummary, String> {
    if !Timeframe::ALL.contains(&timeframe) {
        return Err("unsupported timeframe".into());
    }
    let seconds = timeframe.seconds() as u64;
    let now_seconds = now_ms / 1000;
    let mut closed = Vec::new();
    let mut previous_time = None;
    for candle in bars {
        validate_candle(candle)?;
        if let Some(previous) = previous_time {
            if candle.time as u64 <= previous {
                return Err("candles must be strictly chronological".into());
            }
        }
        previous_time = Some(candle.time as u64);
        let end = (candle.time as u64).saturating_add(seconds);
        if end <= now_seconds {
            closed.push(*candle);
        }
    }
    if closed.len() < 21 {
        return Err("at least 21 closed candles are required to compute prior 20-bar levels".into());
    }
    let last_index = closed.len() - 1;
    let last_closed = closed[last_index];
    let prior = &closed[last_index - 20..last_index];
    let support = prior.iter().map(|bar| bar.low).fold(f64::INFINITY, f64::min);
    let resistance = prior.iter().map(|bar| bar.high).fold(f64::NEG_INFINITY, f64::max);
    let atr_start = closed.len().saturating_sub(14);
    let mut ranges = Vec::with_capacity(closed.len() - atr_start);
    for index in atr_start..closed.len() {
        let bar = closed[index];
        let true_range = if index == 0 {
            bar.high - bar.low
        } else {
            (bar.high - bar.low)
                .max((bar.high - closed[index - 1].close).abs())
                .max((bar.low - closed[index - 1].close).abs())
        };
        ranges.push(true_range);
    }
    let atr = ranges.iter().sum::<f64>() / ranges.len() as f64;
    if !finite_positive(atr) || !finite_positive(support) || !finite_positive(resistance) {
        return Err("candle summary produced invalid levels".into());
    }
    let trend_start = last_index.saturating_sub(4);
    let trend = if last_closed.close > closed[trend_start].close {
        "up"
    } else if last_closed.close < closed[trend_start].close {
        "down"
    } else {
        "flat"
    }
    .to_owned();
    Ok(FrameSummary {
        timeframe,
        closed_at_ms: (last_closed.time as u64).saturating_add(seconds).saturating_mul(1000),
        last_closed,
        closed_candles: closed[closed.len().saturating_sub(6)..].to_vec(),
        support,
        resistance,
        atr,
        trend,
    })
}

fn validate_candle(candle: &Candle) -> Result<(), String> {
    if candle.time < 0
        || !finite_positive(candle.open)
        || !finite_positive(candle.high)
        || !finite_positive(candle.low)
        || !finite_positive(candle.close)
        || !finite_nonnegative(candle.volume)
    {
        return Err("candle contains invalid time, price, or volume".into());
    }
    if candle.high < candle.low
        || candle.high < candle.open
        || candle.high < candle.close
        || candle.low > candle.open
        || candle.low > candle.close
    {
        return Err("candle OHLC geometry is invalid".into());
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    pub schema_version: u32,
    pub id: u64,
    pub market: MarketSample,
    pub frames: Vec<FrameSummary>,
}

/// Stable SHA-256 fingerprint of the serialized, ordered snapshot.
pub fn fingerprint(snapshot: &Snapshot) -> String {
    let bytes = serde_json::to_vec(snapshot).unwrap_or_else(|error| error.to_string().into_bytes());
    format!("{:x}", Sha256::digest(bytes))
}

pub fn system_prompt(config: &ObserverConfig, strategy: &StrategyConfig) -> String {
    let example = json!({
        "snapshot_id": 1,
        "action": "wait",
        "reason": "setup не подтверждён",
        "stop": null,
        "target": null,
        "used_timeframes": [strategy.timeframes[0].as_str()],
        "checks": required_rules(&strategy.id).iter().map(|rule| json!({
            "rule": rule,
            "met": false,
            "evidence": "пример; проверь фактические данные",
        })).collect::<Vec<_>>(),
    });
    format!(
        "Read-only XAUUSD paper analyst. Never invent data or executions. Adherence {}/100: \
         100 requires configured frames and all rules; near 0 choose freely among supplied frames. \
         Safety never changes. Without DOM use price_level_proxy, with DOM order_book_density. \
         Rule IDs: {}. Strategy {}: {}\n\
         Return short JSON only, без markdown. Copy actual snapshot id, not example id. \
         Uncertainty means wait, null stop/target. Long stop below entry/target above; short reversed. \
         At 100 return every rule with honest evidence even for wait. \
         OHLC arrays=[open,high,low,close]; S/R=prior support/resistance. Example: {}",
        config.strictness,
        required_rules(&strategy.id).join(", "),
        strategy.id,
        strategy.prompt,
        example
    )
}

pub fn request_payload(snapshot: &Snapshot, strategy: &StrategyConfig) -> Value {
    let frames: Vec<Value> = snapshot
        .frames
        .iter()
        .map(|frame| {
            let context: Vec<Candle> = frame
                .closed_candles
                .iter()
                .rev()
                .take(3)
                .copied()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            json!({
                "timeframe": frame.timeframe,
                "closed_at_ms": frame.closed_at_ms,
                "last_closed": frame.last_closed,
                "context_candles": context,
                "support": frame.support,
                "resistance": frame.resistance,
                "atr": frame.atr,
                "trend": frame.trend,
            })
        })
        .collect();
    let mut payload = json!({
        "snapshot": {
            "schema_version": snapshot.schema_version,
            "id": snapshot.id,
            "market": snapshot.market,
            "frames": frames,
        },
        "strategy": { "id": strategy.id, "timeframes": strategy.timeframes }
    });
    if let Some(market) = payload.pointer_mut("/snapshot/market").and_then(Value::as_object_mut) {
        if let Some(ticks) = market.get_mut("ticks").and_then(Value::as_array_mut) {
            let keep_from = ticks.len().saturating_sub(6);
            ticks.drain(..keep_from);
        }
        if let Some(book) = market.get_mut("book").and_then(Value::as_object_mut) {
            for side in ["bids", "asks"] {
                if let Some(levels) = book.get_mut(side).and_then(Value::as_array_mut) {
                    levels.truncate(5);
                }
            }
        }
    }
    payload
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelDecision {
    pub snapshot_id: u64,
    pub action: String,
    pub reason: String,
    pub stop: Option<f64>,
    pub target: Option<f64>,
    pub used_timeframes: Vec<Timeframe>,
    pub checks: Vec<RuleCheck>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RuleCheck {
    pub rule: String,
    pub met: bool,
    pub evidence: String,
}

pub fn parse_decision(response: &str) -> Result<ModelDecision, String> {
    if response.len() > 8_192 {
        return Err("decision response exceeds 8192 bytes".into());
    }
    let decision: ModelDecision =
        serde_json::from_str(response).map_err(|error| format!("invalid decision JSON: {error}"))?;
    if decision.reason.trim().is_empty() || decision.reason.chars().count() > 2_000 {
        return Err("decision reason must contain 1..=2000 characters".into());
    }
    if decision.used_timeframes.len() > Timeframe::ALL.len() {
        return Err("too many timeframes in decision".into());
    }
    if decision.checks.len() > 12
        || decision.checks.iter().any(|check| {
            check.rule.trim().is_empty() || check.rule.chars().count() > 64 || check.evidence.chars().count() > 500
        })
    {
        return Err("decision checks exceed their size limits".into());
    }
    Ok(decision)
}

pub fn validate_decision(
    snapshot: &Snapshot,
    config: &ObserverConfig,
    strategy: &StrategyConfig,
    decision: &ModelDecision,
    fresh: &MarketSample,
    now_ms: u64,
) -> Result<(), String> {
    config.validate()?;
    if !REQUIRED_STRATEGIES.contains(&strategy.id.as_str())
        || config.strategy(&strategy.id) != Some(strategy)
        || !strategy.enabled
    {
        return Err("strategy is missing, disabled, or not part of this configuration".into());
    }
    fresh.validate(now_ms)?;
    if decision.reason.trim().is_empty() || decision.reason.chars().count() > 2_000 {
        return Err("decision reason must contain 1..=2000 characters".into());
    }
    if decision.checks.len() > 12
        || decision.checks.iter().any(|check| {
            check.rule.trim().is_empty() || check.rule.chars().count() > 64 || check.evidence.chars().count() > 500
        })
    {
        return Err("decision checks exceed their size limits".into());
    }
    if decision.snapshot_id != snapshot.id {
        return Err("decision snapshot_id does not match the request".into());
    }
    if snapshot.schema_version != 1 || snapshot.id == 0 || snapshot.frames.is_empty() {
        return Err("snapshot schema, id, or frame summaries are invalid".into());
    }
    if snapshot.market.symbol != fresh.symbol {
        return Err("fresh market symbol differs from the snapshot".into());
    }
    snapshot.market.validate(snapshot.market.observed_at_ms)?;
    if snapshot.market.observed_at_ms > now_ms
        || fresh.observed_at_ms > now_ms
        || now_ms.saturating_sub(snapshot.market.observed_at_ms) > MAX_SAMPLE_AGE_MS
        || now_ms.saturating_sub(fresh.observed_at_ms) > MAX_SAMPLE_AGE_MS
    {
        return Err("snapshot or current market sample is stale".into());
    }
    let fresh_spread = fresh.quote.ask - fresh.quote.bid;
    if !finite_nonnegative(fresh_spread) || fresh_spread > config.max_spread {
        return Err("current spread exceeds the configured maximum".into());
    }
    if !matches!(decision.action.as_str(), "wait" | "long" | "short") {
        return Err("action must be wait, long, or short".into());
    }
    if decision.used_timeframes.is_empty() {
        return Err("decision must name at least one used timeframe".into());
    }
    let mut seen_timeframes = Vec::new();
    for timeframe in &decision.used_timeframes {
        if seen_timeframes.contains(timeframe) {
            return Err("decision contains duplicate timeframes".into());
        }
        seen_timeframes.push(*timeframe);
        if config.strictness == 100 && !strategy.timeframes.contains(timeframe) {
            return Err("strict decision used an unconfigured timeframe".into());
        }
        if !snapshot.frames.iter().any(|frame| frame.timeframe == *timeframe) {
            return Err("decision names a timeframe absent from its snapshot".into());
        }
    }
    if config.strictness == 100 {
        validate_strict_checks(snapshot, strategy, decision, decision.action != "wait")?;
    }
    if decision.action == "wait" {
        if decision.stop.is_some() || decision.target.is_some() {
            return Err("wait decisions must not include stop or target prices".into());
        }
        return Ok(());
    }
    let stop = decision.stop.ok_or("trade decision requires a stop")?;
    let target = decision.target.ok_or("trade decision requires a target")?;
    if !finite_positive(stop) || !finite_positive(target) {
        return Err("stop and target must be finite positive prices".into());
    }
    let entry = if decision.action == "long" {
        fresh.quote.ask
    } else {
        fresh.quote.bid
    };
    let stop_distance = if decision.action == "long" {
        entry - stop
    } else {
        stop - entry
    };
    let target_distance = if decision.action == "long" {
        target - entry
    } else {
        entry - target
    };
    let frame = decision
        .used_timeframes
        .iter()
        .filter_map(|timeframe| snapshot.frames.iter().find(|frame| frame.timeframe == *timeframe))
        .max_by(|left, right| left.atr.total_cmp(&right.atr))
        .ok_or("decision has no usable frame summary")?;
    if stop_distance < 2.0 * fresh_spread || stop_distance > 3.0 * frame.atr {
        return Err("stop must be at least two spreads away and within three ATR".into());
    }
    if target_distance <= fresh_spread {
        return Err("target must be beyond the spread in the profit direction".into());
    }
    let snapshot_mid = (snapshot.market.quote.bid + snapshot.market.quote.ask) / 2.0;
    let current_mid = (fresh.quote.bid + fresh.quote.ask) / 2.0;
    if (current_mid - snapshot_mid).abs() > 0.25 * frame.atr {
        return Err("market moved more than 0.25 ATR since the decision snapshot".into());
    }
    if config.strictness == 100 {
        validate_strict_rules(snapshot, strategy, decision)?;
    }
    Ok(())
}

fn required_rules(strategy_id: &str) -> &'static [&'static str] {
    match strategy_id {
        "density_bounce" => &["level", "rejection"],
        "structural" => &["raid", "reclaim", "shift"],
        "breakout" => &["level", "close", "acceptance"],
        "liquidity_sweep" => &["swing", "raid", "reclaim"],
        _ => &[],
    }
}

fn validate_strict_rules(
    snapshot: &Snapshot,
    strategy: &StrategyConfig,
    decision: &ModelDecision,
) -> Result<(), String> {
    validate_strict_checks(snapshot, strategy, decision, true)?;
    let frame = decision
        .used_timeframes
        .iter()
        .filter_map(|timeframe| snapshot.frames.iter().find(|frame| frame.timeframe == *timeframe))
        .max_by(|left, right| left.atr.total_cmp(&right.atr))
        .ok_or("strict decision has no frame summary")?;
    let candle = frame.last_closed;
    let prior = &frame.closed_candles[..frame.closed_candles.len().saturating_sub(1)];
    let prior_close = prior.last().map(|bar| bar.close).unwrap_or(candle.close);
    let recent_high = prior.iter().map(|bar| bar.high).fold(f64::NEG_INFINITY, f64::max);
    let recent_low = prior.iter().map(|bar| bar.low).fold(f64::INFINITY, f64::min);
    let long = decision.action == "long";
    let mechanical = match strategy.id.as_str() {
        "density_bounce" if long => density_rejection(snapshot, frame, true),
        "density_bounce" => density_rejection(snapshot, frame, false),
        "structural" | "liquidity_sweep" if long => {
            candle.low < frame.support
                && candle.close > frame.support
                && (strategy.id != "structural" || candle.close > prior_close)
        }
        "structural" | "liquidity_sweep" => {
            candle.high > frame.resistance
                && candle.close < frame.resistance
                && (strategy.id != "structural" || candle.close < prior_close)
        }
        "breakout" if long => candle.close > frame.resistance + frame.atr * 0.1 && candle.close > candle.open,
        "breakout" => candle.close < frame.support - frame.atr * 0.1 && candle.close < candle.open,
        _ => false,
    };
    if !mechanical {
        return Err("closed candle data does not confirm the strict strategy setup".into());
    }
    if strategy.id == "breakout" {
        let acceptance = if long {
            candle.close > frame.resistance + frame.atr * 0.1
        } else {
            candle.close < frame.support - frame.atr * 0.1
        };
        if !acceptance {
            return Err("breakout lacks a closed-candle acceptance beyond the level".into());
        }
    }
    if strategy.id == "liquidity_sweep" && !(recent_high.is_finite() && recent_low.is_finite()) {
        return Err("sweep context is unavailable".into());
    }
    Ok(())
}

fn validate_strict_checks(
    snapshot: &Snapshot,
    strategy: &StrategyConfig,
    decision: &ModelDecision,
    require_confirmed: bool,
) -> Result<(), String> {
    let required = required_rules(&strategy.id);
    for rule in required {
        let matching: Vec<_> = decision.checks.iter().filter(|check| check.rule == *rule).collect();
        if matching.len() != 1 || (require_confirmed && !matching[0].met) || matching[0].evidence.trim().is_empty() {
            return Err(format!(
                "strict strategy requires one {} check with evidence",
                if require_confirmed { "confirmed" } else { "reported" }
            ));
        }
    }
    if decision
        .checks
        .iter()
        .any(|check| !required.contains(&check.rule.as_str()))
    {
        return Err("strict decision contains an unknown strategy rule".into());
    }
    if strategy.id == "density_bounce" {
        let level = decision.checks.iter().find(|check| check.rule == "level").unwrap();
        let evidence_kind = if snapshot.market.book.is_some() {
            "order_book_density"
        } else {
            "price_level_proxy"
        };
        if !level.evidence.contains(evidence_kind) {
            return Err(format!("density level evidence must name {evidence_kind}"));
        }
    }
    Ok(())
}

fn density_rejection(snapshot: &Snapshot, frame: &FrameSummary, long: bool) -> bool {
    let Some(book) = &snapshot.market.book else {
        return if long {
            frame.last_closed.low <= frame.support + frame.atr * 0.1 && frame.last_closed.close > frame.support
        } else {
            frame.last_closed.high >= frame.resistance - frame.atr * 0.1 && frame.last_closed.close < frame.resistance
        };
    };
    let levels = if long { &book.bids } else { &book.asks };
    let Some(level) = levels
        .iter()
        .max_by(|left, right| left.quantity.total_cmp(&right.quantity))
    else {
        return false;
    };
    if long {
        frame.last_closed.low <= level.price + frame.atr * 0.1 && frame.last_closed.close > level.price
    } else {
        frame.last_closed.high >= level.price - frame.atr * 0.1 && frame.last_closed.close < level.price
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PaperPosition {
    pub id: u64,
    pub strategy_id: String,
    pub symbol: String,
    pub side: String,
    pub entry: f64,
    pub stop: f64,
    pub target: f64,
    pub quantity: f64,
    pub opened_at_ms: u64,
    pub initial_risk: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ClosedOutcome {
    pub position_id: u64,
    pub strategy_id: String,
    pub closed_at_ms: u64,
    pub exit: f64,
    pub pnl: f64,
    pub net_r: f64,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SimState {
    pub equity: f64,
    pub position: Option<PaperPosition>,
    pub next_id: u64,
}

impl SimState {
    pub fn new(initial_equity: f64) -> Result<Self, String> {
        if !finite_positive(initial_equity) {
            return Err("initial equity must be finite and positive".into());
        }
        Ok(Self {
            equity: initial_equity,
            position: None,
            next_id: 1,
        })
    }

    /// Validate state loaded from the persistent simulation journal before use.
    pub fn validate(&self) -> Result<(), String> {
        if !self.equity.is_finite() {
            return Err("simulator equity must be finite".into());
        }
        if self.next_id == 0 {
            return Err("next paper position id must be positive".into());
        }
        let Some(position) = &self.position else {
            return Ok(());
        };
        if position.id == 0 || position.id >= self.next_id {
            return Err("open position id must be below next_id".into());
        }
        if !REQUIRED_STRATEGIES.contains(&position.strategy_id.as_str()) {
            return Err("open position has an unknown strategy".into());
        }
        if position.symbol.trim().is_empty() {
            return Err("open position symbol must not be empty".into());
        }
        if !matches!(position.side.as_str(), "long" | "short") {
            return Err("open position side must be long or short".into());
        }
        if !finite_positive(position.entry)
            || !finite_positive(position.stop)
            || !finite_positive(position.target)
            || !finite_positive(position.quantity)
            || !finite_positive(position.initial_risk)
            || position.opened_at_ms == 0
        {
            return Err("open position contains invalid prices, quantity, risk, or time".into());
        }
        let valid_sides = if position.side == "long" {
            position.stop < position.entry && position.target > position.entry
        } else {
            position.stop > position.entry && position.target < position.entry
        };
        if !valid_sides {
            return Err("open position stop and target are on the wrong side of entry".into());
        }
        let notional = position.quantity * position.entry;
        let price_risk = position.quantity * (position.entry - position.stop).abs();
        if !finite_positive(notional)
            || notional > self.equity.max(0.0) * 10.0
            || !finite_positive(price_risk)
            || price_risk > position.initial_risk
            || position.initial_risk > self.equity.max(0.0) * 0.01 + f64::EPSILON * self.equity.abs()
        {
            return Err("open position exceeds notional or 1% risk limits".into());
        }
        Ok(())
    }

    pub fn apply(
        &mut self,
        snapshot: &Snapshot,
        config: &ObserverConfig,
        strategy: &StrategyConfig,
        decision: &ModelDecision,
        fresh: &MarketSample,
        now_ms: u64,
    ) -> Result<Option<PaperPosition>, String> {
        validate_decision(snapshot, config, strategy, decision, fresh, now_ms)?;
        self.validate()?;
        if decision.action == "wait" {
            return Ok(None);
        }
        if self.position.is_some() {
            return Err("paper simulator already has an open position".into());
        }
        if !finite_positive(self.equity) {
            return Err("simulator equity must be finite and positive".into());
        }
        let risk_budget = self.equity * config.risk_pct / 100.0;
        let entry = if decision.action == "long" {
            fresh.quote.ask + config.slippage
        } else {
            fresh.quote.bid - config.slippage
        };
        let stop = decision.stop.expect("validated trade decision has a stop");
        let target = decision.target.expect("validated trade decision has a target");
        let distance = (entry - stop).abs();
        let per_unit_risk = distance + 2.0 * config.slippage + 2.0 * config.commission_per_oz;
        if !finite_positive(per_unit_risk) {
            return Err("position risk per unit must be finite and positive".into());
        }
        let quantity = (risk_budget / per_unit_risk).min(self.equity * 10.0 / entry);
        if !finite_positive(quantity) {
            return Err("calculated paper position size is invalid".into());
        }
        let position = PaperPosition {
            id: self.next_id,
            strategy_id: strategy.id.clone(),
            symbol: fresh.symbol.clone(),
            side: decision.action.clone(),
            entry,
            stop,
            target,
            quantity,
            opened_at_ms: now_ms,
            initial_risk: quantity * per_unit_risk,
        };
        self.next_id = self.next_id.checked_add(1).ok_or("paper position id exhausted")?;
        self.position = Some(position.clone());
        Ok(Some(position))
    }

    /// Mark against the current quote. If stop and target are both crossed in
    /// one observation, the stop is conservatively assumed to fill first.
    pub fn mark(&mut self, sample: &MarketSample, config: &ObserverConfig) -> Option<ClosedOutcome> {
        if config.validate().is_err() || sample.validate(sample.observed_at_ms).is_err() {
            return None;
        }
        let position = self.position.as_ref()?;
        if position.symbol != sample.symbol
            || sample.quote.time_ms < position.opened_at_ms
            || sample.observed_at_ms < position.opened_at_ms
        {
            return None;
        }
        let long = position.side == "long";
        let quote_exit = if long { sample.quote.bid } else { sample.quote.ask };
        let stop_hit = if long {
            quote_exit <= position.stop
        } else {
            quote_exit >= position.stop
        };
        let target_hit = if long {
            quote_exit >= position.target
        } else {
            quote_exit <= position.target
        };
        if !stop_hit && !target_hit {
            return None;
        }
        self.exit_at(sample, config, if stop_hit { "stop" } else { "target" })
    }

    /// Close an open paper position at the supplied fresh quote. If there is
    /// no valid quote or position, nothing is closed and no price is invented.
    pub fn exit_at(&mut self, sample: &MarketSample, config: &ObserverConfig, reason: &str) -> Option<ClosedOutcome> {
        if self.validate().is_err()
            || config.validate().is_err()
            || sample.validate(sample.observed_at_ms).is_err()
            || reason.trim().is_empty()
            || reason.chars().count() > 120
        {
            return None;
        }
        let position = self.position.as_ref()?;
        if position.symbol != sample.symbol
            || sample.quote.time_ms < position.opened_at_ms
            || sample.observed_at_ms < position.opened_at_ms
        {
            return None;
        }
        let long = position.side == "long";
        let quote_exit = if long { sample.quote.bid } else { sample.quote.ask };
        let exit = if long {
            quote_exit - config.slippage
        } else {
            quote_exit + config.slippage
        };
        let direction = if long { 1.0 } else { -1.0 };
        let pnl = (exit - position.entry) * position.quantity * direction
            - 2.0 * config.commission_per_oz * position.quantity;
        let net_r = pnl / position.initial_risk;
        if !pnl.is_finite() || !net_r.is_finite() || !exit.is_finite() {
            return None;
        }
        let outcome = ClosedOutcome {
            position_id: position.id,
            strategy_id: position.strategy_id.clone(),
            closed_at_ms: sample.quote.time_ms,
            exit,
            pnl,
            net_r,
            reason: reason.into(),
        };
        self.equity += pnl;
        self.position = None;
        Some(outcome)
    }
}

pub struct Journal {
    path: PathBuf,
}

impl Journal {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|e| format!("create journal directory: {e}"))?;
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&path)
            .map_err(|e| format!("open journal: {e}"))?;
        set_private_permissions(&file)?;
        if file.metadata().map_err(|e| format!("stat journal: {e}"))?.len() > MAX_JOURNAL_BYTES {
            return Err("journal exceeds the 32 MiB limit".into());
        }
        Ok(Self { path })
    }

    pub fn append(&mut self, value: &Value) -> Result<(), String> {
        let mut line = serde_json::to_vec(value).map_err(|e| format!("serialize journal record: {e}"))?;
        line.push(b'\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&self.path)
            .map_err(|e| format!("open journal for append: {e}"))?;
        set_private_permissions(&file)?;
        let size = file.metadata().map_err(|e| format!("stat journal: {e}"))?.len();
        if size.saturating_add(line.len() as u64) > MAX_JOURNAL_BYTES {
            return Err("journal append would exceed the 32 MiB limit".into());
        }
        if let Err(error) = file.write_all(&line).and_then(|()| file.sync_data()) {
            file.set_len(size)
                .and_then(|()| file.sync_data())
                .map_err(|rollback| format!("journal write failed ({error}); rollback failed ({rollback})"))?;
            return Err(format!("journal record was rolled back: {error}"));
        }
        Ok(())
    }

    pub fn replay(&self) -> Result<Vec<Value>, String> {
        let file = File::open(&self.path).map_err(|e| format!("open journal for replay: {e}"))?;
        if file.metadata().map_err(|e| format!("stat journal: {e}"))?.len() > MAX_JOURNAL_BYTES {
            return Err("journal exceeds the 32 MiB limit".into());
        }
        let mut entries = Vec::new();
        for (index, line) in BufReader::new(file).lines().enumerate() {
            let line = line.map_err(|e| format!("read journal line {}: {e}", index + 1))?;
            if line.trim().is_empty() {
                return Err(format!("journal line {} is empty", index + 1));
            }
            let value =
                serde_json::from_str(&line).map_err(|e| format!("invalid journal JSON at line {}: {e}", index + 1))?;
            entries.push(value);
        }
        Ok(entries)
    }

    pub fn export(&self) -> Result<String, String> {
        let file = File::open(&self.path).map_err(|e| format!("open journal for export: {e}"))?;
        if file.metadata().map_err(|e| format!("stat journal: {e}"))?.len() > MAX_JOURNAL_BYTES {
            return Err("journal exceeds the 32 MiB limit".into());
        }
        let mut contents = String::new();
        BufReader::new(file)
            .take(MAX_JOURNAL_BYTES + 1)
            .read_to_string(&mut contents)
            .map_err(|e| format!("read journal export: {e}"))?;
        if contents.len() as u64 > MAX_JOURNAL_BYTES {
            return Err("journal exceeds the 32 MiB limit".into());
        }
        Ok(contents)
    }
}

#[cfg(unix)]
fn set_private_permissions(file: &File) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = file
        .metadata()
        .map_err(|e| format!("stat journal permissions: {e}"))?
        .permissions();
    if permissions.mode() & 0o777 != 0o600 {
        permissions.set_mode(0o600);
        file.set_permissions(permissions)
            .map_err(|e| format!("set private journal permissions: {e}"))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_private_permissions(_file: &File) -> Result<(), String> {
    Ok(())
}

fn finite_positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn finite_nonnegative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live_market::{Quote, Tick};
    use crate::market_depth::{BookLevel, OrderBookSnapshot};

    fn sample(time_ms: u64) -> MarketSample {
        MarketSample {
            symbol: "XAUUSD".into(),
            observed_at_ms: time_ms,
            quote: Quote {
                time_ms,
                bid: 100.0,
                ask: 100.2,
            },
            ticks: vec![Tick {
                time_ms,
                bid: 100.0,
                ask: 100.2,
                last: None,
                volume: None,
            }],
            book: None,
            book_note: "Стакан недоступен".into(),
            volume_kind: "mt5_ticks_not_exchange_tape".into(),
        }
    }

    fn candles(count: usize, base: i64) -> Vec<Candle> {
        (0..count)
            .map(|index| {
                let open = 100.0 + index as f64 * 0.01;
                Candle {
                    time: base + index as i64 * 60,
                    open,
                    high: open + 0.5,
                    low: open - 0.5,
                    close: open + 0.1,
                    volume: 1.0,
                }
            })
            .collect()
    }

    fn snapshot(now_ms: u64) -> Snapshot {
        let market = sample(now_ms);
        let frame = summarize(Timeframe::M1, &candles(24, 1_700_000_000), now_ms).unwrap();
        Snapshot {
            schema_version: 1,
            id: 7,
            market,
            frames: vec![frame],
        }
    }

    fn wait_decision() -> ModelDecision {
        ModelDecision {
            snapshot_id: 7,
            action: "wait".into(),
            reason: "Сетапа пока нет".into(),
            stop: None,
            target: None,
            used_timeframes: vec![Timeframe::M1],
            checks: vec![
                RuleCheck {
                    rule: "level".into(),
                    met: false,
                    evidence: "price_level_proxy: уровень не подтверждён".into(),
                },
                RuleCheck {
                    rule: "rejection".into(),
                    met: false,
                    evidence: "закрытый бар не показал отбой".into(),
                },
            ],
        }
    }

    #[test]
    fn config_bounds_and_timeframe_uniqueness_are_enforced() {
        let config = ObserverConfig::default();
        assert!(config.validate().is_ok());
        let mut long_prompt = config.clone();
        long_prompt.strategies[0].prompt = "я".repeat(MAX_PROMPT_CHARS);
        assert!(long_prompt.validate().is_ok());
        long_prompt.strategies[0].prompt.push('я');
        assert!(long_prompt.validate().is_err());
        let mut invalid = config.clone();
        invalid.strictness = 101;
        assert!(invalid.validate().is_err());
        let mut invalid = config.clone();
        invalid.risk_pct = 1.01;
        assert!(invalid.validate().is_err());
        let mut invalid = config;
        invalid.strategies[0].timeframes.push(Timeframe::M1);
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn summaries_use_closed_bars_and_prior_only_levels() {
        let now = 1_700_000_000_000 + 24 * 60_000 + 30_000;
        let bars = candles(26, 1_700_000_000);
        let result = summarize(Timeframe::M1, &bars, now).unwrap();
        assert_eq!(result.closed_candles.len(), 6);
        assert_eq!(result.last_closed.time, 1_700_000_000 + 23 * 60);
        assert!(result.support < result.last_closed.low);
        assert!(summarize(Timeframe::M1, &candles(20, 1_700_000_000), now).is_err());
        assert!(summarize(Timeframe::M1, &candles(21, 1_700_000_000), now).is_ok());
        let forming_only = summarize(Timeframe::M1, &bars[..19], now);
        assert!(forming_only.is_err());
        let mut unordered = bars;
        unordered.swap(0, 1);
        assert!(summarize(Timeframe::M1, &unordered, now).is_err());
    }

    #[test]
    fn stale_wrong_id_and_malformed_decisions_are_rejected() {
        let now = 1_700_001_500_000;
        let snapshot = snapshot(now);
        let config = ObserverConfig::default();
        let strategy = config.strategy("density_bounce").unwrap();
        let fresh = sample(now);
        let mut decision = wait_decision();
        assert!(validate_decision(&snapshot, &config, strategy, &decision, &fresh, now).is_ok());
        let delayed = sample(now + 20_000);
        assert!(validate_decision(&snapshot, &config, strategy, &wait_decision(), &delayed, now + 20_000).is_ok());
        decision.snapshot_id += 1;
        assert!(validate_decision(&snapshot, &config, strategy, &decision, &fresh, now).is_err());
        assert!(validate_decision(&snapshot, &config, strategy, &wait_decision(), &fresh, now + 31_000).is_err());
        assert!(parse_decision("```json {} ```").is_err());
        assert!(parse_decision(r#"{"snapshot_id":7,"action":"wait","reason":"ok","stop":null,"target":null,"used_timeframes":["1m"],"checks":[],"extra":1}"#).is_err());
        assert!(parse_decision(r#"{"snapshot_id":7,"snapshot_id":7,"action":"wait","reason":"ok","stop":null,"target":null,"used_timeframes":["1m"],"checks":[]}"#).is_err());
    }

    #[test]
    fn prompt_and_payload_are_canonical_and_bounded() {
        let now = 1_700_001_500_000;
        let mut snapshot = snapshot(now);
        snapshot.market.ticks = (0..10)
            .map(|offset| Tick {
                time_ms: now + offset,
                bid: 100.0,
                ask: 100.2,
                last: None,
                volume: None,
            })
            .collect();
        snapshot.market.book = Some(OrderBookSnapshot {
            symbol: "XAUUSD".into(),
            timestamp: now,
            bids: (0..7)
                .map(|offset| BookLevel {
                    price: 99.0 - offset as f64,
                    quantity: 1.0,
                })
                .collect(),
            asks: (0..7)
                .map(|offset| BookLevel {
                    price: 101.0 + offset as f64,
                    quantity: 1.0,
                })
                .collect(),
        });
        let config = ObserverConfig::default();
        let strategy = config.strategy("density_bounce").unwrap();
        let prompt = system_prompt(&config, strategy);
        assert!(prompt.contains("level, rejection"));
        assert!(prompt.contains("price_level_proxy"));
        assert!(prompt.contains("без markdown"));
        let payload = request_payload(&snapshot, strategy);
        assert!(payload.pointer("/strategy/prompt").is_none());
        assert_eq!(
            payload
                .pointer("/snapshot/frames/0/context_candles")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert!(payload.pointer("/snapshot/frames/0/closed_candles").is_none());
        assert_eq!(
            payload
                .pointer("/snapshot/market/ticks")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            6
        );
        assert_eq!(
            payload
                .pointer("/snapshot/market/book/bids")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            5
        );
        assert_eq!(fingerprint(&snapshot), fingerprint(&snapshot));
    }

    #[test]
    fn soft_strictness_can_consider_available_unconfigured_frames() {
        let now = 1_700_001_500_000;
        let mut snapshot = snapshot(now);
        let mut higher = snapshot.frames[0].clone();
        higher.timeframe = Timeframe::H1;
        snapshot.frames.push(higher);
        let mut config = ObserverConfig::default();
        config.strategies[0].timeframes = vec![Timeframe::M1];
        let mut decision = wait_decision();
        decision.used_timeframes = vec![Timeframe::H1];
        let strategy = config.strategy("density_bounce").unwrap();
        assert!(validate_decision(&snapshot, &config, strategy, &decision, &sample(now), now).is_err());
        config.strictness = 15;
        let strategy = config.strategy("density_bounce").unwrap();
        assert!(validate_decision(&snapshot, &config, strategy, &decision, &sample(now), now).is_ok());
    }

    #[test]
    fn strict_strategy_requires_all_rules_and_price_confirmation() {
        let now = 1_700_001_500_000;
        let mut snapshot = snapshot(now);
        let frame = &mut snapshot.frames[0];
        let last = frame.closed_candles.last_mut().unwrap();
        last.low = frame.support - 0.1;
        last.close = frame.support + 0.1;
        last.open = frame.support - 0.05;
        last.high = frame.support + 0.2;
        frame.last_closed = *last;
        let config = ObserverConfig::default();
        let strategy = config.strategy("density_bounce").unwrap();
        let decision = ModelDecision {
            snapshot_id: snapshot.id,
            action: "long".into(),
            reason: "Отбой от ценового прокси".into(),
            stop: Some(99.0),
            target: Some(102.0),
            used_timeframes: vec![Timeframe::M1],
            checks: vec![
                RuleCheck {
                    rule: "level".into(),
                    met: true,
                    evidence: "price_level_proxy: предыдущий минимум".into(),
                },
                RuleCheck {
                    rule: "rejection".into(),
                    met: true,
                    evidence: "Закрытие вернулось выше".into(),
                },
            ],
        };
        assert!(validate_decision(&snapshot, &config, strategy, &decision, &sample(now), now).is_ok());
        let mut wrong_stop = decision.clone();
        wrong_stop.stop = Some(101.0);
        assert!(validate_decision(&snapshot, &config, strategy, &wrong_stop, &sample(now), now).is_err());
        let mut wrong_source = decision.clone();
        wrong_source.checks[0].evidence = "неподтверждённая плотность".into();
        assert!(validate_decision(&snapshot, &config, strategy, &wrong_source, &sample(now), now).is_err());
        let mut with_book = snapshot.clone();
        with_book.market.book = Some(OrderBookSnapshot {
            symbol: "XAUUSD".into(),
            timestamp: now,
            bids: vec![BookLevel {
                price: with_book.frames[0].support,
                quantity: 10.0,
            }],
            asks: vec![BookLevel {
                price: 100.5,
                quantity: 2.0,
            }],
        });
        let mut book_decision = decision.clone();
        book_decision.checks[0].evidence = "order_book_density: крупный bid".into();
        assert!(validate_decision(&with_book, &config, strategy, &book_decision, &sample(now), now).is_ok());
        let mut missing = decision;
        missing.checks.pop();
        assert!(validate_decision(&snapshot, &config, strategy, &missing, &sample(now), now).is_err());
    }

    #[test]
    fn paper_simulation_books_costs_and_never_stacks_positions() {
        let now = 1_700_001_500_000;
        let snapshot = snapshot(now);
        let config = ObserverConfig {
            strictness: 0,
            commission_per_oz: 0.01,
            slippage: 0.02,
            ..Default::default()
        };
        let strategy = config.strategy("density_bounce").unwrap().clone();
        let decision = ModelDecision {
            snapshot_id: snapshot.id,
            action: "long".into(),
            reason: "test entry".into(),
            stop: Some(99.0),
            target: Some(102.0),
            used_timeframes: vec![Timeframe::M1],
            checks: vec![],
        };
        let mut sim = SimState::new(config.initial_equity).unwrap();
        let position = sim
            .apply(&snapshot, &config, &strategy, &decision, &sample(now), now)
            .unwrap()
            .unwrap();
        assert!(sim.validate().is_ok());
        assert!(position.quantity * position.entry <= sim.equity * 10.0);
        let mut corrupted = sim.clone();
        corrupted.position.as_mut().unwrap().quantity = f64::NAN;
        assert!(corrupted.validate().is_err());
        assert!(sim
            .apply(&snapshot, &config, &strategy, &decision, &sample(now), now)
            .is_err());
        let mut exit_sample = sample(now + 1_000);
        exit_sample.quote.bid = position.target;
        exit_sample.quote.ask = position.target + 0.2;
        let closed = sim.mark(&exit_sample, &config).unwrap();
        assert_eq!(closed.reason, "target");
        assert!(closed.pnl < (position.target - position.entry) * position.quantity);
        assert!(sim.position.is_none());
        assert!(sim.validate().is_ok());
    }

    #[test]
    fn explicit_exit_needs_a_fresh_real_quote() {
        let now = 1_700_001_500_000;
        let snapshot = snapshot(now);
        let config = ObserverConfig {
            strictness: 0,
            ..Default::default()
        };
        let strategy = config.strategy("density_bounce").unwrap().clone();
        let decision = ModelDecision {
            snapshot_id: snapshot.id,
            action: "long".into(),
            reason: "test entry".into(),
            stop: Some(99.0),
            target: Some(102.0),
            used_timeframes: vec![Timeframe::M1],
            checks: vec![],
        };
        let mut sim = SimState::new(config.initial_equity).unwrap();
        sim.apply(&snapshot, &config, &strategy, &decision, &sample(now), now)
            .unwrap();
        let mut stale = sample(now + 1_000);
        stale.quote.time_ms = now.saturating_sub(10_001);
        assert!(sim.exit_at(&stale, &config, "manual").is_none());
        assert!(sim.position.is_some());
        let fresh = sample(now + 2_000);
        let exit = sim.exit_at(&fresh, &config, "manual").unwrap();
        assert_eq!(exit.reason, "manual");
        assert!(sim.position.is_none());
    }

    #[test]
    fn journal_appends_replays_and_exports_jsonl() {
        let directory = std::env::temp_dir().join(format!("aegis-observer-{}-{}", std::process::id(), 17));
        let _ = fs::remove_dir_all(&directory);
        let path = directory.join("observer.jsonl");
        let mut journal = Journal::new(&path).unwrap();
        journal.append(&json!({"event":"wait","snapshot_id":7})).unwrap();
        journal.append(&json!({"event":"entry","id":1})).unwrap();
        assert_eq!(journal.replay().unwrap().len(), 2);
        assert_eq!(journal.export().unwrap().lines().count(), 2);
        drop(journal);
        fs::remove_dir_all(directory).unwrap();
    }
}
