//! Drives the real Python bridge through the connector, with the fake MetaTrader5
//! module from `python/tests/fake_mt5` on PYTHONPATH.

use std::path::PathBuf;

use aegis_core::{BrokerId, ConnectOptions, Connector, Credentials, Timeframe};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn options() -> ConnectOptions {
    std::env::set_var("PYTHONPATH", repo().join("python/tests/fake_mt5"));
    ConnectOptions {
        mt5_bridge_script: Some(repo().join("python/aegis_lab/bridges/mt5_bridge.py")),
        ..Default::default()
    }
}

fn creds(password: &str) -> Credentials {
    serde_json::from_value(serde_json::json!({
        "broker": "roboforex", "login": "1", "password": password, "server": "RoboForex-ECN",
        "python": std::env::var("AEGIS_TEST_PYTHON").unwrap_or_default()
    }))
    .unwrap()
}

#[tokio::test]
async fn connects_and_reads_utc_candles_through_the_bridge() {
    let (conn, summary) = Connector::connect(creds("good"), &options()).await.expect("connect");
    assert_eq!(summary.broker, BrokerId::Roboforex);
    assert_eq!(summary.symbol, "XAUUSD.r");
    assert!(summary.account.contains("RoboForex-ECN"), "{}", summary.account);

    let bars = conn.candles(Timeframe::M15, 3).await.expect("candles");
    assert_eq!(
        bars.iter().map(|b| b.time).collect::<Vec<_>>(),
        [1_790_208_000, 1_790_208_900, 1_790_209_800]
    );
    conn.close().await;
}

#[tokio::test]
async fn wrong_password_comes_back_as_an_error() {
    let err = Connector::connect(creds("bad"), &options())
        .await
        .err()
        .expect("must fail")
        .to_string();
    assert!(err.contains("Authorization failed"), "{err}");
}

#[tokio::test]
async fn non_numeric_login_is_rejected_before_starting_python() {
    let c: Credentials = serde_json::from_value(serde_json::json!({
        "broker": "roboforex", "login": "abc", "password": "x", "server": "s"
    }))
    .unwrap();
    let err = Connector::connect(c, &options())
        .await
        .err()
        .expect("must fail")
        .to_string();
    assert_eq!(err, "MT5 login must be a number");
}
