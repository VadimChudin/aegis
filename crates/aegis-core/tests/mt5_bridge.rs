//! Drives the real Python bridge through the connector, with the fake MetaTrader5
//! module from `python/tests/fake_mt5` on PYTHONPATH.

use std::path::PathBuf;

use aegis_core::{CheckStatus, ConnectOptions, Connector, Credentials, Timeframe};

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

fn creds(login: &str, password: &str) -> Credentials {
    serde_json::from_value(serde_json::json!({
        "broker": "roboforex", "login": login, "password": password, "server": "RoboForex-ECN",
        "python": std::env::var("AEGIS_TEST_PYTHON").unwrap_or_default()
    }))
    .unwrap()
}

fn ids(report: &aegis_core::ConnectReport) -> Vec<(&str, CheckStatus)> {
    report.checks.iter().map(|c| (c.id.as_str(), c.status)).collect()
}

#[tokio::test]
async fn connects_with_terminal_checklist_and_utc_candles() {
    let (conn, report) = Connector::connect(creds("1", "good"), &options()).await;
    let conn = conn.unwrap_or_else(|| panic!("connect: {report:?}"));
    use CheckStatus::*;
    assert_eq!(
        ids(&report),
        [
            ("python", Ok),
            ("package", Ok),
            ("login", Ok),
            ("terminal", Ok),
            ("algo", Warn), // fake terminal has Algo Trading off
            ("trading", Ok),
            ("symbol", Ok),
            ("balance", Ok)
        ]
    );
    assert!(report.ready);
    assert_eq!(report.symbol.as_deref(), Some("XAUUSD.r"));
    assert!(
        report.account.as_deref().unwrap_or("").contains("RoboForex-ECN"),
        "{report:?}"
    );

    let bars = conn.candles(Timeframe::M15, 3).await.expect("candles");
    assert_eq!(
        bars.iter().map(|b| b.time).collect::<Vec<_>>(),
        [1_790_208_000, 1_790_208_900, 1_790_209_800]
    );
    conn.close().await;
}

#[tokio::test]
async fn wrong_password_fails_login_and_skips_the_terminal() {
    let (conn, report) = Connector::connect(creds("1", "bad"), &options()).await;
    assert!(conn.is_none() && !report.connected);
    use CheckStatus::*;
    assert_eq!(
        ids(&report),
        [("python", Ok), ("package", Ok), ("login", Fail), ("terminal", Skip)]
    );
    assert!(report.checks[2].detail.contains("Authorization failed"), "{report:?}");
}

#[tokio::test]
async fn non_numeric_login_is_rejected_before_starting_python() {
    let (conn, report) = Connector::connect(creds("abc", "x"), &options()).await;
    assert!(conn.is_none());
    assert_eq!(ids(&report), [("input", CheckStatus::Fail)]);
    assert_eq!(report.checks[0].detail, "MT5 login must be a number");
}

#[tokio::test]
async fn missing_python_is_a_failed_check() {
    let c: Credentials = serde_json::from_value(serde_json::json!({
        "broker": "roboforex", "login": "1", "password": "good", "server": "s", "python": "/nonexistent/python"
    }))
    .unwrap();
    let (conn, report) = Connector::connect(c, &options()).await;
    assert!(conn.is_none());
    assert_eq!(report.checks[0].status, CheckStatus::Fail);
    assert!(report.checks[0].detail.contains("Python was not found"), "{report:?}");
}

#[tokio::test]
async fn crashed_bridge_is_restarted_and_logged_back_in() {
    let (conn, report) = Connector::connect(creds("1", "good"), &options()).await;
    let conn = conn.unwrap_or_else(|| panic!("connect: {report:?}"));
    assert_eq!(conn.candles(Timeframe::M1, 2).await.expect("before").len(), 2);
    conn.kill_bridge_for_test().await;
    // Restarts are paced: right after the crash the feed sees a readable error...
    let err = conn.candles(Timeframe::M1, 2).await.expect_err("dead bridge");
    assert!(err.to_string().contains("restarting"), "{err}");
    // ...and once the pause is over the session works again without a new login.
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    let bars = conn.candles(Timeframe::M15, 3).await.expect("after restart");
    assert_eq!(bars[0].time, 1_790_208_000);
    conn.close().await;
}
