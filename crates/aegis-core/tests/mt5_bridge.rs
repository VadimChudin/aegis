//! Drives the real Python bridge through the connector, with the fake MetaTrader5
//! module from `python/tests/fake_mt5` on PYTHONPATH.

use std::path::PathBuf;

use aegis_core::{broker::TradeRequest, CheckStatus, ConnectOptions, Connector, Credentials, Timeframe};

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
    let book = conn.order_book().await.expect("order book");
    assert_eq!(book.symbol, "XAUUSD.r");
    assert!(book.timestamp > 0);
    assert_eq!(book.bids[0].price, 4293.0);
    assert_eq!(book.asks[0].price, 4294.0);
    conn.close().await;
}

#[tokio::test]
async fn reads_live_market_sample_without_orders() {
    let (conn, report) = Connector::connect(creds("1", "good"), &options()).await;
    let conn = conn.unwrap_or_else(|| panic!("connect: {report:?}"));
    let sample = conn.market_sample().await.expect("market sample");
    assert_eq!(sample.symbol, "XAUUSD.r");
    assert!(sample.quote.bid > 0.0 && sample.quote.ask >= sample.quote.bid);
    assert!(!sample.ticks.is_empty());
    assert_eq!(sample.volume_kind, "mt5_ticks_not_exchange_tape");
    assert!(sample.book.is_some());
    sample
        .validate(sample.observed_at_ms)
        .expect("fresh market sample validates");
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
async fn reads_account_places_confirmed_order_and_closes_owned_position() {
    let ledger = std::env::temp_dir().join(format!("aegis-mt5-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ledger);
    std::fs::create_dir_all(&ledger).unwrap();
    std::env::set_var("AEGIS_TRADE_LEDGER_DIR", &ledger);
    let (conn, report) = Connector::connect(creds("2", "good"), &options()).await;
    let conn = conn.unwrap_or_else(|| panic!("connect: {report:?}"));
    let state = conn.trading_state().await.expect("live account state");
    assert_eq!(state.account.login, 2);
    assert!(state.account.trade_allowed);
    assert_eq!(conn.symbol(), "XAUUSD");
    let result = conn
        .place_order(&TradeRequest {
            request_id: "rust-order-1".into(),
            side: "long".into(),
            risk_pct: 1.0,
            stop: 4291.0,
            target: 4300.0,
            max_spread: 2.0,
            max_positions: 2,
            daily_loss_limit_pct: 5.0,
            confirm_real: true,
        })
        .await
        .expect("trade response");
    assert_eq!(result.status, "filled");
    let state = conn.trading_state().await.expect("account positions");
    assert_eq!(state.positions.len(), 1);
    let ticket = state.positions[0].ticket;
    let tightened = conn.modify_stop(ticket, 4291.5, None).await.expect("tighten stop");
    assert_eq!(tightened.status, "filled");
    let reduced = conn.reduce_position(ticket, 0.5).await.expect("reduce position");
    assert_eq!(reduced.status, "filled");
    let closed = conn.close_position(ticket).await.expect("close position");
    assert_eq!(closed.status, "filled");
    assert!(conn.close_all().await.unwrap().is_empty());
    assert!(conn.trading_state().await.unwrap().positions.is_empty());
    conn.close().await;
    std::env::remove_var("AEGIS_TRADE_LEDGER_DIR");
    let _ = std::fs::remove_dir_all(ledger);
}
