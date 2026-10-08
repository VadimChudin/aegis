//! Read-only event tools. A finite model-selected plan is validated by the host;
//! a deterministic two-call plan is the fallback. No URLs, execution, arbitrary
//! symbols or model-controlled budgets are accepted.
use crate::{
    event_archive::Archive,
    event_scoring::{ScoringContext, SourceIdentity},
    live_market::{MarketSample, Tick},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

pub const MAX_ROWS: usize = 512;
pub const MAX_CALLS: usize = 2;
pub const MAX_RESULT_BYTES: usize = 8_192;
pub const DEADLINE: Duration = Duration::from_secs(3);
const SCAN_RECORDS: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryTicks {
    pub symbol: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub observed_at_ms: u64,
    pub ticks: Vec<Tick>,
    pub truncated: bool,
    pub complete_history: bool,
    pub volume_kind: String,
    pub coverage_note: String,
}
impl HistoryTicks {
    pub fn validate(&self, symbol: &str, start: u64, end: u64, limit: usize, now: u64) -> Result<(), String> {
        validate_history_request(start, end, limit, now)?;
        if self.symbol != symbol
            || self.start_ms != start
            || self.end_ms != end
            || self.observed_at_ms > now.saturating_add(2000)
            || self.observed_at_ms < end.saturating_sub(2000)
            || self.ticks.len() > limit
            || self.complete_history
            || self.volume_kind != "mt5_ticks_not_exchange_tape"
            || self.coverage_note.len() > 4096
        {
            return Err("Historical reply identity, bounds or coverage contract mismatch".into());
        }
        let mut previous = start;
        for tick in &self.ticks {
            validate_tick(tick, start, end)?;
            if tick.time_ms < previous {
                return Err("Historical ticks are not ordered".into());
            }
            previous = tick.time_ms;
        }
        Ok(())
    }
}
pub fn validate_history_request(start: u64, end: u64, limit: usize, now: u64) -> Result<(), String> {
    if start == 0 || start >= end || end - start > 36_000_000 || end > now || !(1..=2000).contains(&limit) {
        return Err("Historical ticks require past inclusive bounds <=10hours and 1..2000 rows".into());
    }
    Ok(())
}
fn validate_tick(t: &Tick, start: u64, end: u64) -> Result<(), String> {
    if t.time_ms < start
        || t.time_ms > end
        || !t.bid.is_finite()
        || !t.ask.is_finite()
        || t.bid <= 0.0
        || t.ask < t.bid
        || t.last.is_some_and(|x| !x.is_finite() || x < 0.0)
        || t.volume.is_some_and(|x| !x.is_finite() || x < 0.0)
    {
        return Err("Tick outside event bounds or invalid price/volume".into());
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolRequest {
    pub tool: ToolName,
    pub event_id: String,
    pub source: SourceIdentity,
    pub start_ms: u64,
    pub end_ms: u64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolName {
    QueryTickSlice,
    QueryBookSlice,
}

pub struct Dispatcher {
    context: ScoringContext,
    started: Instant,
    calls: usize,
}
impl Dispatcher {
    pub fn new(context: &ScoringContext) -> Self {
        Self {
            context: context.clone(),
            started: Instant::now(),
            calls: 0,
        }
    }
    pub fn plan(&self) -> [ToolRequest; 2] {
        let end = self.context.snapshot_time_unix_ms;
        let start = end.saturating_sub(self.context.window.seconds() * 1000);
        [ToolName::QueryTickSlice, ToolName::QueryBookSlice].map(|tool| ToolRequest {
            tool,
            event_id: self.context.event_id.clone(),
            source: self.context.source.clone(),
            start_ms: start,
            end_ms: end,
        })
    }
    pub fn admit(&mut self, request: &ToolRequest, now: u64) -> Result<(), String> {
        if self.calls >= MAX_CALLS || self.started.elapsed() >= DEADLINE {
            return Err("Tool call/deadline budget exhausted".into());
        }
        let ctx = &self.context;
        let earliest = ctx.snapshot_time_unix_ms.saturating_sub(ctx.window.seconds() * 1000);
        if !ctx.source_authorized
            || request.event_id != ctx.event_id
            || request.source != ctx.source
            || request.start_ms < earliest
            || request.end_ms > ctx.snapshot_time_unix_ms
            || request.end_ms > now
            || request.start_ms >= request.end_ms
            || request.start_ms == 0
        {
            return Err("Tool request mixes source/event or leaves authorized historical window".into());
        }
        self.calls += 1;
        Ok(())
    }
    pub fn observed(&self, archive: &Archive, request: &ToolRequest) -> Result<Value, String> {
        let mut guard = Self::new(&self.context);
        guard.admit(request, crate::sign::now_ms().max(0) as u64)?;
        let mut ticks = BTreeMap::new();
        let mut books = BTreeMap::new();
        let mut records = 0;
        let mut truncated = false;
        let mut volume_kind = "observed_mt5_ticks_not_exchange_tape".to_string();
        let result = archive.visit::<MarketSample>(|sample| {
            records += 1;
            if records > SCAN_RECORDS || self.started.elapsed() >= DEADLINE {
                return Err("archive_scan_budget".into());
            }
            if sample.symbol != request.source.instrument { return Err("Source archive instrument mismatch".into()); }
            // The owning EventCycle binds this permanent archive to its source store.
            match request.tool {
                ToolName::QueryTickSlice => {
                    volume_kind = sample.volume_kind;
                    for tick in sample.ticks {
                        if tick.time_ms < request.start_ms || tick.time_ms > request.end_ms { continue; }
                        validate_tick(&tick, request.start_ms, request.end_ms)?;
                        ticks.insert((tick.time_ms, tick.bid.to_bits(), tick.ask.to_bits(), tick.last.map(f64::to_bits), tick.volume.map(f64::to_bits)), tick);
                        if ticks.len() > MAX_ROWS { ticks.pop_first(); truncated = true; }
                    }
                }
                ToolName::QueryBookSlice => if let Some(book) = sample.book {
                    if book.symbol != request.source.instrument { return Err("Book source mismatch".into()); }
                    if book.timestamp >= request.start_ms && book.timestamp <= request.end_ms {
                        book.validate()?;
                        let bid: f64 = book.bids.iter().map(|x| x.quantity).sum();
                        let ask: f64 = book.asks.iter().map(|x| x.quantity).sum();
                        books.insert(book.timestamp, json!({"time_ms":book.timestamp,"imbalance":if bid+ask>0.0 {Some((bid-ask)/(bid+ask))} else {None},"bid_quantity":bid,"ask_quantity":ask,"observed_bid_levels":book.bids.len(),"observed_ask_levels":book.asks.len(),"bids":book.bids.iter().take(4).collect::<Vec<_>>(),"asks":book.asks.iter().take(4).collect::<Vec<_>>()}));
                        if books.len() > 24 { books.pop_first(); truncated = true; }
                    }
                },
            }
            Ok(())
        });
        let scan_note = match result {
            Ok(()) => "permanent observed archive; sampled, not continuous",
            Err(e) if e == "archive_scan_budget" => {
                truncated = true;
                "archive scan deadline/record budget; partial selection"
            }
            Err(e) => return Err(e),
        };
        let rows: Vec<_> = ticks.into_values().collect();
        let mut value = match request.tool {
            ToolName::QueryTickSlice => tick_result(
                request,
                &rows,
                &volume_kind,
                "permanent_observed_archive",
                truncated,
                scan_note,
            ),
            ToolName::QueryBookSlice => {
                json!({"tool":"query_book_slice","request":request,"origin":"permanent_observed_archive","complete_history":false,"truncated":truncated,"missing":books.is_empty(),"coverage_note":scan_note,"historical_book_reconstruction":false,"rows":books.into_values().collect::<Vec<_>>()})
            }
        };
        cap_result(&mut value)?;
        Ok(value)
    }
    pub fn historical(&self, request: &ToolRequest, history: &HistoryTicks) -> Result<Value, String> {
        let mut guard = Self::new(&self.context);
        guard.admit(request, crate::sign::now_ms().max(0) as u64)?;
        history.validate(
            &request.source.instrument,
            request.start_ms,
            request.end_ms,
            MAX_ROWS,
            crate::sign::now_ms().max(0) as u64,
        )?;
        if !matches!(request.tool, ToolName::QueryTickSlice) {
            return Err("Historical books are unavailable".into());
        }
        let mut result = tick_result(
            request,
            &history.ticks,
            &history.volume_kind,
            "mt5_history_ticks",
            history.truncated,
            &history.coverage_note,
        );
        cap_result(&mut result)?;
        Ok(result)
    }
}
fn tick_result(
    request: &ToolRequest,
    ticks: &[Tick],
    volume_kind: &str,
    origin: &str,
    truncated: bool,
    note: &str,
) -> Value {
    let n = ticks.len();
    let mids: Vec<_> = ticks.iter().map(|t| (t.ask + t.bid) / 2.0).collect();
    let first = ticks.first().map(|x| x.time_ms);
    let last = ticks.last().map(|x| x.time_ms);
    let elapsed = first.zip(last).map(|(a, b)| (b.saturating_sub(a)) as f64 / 1000.0);
    let gaps = ticks
        .windows(2)
        .filter(|w| w[1].time_ms.saturating_sub(w[0].time_ms) > 60_000)
        .count();
    json!({"tool":"query_tick_slice","request":request,"origin":origin,"volume_kind":volume_kind,"complete_history":false,"truncated":truncated,"missing":n==0,"coverage_note":note,"features":{"rows":n,"first_ms":first,"last_ms":last,"gap_over_60s_count":gaps,"spread_mean":if n>0 {Some(ticks.iter().map(|x|x.ask-x.bid).sum::<f64>()/n as f64)} else {None},"mid_range":if n>0 {Some(mids.iter().copied().fold(f64::NEG_INFINITY,f64::max)-mids.iter().copied().fold(f64::INFINITY,f64::min))} else {None},"return_fraction":mids.first().zip(mids.last()).map(|(a,b)|b/a-1.0),"observed_tick_rate_per_second":elapsed.filter(|x|*x>0.0).map(|x|n.saturating_sub(1) as f64/x),"native_volume_sum":ticks.iter().filter_map(|x|x.volume).sum::<f64>(),"volume_present_rows":ticks.iter().filter(|x|x.volume.is_some()).count()},"raw_selection":"first and last selected ticks; features describe returned subset only","rows":ticks.iter().take(8).chain(ticks.iter().skip(n.saturating_sub(8).max(8))).collect::<Vec<_>>()})
}
fn cap_result(value: &mut Value) -> Result<(), String> {
    while serde_json::to_vec(value).map_err(|e| e.to_string())?.len() > MAX_RESULT_BYTES {
        if let Some(rows) = value["rows"].as_array_mut() {
            if !rows.is_empty() {
                rows.remove(0);
                value["truncated"] = json!(true);
                continue;
            }
        }
        return Err("Tool result exceeds byte budget".into());
    }
    Ok(())
}
/// One shared payload is carried through the local score and cloud event review.
pub fn attach_results(telemetry: &mut Value, results: Vec<Value>) -> Result<(), String> {
    if results.len() > MAX_CALLS
        || serde_json::to_vec(&results).map_err(|e| e.to_string())?.len() > 2 * MAX_RESULT_BYTES
    {
        return Err("Tool bundle budget exceeded".into());
    }
    telemetry["tool_results"] = json!({"plan":"deterministic_event_bound_read_only","results":results,"max_calls":MAX_CALLS,"max_rows":MAX_ROWS,"max_result_bytes":MAX_RESULT_BYTES,"deadline_ms":DEADLINE.as_millis(),"complete_history":false});
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    requests: Vec<ToolRequest>,
}
pub fn parse_plan(text: &str, context: &ScoringContext, now: u64) -> Result<Vec<ToolRequest>, String> {
    if text.len() > 4096 {
        return Err("Tool selection exceeds JSON budget".into());
    }
    let plan: Plan = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let mut dispatcher = Dispatcher::new(context);
    for request in &plan.requests {
        dispatcher.admit(request, now)?;
    }
    Ok(plan.requests)
}

/// Tool stages never extend the age permission on the trusted event snapshot.
pub fn require_fresh(context: &ScoringContext, now: u64) -> Result<(), String> {
    if !context.source_authorized
        || context.snapshot_time_unix_ms > now
        || context.max_source_age_ms == 0
        || now.saturating_sub(context.snapshot_time_unix_ms) > context.max_source_age_ms.min(30_000)
    {
        return Err("Event tool snapshot is stale/future/unauthorized".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_scoring::ScoringWindow;
    fn context() -> ScoringContext {
        ScoringContext {
            event_id: "event".into(),
            snapshot_id: "snapshot".into(),
            source: SourceIdentity {
                connector_id: "roboforex".into(),
                source_id: "bound".into(),
                instrument: "XAUUSD".into(),
            },
            snapshot_time_unix_ms: 40_000_000,
            window: ScoringWindow::TwoHours,
            max_source_age_ms: 10000,
            source_authorized: true,
            snapshot_complete: true,
        }
    }
    #[test]
    fn rejects_mixing_future_outside_window_and_calls() {
        let ctx = context();
        let request = Dispatcher::new(&ctx).plan()[0].clone();
        let mut wrong = request.clone();
        wrong.source.source_id = "other".into();
        assert!(Dispatcher::new(&ctx).admit(&wrong, 40_000_000).is_err());
        wrong = request.clone();
        wrong.event_id = "other".into();
        assert!(Dispatcher::new(&ctx).admit(&wrong, 40_000_000).is_err());
        wrong = request.clone();
        wrong.start_ms -= 1;
        assert!(Dispatcher::new(&ctx).admit(&wrong, 40_000_000).is_err());
        wrong = request.clone();
        wrong.end_ms += 1;
        assert!(Dispatcher::new(&ctx).admit(&wrong, 40_000_000).is_err());
        assert!(Dispatcher::new(&ctx).admit(&request, 39_999_999).is_err());
        let mut dispatcher = Dispatcher::new(&ctx);
        dispatcher.admit(&request, 40_000_000).unwrap();
        dispatcher.admit(&request, 40_000_000).unwrap();
        assert!(dispatcher.admit(&request, 40_000_000).is_err());
    }
    #[test]
    fn strict_json_has_no_execution_or_urls() {
        let ctx = context();
        let req = Dispatcher::new(&ctx).plan()[0].clone();
        let valid = json!({"requests":[req]});
        assert!(parse_plan(&valid.to_string(), &ctx, 40_000_000).is_ok());
        let mut bad = valid.clone();
        bad["requests"][0]["url"] = json!("https://evil.invalid");
        assert!(parse_plan(&bad.to_string(), &ctx, 40_000_000).is_err());
        bad = valid;
        bad["requests"][0]["tool"] = json!("execute");
        assert!(parse_plan(&bad.to_string(), &ctx, 40_000_000).is_err());
    }
    #[test]
    fn validates_history_and_features_without_full_tape_claim() {
        let ctx = context();
        let request = Dispatcher::new(&ctx).plan()[0].clone();
        let mut history = HistoryTicks {
            symbol: "XAUUSD".into(),
            start_ms: request.start_ms,
            end_ms: request.end_ms,
            observed_at_ms: 40_000_000,
            ticks: vec![
                Tick {
                    time_ms: request.start_ms,
                    bid: 100.0,
                    ask: 102.0,
                    last: None,
                    volume: Some(3.0),
                },
                Tick {
                    time_ms: request.start_ms + 1000,
                    bid: 101.0,
                    ask: 103.0,
                    last: None,
                    volume: Some(4.0),
                },
            ],
            truncated: true,
            complete_history: false,
            volume_kind: "mt5_ticks_not_exchange_tape".into(),
            coverage_note: "partial first requested rows".into(),
        };
        history
            .validate("XAUUSD", request.start_ms, request.end_ms, 512, 40_000_000)
            .unwrap();
        let result = Dispatcher::new(&ctx).historical(&request, &history).unwrap();
        assert_eq!(result["features"]["spread_mean"], 2.0);
        assert_eq!(result["features"]["mid_range"], 1.0);
        assert_eq!(result["features"]["native_volume_sum"], 7.0);
        assert_eq!(result["complete_history"], false);
        history.ticks[0].time_ms -= 1;
        assert!(history
            .validate("XAUUSD", request.start_ms, request.end_ms, 512, 40_000_000)
            .is_err());
    }
    #[test]
    fn observed_slice_preserves_distinct_same_millisecond_ticks() {
        let ctx = context();
        let request = Dispatcher::new(&ctx).plan()[0].clone();
        let root = std::env::temp_dir().join(format!(
            "aegis-tool-tick-{}-{}",
            std::process::id(),
            crate::sign::now_ms()
        ));
        let archive = Archive::open(&root).unwrap();
        let stamp = request.end_ms - 1000;
        let sample = MarketSample {
            symbol: "XAUUSD".into(),
            observed_at_ms: stamp,
            quote: crate::live_market::Quote {
                time_ms: stamp,
                bid: 100.,
                ask: 100.1,
            },
            ticks: vec![
                Tick {
                    time_ms: stamp,
                    bid: 100.,
                    ask: 100.1,
                    last: None,
                    volume: Some(1.),
                },
                Tick {
                    time_ms: stamp,
                    bid: 101.,
                    ask: 101.1,
                    last: None,
                    volume: Some(2.),
                },
            ],
            book: None,
            book_note: "unavailable".into(),
            volume_kind: "mt5_ticks_not_exchange_tape".into(),
        };
        archive.append("one", &sample).unwrap();
        let result = Dispatcher::new(&ctx).observed(&archive, &request).unwrap();
        assert_eq!(result["features"]["rows"], 2);
        assert_eq!(result["features"]["native_volume_sum"], 3.);
        let mut empty = request;
        empty.start_ms = empty.end_ms;
        assert!(Dispatcher::new(&ctx).admit(&empty, ctx.snapshot_time_unix_ms).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn same_observer_tool_payload_reaches_local_prompt_and_cloud_review() {
        let ctx = context();
        let mut telemetry = json!({"full_tick_history":false});
        attach_results(
            &mut telemetry,
            vec![json!({"tool":"query_book_slice","missing":true,"complete_history":false})],
        )
        .unwrap();
        let prompt = crate::event_scoring::build_local_prompt(&ctx, &telemetry.to_string(), 40_000_000).unwrap();
        assert!(prompt.user.contains("query_book_slice"));
        let review = json!({"event_id":ctx.event_id,"telemetry":telemetry});
        assert_eq!(
            review["telemetry"]["tool_results"]["results"][0]["tool"],
            "query_book_slice"
        );
        let observer = include_str!("../../../app/src-tauri/src/observer_cmd.rs");
        assert!(
            observer.find("attach_results(&mut telemetry, results)").unwrap()
                < observer.find("local.score(&context, &telemetry)").unwrap()
        );
        assert!(observer.contains("\"telemetry\":telemetry"));
    }
}
