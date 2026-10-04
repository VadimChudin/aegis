//! Autonomous AI decision support backed by a paper-only execution engine.
//!
//! No function in this module places or forwards real orders. Quantities are
//! units of the asset, not broker lots.

use std::{
    collections::VecDeque,
    net::IpAddr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use reqwest::{header, Url};
use serde::{Deserialize, Serialize};

use crate::market::Candle;

const LOCAL_URL: &str = "http://127.0.0.1:11434/v1/chat/completions";
const CLOUD_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
const MAX_JOURNAL: usize = 1_000;
const MAX_CANDLES: usize = 60;
const MAX_REQUEST_BYTES: usize = 65_536;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiSettings {
    #[serde(default = "default_local_url")]
    pub local_url: String,
    #[serde(default = "default_local_model")]
    pub local_model: String,
    #[serde(default = "default_cloud_model")]
    pub cloud_model: String,
    #[serde(default = "default_strategy")]
    pub strategy: String,
    #[serde(default)]
    pub preprompt: String,
    #[serde(default = "default_equity")]
    pub initial_equity: f64,
    #[serde(default = "default_risk_pct")]
    pub max_risk_pct: f64,
    #[serde(default = "default_leverage")]
    pub max_leverage: f64,
    #[serde(default = "default_positions")]
    pub max_positions: usize,
    #[serde(default = "default_daily_loss")]
    pub daily_loss_limit: f64,
    #[serde(default = "default_interval")]
    pub interval_seconds: u64,
    #[serde(default = "default_daily_budget")]
    pub api_daily_budget_usd: f64,
    #[serde(default = "default_monthly_budget")]
    pub api_monthly_budget_usd: f64,
    #[serde(default = "default_cloud_requests")]
    pub max_cloud_requests_per_day: u32,
}

fn default_local_url() -> String {
    LOCAL_URL.into()
}
fn default_local_model() -> String {
    "qwen3:8b".into()
}
fn default_cloud_model() -> String {
    "anthropic/claude-sonnet-4.5".into()
}
fn default_strategy() -> String {
    "bounce".into()
}
fn default_equity() -> f64 {
    10_000.0
}
fn default_risk_pct() -> f64 {
    1.0
}
fn default_leverage() -> f64 {
    1.0
}
fn default_positions() -> usize {
    3
}
fn default_daily_loss() -> f64 {
    300.0
}
fn default_interval() -> u64 {
    60
}
fn default_daily_budget() -> f64 {
    1.0
}
fn default_monthly_budget() -> f64 {
    10.0
}
fn default_cloud_requests() -> u32 {
    10
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            local_url: default_local_url(),
            local_model: default_local_model(),
            cloud_model: default_cloud_model(),
            strategy: default_strategy(),
            preprompt: String::new(),
            initial_equity: default_equity(),
            max_risk_pct: default_risk_pct(),
            max_leverage: default_leverage(),
            max_positions: default_positions(),
            daily_loss_limit: default_daily_loss(),
            interval_seconds: default_interval(),
            api_daily_budget_usd: default_daily_budget(),
            api_monthly_budget_usd: default_monthly_budget(),
            max_cloud_requests_per_day: default_cloud_requests(),
        }
    }
}

impl AiSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.local_model.trim().is_empty() || self.cloud_model.trim().is_empty() {
            return Err("AI model names must not be empty".into());
        }
        if !self.initial_equity.is_finite() || self.initial_equity <= 0.0 {
            return Err("initial_equity must be finite and positive".into());
        }
        if !self.max_risk_pct.is_finite() || self.max_risk_pct <= 0.0 || self.max_risk_pct > 100.0 {
            return Err("max_risk_pct must be greater than 0 and at most 100".into());
        }
        if !self.max_leverage.is_finite() || self.max_leverage <= 0.0 || self.max_leverage > 100.0 {
            return Err("max_leverage must be finite, greater than 0, and at most 100".into());
        }
        if !(self.initial_equity * self.max_risk_pct / 100.0).is_finite()
            || !(self.initial_equity * self.max_leverage).is_finite()
        {
            return Err("initial_equity combined with risk or leverage limits overflows".into());
        }
        if self.max_positions == 0 {
            return Err("max_positions must be positive".into());
        }
        if !self.daily_loss_limit.is_finite() || self.daily_loss_limit <= 0.0 {
            return Err("daily_loss_limit must be finite and positive".into());
        }
        if self.interval_seconds == 0 {
            return Err("interval_seconds must be positive".into());
        }
        if !self.api_daily_budget_usd.is_finite()
            || self.api_daily_budget_usd < 0.0
            || !self.api_monthly_budget_usd.is_finite()
            || self.api_monthly_budget_usd < 0.0
        {
            return Err("API budgets must be finite and non-negative".into());
        }
        if self.preprompt.len() > 10_000 {
            return Err("preprompt must not exceed 10000 bytes".into());
        }
        strategy_prompt(&self.strategy)?;
        validate_local_url(&self.local_url)?;
        Ok(())
    }
}

pub fn strategy_prompt(strategy: &str) -> Result<&'static str, String> {
    match strategy {
        "bounce" => Ok("Bounce: look for a well-tested support/resistance level, a controlled touch, and evidence of rejection. Avoid chasing; identify invalidation beyond the level."),
        "structural" => Ok("Structural: assess prior-session liquidity raid, reclaim, and confirmed market-structure shift. Avoid anticipation before confirmation and define invalidation."),
        "breakout" => Ok("Breakout: require a decisive close and acceptance beyond a clearly identified range/level; distinguish a hold from a wick or false break and define invalidation."),
        "liquidity_sweep" => Ok("Liquidity sweep: identify a run beyond a meaningful swing high/low followed by reclaim or rejection. Do not treat an unconfirmed stop run as a reversal."),
        "data" => Ok("Data: reason only from supplied OHLCV and quote evidence. State uncertainty, avoid unsupported statistical claims, and prefer wait when evidence is insufficient."),
        other => Err(format!("unsupported AI strategy: {other}")),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: u64,
    pub broker: String,
    pub symbol: String,
    /// Unix timestamp in seconds.
    pub time: u64,
    pub bid: f64,
    pub ask: f64,
    pub candles: Vec<Candle>,
}

impl Snapshot {
    fn validate(&self) -> Result<(), String> {
        if self.broker.trim().is_empty() || self.symbol.trim().is_empty() {
            return Err("snapshot broker and symbol must not be empty".into());
        }
        if !self.bid.is_finite() || !self.ask.is_finite() || self.bid <= 0.0 || self.ask < self.bid {
            return Err("snapshot bid/ask must be finite positive prices with ask >= bid".into());
        }
        if self.candles.len() > MAX_CANDLES {
            return Err(format!("snapshot contains more than {MAX_CANDLES} candles"));
        }
        for candle in &self.candles {
            if ![candle.open, candle.high, candle.low, candle.close, candle.volume]
                .iter()
                .all(|x| x.is_finite())
                || candle.open <= 0.0
                || candle.high <= 0.0
                || candle.low <= 0.0
                || candle.close <= 0.0
                || candle.volume < 0.0
                || candle.high < candle.low
            {
                return Err("snapshot contains an invalid candle".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Decision {
    Wait {
        snapshot_id: u64,
        reason: String,
    },
    Open {
        snapshot_id: u64,
        side: String,
        quantity: f64,
        stop: f64,
        reason: String,
    },
    Close {
        snapshot_id: u64,
        position_id: u64,
        reason: String,
    },
    Reduce {
        snapshot_id: u64,
        position_id: u64,
        quantity: f64,
        reason: String,
    },
    Stop {
        snapshot_id: u64,
        position_id: u64,
        stop: f64,
        reason: String,
    },
    Consult {
        snapshot_id: u64,
        reason: String,
    },
}

impl Decision {
    fn snapshot_id(&self) -> u64 {
        match self {
            Self::Wait { snapshot_id, .. }
            | Self::Open { snapshot_id, .. }
            | Self::Close { snapshot_id, .. }
            | Self::Reduce { snapshot_id, .. }
            | Self::Stop { snapshot_id, .. }
            | Self::Consult { snapshot_id, .. } => *snapshot_id,
        }
    }

    fn reason(&self) -> &str {
        match self {
            Self::Wait { reason, .. }
            | Self::Open { reason, .. }
            | Self::Close { reason, .. }
            | Self::Reduce { reason, .. }
            | Self::Stop { reason, .. }
            | Self::Consult { reason, .. } => reason,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperSide {
    Long,
    Short,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaperPosition {
    pub id: u64,
    pub broker: String,
    pub symbol: String,
    pub side: PaperSide,
    /// Asset units, never MT5 lots or another broker contract multiplier.
    pub quantity: f64,
    pub entry_price: f64,
    #[serde(default)]
    pub mark_price: f64,
    pub stop: f64,
    /// Risk budget assigned at entry; stop changes may not exceed this amount.
    pub risk_cap: f64,
    pub opened_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub time: u64,
    pub snapshot_id: u64,
    pub action: String,
    pub reason: String,
    pub position_id: Option<u64>,
    pub pnl: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaperState {
    #[serde(default)]
    pub positions: Vec<PaperPosition>,
    #[serde(default = "default_equity")]
    pub equity: f64,
    #[serde(default = "default_equity")]
    initial_equity: f64,
    #[serde(default)]
    pub realized: f64,
    #[serde(default)]
    pub journal: VecDeque<DecisionRecord>,
    #[serde(default = "first_position_id")]
    next_position_id: u64,
    #[serde(default)]
    daily_realized: f64,
    #[serde(default)]
    daily_marker: u64,
    #[serde(default)]
    daily_unrealized_baseline: f64,
}

fn first_position_id() -> u64 {
    1
}

impl Default for PaperState {
    fn default() -> Self {
        Self {
            positions: Vec::new(),
            equity: default_equity(),
            initial_equity: default_equity(),
            realized: 0.0,
            journal: VecDeque::new(),
            next_position_id: 1,
            daily_realized: 0.0,
            daily_marker: 0,
            daily_unrealized_baseline: 0.0,
        }
    }
}

pub struct PaperEngine {
    state: PaperState,
}

impl Default for PaperEngine {
    fn default() -> Self {
        Self::new(&AiSettings::default())
    }
}

impl PaperEngine {
    pub fn new(settings: &AiSettings) -> Self {
        Self {
            state: PaperState {
                equity: settings.initial_equity,
                initial_equity: settings.initial_equity,
                ..PaperState::default()
            },
        }
    }

    pub fn from_state(state: PaperState, settings: &AiSettings) -> Result<Self, String> {
        settings.validate()?;
        if !state.equity.is_finite()
            || !state.realized.is_finite()
            || !state.initial_equity.is_finite()
            || state.initial_equity <= 0.0
            || !state.daily_realized.is_finite()
            || !state.daily_unrealized_baseline.is_finite()
            || state.positions.len() > settings.max_positions
        {
            return Err("invalid persisted paper state".into());
        }
        let mut ids = std::collections::HashSet::new();
        for position in &state.positions {
            if !valid_position(position) || !ids.insert(position.id) {
                return Err("invalid persisted paper position".into());
            }
        }
        let mut state = state;
        state.equity = equity_from_marks(&state);
        if !state.equity.is_finite() || state.equity < 0.0 {
            return Err("persisted paper equity is invalid".into());
        }
        let aggregate_risk = state.positions.iter().map(current_risk).sum::<f64>();
        if !aggregate_risk.is_finite() || aggregate_risk > risk_limit(&state, settings) {
            return Err("persisted positions exceed aggregate risk limit".into());
        }
        let aggregate_notional = position_notional_total(&state, None);
        let notional_limit = state.equity * settings.max_leverage;
        if !aggregate_notional.is_finite() || !notional_limit.is_finite() || aggregate_notional > notional_limit {
            return Err("persisted positions exceed maximum leverage".into());
        }
        let max_id = state.positions.iter().map(|p| p.id).max().unwrap_or(0);
        if max_id == u64::MAX {
            return Err("persisted paper position id is exhausted".into());
        }
        let state = PaperState {
            next_position_id: state.next_position_id.max(max_id + 1),
            ..state
        };
        let mut state = state;
        while state.journal.len() > MAX_JOURNAL {
            state.journal.pop_front();
        }
        Ok(Self { state })
    }

    pub fn state(&self) -> PaperState {
        self.state.clone()
    }

    /// Refresh mark-to-market equity and execute hit stops at the executable quote.
    pub fn mark(&mut self, snapshot: &Snapshot, settings: &AiSettings) -> Result<(), String> {
        settings.validate()?;
        snapshot.validate()?;
        check_freshness(snapshot.time, unix_now()?)?;
        if snapshot.time < self.state.daily_marker {
            return Err("snapshot timestamp moved backwards".into());
        }
        self.update_marks(snapshot);
        self.roll_day(snapshot.time, snapshot);
        let mut i = 0;
        while i < self.state.positions.len() {
            let p = &self.state.positions[i];
            if p.broker != snapshot.broker || p.symbol != snapshot.symbol {
                i += 1;
                continue;
            }
            let hit = match p.side {
                PaperSide::Long => snapshot.bid <= p.stop,
                PaperSide::Short => snapshot.ask >= p.stop,
            };
            if hit {
                let price = match p.side {
                    PaperSide::Long => snapshot.bid,
                    PaperSide::Short => snapshot.ask,
                };
                let position_id = p.id;
                let pnl = self.close_index(i, price, snapshot.time);
                self.push_journal(DecisionRecord {
                    time: snapshot.time,
                    snapshot_id: snapshot.id,
                    action: "stop_close".into(),
                    reason: "paper stop triggered".into(),
                    position_id: Some(position_id),
                    pnl,
                });
            } else {
                i += 1;
            }
        }
        self.refresh_equity(snapshot);
        Ok(())
    }

    pub fn apply(
        &mut self,
        decision: Decision,
        snapshot: &Snapshot,
        settings: &AiSettings,
        now: u64,
    ) -> Result<(), String> {
        settings.validate()?;
        snapshot.validate()?;
        if decision.snapshot_id() != snapshot.id {
            return Err("decision snapshot_id does not match current snapshot".into());
        }
        check_freshness(snapshot.time, now)?;
        if decision.reason().trim().is_empty() || decision.reason().len() > 2_000 {
            return Err("decision reason must contain 1 to 2000 characters".into());
        }
        let reason = decision.reason().to_owned();
        self.roll_day(now, snapshot);
        let market_daily_pnl = daily_pnl(&self.state, snapshot);
        let (action, position_id, pnl) = match decision {
            Decision::Wait { .. } => ("wait", None, 0.0),
            Decision::Consult { .. } => ("consult", None, 0.0),
            Decision::Open {
                side, quantity, stop, ..
            } => {
                if self.state.positions.len() >= settings.max_positions {
                    return Err("maximum paper positions reached".into());
                }
                if market_daily_pnl <= -settings.daily_loss_limit {
                    return Err("daily loss limit reached; new positions are blocked".into());
                }
                if !quantity.is_finite() || quantity <= 0.0 || !stop.is_finite() || stop <= 0.0 {
                    return Err("open requires positive finite quantity and stop".into());
                }
                let (side, entry) = match side.as_str() {
                    "long" => (PaperSide::Long, snapshot.ask),
                    "short" => (PaperSide::Short, snapshot.bid),
                    _ => return Err("open side must be 'long' or 'short'".into()),
                };
                let risk = match side {
                    PaperSide::Long => (entry - stop) * quantity,
                    PaperSide::Short => (stop - entry) * quantity,
                };
                if !risk.is_finite() || risk <= 0.0 {
                    return Err("open stop must be beyond entry on the loss side".into());
                }
                let current_equity = equity_at(&self.state, snapshot);
                if current_risk_total(&self.state) + risk > current_equity * settings.max_risk_pct / 100.0 + 1e-8 {
                    return Err("open exceeds aggregate paper risk limit".into());
                }
                if position_notional_total(&self.state, Some(snapshot)) + quantity * entry
                    > current_equity * settings.max_leverage + 1e-8
                {
                    return Err("open exceeds maximum paper leverage".into());
                }
                let id = self.state.next_position_id.max(1);
                self.state.next_position_id = id
                    .checked_add(1)
                    .ok_or_else(|| "paper position id exhausted".to_string())?;
                self.state.positions.push(PaperPosition {
                    id,
                    broker: snapshot.broker.clone(),
                    symbol: snapshot.symbol.clone(),
                    side,
                    quantity,
                    entry_price: entry,
                    mark_price: exit_price(side, snapshot),
                    stop,
                    risk_cap: risk,
                    opened_at: now,
                });
                ("open", Some(id), 0.0)
            }
            Decision::Close { position_id, .. } => {
                let index = self.find_position(position_id, snapshot)?;
                let price = exit_price(self.state.positions[index].side, snapshot);
                let pnl = self.close_index(index, price, now);
                ("close", Some(position_id), pnl)
            }
            Decision::Reduce {
                position_id, quantity, ..
            } => {
                let index = self.find_position(position_id, snapshot)?;
                let existing = &self.state.positions[index];
                if !quantity.is_finite() || quantity <= 0.0 || quantity >= existing.quantity {
                    return Err("reduce quantity must be positive and smaller than the position".into());
                }
                let p = self.state.positions[index].clone();
                let pnl = position_pnl(&p, exit_price(p.side, snapshot), quantity);
                self.state.positions[index].quantity -= quantity;
                self.state.positions[index].risk_cap *= self.state.positions[index].quantity / p.quantity;
                self.record_pnl(pnl, now);
                ("reduce", Some(position_id), pnl)
            }
            Decision::Stop { position_id, stop, .. } => {
                let index = self.find_position(position_id, snapshot)?;
                let p = &self.state.positions[index];
                if !stop.is_finite() || stop <= 0.0 {
                    return Err("stop must be finite and positive".into());
                }
                let valid = match p.side {
                    PaperSide::Long => stop < snapshot.bid,
                    PaperSide::Short => stop > snapshot.ask,
                };
                let old_risk = current_risk(p);
                let risk_cap = p.risk_cap;
                let risk = match p.side {
                    PaperSide::Long => (p.entry_price - stop) * p.quantity,
                    PaperSide::Short => (stop - p.entry_price) * p.quantity,
                }
                .max(0.0);
                if !valid || risk > risk_cap + 1e-8 {
                    return Err("stop must remain beyond market and within original risk".into());
                }
                let total_risk = current_risk_total(&self.state) - old_risk + risk;
                if total_risk > equity_at(&self.state, snapshot) * settings.max_risk_pct / 100.0 + 1e-8 {
                    return Err("stop update exceeds aggregate paper risk limit".into());
                }
                self.state.positions[index].stop = stop;
                ("stop", Some(position_id), 0.0)
            }
        };
        self.update_marks(snapshot);
        self.refresh_equity(snapshot);
        self.push_journal(DecisionRecord {
            time: now,
            snapshot_id: snapshot.id,
            action: action.into(),
            reason,
            position_id,
            pnl,
        });
        Ok(())
    }

    fn find_position(&self, id: u64, snapshot: &Snapshot) -> Result<usize, String> {
        self.state
            .positions
            .iter()
            .position(|p| p.id == id && p.broker == snapshot.broker && p.symbol == snapshot.symbol)
            .ok_or_else(|| "paper position not found for snapshot broker/symbol".into())
    }

    fn close_index(&mut self, index: usize, price: f64, time: u64) -> f64 {
        let p = self.state.positions.remove(index);
        let pnl = position_pnl(&p, price, p.quantity);
        self.record_pnl(pnl, time);
        pnl
    }

    fn record_pnl(&mut self, pnl: f64, _time: u64) {
        self.state.realized += pnl;
        self.state.daily_realized += pnl;
    }

    fn roll_day(&mut self, time: u64, snapshot: &Snapshot) {
        let day = time / 86_400;
        if self.state.daily_marker / 86_400 != day {
            self.state.daily_marker = day * 86_400;
            self.state.daily_realized = 0.0;
            self.state.daily_unrealized_baseline = unrealized_pnl(&self.state.positions, snapshot);
        }
    }

    /// Record a model or integration rejection without mutating paper positions.
    pub fn record_rejection(&mut self, snapshot: &Snapshot, reason: &str, now: u64) -> Result<(), String> {
        snapshot.validate()?;
        self.push_journal(DecisionRecord {
            time: now,
            snapshot_id: snapshot.id,
            action: "rejected".into(),
            reason: reason.chars().take(2_000).collect(),
            position_id: None,
            pnl: 0.0,
        });
        Ok(())
    }

    fn refresh_equity(&mut self, snapshot: &Snapshot) {
        self.state.equity = equity_at(&self.state, snapshot).max(0.0);
    }

    fn update_marks(&mut self, snapshot: &Snapshot) {
        for position in &mut self.state.positions {
            if position.broker == snapshot.broker && position.symbol == snapshot.symbol {
                position.mark_price = exit_price(position.side, snapshot);
            }
        }
    }

    fn push_journal(&mut self, entry: DecisionRecord) {
        self.state.journal.push_back(entry);
        while self.state.journal.len() > MAX_JOURNAL {
            self.state.journal.pop_front();
        }
    }
}

fn check_freshness(time: u64, now: u64) -> Result<(), String> {
    if time > now || now - time > 30 {
        Err("snapshot is stale or from the future".into())
    } else {
        Ok(())
    }
}

fn valid_position(p: &PaperPosition) -> bool {
    p.quantity.is_finite()
        && p.quantity > 0.0
        && p.entry_price.is_finite()
        && p.entry_price > 0.0
        && p.mark_price.is_finite()
        && p.mark_price > 0.0
        && p.stop.is_finite()
        && p.stop > 0.0
        && p.risk_cap.is_finite()
        && p.risk_cap >= 0.0
        && !p.broker.is_empty()
        && !p.symbol.is_empty()
        && current_risk(p) <= p.risk_cap + 1e-8
}

fn current_risk(p: &PaperPosition) -> f64 {
    match p.side {
        PaperSide::Long => (p.entry_price - p.stop) * p.quantity,
        PaperSide::Short => (p.stop - p.entry_price) * p.quantity,
    }
    .max(0.0)
}
fn current_risk_total(state: &PaperState) -> f64 {
    state.positions.iter().map(current_risk).sum()
}
fn risk_limit(state: &PaperState, settings: &AiSettings) -> f64 {
    state.equity * settings.max_risk_pct / 100.0
}
fn position_notional_total(state: &PaperState, snapshot: Option<&Snapshot>) -> f64 {
    state
        .positions
        .iter()
        .map(|p| {
            let price = snapshot
                .filter(|s| s.broker == p.broker && s.symbol == p.symbol)
                .map(|s| exit_price(p.side, s))
                .unwrap_or(p.mark_price);
            price * p.quantity
        })
        .sum()
}
fn exit_price(side: PaperSide, snapshot: &Snapshot) -> f64 {
    match side {
        PaperSide::Long => snapshot.bid,
        PaperSide::Short => snapshot.ask,
    }
}
fn position_pnl(p: &PaperPosition, price: f64, quantity: f64) -> f64 {
    match p.side {
        PaperSide::Long => (price - p.entry_price) * quantity,
        PaperSide::Short => (p.entry_price - price) * quantity,
    }
}
fn equity_at(state: &PaperState, snapshot: &Snapshot) -> f64 {
    state.initial_equity + state.realized + unrealized_pnl(&state.positions, snapshot)
}
fn equity_from_marks(state: &PaperState) -> f64 {
    state.initial_equity
        + state.realized
        + state
            .positions
            .iter()
            .map(|p| position_pnl(p, p.mark_price, p.quantity))
            .sum::<f64>()
}
fn unrealized_pnl(positions: &[PaperPosition], snapshot: &Snapshot) -> f64 {
    positions
        .iter()
        .map(|p| {
            let price = if p.broker == snapshot.broker && p.symbol == snapshot.symbol {
                exit_price(p.side, snapshot)
            } else {
                p.mark_price
            };
            position_pnl(p, price, p.quantity)
        })
        .sum()
}
fn daily_pnl(state: &PaperState, snapshot: &Snapshot) -> f64 {
    state.daily_realized + unrealized_pnl(&state.positions, snapshot) - state.daily_unrealized_baseline
}

fn unix_now() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| "system clock is before Unix epoch".into())
}

fn validate_local_url(endpoint: &str) -> Result<(), String> {
    let url = Url::parse(endpoint).map_err(|_| "local_url must be a valid loopback HTTP(S) URL".to_string())?;
    url.host()
        .ok_or_else(|| "local_url must use a loopback host".to_string())?;
    let loopback = url.host_str().is_some_and(|name| {
        name.eq_ignore_ascii_case("localhost") || name.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
    });
    if !matches!(url.scheme(), "http" | "https")
        || !loopback
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/v1/chat/completions"
    {
        return Err(
            "local_url must be a loopback HTTP(S) chat-completions URL without credentials, query, or fragment".into(),
        );
    }
    Ok(())
}

/// Ask a local OpenAI-compatible endpoint or OpenRouter for a strict JSON decision.
/// Returns optional OpenRouter reported cost; budgets are enforced by the caller.
pub async fn request_decision(
    settings: &AiSettings,
    snapshot: &Snapshot,
    state: &PaperState,
    api_key: Option<&str>,
    cloud: bool,
) -> Result<(Decision, Option<f64>), String> {
    settings.validate()?;
    snapshot.validate()?;
    if cloud && api_key.map(str::trim).filter(|s| !s.is_empty()).is_none() {
        return Err("cloud API key is required".into());
    }
    let endpoint = if cloud { CLOUD_URL } else { settings.local_url.as_str() };
    let model = if cloud {
        &settings.cloud_model
    } else {
        &settings.local_model
    };
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none());
    if cloud {
        builder = builder.default_headers({
            let mut headers = header::HeaderMap::new();
            headers.insert(
                header::HeaderName::from_static("http-referer"),
                header::HeaderValue::from_static("https://aegis.local"),
            );
            headers.insert(
                header::HeaderName::from_static("x-title"),
                header::HeaderValue::from_static("AEGIS Paper AI"),
            );
            headers
        });
    }
    let client = builder
        .build()
        .map_err(|_| "failed to initialize AI HTTP client".to_string())?;
    let body = request_body(settings, snapshot, state, model, !cloud)?;
    let mut request = client
        .post(endpoint)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body);
    if cloud {
        request = request.bearer_auth(api_key.unwrap_or_default());
    }
    let response = request
        .send()
        .await
        .map_err(|_| "AI request failed or timed out".to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("AI endpoint returned HTTP {}", status.as_u16()));
    }
    let bytes = response_bytes(response).await?;
    let body: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "AI endpoint returned invalid JSON response".to_string())?;
    let text = body
        .pointer("/choices/0/message/content")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "AI response has no decision content".to_string())?;
    let decision: Decision =
        serde_json::from_str(text).map_err(|_| "AI decision is not valid strict decision JSON".to_string())?;
    let cost = body
        .pointer("/usage/cost")
        .and_then(serde_json::Value::as_f64)
        .filter(|v| v.is_finite() && *v >= 0.0);
    Ok((decision, cost))
}

fn request_body(
    settings: &AiSettings,
    snapshot: &Snapshot,
    state: &PaperState,
    model: &str,
    local: bool,
) -> Result<Vec<u8>, String> {
    let strategy = strategy_prompt(&settings.strategy)?;
    let system = format!(
        "{}You are a cautious trading analyst. PAPER TRADING ONLY: suggest decisions, never claim to place orders. Quantities are asset units, not MT5 lots. Available market data is limited to bid/ask and OHLCV candles; no order-book depth or order-flow data is provided, so do not infer absorption or flow. Strategy: {strategy}\n{}\n{}Return exactly one JSON object and no markdown, using this schema: {{\"action\":\"wait|open|close|reduce|stop|consult\",\"snapshot_id\":integer,\"reason\":string, plus action fields: open={{\"side\":\"long|short\",\"quantity\":number,\"stop\":number}}, close={{\"position_id\":integer}}, reduce={{\"position_id\":integer,\"quantity\":number}}, stop={{\"position_id\":integer,\"stop\":number}}. Every action needs reason and snapshot_id. Prefer wait if uncertain. Do not invent market data.",
        if local { "/no_think\n" } else { "" },
        settings.preprompt,
        if local { "" } else { "Do not return consult; no further AI consultation is available.\n" }
    );
    let mut state = state.clone();
    state.journal = state
        .journal
        .into_iter()
        .rev()
        .take(10)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let risk_limits = serde_json::json!({
        "max_risk_pct": settings.max_risk_pct,
        "max_leverage": settings.max_leverage,
        "max_positions": settings.max_positions,
        "daily_loss_limit": settings.daily_loss_limit,
    });
    let user = serde_json::json!({ "snapshot": snapshot, "paper_state": state, "risk_limits": risk_limits });
    let mut payload = serde_json::json!({
        "model": model,
        "messages": [{"role":"system","content":system},{"role":"user","content":user.to_string()}],
        "temperature": 0,
        "max_tokens": 1024,
        "response_format": {"type":"json_object"}
    });
    if local {
        payload["reasoning_effort"] = "none".into();
    }
    let bytes = serde_json::to_vec(&payload).map_err(|_| "failed to encode AI request".to_string())?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err("AI request exceeds the 65536-byte limit".into());
    }
    Ok(bytes)
}

async fn response_bytes(mut response: reqwest::Response) -> Result<Vec<u8>, String> {
    const MAX_RESPONSE_BYTES: usize = 64 * 1024;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "AI endpoint response could not be read".to_string())?
    {
        if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err("AI response exceeded the 64 KiB limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
    };

    fn mock_endpoint(response: &'static str) -> (String, Arc<Mutex<String>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1/chat/completions", listener.local_addr().unwrap());
        let request_capture = Arc::new(Mutex::new(String::new()));
        let captured = Arc::clone(&request_capture);
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut request = String::new();
            let mut content_length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some((name, value)) = line.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        content_length = value.trim().parse::<usize>().unwrap();
                    }
                }
                request.push_str(&line);
            }
            let mut body = vec![0; content_length];
            reader.read_exact(&mut body).unwrap();
            request.push_str(&String::from_utf8_lossy(&body));
            *captured.lock().unwrap() = request;
            let mut stream = reader.into_inner();
            let status = if response.starts_with("HTTP/") {
                response
            } else {
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n"
            };
            if status == "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n" {
                write!(
                    stream,
                    "{status}Content-Length: {}\r\nConnection: close\r\n\r\n{response}",
                    response.len()
                )
                .unwrap();
            } else {
                write!(
                    stream,
                    "{status}Location: http://127.0.0.1:9/redirect\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
            }
        });
        (endpoint, request_capture, handle)
    }

    fn snapshot(time: u64, id: u64) -> Snapshot {
        Snapshot {
            id,
            broker: "paper".into(),
            symbol: "TEST".into(),
            time,
            bid: 99.0,
            ask: 100.0,
            candles: vec![],
        }
    }

    #[test]
    fn defaults_and_strategy_prompts_are_defined() {
        let settings = AiSettings::default();
        assert_eq!(settings.local_url, LOCAL_URL);
        assert_eq!(settings.local_model, "qwen3:8b");
        assert_eq!(settings.cloud_model, "anthropic/claude-sonnet-4.5");
        assert_eq!(settings.max_leverage, 1.0);
        for strategy in ["bounce", "structural", "breakout", "liquidity_sweep", "data"] {
            assert!(strategy_prompt(strategy).is_ok());
        }
        assert!(strategy_prompt("other").is_err());
        assert!(serde_json::from_str::<AiSettings>("{}").is_ok());
    }

    #[test]
    fn malformed_risk_and_non_loopback_local_url_are_rejected() {
        let settings = AiSettings {
            max_risk_pct: f64::NAN,
            ..AiSettings::default()
        };
        assert!(settings.validate().is_err());
        let settings = AiSettings {
            local_url: "http://example.com/v1/chat/completions".into(),
            ..AiSettings::default()
        };
        assert!(settings.validate().is_err());
        let settings = AiSettings {
            local_url: "http://user:pass@127.0.0.1:11434/v1/chat/completions".into(),
            ..AiSettings::default()
        };
        assert!(settings.validate().is_err());
        let settings = AiSettings {
            local_url: "http://127.0.0.1:11434/custom".into(),
            ..AiSettings::default()
        };
        assert!(settings.validate().is_err());
        let settings = AiSettings {
            local_url: "http://127.0.0.1:11434/v1/chat/completions?token=secret".into(),
            ..AiSettings::default()
        };
        assert!(settings.validate().is_err());
        let settings = AiSettings {
            max_leverage: f64::INFINITY,
            ..AiSettings::default()
        };
        assert!(settings.validate().is_err());
        let settings = AiSettings {
            initial_equity: f64::MAX,
            max_leverage: 100.0,
            ..AiSettings::default()
        };
        assert!(settings.validate().is_err());
    }

    #[test]
    fn open_requires_valid_stop_and_obeys_aggregate_risk() {
        let settings = AiSettings::default();
        let mut engine = PaperEngine::new(&settings);
        let snap = snapshot(100, 1);
        assert!(engine
            .apply(
                Decision::Open {
                    snapshot_id: 1,
                    side: "long".into(),
                    quantity: 10.1,
                    stop: 90.0,
                    reason: "setup".into()
                },
                &snap,
                &settings,
                100
            )
            .is_err());
        engine
            .apply(
                Decision::Open {
                    snapshot_id: 1,
                    side: "long".into(),
                    quantity: 9.0,
                    stop: 90.0,
                    reason: "setup".into(),
                },
                &snap,
                &settings,
                100,
            )
            .unwrap();
        assert_eq!(engine.state().positions[0].entry_price, snap.ask);
        assert!(engine
            .apply(
                Decision::Open {
                    snapshot_id: 1,
                    side: "long".into(),
                    quantity: 2.0,
                    stop: 90.0,
                    reason: "too much".into()
                },
                &snap,
                &settings,
                100
            )
            .is_err());
    }

    #[test]
    fn stale_mismatched_and_unknown_decisions_are_rejected() {
        let settings = AiSettings::default();
        let mut engine = PaperEngine::new(&settings);
        let snap = snapshot(10, 7);
        assert!(engine
            .apply(
                Decision::Wait {
                    snapshot_id: 6,
                    reason: "wait".into()
                },
                &snap,
                &settings,
                10
            )
            .is_err());
        assert!(engine
            .apply(
                Decision::Wait {
                    snapshot_id: 7,
                    reason: "wait".into()
                },
                &snap,
                &settings,
                41
            )
            .is_err());
        assert!(
            serde_json::from_str::<Decision>(r#"{"action":"wait","snapshot_id":1,"reason":"x","extra":true}"#).is_err()
        );
    }

    #[test]
    fn stops_close_deterministically_at_bid_and_stop_updates_cannot_expand_risk() {
        let settings = AiSettings::default();
        let mut engine = PaperEngine::new(&settings);
        let first = snapshot(100, 1);
        engine
            .apply(
                Decision::Open {
                    snapshot_id: 1,
                    side: "long".into(),
                    quantity: 1.0,
                    stop: 90.0,
                    reason: "setup".into(),
                },
                &first,
                &settings,
                100,
            )
            .unwrap();
        let position = engine.state().positions[0].id;
        assert!(engine
            .apply(
                Decision::Stop {
                    snapshot_id: 1,
                    position_id: position,
                    stop: 89.0,
                    reason: "widen".into()
                },
                &first,
                &settings,
                100
            )
            .is_err());
        let stopped = snapshot(unix_now().unwrap(), 2);
        let mut stopped = stopped;
        stopped.bid = 88.0;
        stopped.ask = 89.0;
        engine.mark(&stopped, &settings).unwrap();
        let state = engine.state();
        assert!(state.positions.is_empty());
        assert!((state.realized - (-12.0)).abs() < 1e-9);
    }

    #[test]
    fn daily_loss_blocks_new_risk_but_allows_closing_positions() {
        let settings = AiSettings {
            daily_loss_limit: 5.0,
            ..AiSettings::default()
        };
        let mut engine = PaperEngine::new(&settings);
        let first = snapshot(100, 1);
        engine
            .apply(
                Decision::Open {
                    snapshot_id: 1,
                    side: "long".into(),
                    quantity: 1.0,
                    stop: 80.0,
                    reason: "setup".into(),
                },
                &first,
                &settings,
                100,
            )
            .unwrap();
        let position_id = engine.state().positions[0].id;
        let mut loss = snapshot(101, 2);
        loss.bid = 93.0;
        loss.ask = 94.0;
        assert!(engine
            .apply(
                Decision::Open {
                    snapshot_id: 2,
                    side: "long".into(),
                    quantity: 0.1,
                    stop: 90.0,
                    reason: "blocked".into(),
                },
                &loss,
                &settings,
                101,
            )
            .is_err());
        engine
            .apply(
                Decision::Close {
                    snapshot_id: 2,
                    position_id,
                    reason: "risk control".into(),
                },
                &loss,
                &settings,
                101,
            )
            .unwrap();
        let state = engine.state();
        assert!(state.positions.is_empty());
        assert_eq!(state.realized, -7.0);
        assert_eq!(state.equity, 9_993.0);
    }

    #[test]
    fn reduction_and_rehydration_preserve_paper_state() {
        let settings = AiSettings::default();
        let mut engine = PaperEngine::new(&settings);
        let snap = snapshot(100, 1);
        engine
            .apply(
                Decision::Open {
                    snapshot_id: 1,
                    side: "short".into(),
                    quantity: 0.5,
                    stop: 120.0,
                    reason: "setup".into(),
                },
                &snap,
                &settings,
                100,
            )
            .unwrap();
        let position = engine.state().positions[0].id;
        engine
            .apply(
                Decision::Reduce {
                    snapshot_id: 1,
                    position_id: position,
                    quantity: 0.25,
                    reason: "trim".into(),
                },
                &snap,
                &settings,
                100,
            )
            .unwrap();
        let restored = PaperEngine::from_state(engine.state(), &settings).unwrap();
        assert_eq!(restored.state().positions[0].quantity, 0.25);
        assert_eq!(restored.state().journal.len(), 2);
    }

    #[test]
    fn request_is_capped_trims_journal_and_uses_small_candle_contract() {
        let settings = AiSettings::default();
        let mut state = PaperState::default();
        for index in 0..20 {
            state.journal.push_back(DecisionRecord {
                time: index,
                snapshot_id: index,
                action: "wait".into(),
                reason: "x".repeat(2_000),
                position_id: None,
                pnl: 0.0,
            });
        }
        let body = request_body(&settings, &snapshot(1, 1), &state, "qwen3:8b", true).unwrap();
        assert!(body.len() <= MAX_REQUEST_BYTES);
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["max_tokens"], 1024);
        assert!(parsed["messages"][0]["content"]
            .as_str()
            .unwrap()
            .starts_with("/no_think\n"));
        let embedded: serde_json::Value =
            serde_json::from_str(parsed["messages"][1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(embedded["paper_state"]["journal"].as_array().unwrap().len(), 10);
        assert_eq!(embedded["risk_limits"]["max_risk_pct"], settings.max_risk_pct);
        assert_eq!(embedded["risk_limits"]["max_leverage"], settings.max_leverage);
        assert_eq!(embedded["risk_limits"]["max_positions"], settings.max_positions);
        assert_eq!(embedded["risk_limits"]["daily_loss_limit"], settings.daily_loss_limit);
        let system_prompt = parsed["messages"][0]["content"].as_str().unwrap();
        assert!(system_prompt.contains("not MT5 lots"));
        assert!(system_prompt.contains("no order-book depth or order-flow data"));

        let cloud_body = request_body(&settings, &snapshot(1, 1), &state, "model", false).unwrap();
        let cloud_body: serde_json::Value = serde_json::from_slice(&cloud_body).unwrap();
        let cloud_prompt = cloud_body["messages"][0]["content"].as_str().unwrap();
        assert!(cloud_prompt.contains("Do not return consult"));

        let mut oversized = snapshot(1, 1);
        oversized.symbol = "S".repeat(MAX_REQUEST_BYTES);
        assert!(request_body(&settings, &oversized, &PaperState::default(), "qwen3:8b", true).is_err());
        oversized.symbol = "TEST".into();
        oversized.candles = vec![
            Candle {
                time: 0,
                open: 1.0,
                high: 1.0,
                low: 1.0,
                close: 1.0,
                volume: 0.0
            };
            61
        ];
        assert!(oversized.validate().is_err());
    }

    #[tokio::test]
    async fn local_request_omits_credentials_and_rejects_malformed_decision() {
        let response = r#"{"choices":[{"message":{"content":"{\"action\":\"wait\",\"snapshot_id\":1,\"reason\":\"ok\",\"unexpected\":true}"}}]}"#;
        let (endpoint, captured, server) = mock_endpoint(response);
        let settings = AiSettings {
            local_url: endpoint,
            ..AiSettings::default()
        };
        let result = request_decision(
            &settings,
            &snapshot(unix_now().unwrap(), 1),
            &PaperState::default(),
            Some("local-secret"),
            false,
        )
        .await;
        assert!(result.is_err());
        let request = captured.lock().unwrap().clone();
        assert!(!request.to_ascii_lowercase().contains("authorization:"));
        assert!(!request.contains("local-secret"));
        assert!(request.contains("\"max_tokens\":1024"));
        server.join().unwrap();
    }

    #[tokio::test]
    async fn local_client_does_not_follow_redirects() {
        let (endpoint, _, server) = mock_endpoint("HTTP/1.1 302 Found\r\n");
        let settings = AiSettings {
            local_url: endpoint,
            ..AiSettings::default()
        };
        let result = request_decision(
            &settings,
            &snapshot(unix_now().unwrap(), 1),
            &PaperState::default(),
            None,
            false,
        )
        .await;
        assert_eq!(result.unwrap_err(), "AI endpoint returned HTTP 302");
        server.join().unwrap();
    }

    #[test]
    fn leverage_caps_open_notional() {
        let settings = AiSettings {
            max_leverage: 0.01,
            ..AiSettings::default()
        };
        let mut engine = PaperEngine::new(&settings);
        let snap = snapshot(100, 1);
        assert!(engine
            .apply(
                Decision::Open {
                    snapshot_id: 1,
                    side: "long".into(),
                    quantity: 2.0,
                    stop: 90.0,
                    reason: "excess notional".into(),
                },
                &snap,
                &settings,
                100
            )
            .is_err());
    }

    #[test]
    fn restored_state_rejects_non_finite_aggregate_notional() {
        let settings = AiSettings::default();
        let state = PaperState {
            positions: (1..=2)
                .map(|id| PaperPosition {
                    id,
                    broker: "paper".into(),
                    symbol: format!("TEST{id}"),
                    side: PaperSide::Long,
                    quantity: 2.0,
                    entry_price: 1e308,
                    mark_price: 1e308,
                    stop: 1e308,
                    risk_cap: 0.0,
                    opened_at: 1,
                })
                .collect(),
            ..PaperState::default()
        };
        assert!(PaperEngine::from_state(state, &settings).is_err());
    }

    #[test]
    fn mark_rejects_stale_quotes_and_daily_baseline_captures_floating_loss() {
        let settings = AiSettings {
            daily_loss_limit: 5.0,
            ..AiSettings::default()
        };
        let mut engine = PaperEngine::new(&settings);
        let now = unix_now().unwrap();
        let first = snapshot(now, 1);
        engine
            .apply(
                Decision::Open {
                    snapshot_id: 1,
                    side: "long".into(),
                    quantity: 1.0,
                    stop: 80.0,
                    reason: "setup".into(),
                },
                &first,
                &settings,
                now,
            )
            .unwrap();

        let mut stale = snapshot(now.saturating_sub(60), 2);
        assert!(engine.mark(&stale, &settings).is_err());
        stale.time = now;
        stale.bid = 90.0;
        stale.ask = 91.0;
        engine.state.daily_marker = (now / 86_400).saturating_sub(1) * 86_400;
        engine.mark(&stale, &settings).unwrap();
        assert_eq!(engine.state.daily_unrealized_baseline, -10.0);

        let mut worsened = snapshot(now, 3);
        worsened.bid = 84.0;
        worsened.ask = 85.0;
        assert!(engine
            .apply(
                Decision::Open {
                    snapshot_id: 3,
                    side: "long".into(),
                    quantity: 0.1,
                    stop: 80.0,
                    reason: "daily limit".into(),
                },
                &worsened,
                &settings,
                now,
            )
            .is_err());
    }
}
