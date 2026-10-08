//! Fixed-model cloud decision support through OpenRouter.

use std::time::{Duration, Instant};

use reqwest::header;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::observer::{parse_decision, ModelDecision};

pub const OPENROUTER_CHAT_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
pub const ANTHROPIC_MODEL: &str = "anthropic/claude-fable-5";
const MAX_PROMPT_BYTES: usize = 32 * 1024;
const MAX_REQUEST_BYTES: usize = 36 * 1024;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_TIMEOUT_SECS: u64 = 60;
const MAX_COMPLETION_TOKENS: u32 = 512;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CloudSettings {
    #[serde(default = "default_model")]
    pub model: String,
}

fn default_model() -> String {
    ANTHROPIC_MODEL.into()
}

impl Default for CloudSettings {
    fn default() -> Self {
        Self { model: default_model() }
    }
}

impl CloudSettings {
    pub fn validate(&self) -> Result<(), String> {
        validate_model(&self.model)
    }
}

pub fn validate_model(model: &str) -> Result<(), String> {
    if model == ANTHROPIC_MODEL {
        Ok(())
    } else {
        Err(format!("only {ANTHROPIC_MODEL} is supported"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderAnswer {
    pub response: String,
    pub latency_ms: u64,
    pub model: String,
}

/// Request one bounded, JSON-formatted answer. The API key is sent only as a
/// bearer header; request failures deliberately omit provider response bodies.
pub async fn consult(key: &str, prompt: &str, system: &str, timeout_secs: u64) -> Result<ProviderAnswer, String> {
    consult_at(OPENROUTER_CHAT_URL, key, prompt, system, timeout_secs).await
}

async fn consult_at(
    endpoint: &str,
    key: &str,
    prompt: &str,
    system: &str,
    timeout_secs: u64,
) -> Result<ProviderAnswer, String> {
    CloudSettings::default().validate()?;
    if key.trim().is_empty() {
        return Err("OpenRouter API key is required".into());
    }
    if prompt.trim().is_empty() || system.trim().is_empty() {
        return Err("prompt and system prompt must not be empty".into());
    }
    if prompt.len().saturating_add(system.len()) > MAX_PROMPT_BYTES {
        return Err("combined prompt exceeds the 32768-byte limit".into());
    }
    if !(1..=MAX_TIMEOUT_SECS).contains(&timeout_secs) {
        return Err(format!("timeout must be between 1 and {MAX_TIMEOUT_SECS} seconds"));
    }

    let body = serde_json::to_vec(&json!({
        "model": ANTHROPIC_MODEL,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": prompt}
        ],
        "temperature": 0,
        "max_tokens": MAX_COMPLETION_TOKENS,
        "response_format": {"type": "json_object"}
    }))
    .map_err(|_| "failed to encode OpenRouter request".to_string())?;
    if body.len() > MAX_REQUEST_BYTES {
        return Err("OpenRouter request exceeds the size limit".into());
    }

    let timeout = Duration::from_secs(timeout_secs);
    let client = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(timeout.min(Duration::from_secs(5)))
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "failed to initialize OpenRouter HTTP client".to_string())?;
    let started = Instant::now();
    let mut response = client
        .post(endpoint)
        .header(header::CONTENT_TYPE, "application/json")
        .header("HTTP-Referer", "https://aegis.local")
        .header("X-Title", "AEGIS")
        .bearer_auth(key.trim())
        .body(body)
        .send()
        .await
        .map_err(|_| "OpenRouter request failed or timed out".to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("OpenRouter returned HTTP {}", status.as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err("OpenRouter response exceeds the 65536-byte limit".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "failed to read OpenRouter response".to_string())?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err("OpenRouter response exceeds the 65536-byte limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| "OpenRouter returned invalid JSON".to_string())?;
    if body.get("error").is_some_and(|error| !error.is_null()) {
        return Err("OpenRouter returned an error envelope; no action accepted".into());
    }
    let finish_reason = body.pointer("/choices/0/finish_reason").and_then(Value::as_str);
    if finish_reason == Some("length") {
        return Err("OpenRouter response was truncated".into());
    }
    let content = body
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| "OpenRouter response has no decision content".to_string())?;
    if content.len() > MAX_RESPONSE_BYTES {
        return Err("OpenRouter decision content exceeds the size limit".into());
    }
    if finish_reason != Some("stop") {
        return Err("OpenRouter response did not finish successfully; no action accepted".into());
    }
    Ok(ProviderAnswer {
        response: content.to_owned(),
        latency_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        model: ANTHROPIC_MODEL.into(),
    })
}

/// Convenience wrapper for callers consuming the observer decision schema.
pub async fn cloud_decision(key: &str, prompt: &str, system: &str) -> Result<ModelDecision, String> {
    let answer = consult(key, prompt, system, 25).await?;
    parse_decision(&answer.response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    #[test]
    fn settings_pin_model_and_reject_overrides() {
        assert_eq!(CloudSettings::default().model, "anthropic/claude-fable-5");
        assert!(CloudSettings::default().validate().is_ok());
        assert!(validate_model("anthropic/claude-sonnet-4.5").is_err());
    }

    #[tokio::test]
    async fn request_uses_fixed_model_json_and_header_only_secret() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
                let text = String::from_utf8_lossy(&request);
                if let Some((headers, body)) = text.split_once("\r\n\r\n") {
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    if body.len() >= length {
                        break;
                    }
                }
            }
            let text = String::from_utf8_lossy(&request);
            assert!(text.to_ascii_lowercase().contains("authorization: bearer test-secret"));
            let (_, body) = text.split_once("\r\n\r\n").unwrap();
            let body: Value = serde_json::from_str(body.trim()).unwrap();
            assert_eq!(body["model"], ANTHROPIC_MODEL);
            assert_eq!(body["max_tokens"], MAX_COMPLETION_TOKENS);
            assert_eq!(body["temperature"], 0);
            assert_eq!(body["response_format"]["type"], "json_object");
            let response = r#"{"choices":[{"finish_reason":"stop","message":{"content":"{\"ready\":true}"}}]}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
        });
        let answer = consult_at(
            &format!("http://{address}/chat"),
            "test-secret",
            "Return JSON",
            "JSON only",
            5,
        )
        .await
        .unwrap();
        server.join().unwrap();
        assert_eq!(answer.response, r#"{"ready":true}"#);
        assert_eq!(answer.model, ANTHROPIC_MODEL);
    }

    #[tokio::test]
    async fn rejects_invalid_call_parameters_without_network_access() {
        assert!(consult(" ", "x", "y", 5).await.is_err());
        assert!(consult("key", "x", "y", 0).await.is_err());
        assert!(consult("key", &"x".repeat(MAX_PROMPT_BYTES), "y", 5).await.is_err());
    }
}

#[cfg(test)]
#[path = "ai_provider_audit_tests.rs"]
mod audit_tests;

#[cfg(test)]
pub(crate) async fn audit_consult_at(
    endpoint: &str,
    prompt: &str,
    system: &str,
    timeout_secs: u64,
) -> Result<ProviderAnswer, String> {
    assert!(endpoint.starts_with("http://127.0.0.1:"));
    consult_at(endpoint, "fixture-key", prompt, system, timeout_secs).await
}
