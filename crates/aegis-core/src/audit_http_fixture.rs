//! Audit-only loopback fixture. Never calls a model or broker.
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    thread,
    time::Duration,
};

pub(crate) fn reply(status: u16, body: String, delay: Duration) -> (String, thread::JoinHandle<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(line.starts_with("POST "));
        let mut length = 0;
        loop {
            line.clear();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse::<usize>().unwrap();
            }
        }
        let mut request = vec![0; length];
        reader.read_exact(&mut request).unwrap();
        let request: Value = serde_json::from_slice(&request).unwrap();
        thread::sleep(delay);
        // A timed-out client may already have closed the connection.
        let _ = write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        request
    });
    (endpoint, worker)
}

pub(crate) fn decision() -> String {
    serde_json::json!({"snapshot_id":7,"action":"wait","reason":"fixture","stop":null,"target":null,"position_id":null,"quantity_fraction":null,"used_timeframes":["1m"],"checks":[]}).to_string()
}

use crate::observer::ModelDecision;
include!("audit_app_agree.rs");

pub(crate) fn agreement(local: &ModelDecision, cloud: &ModelDecision) -> Result<(), String> {
    agree(local, cloud)
}

#[test]
fn audit_agreement_harness_is_exact_app_source() {
    let source = include_str!("../../../app/src-tauri/src/observer_cmd.rs");
    let start = source.find("fn agree(").unwrap();
    let end = start + source[start..].find("\n}\n").unwrap() + 2;
    assert_eq!(&source[start..end], include_str!("audit_app_agree.rs").trim_end());
    assert!(source.contains("agree(&local_decision, &confirmed)?;"));
    assert!(source.contains("None => Err(\"OpenRouter key unavailable; no new entries\".into())"));
}

#[test]
fn audit_agreement_exact_fields_and_allowed_explanatory_differences() {
    let local = crate::observer::parse_decision(&decision()).unwrap();
    let mut cloud = local.clone();
    cloud.reason = "independent explanation".into();
    assert!(agreement(&local, &cloud).is_ok());
    let mut changes = vec![];
    let mut d = local.clone();
    d.snapshot_id = 8;
    changes.push(d);
    let mut d = local.clone();
    d.action = "long".into();
    changes.push(d);
    let mut d = local.clone();
    d.position_id = Some(1);
    changes.push(d);
    let mut d = local.clone();
    d.stop = Some(99.0);
    changes.push(d);
    let mut d = local.clone();
    d.target = Some(102.0);
    changes.push(d);
    let mut d = local.clone();
    d.quantity_fraction = Some(0.5);
    changes.push(d);
    for cloud in changes {
        assert!(agreement(&local, &cloud).is_err());
    }
    let mut local = local;
    local.stop = Some(99.0);
    let mut cloud = local.clone();
    cloud.stop = Some(99.000000001);
    assert!(agreement(&local, &cloud).is_err(), "price comparison must be exact");
}

pub(crate) fn snapshot() -> (crate::observer::Snapshot, u64) {
    use crate::{
        live_market::{MarketSample, Quote, Tick},
        Candle, Timeframe,
    };
    let base = 1_700_000_000;
    let now = (base as u64 + 24 * 60) * 1000;
    let market = MarketSample {
        symbol: "XAUUSD".into(),
        observed_at_ms: now,
        quote: Quote {
            time_ms: now,
            bid: 100.0,
            ask: 100.2,
        },
        ticks: vec![Tick {
            time_ms: now,
            bid: 100.0,
            ask: 100.2,
            last: None,
            volume: None,
        }],
        book: None,
        book_note: "fixture: no DOM".into(),
        volume_kind: "mt5_ticks_not_exchange_tape".into(),
    };
    let bars: Vec<Candle> = (0..24)
        .map(|i| {
            let open = 100.0 + i as f64 * 0.01;
            Candle {
                time: base + i * 60,
                open,
                high: open + 0.5,
                low: open - 0.5,
                close: open + 0.1,
                volume: 1.0,
            }
        })
        .collect();
    let frame = crate::observer::summarize(Timeframe::M1, &bars, now).unwrap();
    (
        crate::observer::Snapshot {
            schema_version: 1,
            id: 7,
            market,
            frames: vec![frame],
        },
        now,
    )
}

#[test]
fn audit_compact_payload_preserves_closed_bar_evidence() {
    let (snapshot, _) = snapshot();
    let config = crate::observer::ObserverConfig::default();
    for name in ["density_bounce", "data"] {
        let strategy = config.strategy(name).unwrap();
        let mut payload = crate::observer::request_payload(&snapshot, strategy);
        let original = payload["snapshot"]["frames"][0].clone();
        crate::observer::compact_payload(&mut payload);
        let compact = &payload["snapshot"]["frames"][0];
        assert_eq!(
            compact["context_candles"], original["context_candles"],
            "lost closed-bar context for {name}"
        );
        if name == "data" {
            assert_eq!(original["closed_count"], 20);
            assert_eq!(compact["closed_bars"], original["closed_bars"]);
            assert_eq!(compact["closed_count"], original["closed_count"]);
        }
    }
    let source = include_str!("../../../app/src-tauri/src/observer_cmd.rs");
    assert!(source.contains("observer::compact_payload(&mut input);"));
}

#[test]
fn audit_decision_schema_rejection_matrix() {
    use crate::observer::parse_decision;
    let valid: Value = serde_json::from_str(&decision()).unwrap();
    let mut malformed = vec!["```json\n{}\n```".into(), "not JSON".into()];
    let mut v = valid.clone();
    v["extra"] = serde_json::json!(1);
    malformed.push(v.to_string());
    let mut v = valid.clone();
    v["snapshot_id"] = serde_json::json!("7");
    malformed.push(v.to_string());
    let mut v = valid.clone();
    v["snapshot_id"] = serde_json::json!(-1);
    malformed.push(v.to_string());
    let mut v = valid.clone();
    v.as_object_mut().unwrap().remove("checks");
    malformed.push(v.to_string());
    let mut v = valid.clone();
    v["used_timeframes"] = serde_json::json!(["unsupported"]);
    malformed.push(v.to_string());
    let mut v = valid.clone();
    v["checks"] = serde_json::json!([{"rule":"level","met":"yes","evidence":"x"}]);
    malformed.push(v.to_string());
    let mut v = valid.clone();
    v["checks"] = serde_json::json!([{"rule":"level","met":true,"evidence":"x","extra":1}]);
    malformed.push(v.to_string());
    let mut v = valid.clone();
    v["quantity_fraction"] = serde_json::json!(1.01);
    malformed.push(v.to_string());
    malformed.push(decision().replacen("\"snapshot_id\":7", "\"snapshot_id\":7,\"snapshot_id\":7", 1));
    for value in malformed {
        assert!(parse_decision(&value).is_err(), "accepted malformed schema: {value}");
    }
}

#[test]
fn audit_decision_stale_quote_wrong_snapshot_and_timeframes() {
    use crate::observer::{parse_decision, validate_decision, ObserverConfig};
    let (snapshot, now) = snapshot();
    let config = ObserverConfig {
        strictness: 50,
        ..Default::default()
    };
    let strategy = config.strategy("density_bounce").unwrap();
    let d = parse_decision(&decision()).unwrap();
    assert!(validate_decision(&snapshot, &config, strategy, &d, &snapshot.market, now).is_ok());
    assert!(validate_decision(&snapshot, &config, strategy, &d, &snapshot.market, now + 60_000).is_err());
    let mut changes = vec![];
    let mut d = d.clone();
    d.snapshot_id = 8;
    changes.push(d);
    let mut d = parse_decision(&decision()).unwrap();
    d.action = "execute".into();
    changes.push(d);
    let mut d = parse_decision(&decision()).unwrap();
    d.used_timeframes.clear();
    changes.push(d);
    let mut d = parse_decision(&decision()).unwrap();
    d.used_timeframes.push(crate::Timeframe::M1);
    changes.push(d);
    let mut d = parse_decision(&decision()).unwrap();
    d.used_timeframes = vec![crate::Timeframe::H1];
    changes.push(d);
    let mut d = parse_decision(&decision()).unwrap();
    d.stop = Some(99.0);
    changes.push(d);
    for d in changes {
        assert!(validate_decision(&snapshot, &config, strategy, &d, &snapshot.market, now).is_err());
    }
    let mut wide = snapshot.market.clone();
    wide.quote.ask = 110.0;
    assert!(validate_decision(
        &snapshot,
        &config,
        strategy,
        &parse_decision(&decision()).unwrap(),
        &wide,
        now
    )
    .is_err());
}
