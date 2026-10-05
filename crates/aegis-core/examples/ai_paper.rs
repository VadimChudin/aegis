//! Local-model smoke check using explicitly synthetic prices, never broker credentials.
use aegis_core::ai::{request_decision, AiSettings, PaperEngine, Snapshot};
use aegis_core::Candle;
use std::time::{SystemTime, UNIX_EPOCH};

#[tokio::main]
async fn main() -> Result<(), String> {
    let now = || {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    };
    let settings = AiSettings::default();
    let snapshot = Snapshot {
        id: 1,
        broker: "synthetic-smoke-test".into(),
        symbol: "TEST-XAU".into(),
        time: now(),
        bid: 2000.0,
        ask: 2000.2,
        candles: (0..20)
            .map(|i| Candle {
                time: (now() - ((20 - i) * 300)) as i64,
                open: 2000.0,
                high: 2001.0,
                low: 1999.0,
                close: 2000.0,
                volume: 10.0,
            })
            .collect(),
    };
    let mut engine = PaperEngine::new(&settings);
    let start = std::time::Instant::now();
    let (decision, _) = request_decision(&settings, &snapshot, &engine.state(), None, false).await?;
    println!(
        "Local model decision (synthetic data): {}",
        serde_json::to_string(&decision).map_err(|_| "serialization failed")?
    );
    println!("Elapsed seconds: {:.2}", start.elapsed().as_secs_f64());
    match engine.apply(decision, &snapshot, &settings, now()) {
        Ok(()) => println!("Paper validation accepted decision"),
        Err(error) => println!("Paper validation safely rejected decision: {error}"),
    }
    Ok(())
}
