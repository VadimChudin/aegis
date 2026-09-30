//! Binance and Bybit connectors against `scripts/mock_venues.py`, which checks
//! the HMAC signatures with Python's own implementation.

use std::{
    net::TcpStream,
    path::PathBuf,
    process::{Child, Command},
    thread::sleep,
    time::Duration,
};

use aegis_core::{CheckStatus, ConnectOptions, ConnectReport, Connector, Credentials, Timeframe};

struct Mock(Child);

impl Drop for Mock {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_mock(port: u16) -> Mock {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/mock_venues.py");
    let python = std::env::var("AEGIS_TEST_PYTHON").unwrap_or_else(|_| "python3".into());
    let mock = Mock(
        Command::new(python)
            .arg(script)
            .arg(port.to_string())
            .spawn()
            .expect("start mock"),
    );
    for _ in 0..50 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return mock;
        }
        sleep(Duration::from_millis(100));
    }
    panic!("mock did not start");
}

fn options(port: u16) -> ConnectOptions {
    ConnectOptions {
        binance_url: Some(format!("http://127.0.0.1:{port}/binance")),
        binance_spot_url: Some(format!("http://127.0.0.1:{port}/binance")),
        bybit_url: Some(format!("http://127.0.0.1:{port}/bybit")),
        ..Default::default()
    }
}

fn creds(broker: &str, secret: &str) -> Credentials {
    serde_json::from_value(serde_json::json!({"broker": broker, "api_key": "test-key", "api_secret": secret})).unwrap()
}

fn status(report: &ConnectReport, id: &str) -> CheckStatus {
    report
        .checks
        .iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("no check {id}: {report:?}"))
        .status
}

async fn check_venue(broker: &str, port: u16, expected: &[(&str, CheckStatus)]) {
    let (conn, bad) = Connector::connect(creds(broker, "wrong"), &options(port)).await;
    assert!(
        conn.is_none() && !bad.connected,
        "{broker}: bad secret must not connect"
    );
    assert_eq!(status(&bad, "reach"), CheckStatus::Ok);
    assert_eq!(status(&bad, "key"), CheckStatus::Fail);
    assert!(bad
        .checks
        .iter()
        .skip_while(|c| c.id != "key")
        .skip(1)
        .all(|c| c.status == CheckStatus::Skip));

    let (conn, report) = Connector::connect(creds(broker, "test-secret"), &options(port)).await;
    let conn = conn.expect("connect");
    assert!(report.connected && report.ready, "{broker}: {report:?}");
    assert_eq!(report.symbol.as_deref(), Some("XAUUSDT"));
    for (id, want) in expected {
        assert_eq!(status(&report, id), *want, "{broker} {id}: {report:?}");
    }

    for tf in Timeframe::ALL {
        let bars = conn
            .candles(tf, 300)
            .await
            .unwrap_or_else(|e| panic!("{broker} {tf:?}: {e}"));
        assert_eq!(bars.len(), 300, "{broker} {tf:?}");
        assert!(
            bars.windows(2).all(|w| w[1].time - w[0].time == tf.seconds()),
            "{broker} {tf:?}: gaps or order"
        );
        let live = conn.candles(tf, 2).await.unwrap();
        assert_eq!(
            live[0].time, bars[298].time,
            "{broker} {tf:?}: live poll must continue the history"
        );
    }

    let book = conn
        .order_book()
        .await
        .unwrap_or_else(|e| panic!("{broker} order book: {e}"));
    assert_eq!(book.symbol, "XAUUSDT");
    assert!(book.timestamp > 0);
    assert_eq!(book.bids.len(), 4);
    assert_eq!(book.asks.len(), 4);
    assert!(book.bids.windows(2).all(|w| w[0].price > w[1].price));
    assert!(book.asks.windows(2).all(|w| w[0].price < w[1].price));
    assert_eq!(book.bids[0].quantity, 2.5);
    let bids: Vec<_> = book.bids.iter().map(|l| (l.price, l.quantity)).collect();
    let asks: Vec<_> = book.asks.iter().map(|l| (l.price, l.quantity)).collect();
    let (densities, _) = aegis_core::density::Tracker::default().sample(book.timestamp, &bids, &asks);
    assert_eq!(densities.len(), 2);
    assert_eq!(book.asks[0].price, 4293.2);
}

#[tokio::test]
async fn binance_checklist_and_every_timeframe() {
    let _mock = start_mock(18765);
    use CheckStatus::*;
    check_venue(
        "binance",
        18765,
        &[
            ("reach", Ok),
            ("clock", Ok),
            ("symbol", Ok),
            ("key", Ok),
            ("perms", Warn), // mock key: no IP restriction, futures off
            ("mode", Ok),
            ("balance", Ok),
        ],
    )
    .await;
}

#[tokio::test]
async fn bybit_checklist_and_every_timeframe() {
    let _mock = start_mock(18766);
    use CheckStatus::*;
    check_venue(
        "bybit",
        18766,
        &[
            ("reach", Ok),
            ("clock", Ok),
            ("symbol", Ok),
            ("key", Ok),
            ("uta", Ok),
            ("perms", Warn), // read-only
            ("ip", Warn),    // not bound
            ("balance", Ok),
        ],
    )
    .await;
}

#[tokio::test]
async fn unreachable_venue_fails_reach_and_skips_the_rest() {
    let opts = ConnectOptions {
        bybit_url: Some("http://127.0.0.1:9".into()),
        ..Default::default()
    };
    let (conn, report) = Connector::connect(creds("bybit", "x"), &opts).await;
    assert!(conn.is_none());
    assert_eq!(report.checks[0].status, CheckStatus::Fail);
    assert!(report.checks[1..].iter().all(|c| c.status == CheckStatus::Skip));
}
