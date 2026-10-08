use aegis_core::{
    live_market::{MarketSample, Quote},
    observer::{ObserverConfig, PaperPosition, SimState},
};
fn sample(time: u64, bid: f64, ask: f64) -> MarketSample {
    MarketSample {
        symbol: "XAUUSD".into(),
        observed_at_ms: time,
        quote: Quote {
            time_ms: time,
            bid,
            ask,
        },
        ticks: vec![],
        book: None,
        book_note: "Fixture: broker depth unavailable".into(),
        volume_kind: "mt5_ticks_not_exchange_tape".into(),
    }
}
fn close_sequence(side: &str) -> (Vec<f64>, f64, f64) {
    let config = ObserverConfig {
        commission_per_oz: 0.3,
        slippage: 0.2,
        ..ObserverConfig::default()
    };
    let opened = 1_700_000_000_000;
    let position = PaperPosition {
        id: 42,
        strategy_id: "density_bounce".into(),
        symbol: "XAUUSD".into(),
        side: side.into(),
        entry: 100.,
        stop: if side == "long" { 95. } else { 105. },
        target: if side == "long" { 120. } else { 80. },
        quantity: 8.,
        opened_at_ms: opened,
        initial_risk: 40.,
    };
    let mut sim = SimState {
        equity: 10_000.,
        position: Some(position),
        next_id: 43,
    };
    let quotes = if side == "long" {
        [(103., 103.1), (105., 105.1), (106., 106.1)]
    } else {
        [(96.9, 97.), (94.9, 95.), (93.9, 94.)]
    };
    let first = sim
        .reduce_at(42, 0.25, &sample(opened + 1000, quotes[0].0, quotes[0].1), &config)
        .unwrap();
    assert_eq!(sim.position.as_ref().unwrap().quantity, 6.);
    let persisted = serde_json::to_vec(&sim).unwrap();
    sim = serde_json::from_slice(&persisted).unwrap();
    let second = sim
        .reduce_at(42, 0.5, &sample(opened + 2000, quotes[1].0, quotes[1].1), &config)
        .unwrap();
    assert_eq!(sim.position.as_ref().unwrap().quantity, 3.);
    let third = sim
        .exit_at(
            &sample(opened + 3000, quotes[2].0, quotes[2].1),
            &config,
            "terminal fixture",
        )
        .unwrap();
    assert!(sim.position.is_none());
    let portions = vec![first.pnl, second.pnl, third.pnl];
    let total: f64 = portions.iter().sum();
    assert!((sim.equity - 10_000. - total).abs() < 1e-9);
    // 2+3+3 ounces: favorable gross moves 3,5,6 minus exit slippage .2 and allocated roundtrip commission .6.
    let expected = (3. - 0.2 - 0.6) * 2. + (5. - 0.2 - 0.6) * 3. + (6. - 0.2 - 0.6) * 3.;
    (portions, total, expected)
}
#[test]
fn long_partial_and_terminal_realizations_match_equity_and_costs() {
    let (p, total, expected) = close_sequence("long");
    assert_eq!(p.len(), 3);
    assert!((total - expected).abs() < 1e-9);
    assert!((total - p[2]).abs() > 1.);
}
#[test]
fn short_partial_and_terminal_realizations_match_equity_and_costs() {
    let (p, total, expected) = close_sequence("short");
    assert_eq!(p.len(), 3);
    assert!((total - expected).abs() < 1e-9);
    assert!((total - p[2]).abs() > 1.);
}

#[test]
fn losing_partial_trade_sums_net_losses_without_double_counting_costs() {
    let config = ObserverConfig {
        commission_per_oz: 0.3,
        slippage: 0.2,
        ..ObserverConfig::default()
    };
    let time = 1_700_000_010_000;
    for side in ["long", "short"] {
        let position = PaperPosition {
            id: 7,
            strategy_id: "breakout".into(),
            symbol: "XAUUSD".into(),
            side: side.into(),
            entry: 100.,
            stop: if side == "long" { 90. } else { 110. },
            target: if side == "long" { 110. } else { 90. },
            quantity: 4.,
            opened_at_ms: time,
            initial_risk: 40.,
        };
        let mut sim = SimState {
            equity: 10_000.,
            position: Some(position),
            next_id: 8,
        };
        let first_quote = if side == "long" {
            sample(time + 1000, 99., 99.1)
        } else {
            sample(time + 1000, 100.9, 101.)
        };
        let second_quote = if side == "long" {
            sample(time + 2000, 98., 98.1)
        } else {
            sample(time + 2000, 101.9, 102.)
        };
        let first = sim.reduce_at(7, 0.5, &first_quote, &config).unwrap();
        let final_exit = sim.exit_at(&second_quote, &config, "loss fixture").unwrap();
        let total = first.pnl + final_exit.pnl;
        let expected = (-1. - 0.2 - 0.6) * 2. + (-2. - 0.2 - 0.6) * 2.;
        assert!((total - expected).abs() < 1e-9);
        assert!((sim.equity - 10_000. - total).abs() < 1e-9);
        assert!(total < 0.);
    }
}
