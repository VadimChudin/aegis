//! Local runtime smoke test; downloads are explicit, never part of cargo test.
use aegis_core::local_ai::{Progress, Runtime, ENDPOINT};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[tokio::main]
async fn main() -> Result<(), String> {
    let root = std::env::var_os("AEGIS_AI_DIR")
        .map(PathBuf::from)
        .ok_or("Set AEGIS_AI_DIR to the local model data directory")?;
    let mut runtime = Runtime::new(root);
    let mode = std::env::args().nth(1).unwrap_or_else(|| "status".into());
    if mode == "status" {
        println!(
            "{}",
            serde_json::to_string_pretty(&Runtime::inspect(ENDPOINT).await).unwrap()
        );
        return Ok(());
    }
    let result = match mode.as_str() {
        "install" => {
            let progress = Arc::new(Mutex::new(Progress::default()));
            let monitor = progress.clone();
            let task = tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    println!("{}", serde_json::to_string(&*monitor.lock().unwrap()).unwrap());
                }
            });
            let result = runtime.install(&progress).await;
            task.abort();
            result
        }
        "ping" => {
            runtime.start().await?;
            runtime
                .ping()
                .await
                .map(|answer| println!("{}", serde_json::to_string(&answer).unwrap()))
        }
        _ => Err("Use status, install or ping".into()),
    };
    let mut status = Runtime::inspect(ENDPOINT).await;
    status.installed = runtime.installed();
    status.owned_server = runtime.owned();
    println!("{}", serde_json::to_string_pretty(&status).unwrap());
    if runtime.owned() {
        runtime.stop().await?;
    }
    result
}
