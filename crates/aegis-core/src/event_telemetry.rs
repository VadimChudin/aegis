//! Bounded, single-writer telemetry foundation (not broker/app integration).
//!
//! All timestamps are Unix seconds UTC. Windows are HOURS, never sample counts.
//! An unresolved event owns its evidence until an explicit terminal outcome is
//! persisted. Capacity exhaustion fails closed: active evidence is never evicted.
//! The caller must serialize access (including across processes), provide truthful
//! broker close confirmations/costs, and feed live DOM only; this module neither
//! reconstructs historical DOM nor changes live risk. Register in lib.rs separately.
//! Atomic snapshots use a sibling temporary file, fsync, rename, directory fsync
//! on Unix. A crash before rename leaves the previous committed snapshot intact.

use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub const WINDOW_HOURS: [u64; 3] = [2, 3, 10];
const SCHEMA: u32 = 1;
const WEEK: i64 = 7 * 24 * 3600;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceIdentity {
    pub venue: String,
    pub server: String,
    pub account: String,
    pub symbol: String,
    /// Identifies the feed/session origin; never merge different sources silently.
    pub feed: String,
}
impl SourceIdentity {
    pub fn mt5_xauusd(server: String, account: String, feed: String) -> Self {
        Self {
            venue: "MT5".into(),
            server,
            account,
            symbol: "XAUUSD".into(),
            feed,
        }
    }
    fn valid(&self) -> bool {
        self.venue == "MT5"
            && self.symbol == "XAUUSD"
            && [&self.server, &self.account, &self.feed]
                .iter()
                .all(|s| !s.trim().is_empty())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Budgets {
    /// Serialized committed snapshot ceiling; temporary write needs up to twice this.
    pub max_bytes: usize,
    pub max_samples: usize,
    pub max_events: usize,
    /// Bounds each string, including outcome conditions and DOM missing reasons.
    pub max_text_bytes: usize,
    pub max_dom_levels: usize,
}
impl Default for Budgets {
    fn default() -> Self {
        Self {
            max_bytes: 8 * 1024 * 1024,
            max_samples: 20_000,
            max_events: 1_000,
            max_text_bytes: 2_048,
            max_dom_levels: 50,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Level {
    pub price: f64,
    pub quantity: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Dom {
    /// Live observed book, never inferred/backfilled from candles or quotes.
    Observed {
        bids: Vec<Level>,
        asks: Vec<Level>,
    },
    Missing {
        reason: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sample {
    pub source: SourceIdentity,
    pub at_utc: i64,
    pub bid: f64,
    pub ask: f64,
    pub dom: Dom,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutcomeConditions {
    pub config_version: String,
    pub entry_conditions: String,
    pub exit_conditions: String,
    /// Explicit denomination for all realized P/L and cost fields.
    pub account_currency: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RealizedPnl {
    pub gross: f64,
    pub commission: f64,
    pub fees: f64,
    /// Signed charge: positive costs, negative credits.
    pub swap_cost: f64,
    /// Incremental cost not already included in gross (avoid double counting).
    pub other_cost: f64,
}
impl RealizedPnl {
    pub fn net(&self) -> f64 {
        self.gross - self.commission - self.fees - self.swap_cost - self.other_cost
    }
    fn valid(&self) -> bool {
        [
            self.gross,
            self.commission,
            self.fees,
            self.swap_cost,
            self.other_cost,
            self.net(),
        ]
        .iter()
        .all(|v| v.is_finite())
            && self.commission >= 0.0
            && self.fees >= 0.0
            && self.other_cost >= 0.0
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Outcome {
    Unknown,
    TradeClosed {
        trade_id: String,
        /// Caller reconciled with broker; a timeout/absence of data is not closure.
        confirmed_closed: bool,
        pnl: RealizedPnl,
    },
    Rejected {
        reason: String,
    },
    Expired {
        reason: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Finalized {
    pub at_utc: i64,
    pub outcome: Outcome,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub started_utc: i64,
    pub conditions: OutcomeConditions,
    pub trade_id: Option<String>,
    /// Owned evidence is implicitly pinned while finalized is None.
    pub evidence: Vec<Sample>,
    pub finalized: Option<Finalized>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct State {
    schema: u32,
    source: SourceIdentity,
    budgets: Budgets,
    samples: Vec<Sample>,
    events: Vec<Event>,
    evicted_samples: u64,
    evicted_finalized_events: u64,
}

#[derive(Debug)]
pub enum TelemetryError {
    Io(io::Error),
    Json(serde_json::Error),
    Invalid(&'static str),
    SourceMismatch,
    CapacityPinned,
    NotFound,
    AlreadyFinalized,
    UnknownOutcome,
}
impl std::fmt::Display for TelemetryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for TelemetryError {}
impl From<io::Error> for TelemetryError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<serde_json::Error> for TelemetryError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
pub type Result<T> = std::result::Result<T, TelemetryError>;

pub struct TelemetryStore {
    path: PathBuf,
    state: State,
}
impl TelemetryStore {
    /// Reopens only exact source/budget matches. Corrupt/oversize files fail closed.
    pub fn open(path: impl AsRef<Path>, source: SourceIdentity, budgets: Budgets) -> Result<Self> {
        let path = path.as_ref().to_owned();
        if !source.valid()
            || budgets.max_bytes == 0
            || budgets.max_samples == 0
            || budgets.max_events == 0
            || budgets.max_text_bytes == 0
            || budgets.max_dom_levels == 0
        {
            return Err(TelemetryError::Invalid("source or budgets"));
        }
        let state = match File::open(&path) {
            Ok(file) => {
                if file.metadata()?.len() > budgets.max_bytes as u64 {
                    return Err(TelemetryError::Invalid("oversize snapshot"));
                }
                let mut bytes = Vec::new();
                file.take(budgets.max_bytes as u64 + 1).read_to_end(&mut bytes)?;
                if bytes.len() > budgets.max_bytes {
                    return Err(TelemetryError::Invalid("oversize snapshot"));
                }
                let state: State = serde_json::from_slice(&bytes)?;
                if state.source != source {
                    return Err(TelemetryError::SourceMismatch);
                }
                // Caller cannot silently relax/change persisted budgets on restart.
                if serde_json::to_vec(&state.budgets)? != serde_json::to_vec(&budgets)? {
                    return Err(TelemetryError::Invalid("budget mismatch"));
                }
                state
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => State {
                schema: SCHEMA,
                source,
                budgets,
                samples: Vec::new(),
                events: Vec::new(),
                evicted_samples: 0,
                evicted_finalized_events: 0,
            },
            Err(e) => return Err(e.into()),
        };
        let store = Self { path, state };
        store.validate(&store.state)?;
        Ok(store)
    }
    pub fn events(&self) -> &[Event] {
        &self.state.events
    }
    pub fn ambient_samples(&self) -> &[Sample] {
        &self.state.samples
    }
    pub fn source(&self) -> &SourceIdentity {
        &self.state.source
    }

    /// Commits before returning success; failures leave in-memory state unchanged.
    pub fn append_sample(&mut self, sample: Sample) -> Result<()> {
        self.check_sample(&sample)?;
        let mut next = self.state.clone();
        next.samples.push(sample);
        self.commit(next)
    }
    pub fn begin_event(
        &mut self,
        id: String,
        started_utc: i64,
        conditions: OutcomeConditions,
        evidence: Vec<Sample>,
    ) -> Result<()> {
        self.text(&id)?;
        if id.is_empty() || started_utc < 0 || self.state.events.iter().any(|e| e.id == id) {
            return Err(TelemetryError::Invalid("event identity or time"));
        }
        self.check_conditions(&conditions)?;
        for sample in &evidence {
            self.check_sample(sample)?;
        }
        let mut next = self.state.clone();
        next.events.push(Event {
            id,
            started_utc,
            conditions,
            trade_id: None,
            evidence,
            finalized: None,
        });
        self.commit(next)
    }
    pub fn append_evidence(&mut self, id: &str, sample: Sample) -> Result<()> {
        self.check_sample(&sample)?;
        let mut next = self.state.clone();
        let event = next
            .events
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or(TelemetryError::NotFound)?;
        if event.finalized.is_some() {
            return Err(TelemetryError::AlreadyFinalized);
        }
        event.evidence.push(sample);
        self.commit(next)
    }
    /// Pin trade identity immediately, before waiting for broker outcome.
    pub fn bind_trade(&mut self, id: &str, trade_id: String) -> Result<()> {
        self.text(&trade_id)?;
        if trade_id.is_empty() {
            return Err(TelemetryError::Invalid("empty trade id"));
        }
        let mut next = self.state.clone();
        let event = next
            .events
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or(TelemetryError::NotFound)?;
        if event.finalized.is_some() {
            return Err(TelemetryError::AlreadyFinalized);
        }
        if event.trade_id.as_ref().is_some_and(|old| old != &trade_id) {
            return Err(TelemetryError::Invalid("trade identity change"));
        }
        event.trade_id = Some(trade_id);
        self.commit(next)
    }
    pub fn finalize(&mut self, id: &str, at_utc: i64, outcome: Outcome) -> Result<()> {
        let mut next = self.state.clone();
        let event = next
            .events
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or(TelemetryError::NotFound)?;
        if event.finalized.is_some() {
            return Err(TelemetryError::AlreadyFinalized);
        }
        self.check_outcome(event, at_utc, &outcome)?;
        event.finalized = Some(Finalized { at_utc, outcome });
        self.commit(next)
    }
    fn text(&self, text: &str) -> Result<()> {
        if text.len() > self.state.budgets.max_text_bytes {
            Err(TelemetryError::Invalid("text budget"))
        } else {
            Ok(())
        }
    }
    fn check_conditions(&self, c: &OutcomeConditions) -> Result<()> {
        for text in [
            &c.config_version,
            &c.entry_conditions,
            &c.exit_conditions,
            &c.account_currency,
        ] {
            self.text(text)?;
            if text.trim().is_empty() {
                return Err(TelemetryError::Invalid("missing outcome conditions"));
            }
        }
        Ok(())
    }
    fn check_sample(&self, s: &Sample) -> Result<()> {
        if s.source != self.state.source {
            return Err(TelemetryError::SourceMismatch);
        }
        if s.at_utc < 0 || !s.bid.is_finite() || !s.ask.is_finite() || s.bid <= 0.0 || s.ask < s.bid {
            return Err(TelemetryError::Invalid("quote or timestamp"));
        }
        match &s.dom {
            Dom::Missing { reason } => {
                self.text(reason)?;
                if reason.trim().is_empty() {
                    return Err(TelemetryError::Invalid("missing DOM reason"));
                }
            }
            Dom::Observed { bids, asks } => {
                if bids.is_empty()
                    || asks.is_empty()
                    || bids.len() > self.state.budgets.max_dom_levels
                    || asks.len() > self.state.budgets.max_dom_levels
                    || bids
                        .iter()
                        .chain(asks)
                        .any(|l| !l.price.is_finite() || !l.quantity.is_finite() || l.price <= 0.0 || l.quantity < 0.0)
                {
                    return Err(TelemetryError::Invalid("DOM levels"));
                }
            }
        }
        Ok(())
    }
    fn check_outcome(&self, event: &Event, at: i64, outcome: &Outcome) -> Result<()> {
        if at < event.started_utc {
            return Err(TelemetryError::Invalid("outcome before event"));
        }
        match outcome {
            Outcome::Unknown => return Err(TelemetryError::UnknownOutcome),
            Outcome::TradeClosed {
                trade_id,
                confirmed_closed,
                pnl,
            } => {
                self.text(trade_id)?;
                if !confirmed_closed || !pnl.valid() || event.trade_id.as_ref() != Some(trade_id) {
                    return Err(TelemetryError::Invalid("unconfirmed close, trade identity or P/L"));
                }
            }
            Outcome::Rejected { reason } | Outcome::Expired { reason } => {
                self.text(reason)?;
                if event.trade_id.is_some() || reason.trim().is_empty() {
                    return Err(TelemetryError::Invalid("bound trade cannot reject/expire"));
                }
            }
        }
        Ok(())
    }
    fn sample_count(state: &State) -> usize {
        state.samples.len() + state.events.iter().map(|e| e.evidence.len()).sum::<usize>()
    }
    fn validate(&self, state: &State) -> Result<()> {
        if state.schema != SCHEMA
            || state.events.len() > state.budgets.max_events
            || Self::sample_count(state) > state.budgets.max_samples
        {
            return Err(TelemetryError::Invalid("schema or count budget"));
        }
        for text in [
            &state.source.venue,
            &state.source.server,
            &state.source.account,
            &state.source.symbol,
            &state.source.feed,
        ] {
            self.text(text)?;
        }
        for sample in &state.samples {
            self.check_sample(sample)?;
        }
        let mut ids = std::collections::HashSet::new();
        for event in &state.events {
            self.text(&event.id)?;
            if event.id.is_empty() || event.started_utc < 0 || !ids.insert(&event.id) {
                return Err(TelemetryError::Invalid("event identity or time"));
            }
            self.check_conditions(&event.conditions)?;
            if let Some(id) = &event.trade_id {
                self.text(id)?;
                if id.is_empty() {
                    return Err(TelemetryError::Invalid("trade identity"));
                }
            }
            for sample in &event.evidence {
                self.check_sample(sample)?;
            }
            if let Some(finalized) = &event.finalized {
                self.check_outcome(event, finalized.at_utc, &finalized.outcome)?;
            }
        }
        Ok(())
    }
    fn commit(&mut self, mut next: State) -> Result<()> {
        let bytes = loop {
            let bytes = serde_json::to_vec(&next)?;
            if bytes.len() <= next.budgets.max_bytes
                && Self::sample_count(&next) <= next.budgets.max_samples
                && next.events.len() <= next.budgets.max_events
            {
                break bytes;
            }
            // Deterministic oldest unpinned first, regardless of arrival order.
            if let Some((index, _)) = next.samples.iter().enumerate().min_by_key(|(_, s)| s.at_utc) {
                next.samples.remove(index);
                next.evicted_samples = next.evicted_samples.saturating_add(1);
            } else if let Some((index, _)) = next
                .events
                .iter()
                .enumerate()
                .filter(|(_, e)| e.finalized.is_some())
                .min_by_key(|(_, e)| e.finalized.as_ref().unwrap().at_utc)
            {
                let removed = next.events.remove(index);
                next.evicted_samples = next.evicted_samples.saturating_add(removed.evidence.len() as u64);
                next.evicted_finalized_events = next.evicted_finalized_events.saturating_add(1);
            } else {
                return Err(TelemetryError::CapacityPinned);
            }
        };
        self.validate(&next)?;
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        let mut tmp_name = self.path.as_os_str().to_os_string();
        tmp_name.push(".tmp");
        let tmp = PathBuf::from(tmp_name);
        let mut file = OpenOptions::new().write(true).create(true).truncate(true).open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, &self.path)?;
        // Once renamed, disk and memory must agree even if directory fsync fails.
        self.state = next;
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        Ok(())
    }

    /// Ambient observations only; pinned evidence is intentionally not duplicated.
    /// Future samples excluded, inclusive lower bound and exclusive end. No DOM backfill.
    pub fn rolling_windows(&self, now_utc: i64) -> [WindowCoverage; 3] {
        WINDOW_HOURS.map(|hours| {
            let start = now_utc.saturating_sub((hours * 3600) as i64);
            let mut times = Vec::new();
            let mut dom_observed = 0;
            let mut dom_missing = 0;
            for s in &self.state.samples {
                if s.at_utc >= start && s.at_utc < now_utc {
                    times.push(s.at_utc);
                    match s.dom {
                        Dom::Observed { .. } => dom_observed += 1,
                        Dom::Missing { .. } => dom_missing += 1,
                    }
                }
            }
            times.sort_unstable();
            WindowCoverage {
                hours,
                start_utc: start,
                end_utc: now_utc,
                retained_samples: times.len(),
                dom_observed,
                dom_missing,
                first_utc: times.first().copied(),
                last_utc: times.last().copied(),
                largest_observed_gap_seconds: times.windows(2).map(|w| w[1] - w[0]).max(),
                evicted_samples_total: self.state.evicted_samples,
                complete_history: false,
                historical_dom_available: false,
                expected_tick_cadence_seconds: None,
            }
        })
    }

    /// Monday 00:00 UTC half-open week, grouped by persisted config AND currency.
    /// Counts rejected/expired signals but never assigns them imaginary trade P/L.
    /// Retention loss is explicit; these are retained-event summaries, not a ledger.
    pub fn weekly_summary(&self, any_utc: i64) -> WeeklySummary {
        let start = utc_week_start(any_utc);
        let end = start.saturating_add(WEEK);
        let mut groups: Vec<WeeklyGroup> = Vec::new();
        let mut excluded_unresolved = 0;
        for event in &self.state.events {
            let Some(f) = &event.finalized else {
                if event.started_utc >= start && event.started_utc < end {
                    excluded_unresolved += 1;
                }
                continue;
            };
            if f.at_utc < start || f.at_utc >= end {
                continue;
            }
            let index = groups
                .iter()
                .position(|g| {
                    g.config_version == event.conditions.config_version
                        && g.currency == event.conditions.account_currency
                })
                .unwrap_or_else(|| {
                    groups.push(WeeklyGroup {
                        config_version: event.conditions.config_version.clone(),
                        currency: event.conditions.account_currency.clone(),
                        closed_trades: 0,
                        rejected: 0,
                        expired: 0,
                        wins: 0,
                        losses: 0,
                        gross_pnl: 0.0,
                        costs: 0.0,
                        net_pnl: 0.0,
                    });
                    groups.len() - 1
                });
            let group = &mut groups[index];
            match &f.outcome {
                Outcome::TradeClosed {
                    confirmed_closed: true,
                    pnl,
                    ..
                } => {
                    group.closed_trades += 1;
                    group.gross_pnl += pnl.gross;
                    group.net_pnl += pnl.net();
                    group.costs += pnl.gross - pnl.net();
                    if pnl.net() > 0.0 {
                        group.wins += 1;
                    }
                    if pnl.net() < 0.0 {
                        group.losses += 1;
                    }
                }
                Outcome::Rejected { .. } => group.rejected += 1,
                Outcome::Expired { .. } => group.expired += 1,
                _ => {} // Defensive: validation never allows unknown/unconfirmed finalized outcomes.
            }
        }
        WeeklySummary { source: self.state.source.clone(), start_utc: start, end_utc: end,
            groups, excluded_unresolved, evicted_finalized_events_total: self.state.evicted_finalized_events,
            complete_history: false, recommendations: vec![
                "Review missing DOM and retention coverage before drawing conclusions.".into(),
                "Review net costs and rejected/expired signals by config version; manual approval required for any risk change.".into(),
            ] }
    }
}
/// Unix epoch was Thursday; Monday anchor is 1970-01-05 (345600 seconds).
pub fn utc_week_start(at_utc: i64) -> i64 {
    let offset = at_utc.rem_euclid(WEEK);
    at_utc.saturating_sub((offset - 345_600).rem_euclid(WEEK))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WindowCoverage {
    pub hours: u64,
    pub start_utc: i64,
    pub end_utc: i64,
    pub retained_samples: usize,
    pub dom_observed: usize,
    pub dom_missing: usize,
    pub first_utc: Option<i64>,
    pub last_utc: Option<i64>,
    pub largest_observed_gap_seconds: Option<i64>,
    pub evicted_samples_total: u64,
    /// Historical DOM cannot be synthesized; only retained live observations exist.
    pub historical_dom_available: bool,
    /// None means gaps are measured observations, not a claim about lost tick counts.
    pub expected_tick_cadence_seconds: Option<u64>,
    /// No expected cadence supplied; absence/eviction cannot be represented as full coverage.
    pub complete_history: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WeeklyGroup {
    pub config_version: String,
    pub currency: String,
    pub closed_trades: usize,
    pub rejected: usize,
    pub expired: usize,
    pub wins: usize,
    pub losses: usize,
    pub gross_pnl: f64,
    pub costs: f64,
    pub net_pnl: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WeeklySummary {
    pub source: SourceIdentity,
    pub start_utc: i64,
    pub end_utc: i64,
    pub groups: Vec<WeeklyGroup>,
    pub excluded_unresolved: usize,
    pub evicted_finalized_events_total: u64,
    pub complete_history: bool,
    /// Advisory strings only: no settings mutation, broker action or network access.
    pub recommendations: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "aegis-telemetry-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn path(&self) -> PathBuf {
            self.0.join("state.json")
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn source() -> SourceIdentity {
        SourceIdentity::mt5_xauusd("broker-server".into(), "account-1".into(), "live-feed".into())
    }
    fn sample(at_utc: i64) -> Sample {
        Sample {
            source: source(),
            at_utc,
            bid: 2000.0,
            ask: 2000.2,
            dom: Dom::Missing {
                reason: "MT5 feed supplies no depth".into(),
            },
        }
    }
    fn conditions() -> OutcomeConditions {
        OutcomeConditions {
            config_version: "v1".into(),
            entry_conditions: "spread <= cap".into(),
            exit_conditions: "broker reconciled".into(),
            account_currency: "USD".into(),
        }
    }
    fn closed(confirmed_closed: bool) -> Outcome {
        Outcome::TradeClosed {
            trade_id: "t1".into(),
            confirmed_closed,
            pnl: RealizedPnl {
                gross: 10.0,
                commission: 2.0,
                fees: 1.0,
                swap_cost: -0.5,
                other_cost: 0.5,
            },
        }
    }
    #[test]
    fn append_is_durable_and_restart_keeps_pin_despite_interrupted_temp() {
        let temp = Temp::new();
        let b = Budgets::default();
        let mut store = TelemetryStore::open(temp.path(), source(), b.clone()).unwrap();
        store.append_sample(sample(100)).unwrap();
        store
            .begin_event("e1".into(), 100, conditions(), vec![sample(100)])
            .unwrap();
        store.bind_trade("e1", "t1".into()).unwrap();
        store.append_evidence("e1", sample(101)).unwrap();
        drop(store);
        // Simulate death during write, before atomic rename. Committed state wins.
        fs::write(temp.0.join("state.json.tmp"), b"{partial").unwrap();
        let store = TelemetryStore::open(temp.path(), source(), b).unwrap();
        assert_eq!(store.ambient_samples().len(), 1);
        assert_eq!(store.events()[0].evidence.len(), 2);
        assert_eq!(store.events()[0].trade_id.as_deref(), Some("t1"));
        assert!(store.events()[0].finalized.is_none());
    }
    #[test]
    fn eviction_never_discards_active_evidence_and_capacity_fails_closed() {
        let temp = Temp::new();
        let b = Budgets {
            max_samples: 2,
            ..Budgets::default()
        };
        let mut store = TelemetryStore::open(temp.path(), source(), b.clone()).unwrap();
        store
            .begin_event("e1".into(), 100, conditions(), vec![sample(100)])
            .unwrap();
        store.append_sample(sample(101)).unwrap();
        store.append_sample(sample(102)).unwrap();
        assert_eq!(store.ambient_samples()[0].at_utc, 102);
        store.append_evidence("e1", sample(103)).unwrap();
        let before = fs::read(temp.path()).unwrap();
        assert!(matches!(
            store.append_evidence("e1", sample(104)),
            Err(TelemetryError::CapacityPinned)
        ));
        assert_eq!(fs::read(temp.path()).unwrap(), before);
        let reopened = TelemetryStore::open(temp.path(), source(), b).unwrap();
        assert_eq!(reopened.events()[0].evidence.len(), 2);
    }
    #[test]
    fn unknown_or_unconfirmed_trade_never_finalizes_or_rejects() {
        let temp = Temp::new();
        let mut store = TelemetryStore::open(temp.path(), source(), Budgets::default()).unwrap();
        store
            .begin_event("e1".into(), 100, conditions(), vec![sample(100)])
            .unwrap();
        store.bind_trade("e1", "t1".into()).unwrap();
        assert!(matches!(
            store.finalize("e1", 101, Outcome::Unknown),
            Err(TelemetryError::UnknownOutcome)
        ));
        assert!(store.finalize("e1", 101, closed(false)).is_err());
        assert!(store
            .finalize(
                "e1",
                101,
                Outcome::Rejected {
                    reason: "timeout".into()
                }
            )
            .is_err());
        assert!(store
            .finalize(
                "e1",
                101,
                Outcome::Expired {
                    reason: "timeout".into()
                }
            )
            .is_err());
        assert!(store.events()[0].finalized.is_none());
        store.finalize("e1", 102, closed(true)).unwrap();
        assert!(matches!(
            store.finalize("e1", 103, closed(true)),
            Err(TelemetryError::AlreadyFinalized)
        ));
    }
    #[test]
    fn weekly_utc_counts_all_terminal_signals_but_only_closed_trade_pnl() {
        let temp = Temp::new();
        let mut store = TelemetryStore::open(temp.path(), source(), Budgets::default()).unwrap();
        let monday = 345_600;
        for id in ["trade", "rejected", "expired", "unknown", "next-week"] {
            store.begin_event(id.into(), monday, conditions(), vec![]).unwrap();
        }
        store.bind_trade("trade", "t1".into()).unwrap();
        store.finalize("trade", monday + 100, closed(true)).unwrap();
        store
            .finalize("rejected", monday + 101, Outcome::Rejected { reason: "gate".into() })
            .unwrap();
        store
            .finalize("expired", monday + 102, Outcome::Expired { reason: "TTL".into() })
            .unwrap();
        store
            .finalize("next-week", monday + WEEK, Outcome::Expired { reason: "TTL".into() })
            .unwrap();
        let summary = store.weekly_summary(monday + 3600);
        assert_eq!((summary.start_utc, summary.end_utc), (monday, monday + WEEK));
        assert_eq!(summary.excluded_unresolved, 1);
        let g = &summary.groups[0];
        assert_eq!((g.closed_trades, g.rejected, g.expired, g.wins), (1, 1, 1, 1));
        assert_eq!((g.net_pnl, g.costs), (7.0, 3.0));
        assert_eq!(g.config_version, "v1");
        assert_eq!(utc_week_start(monday - 1), monday - WEEK);
    }
    #[test]
    fn windows_are_hours_with_missing_dom_and_no_future_samples() {
        let temp = Temp::new();
        let mut store = TelemetryStore::open(temp.path(), source(), Budgets::default()).unwrap();
        let now = 100_000;
        for at in [now - 10 * 3600, now - 3 * 3600, now - 2 * 3600, now - 1, now, now + 1] {
            store.append_sample(sample(at)).unwrap();
        }
        let w = store.rolling_windows(now);
        assert_eq!(
            w.map(|v| (v.hours, v.retained_samples, v.dom_missing)),
            [(2, 2, 2), (3, 3, 3), (10, 4, 4)]
        );
    }
    #[test]
    fn source_mixing_and_invalid_costs_are_rejected() {
        let temp = Temp::new();
        let b = Budgets::default();
        let mut store = TelemetryStore::open(temp.path(), source(), b.clone()).unwrap();
        store.append_sample(sample(100)).unwrap();
        let mut foreign = sample(101);
        foreign.source.server = "other".into();
        assert!(matches!(
            store.append_sample(foreign),
            Err(TelemetryError::SourceMismatch)
        ));
        let mut other = source();
        other.account = "account-2".into();
        assert!(matches!(
            TelemetryStore::open(temp.path(), other, b),
            Err(TelemetryError::SourceMismatch)
        ));
        store.begin_event("e".into(), 100, conditions(), vec![]).unwrap();
        store.bind_trade("e", "t1".into()).unwrap();
        let mut outcome = closed(true);
        if let Outcome::TradeClosed { pnl, .. } = &mut outcome {
            pnl.commission = f64::NAN;
        }
        assert!(store.finalize("e", 101, outcome).is_err());
    }
    #[test]
    fn byte_budget_and_finalized_eviction_are_bounded() {
        let temp = Temp::new();
        let b = Budgets {
            max_bytes: 2400,
            max_events: 1,
            ..Budgets::default()
        };
        let mut store = TelemetryStore::open(temp.path(), source(), b.clone()).unwrap();
        store
            .begin_event("old".into(), 100, conditions(), vec![sample(100)])
            .unwrap();
        store
            .finalize("old", 101, Outcome::Rejected { reason: "gate".into() })
            .unwrap();
        store
            .begin_event("active".into(), 102, conditions(), vec![sample(102)])
            .unwrap();
        assert_eq!(store.events().len(), 1);
        assert_eq!(store.events()[0].id, "active");
        for i in 0..50 {
            store.append_sample(sample(103 + i)).unwrap();
        }
        assert!(fs::metadata(temp.path()).unwrap().len() <= b.max_bytes as u64);
        assert_eq!(store.events()[0].evidence.len(), 1);
        assert_eq!(store.weekly_summary(102).evicted_finalized_events_total, 1);
    }
    #[test]
    fn weeks_old_active_evidence_survives_and_disk_quota_rejects_new_admissions() {
        let temp = Temp::new();
        let b = Budgets {
            max_bytes: 1800,
            max_events: 10,
            ..Budgets::default()
        };
        let mut store = TelemetryStore::open(temp.path(), source(), b.clone()).unwrap();
        store
            .begin_event("weeks-old".into(), 100, conditions(), vec![sample(100)])
            .unwrap();
        store.bind_trade("weeks-old", "t1".into()).unwrap();
        // Fill only pinned evidence until byte quota (not sample quota) is exhausted.
        let mut at = 101;
        while store.append_evidence("weeks-old", sample(at)).is_ok() {
            at += 1;
        }
        let before = fs::read(temp.path()).unwrap();
        assert!(matches!(
            store.begin_event("new".into(), 100 + 4 * WEEK, conditions(), vec![sample(100 + 4 * WEEK)]),
            Err(TelemetryError::CapacityPinned)
        ));
        assert_eq!(fs::read(temp.path()).unwrap(), before);
        let store = TelemetryStore::open(temp.path(), source(), b).unwrap();
        assert_eq!(store.events().len(), 1);
        assert!(store.events()[0].finalized.is_none());
        assert_eq!(store.events()[0].evidence[0].at_utc, 100);
        let windows = store.rolling_windows(100 + 4 * WEEK);
        assert!(windows
            .iter()
            .all(|w| w.retained_samples == 0 && !w.complete_history && !w.historical_dom_available));
        assert!(!store.events()[0].evidence.is_empty());
    }
    #[test]
    fn configs_and_currencies_remain_separate_and_terminal_conditions_persist() {
        let temp = Temp::new();
        let b = Budgets::default();
        let mut store = TelemetryStore::open(temp.path(), source(), b.clone()).unwrap();
        for (id, version, currency) in [("a", "v1", "USD"), ("b", "v2", "USD"), ("c", "v1", "EUR")] {
            let mut c = conditions();
            c.config_version = version.into();
            c.account_currency = currency.into();
            store.begin_event(id.into(), 345600, c, vec![]).unwrap();
            store
                .finalize(
                    id,
                    345601,
                    Outcome::Rejected {
                        reason: "spread gate".into(),
                    },
                )
                .unwrap();
        }
        let store = TelemetryStore::open(temp.path(), source(), b).unwrap();
        let summary = store.weekly_summary(345601);
        assert_eq!(summary.groups.len(), 3);
        assert!(summary
            .groups
            .iter()
            .all(|g| g.rejected == 1 && g.closed_trades == 0 && g.net_pnl == 0.0));
        assert_eq!(store.events()[0].conditions.entry_conditions, "spread <= cap");
    }
    #[test]
    fn io_failure_does_not_accept_append_and_corrupt_restart_fails_closed() {
        let temp = Temp::new();
        let b = Budgets::default();
        let mut store = TelemetryStore::open(temp.path(), source(), b.clone()).unwrap();
        fs::create_dir(temp.0.join("state.json.tmp")).unwrap();
        assert!(store.append_sample(sample(100)).is_err());
        assert!(store.ambient_samples().is_empty());
        fs::write(temp.path(), b"broken").unwrap();
        assert!(TelemetryStore::open(temp.path(), source(), b).is_err());
    }
}
