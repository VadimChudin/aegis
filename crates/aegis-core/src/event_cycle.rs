//! Opt-in Paper-only event coordinator. Sampled coverage is not full tick history.
use crate::{
    event_scoring as scoring, event_telemetry as telemetry,
    observer::{ClosedOutcome, ObserverConfig},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub struct EventCycle {
    pub store: telemetry::TelemetryStore,
    root: PathBuf,
    healthy: bool,
}
impl EventCycle {
    pub fn open(root: &Path, source: telemetry::SourceIdentity) -> Result<Self, String> {
        std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
        Ok(Self {
            store: telemetry::TelemetryStore::open(root.join("telemetry.json"), source, telemetry::Budgets::default())
                .map_err(|e| e.to_string())?,
            root: root.into(),
            healthy: true,
        })
    }
    /// Bounded source tape: actual observed ticks/DOM only, sampled every 30 seconds.
    pub fn source_sample(&mut self, sample: &crate::live_market::MarketSample) -> Result<(), String> {
        if self
            .store
            .ambient_samples()
            .last()
            .is_some_and(|s| (sample.quote.time_ms / 1000) as i64 - s.at_utc < 30)
        {
            return Ok(());
        }
        let result = (|| {
            use std::io::Write;
            let path = self.root.join("source-tape.jsonl");
            let mut record = serde_json::to_vec(sample).map_err(|e| e.to_string())?;
            record.push(b'\n');
            if record.len() > 256 * 1024 {
                return Err("Source sample exceeds 256 KiB; entries blocked without truncating evidence".into());
            }
            // 8 MiB can fill before a ten-hour window with real tick payloads.
            // Keep a hard bound, but permit the configured sampled windows.
            if std::fs::metadata(&path).map_or(0, |m| m.len()) + record.len() as u64 > 128 * 1024 * 1024 {
                return Err("Source tape quota exhausted; pending data preserved, entries blocked".into());
            }
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|e| e.to_string())?;
            file.write_all(&record).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            let dom = sample
                .book
                .as_ref()
                .map(|b| telemetry::Dom::Observed {
                    bids: b
                        .bids
                        .iter()
                        .take(50)
                        .map(|x| telemetry::Level {
                            price: x.price,
                            quantity: x.quantity,
                        })
                        .collect(),
                    asks: b
                        .asks
                        .iter()
                        .take(50)
                        .map(|x| telemetry::Level {
                            price: x.price,
                            quantity: x.quantity,
                        })
                        .collect(),
                })
                .unwrap_or(telemetry::Dom::Missing {
                    reason: sample.book_note.chars().take(256).collect(),
                });
            self.sample(telemetry::Sample {
                source: self.store.source().clone(),
                at_utc: (sample.quote.time_ms / 1000) as i64,
                bid: sample.quote.bid,
                ask: sample.quote.ask,
                dom,
            })
        })();
        self.healthy = result.is_ok();
        result
    }
    pub fn sample(&mut self, sample: telemetry::Sample) -> Result<(), String> {
        if self
            .store
            .ambient_samples()
            .last()
            .is_some_and(|s| sample.at_utc - s.at_utc < 30)
        {
            return Ok(());
        }
        self.store.append_sample(sample).map_err(|e| e.to_string())
    }
    pub fn candidate(
        &mut self,
        config: &ObserverConfig,
        strategy: &str,
        snapshot: u64,
        quote_ms: u64,
        now_ms: u64,
    ) -> Result<(scoring::ScoringContext, Value), String> {
        if !self.healthy || !config.event_paper_enabled || config.mode != "paper" {
            return Err("Event cycle requires opt-in Paper mode".into());
        }
        let now = (now_ms / 1000) as i64;
        let hours = config.event_window_hours;
        let start = now - (hours * 3600) as i64;
        // Keep the boundary predecessor to prove the requested hours were actually observed.
        let all = self.store.ambient_samples();
        let boundary = all
            .iter()
            .rposition(|s| s.at_utc <= start)
            .ok_or("Event sampled history warming up")?;
        let evidence = all[boundary..].to_vec();
        let coverage = evidence.first().is_some_and(|s| start - s.at_utc <= 45)
            && evidence.last().is_some_and(|s| now - s.at_utc <= 45)
            && evidence.windows(2).all(|s| s[1].at_utc - s[0].at_utc <= 45);
        // Explicitly permitted partial sampled coverage still requires a real hours boundary.
        if !coverage && !config.event_allow_partial_coverage {
            return Err("Sampled history has gaps over 45 seconds".into());
        }
        if now_ms.saturating_sub(quote_ms) > 10_000 || quote_ms > now_ms + 2000 {
            return Err("Event quote is stale".into());
        }
        let prefix = format!("{strategy}:");
        if self.store.events().iter().any(|e| {
            e.id.starts_with(&prefix)
                && (e.finalized.is_none() || now - e.started_utc < config.event_cooldown_seconds as i64)
        }) {
            return Err("Event cooldown or pending outcome".into());
        }
        let id = format!("{strategy}:{snapshot}");
        let source = self.store.source();
        let identity = format!("{}:{}:{}:{}", source.venue, source.server, source.account, source.feed);
        let context = scoring::ScoringContext {
            event_id: id.clone(),
            snapshot_id: snapshot.to_string(),
            source: scoring::SourceIdentity {
                connector_id: "authorized-mt5".into(),
                source_id: format!("{:x}", Sha256::digest(identity.as_bytes())),
                instrument: source.symbol.clone(),
            },
            snapshot_time_unix_ms: quote_ms,
            window: match hours {
                2 => scoring::ScoringWindow::TwoHours,
                3 => scoring::ScoringWindow::ThreeHours,
                10 => scoring::ScoringWindow::TenHours,
                _ => return Err("Unsupported event hours".into()),
            },
            max_source_age_ms: 10_000,
            source_authorized: true,
            snapshot_complete: coverage || config.event_allow_partial_coverage,
        };
        let slices:Vec<_>=evidence.chunks((evidence.len()/24).max(1)).map(|s|json!({"first_utc":s[0].at_utc,"last_utc":s.last().unwrap().at_utc,"count":s.len(),"bid_min":s.iter().map(|x|x.bid).fold(f64::INFINITY,f64::min),"ask_max":s.iter().map(|x|x.ask).fold(f64::NEG_INFINITY,f64::max),"live_dom_samples":s.iter().filter(|x|matches!(x.dom,telemetry::Dom::Observed{..})).count()})).collect();
        let data = json!({"strategy_id":strategy,"window_hours":hours,"coverage_policy":"sampled_30s_max_gap_45s","sampled_coverage_ready":coverage,"explicit_partial_permission":config.event_allow_partial_coverage,"full_tick_history":false,"historical_dom_invented":false,"slices":slices});
        self.store
            .begin_event(
                id,
                now,
                telemetry::OutcomeConditions {
                    config_version: format!(
                        "{:x}",
                        Sha256::digest(serde_json::to_vec(config).map_err(|e| e.to_string())?)
                    ),
                    entry_conditions: format!("{strategy}; local score strictly >8; sampled coverage"),
                    exit_conditions: "existing Paper simulator stops/targets/manual management".into(),
                    account_currency: "simulation equity units".into(),
                },
                evidence,
            )
            .map_err(|e| e.to_string())?;
        Ok((context, data))
    }
    pub fn admit_cloud(&self, config: &ObserverConfig, event_id: &str, now_ms: u64) -> Result<(), String> {
        let now = (now_ms / 1000) as i64;
        let path = self.root.join("cloud-reservations.json");
        let mut reservations: Vec<(String, i64)> = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string())?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.to_string()),
        };
        if reservations.iter().any(|(id, _)| id == event_id) {
            return Err("Cloud dispatch already reserved; never retry ambiguous dispatch".into());
        }
        reservations.retain(|(_, at)| *at > now - 3600);
        if reservations.len() >= config.event_cloud_calls_per_hour as usize {
            return Err("Event cloud hourly budget exhausted".into());
        }
        reservations.push((event_id.into(), now));
        let tmp = path.with_extension("tmp");
        use std::io::Write;
        let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        file.write_all(&serde_json::to_vec(&reservations).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(tmp, path).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn reject(&mut self, id: &str, now_ms: u64, reason: &str) -> Result<(), String> {
        if self
            .store
            .events()
            .iter()
            .any(|e| e.id == id && e.finalized.is_none() && e.trade_id.is_none())
        {
            self.store
                .finalize(
                    id,
                    (now_ms / 1000) as i64,
                    telemetry::Outcome::Rejected {
                        reason: reason.chars().take(256).collect(),
                    },
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    /// Idempotent journal reconciliation also repairs a crash between journal and telemetry writes.
    pub fn reconcile(&mut self, record: &Value) -> Result<(), String> {
        if record["kind"] == "decision" && record["source"] == "paper" {
            if let (Some(strategy), Some(snapshot)) = (record["strategy_id"].as_str(), record["snapshot_id"].as_u64()) {
                let id = format!("{strategy}:{snapshot}");
                if self.store.events().iter().any(|e| e.id == id && e.finalized.is_none()) {
                    if record["accepted"] == true
                        && matches!(record["decision"]["action"].as_str(), Some("long" | "short"))
                    {
                        if let Some(ticket) = record["sim_state"]["position"]["id"].as_u64() {
                            self.store
                                .bind_trade(&id, ticket.to_string())
                                .map_err(|e| e.to_string())?;
                        }
                    } else {
                        self.reject(
                            &id,
                            record["at_ms"].as_u64().unwrap_or(0),
                            record["reason"].as_str().unwrap_or("not entered"),
                        )?;
                    }
                }
            }
        }
        let value = if record["kind"] == "outcome" {
            &record["outcome"]
        } else {
            &record["paper_outcome"]
        };
        if !value.is_null() {
            let outcome: ClosedOutcome = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
            // A reduction is realized P/L but not terminal closure; never unpin its evidence.
            if record["sim_state"]["position"]["id"].as_u64() == Some(outcome.position_id) {
                return Ok(());
            }
            let ids: Vec<_> = self
                .store
                .events()
                .iter()
                .filter(|e| e.trade_id.as_deref() == Some(&outcome.position_id.to_string()) && e.finalized.is_none())
                .map(|e| e.id.clone())
                .collect();
            for id in ids {
                self.store
                    .finalize(
                        &id,
                        (outcome.closed_at_ms / 1000) as i64,
                        telemetry::Outcome::TradeClosed {
                            trade_id: outcome.position_id.to_string(),
                            confirmed_closed: true,
                            pnl: telemetry::RealizedPnl {
                                gross: outcome.pnl,
                                commission: 0.,
                                fees: 0.,
                                swap_cost: 0.,
                                other_cost: 0.,
                            },
                        },
                    )
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
    pub fn weekly_catchup(&self, now_utc: i64) -> Result<(), String> {
        let current = telemetry::utc_week_start(now_utc);
        let first = self
            .store
            .events()
            .iter()
            .map(|e| telemetry::utc_week_start(e.started_utc))
            .min()
            .unwrap_or(current);
        for week in (first.max(current - 52 * 604800)..current).step_by(604800) {
            let path = self.root.join(format!("week-{week}.json"));
            if path.exists() {
                continue;
            }
            let report = json!({"summary":self.store.weekly_summary(week),"recommendations_only":true,"automatic_risk_changes":false,"scope":"retained events, not full ledger","paper_pnl":"already net of simulator costs; no double counting"});
            let tmp = path.with_extension("tmp");
            std::fs::write(&tmp, serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            std::fs::rename(tmp, path).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        audit_http_fixture::reply,
        local_ai::Runtime,
        observer::{self, SimState},
    };
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    fn now() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
    }
    fn setup(stamp: u64) -> (PathBuf, EventCycle, ObserverConfig) {
        let path = std::env::temp_dir().join(format!("aegis-cycle-{}-{stamp}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let config = ObserverConfig {
            event_paper_enabled: true,
            strictness: 0,
            ..ObserverConfig::default()
        };
        let source =
            telemetry::SourceIdentity::mt5_xauusd("fixture".into(), "fixture-account".into(), "fixture-feed".into());
        let mut cycle = EventCycle::open(&path, source.clone()).unwrap();
        let end = (stamp / 1000) as i64;
        for at in (end - 7200..=end).step_by(30) {
            cycle
                .sample(telemetry::Sample {
                    source: source.clone(),
                    at_utc: at,
                    bid: 100.,
                    ask: 100.1,
                    dom: telemetry::Dom::Missing {
                        reason: "fixture no DOM".into(),
                    },
                })
                .unwrap();
        }
        (path, cycle, config)
    }
    fn score(context: &scoring::ScoringContext, value: f64) -> String {
        json!({"score":value,"rationale":"fixture sampled evidence, not probability","forecast_horizon":{"kind":"unknown"},"window":context.window,"event_id":context.event_id,"snapshot_id":context.snapshot_id,"source":context.source,"snapshot_time_unix_ms":context.snapshot_time_unix_ms}).to_string()
    }
    #[tokio::test]
    async fn full_candidate_score_cloud_paper_outcome_restart_week_replay() {
        let stamp = now();
        let (path, mut cycle, config) = setup(stamp);
        let (context, telemetry) = cycle.candidate(&config, "density_bounce", 7, stamp, stamp).unwrap();
        assert_eq!(telemetry["full_tick_history"], false);
        assert_ne!(context.source.source_id, "fixture-account");
        let (endpoint, worker) = reply(
            200,
            json!({"done":true,"done_reason":"stop","prompt_eval_count":100,"message":{"content":score(&context,9.)}})
                .to_string(),
            Duration::ZERO,
        );
        let result = Runtime::fixture(endpoint).score(&context, &telemetry).await.unwrap();
        let request = worker.join().unwrap();
        assert_eq!(request["stream"], false);
        assert!(scoring::cloud_eligible(&result, &context, now()));
        cycle.admit_cloud(&config, &context.event_id, stamp).unwrap();
        assert!(cycle.admit_cloud(&config, &context.event_id, stamp).is_err());
        let base = 1_700_001_500_000u64;
        let mut snapshot = observer::tests::snapshot(base);
        let delta = stamp - base;
        snapshot.market.observed_at_ms += delta;
        snapshot.market.quote.time_ms += delta;
        for tick in &mut snapshot.market.ticks {
            tick.time_ms += delta;
        }
        for frame in &mut snapshot.frames {
            frame.closed_at_ms += delta;
        }
        let decision = json!({"snapshot_id":snapshot.id,"action":"long","reason":"fixture","stop":99.,"target":102.,"position_id":null,"quantity_fraction":null,"used_timeframes":["1m"],"checks":[]});
        let (endpoint, worker) = reply(
            200,
            json!({"choices":[{"finish_reason":"stop","message":{"content":decision.to_string()}}]}).to_string(),
            Duration::ZERO,
        );
        let cloud = crate::ai_provider::audit_consult_at(&endpoint, &decision.to_string(), "fixture", 2)
            .await
            .unwrap();
        worker.join().unwrap();
        let confirmed = observer::parse_decision(&cloud.response).unwrap();
        let mut sim = SimState::new(config.initial_equity).unwrap();
        let strategy = config.strategy("density_bounce").unwrap();
        sim.apply(&snapshot, &config, strategy, &confirmed, &snapshot.market, stamp)
            .unwrap();
        let record = json!({"kind":"decision","source":"paper","accepted":true,"snapshot_id":7,"strategy_id":"density_bounce","decision":confirmed,"sim_state":sim,"at_ms":stamp});
        cycle.reconcile(&record).unwrap();
        cycle.reconcile(&record).unwrap();
        assert_eq!(cycle.store.events()[0].trade_id, Some("1".into()));
        let closed = sim.exit_at(&snapshot.market, &config, "fixture_close").unwrap();
        let record = json!({"kind":"outcome","outcome":closed,"sim_state":sim});
        cycle.reconcile(&record).unwrap();
        cycle.reconcile(&record).unwrap();
        assert!(cycle.store.events()[0].finalized.is_some());
        let source = cycle.store.source().clone();
        drop(cycle);
        let cycle = EventCycle::open(&path, source).unwrap();
        cycle.weekly_catchup((stamp / 1000) as i64 + 604800).unwrap();
        assert!(path
            .join(format!(
                "week-{}.json",
                telemetry::utc_week_start((stamp / 1000) as i64)
            ))
            .exists());
        assert_eq!(
            cycle.store.weekly_summary((stamp / 1000) as i64).groups[0].closed_trades,
            1
        );
        std::fs::remove_dir_all(path).unwrap();
    }
    #[tokio::test]
    async fn score_eight_and_truncated_completion_never_admitted() {
        let stamp = now() + 1;
        let (path, mut cycle, config) = setup(stamp);
        let (context, telemetry) = cycle.candidate(&config, "density_bounce", 7, stamp, stamp).unwrap();
        for reason in ["stop", "length"] {
            let(endpoint,worker)=reply(200,json!({"done":true,"done_reason":reason,"prompt_eval_count":100,"message":{"content":score(&context,8.)}}).to_string(),Duration::ZERO);
            let result = Runtime::fixture(endpoint).score(&context, &telemetry).await;
            worker.join().unwrap();
            if reason == "stop" {
                assert!(!scoring::cloud_eligible(&result.unwrap(), &context, now()));
            } else {
                assert!(result.is_err());
            }
        }
        assert!(!path.join("cloud-reservations.json").exists());
        cycle.reject(&context.event_id, stamp, "score not >8").unwrap();
        assert!(cycle.candidate(&config, "density_bounce", 8, stamp, stamp).is_err());
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn source_tape_quota_preserves_existing_file_and_blocks_admission() {
        let stamp = now() + 777;
        let (path, mut cycle, config) = setup(stamp);
        let tape = path.join("source-tape.jsonl");
        let file = std::fs::File::create(&tape).unwrap();
        file.set_len(128 * 1024 * 1024).unwrap();
        let (mut snapshot, _) = crate::audit_http_fixture::snapshot();
        snapshot.market.quote.time_ms = stamp + 31_000;
        snapshot.market.observed_at_ms = stamp + 31_000;
        assert!(cycle.source_sample(&snapshot.market).unwrap_err().contains("quota"));
        assert_eq!(std::fs::metadata(&tape).unwrap().len(), 128 * 1024 * 1024);
        assert!(cycle
            .candidate(&config, "density_bounce", 123, stamp + 31_000, stamp + 31_000)
            .is_err());
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn paper_opt_in_coverage_warmup_and_budget_fail_closed() {
        let mut config = ObserverConfig::default();
        assert!(!config.event_paper_enabled);
        config.event_paper_enabled = true;
        config.mode = "money".into();
        assert!(config.validate().is_err());
        let stamp = now() + 2;
        let (path, mut cycle, mut config) = setup(stamp);
        config.event_cloud_calls_per_hour = 1;
        let (context, _) = cycle.candidate(&config, "density_bounce", 7, stamp, stamp).unwrap();
        cycle.admit_cloud(&config, &context.event_id, stamp).unwrap();
        assert!(cycle.admit_cloud(&config, "other", stamp).is_err());
        config.event_window_hours = 3;
        assert!(cycle.candidate(&config, "breakout", 8, stamp, stamp).is_err());
        std::fs::remove_dir_all(path).unwrap();
    }
}
