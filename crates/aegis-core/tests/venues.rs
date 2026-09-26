//! Binance and Bybit connectors against `scripts/mock_venues.py`, which checks
//! the HMAC signatures with Python's own implementation.

use std::{
    net::TcpStream,
    path::PathBuf,
    process::{Child, Command},
    thread::sleep,
    time::Duration,
};

use aegis_core::{BrokerError, ConnectOptions, Connector, Credentials, Timeframe};

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
        bybit_url: Some(format!("http://127.0.0.1:{port}/bybit")),
        ..Default::default()
    }
}

fn creds(broker: &str, secret: &str) -> Credentials {
    serde_json::from_value(serde_json::json!({"broker": broker, "api_key": "test-key", "api_secret": secret})).unwrap()
}

async fn check_venue(broker: &str, port: u16) {
    let bad = Connector::connect(creds(broker, "wrong"), &options(port))
        .await
        .err()
        .expect("bad secret must fail");
    assert!(matches!(bad, BrokerError::Auth { .. }), "{broker}: {bad}");

    let (conn, summary) = Connector::connect(creds(broker, "test-secret"), &options(port))
        .await
        .expect("connect");
    assert_eq!(summary.symbol, "XAUUSDT");
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
}

#[tokio::test]
async fn binance_signs_verifies_and_reads_every_timeframe() {
    let _mock = start_mock(18765);
    check_venue("binance", 18765).await;
}

#[tokio::test]
async fn bybit_signs_verifies_and_reads_every_timeframe() {
    let _mock = start_mock(18766);
    check_venue("bybit", 18766).await;
}
