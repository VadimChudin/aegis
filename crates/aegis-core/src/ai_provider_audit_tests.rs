use super::*;
use crate::audit_http_fixture::{decision, reply};

fn envelope(content: &str, finish: &str) -> String {
    json!({"choices":[{"finish_reason":finish,"message":{"content":content}}]}).to_string()
}

#[tokio::test]
async fn audit_cloud_request_contract_and_parser() {
    let (url, worker) = reply(200, envelope(&decision(), "stop"), Duration::ZERO);
    let answer = consult_at(&url, "fixture-key", "snapshot fixture", "system fixture", 2)
        .await
        .unwrap();
    assert_eq!(parse_decision(&answer.response).unwrap().snapshot_id, 7);
    let request = worker.join().unwrap();
    assert_eq!(request["model"], ANTHROPIC_MODEL);
    assert_eq!(request["temperature"], 0);
    assert_eq!(request["max_tokens"], 512);
    assert_eq!(request["response_format"]["type"], "json_object");
    assert_eq!(
        request["messages"][0],
        json!({"role":"system","content":"system fixture"})
    );
    assert_eq!(
        request["messages"][1],
        json!({"role":"user","content":"snapshot fixture"})
    );
}

#[tokio::test]
async fn audit_cloud_http_error_invalid_json_missing_content_and_oversize() {
    for (status, body, expected) in [
        (401, "secret response body".into(), "HTTP 401"),
        (200, "not json".into(), "invalid JSON"),
        (200, json!({"choices":[]}).to_string(), "no decision content"),
        (200, envelope(&decision(), "length"), "truncated"),
        (200, "x".repeat(65537), "65536-byte"),
    ] {
        let (url, worker) = reply(status, body, Duration::ZERO);
        let error = consult_at(&url, "fixture-key", "p", "s", 2).await.unwrap_err();
        worker.join().unwrap();
        assert!(error.contains(expected), "{error}");
        assert!(!error.contains("secret response body"));
    }
}

#[tokio::test]
async fn audit_cloud_timeout_and_missing_key() {
    let (url, worker) = reply(200, envelope(&decision(), "stop"), Duration::from_millis(1200));
    assert!(consult_at(&url, "fixture-key", "p", "s", 1)
        .await
        .unwrap_err()
        .contains("timed out"));
    worker.join().unwrap();
    // No listener: missing key must fail before attempting a connection.
    assert!(consult_at("http://127.0.0.1:1", " ", "p", "s", 1)
        .await
        .unwrap_err()
        .contains("key is required"));
}

#[tokio::test]
async fn audit_cloud_rejects_failed_or_unfinished_envelopes_even_with_valid_decision() {
    for finish in ["error", "content_filter", "tool_calls", ""] {
        let (url, worker) = reply(200, envelope(&decision(), finish), Duration::ZERO);
        let result = consult_at(&url, "fixture-key", "p", "s", 2).await;
        worker.join().unwrap();
        assert!(result.is_err(), "accepted unsuccessful finish_reason={finish:?}");
    }
    let (url, worker) = reply(200, json!({"error":{"message":"fixture failure"},"choices":[{"finish_reason":"stop","message":{"content":decision()}}]}).to_string(), Duration::ZERO);
    let result = consult_at(&url, "fixture-key", "p", "s", 2).await;
    worker.join().unwrap();
    assert!(result.is_err(), "accepted an error envelope");
}
