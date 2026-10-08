use super::*;
use crate::audit_http_fixture::{decision, reply};

fn envelope() -> Value {
    json!({"done":true,"done_reason":"stop","prompt_eval_count":123,"message":{"content":decision()}})
}
async fn invoke(status: u16, body: String) -> Result<Answer, String> {
    let (url, worker) = reply(status, body, Duration::ZERO);
    let mut runtime = Runtime::new(std::env::temp_dir().join("aegis-audit-unused"));
    runtime.endpoint = url;
    let result = runtime.observe(r#"{"snapshot":{"id":7}}"#, "system fixture").await;
    worker.join().unwrap();
    result
}

#[tokio::test]
async fn audit_local_schema_prompt_snapshot_contract() {
    let (url, worker) = reply(200, envelope().to_string(), Duration::ZERO);
    let mut runtime = Runtime::new(std::env::temp_dir().join("aegis-audit-unused"));
    runtime.endpoint = url;
    let prompt = r#"{"snapshot":{"id":7}}"#;
    let answer = runtime.observe(prompt, "system fixture").await.unwrap();
    assert_eq!(
        crate::observer::parse_decision(&answer.response).unwrap().snapshot_id,
        7
    );
    let request = worker.join().unwrap();
    assert_eq!(request["model"], MODEL);
    assert_eq!(request["stream"], false);
    assert_eq!(request["think"], false);
    assert_eq!(request["options"]["num_ctx"], 4096);
    assert_eq!(request["options"]["num_predict"], 256);
    assert_eq!(request["options"]["temperature"], 0);
    assert_eq!(request["format"]["properties"]["snapshot_id"]["const"], 7);
    assert_eq!(request["format"]["additionalProperties"], false);
    assert_eq!(request["messages"][0]["content"], "system fixture");
    assert_eq!(request["messages"][1]["content"], prompt);
}

#[tokio::test]
async fn audit_local_http_malformed_empty_truncated_and_context_budget() {
    assert!(invoke(500, json!({"error":"fixture error"}).to_string()).await.is_err());
    assert!(invoke(200, "not json".into()).await.is_err());
    let mut value = envelope();
    value["message"]["content"] = json!("");
    assert!(invoke(200, value.to_string()).await.is_err());
    let mut value = envelope();
    value["done_reason"] = json!("length");
    assert!(invoke(200, value.to_string())
        .await
        .unwrap_err()
        .contains("token limit"));
    let mut value = envelope();
    value["prompt_eval_count"] = json!(3701);
    assert!(invoke(200, value.to_string())
        .await
        .unwrap_err()
        .contains("context budget"));
    let runtime = Runtime::new(std::env::temp_dir().join("aegis-audit-unused"));
    assert!(runtime
        .observe("{}", "s")
        .await
        .unwrap_err()
        .contains("snapshot id missing"));
    assert!(runtime
        .observe(&"x".repeat(14001), "s")
        .await
        .unwrap_err()
        .contains("context budget"));
}

#[tokio::test]
async fn audit_local_rejects_unfinished_response_even_with_valid_decision() {
    for done in [json!(false), Value::Null] {
        let mut value = envelope();
        value["done"] = done;
        assert!(
            invoke(200, value.to_string()).await.is_err(),
            "accepted unfinished Ollama response"
        );
    }
    for reason in ["error", "", "load"] {
        let mut value = envelope();
        value["done_reason"] = json!(reason);
        assert!(
            invoke(200, value.to_string()).await.is_err(),
            "accepted unsuccessful Ollama termination"
        );
    }
}

#[tokio::test]
async fn audit_actual_local_cloud_chain_agreement_and_rejection_matrix() {
    use crate::{
        ai_provider::audit_consult_at,
        audit_http_fixture::{agreement, snapshot},
        observer,
    };
    let (snapshot, now) = snapshot();
    let config = observer::ObserverConfig {
        strictness: 50,
        ..Default::default()
    };
    let strategy = config.strategy("density_bounce").unwrap();
    let system = observer::system_prompt(&config, strategy);
    let mut payload = observer::request_payload(&snapshot, strategy);
    observer::compact_payload(&mut payload);
    payload["positions"] = Value::Null;
    payload["snapshot"]["market"].as_object_mut().unwrap().remove("ticks");
    payload["snapshot"]["market"]
        .as_object_mut()
        .unwrap()
        .remove("book_note");
    let prompt = payload.to_string();
    let mut valid: Value = serde_json::from_str(&decision()).unwrap();
    valid["action"] = json!("long");
    valid["stop"] = json!(99.0);
    valid["target"] = json!(102.0);
    // Compose real transports, parser and safety gates. The app predicate is
    // extracted verbatim and source-checked; the Tauri event loop is not run.
    for (case, local_content, cloud_content, accepted) in [
        ("long agreement", valid.to_string(), valid.to_string(), true),
        ("wait agreement", decision(), decision(), true),
        ("action disagreement", valid.to_string(), decision(), false),
        ("malformed local decision", "not json".into(), valid.to_string(), false),
        ("malformed cloud decision", valid.to_string(), "not json".into(), false),
        (
            "both wrong snapshot",
            valid.to_string().replace("\"snapshot_id\":7", "\"snapshot_id\":8"),
            valid.to_string().replace("\"snapshot_id\":7", "\"snapshot_id\":8"),
            false,
        ),
        (
            "matching unsafe prices",
            valid.to_string().replace("\"stop\":99.0", "\"stop\":101.0"),
            valid.to_string().replace("\"stop\":99.0", "\"stop\":101.0"),
            false,
        ),
    ] {
        let mut local_reply = envelope();
        local_reply["message"]["content"] = json!(local_content);
        let (local_url, local_worker) = reply(200, local_reply.to_string(), Duration::ZERO);
        let mut runtime = Runtime::new(std::env::temp_dir().join("aegis-audit-unused"));
        runtime.endpoint = local_url;
        let local = runtime
            .observe(&prompt, &system)
            .await
            .and_then(|answer| observer::parse_decision(&answer.response));
        let request = local_worker.join().unwrap();
        assert_eq!(request["format"]["properties"]["snapshot_id"]["const"], snapshot.id);
        let result = match local {
            Err(error) => Err(error),
            Ok(local) => {
                let cloud_payload = json!({"market":payload,"local_proposal":local,
                    "instruction":"Independently check the SPA and supplied market. Return the same decision schema with the actual snapshot_id. Disagreement means wait."}).to_string();
                let cloud_reply =
                    json!({"choices":[{"finish_reason":"stop","message":{"content":cloud_content}}]}).to_string();
                let (cloud_url, cloud_worker) = reply(200, cloud_reply, Duration::ZERO);
                let response = audit_consult_at(&cloud_url, &cloud_payload, &system, 2).await;
                let request = cloud_worker.join().unwrap();
                let sent: Value = serde_json::from_str(request["messages"][1]["content"].as_str().unwrap()).unwrap();
                assert_eq!(sent["market"]["snapshot"]["id"], 7);
                assert_eq!(sent["local_proposal"], serde_json::to_value(&local).unwrap());
                response.and_then(|answer| {
                    let cloud = observer::parse_decision(&answer.response)?;
                    agreement(&local, &cloud)?;
                    observer::validate_decision(&snapshot, &config, strategy, &cloud, &snapshot.market, now)?;
                    observer::validate_decision(&snapshot, &config, strategy, &local, &snapshot.market, now)
                })
            }
        };
        assert_eq!(result.is_ok(), accepted, "{case}: {result:?}");
        println!(
            "fixture chain case: {case}: {}",
            if result.is_ok() { "accepted" } else { "rejected" }
        );
    }
}

#[tokio::test]
async fn audit_local_real_28_second_http_deadline() {
    let (url, worker) = reply(200, envelope().to_string(), Duration::from_secs(29));
    let mut runtime = Runtime::new(std::env::temp_dir().join("aegis-audit-unused"));
    runtime.endpoint = url;
    let started = Instant::now();
    let result = runtime.observe(r#"{"snapshot":{"id":7}}"#, "system fixture").await;
    let elapsed = started.elapsed();
    assert!(result.unwrap_err().contains("deadline"));
    assert!(elapsed >= Duration::from_secs(27) && elapsed < Duration::from_secs(29));
    worker.join().unwrap();
}

#[tokio::test]
async fn audit_data_closed_history_reaches_both_http_providers() {
    use crate::{ai_provider::audit_consult_at, audit_http_fixture::snapshot, observer};
    let (snapshot, _) = snapshot();
    let config = observer::ObserverConfig::default();
    let strategy = config.strategy("data").unwrap();
    let system = observer::system_prompt(&config, strategy);
    let mut payload = observer::request_payload(&snapshot, strategy);
    observer::compact_payload(&mut payload);
    let prompt = payload.to_string();
    assert!(prompt.len() + system.len() < 14_000);
    let (url, worker) = reply(200, envelope().to_string(), Duration::ZERO);
    let mut runtime = Runtime::new(std::env::temp_dir().join("aegis-audit-unused"));
    runtime.endpoint = url;
    let local = runtime.observe(&prompt, &system).await.unwrap();
    let request = worker.join().unwrap();
    let sent: Value = serde_json::from_str(request["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(
        sent["snapshot"]["frames"][0]["closed_bars"].as_array().unwrap().len(),
        20
    );
    assert_eq!(sent["snapshot"]["frames"][0]["closed_count"], 20);
    let local = observer::parse_decision(&local.response).unwrap();
    let cloud_payload = json!({"market":payload,"local_proposal":local,
        "instruction":"Independently check the SPA and supplied market. Return the same decision schema with the actual snapshot_id. Disagreement means wait."});
    let (url, worker) = reply(
        200,
        json!({"choices":[{"finish_reason":"stop","message":{"content":decision()}}]}).to_string(),
        Duration::ZERO,
    );
    audit_consult_at(&url, &cloud_payload.to_string(), &system, 2)
        .await
        .unwrap();
    let request = worker.join().unwrap();
    let sent: Value = serde_json::from_str(request["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(
        sent["market"]["snapshot"]["frames"][0]["closed_bars"]
            .as_array()
            .unwrap()
            .len(),
        20
    );
    assert_eq!(sent["market"]["snapshot"]["frames"][0]["closed_count"], 20);
    assert_eq!(sent["market"]["snapshot"]["id"], 7);
}
