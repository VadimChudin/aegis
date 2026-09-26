//! Binance USDⓈ-M futures: XAUUSDT perpetual (TradFi perps, listed January 2026).

use std::time::Duration;

use serde_json::Value;

use crate::{
    broker::{network, read_json, require, BrokerError},
    market::{int, num, Candle, Timeframe},
    sign::{hmac_sha256_hex, now_ms},
};

pub(crate) const SYMBOL: &str = "XAUUSDT";
const VENUE: &str = "Binance";
const DEFAULT_URL: &str = "https://fapi.binance.com";
const MAX_LIMIT: usize = 1500;

pub struct Binance {
    http: reqwest::Client,
    base: String,
    key: String,
    secret: String,
}

impl Binance {
    pub(crate) fn new(base: Option<String>, key: &str, secret: &str) -> Result<Self, BrokerError> {
        Ok(Self {
            http: http_client()?,
            base: base
                .unwrap_or_else(|| DEFAULT_URL.into())
                .trim_end_matches('/')
                .to_string(),
            key: require(key, "API key")?,
            secret: require(secret, "API secret")?,
        })
    }

    /// Signed balance request: proves the key works and names the account.
    pub(crate) async fn verify(&self) -> Result<String, BrokerError> {
        let query = format!("recvWindow=5000&timestamp={}", now_ms());
        let signature = hmac_sha256_hex(&self.secret, &query);
        let url = format!("{}/fapi/v2/balance?{query}&signature={signature}", self.base);
        let resp = self
            .http
            .get(url)
            .header("X-MBX-APIKEY", &self.key)
            .send()
            .await
            .map_err(network)?;
        let (status, body) = read_json(VENUE, resp).await?;
        if !status.is_success() {
            return Err(api_error(&body, status));
        }
        let usdt = body
            .as_array()
            .and_then(|assets| assets.iter().find(|a| a["asset"] == "USDT"))
            .and_then(|a| num(&a["balance"]));
        Ok(match usdt {
            Some(b) => format!("USDT {b:.2}"),
            None => "futures account".into(),
        })
    }

    pub(crate) async fn candles(&self, tf: Timeframe, limit: usize) -> Result<Vec<Candle>, BrokerError> {
        let url = format!(
            "{}/fapi/v1/klines?symbol={SYMBOL}&interval={}&limit={}",
            self.base,
            tf.as_str(),
            limit.clamp(1, MAX_LIMIT)
        );
        let resp = self.http.get(url).send().await.map_err(network)?;
        let (status, body) = read_json(VENUE, resp).await?;
        if !status.is_success() {
            return Err(api_error(&body, status));
        }
        parse_klines(&body)
    }
}

pub(crate) fn http_client() -> Result<reqwest::Client, BrokerError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("AEGIS/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(network)
}

fn api_error(body: &Value, status: reqwest::StatusCode) -> BrokerError {
    let code = body["code"].as_i64().unwrap_or(status.as_u16() as i64);
    let message = body["msg"].as_str().unwrap_or("request failed").to_string();
    match code {
        -2014 | -2015 | -1022 => BrokerError::Auth { venue: VENUE, message },
        -1021 => BrokerError::Api {
            venue: VENUE,
            code,
            message: format!("{message}; sync the computer clock"),
        },
        _ => BrokerError::Api {
            venue: VENUE,
            code,
            message,
        },
    }
}

/// `[[openTime, "open", "high", "low", "close", "volume", closeTime, ...], ...]`
pub(crate) fn parse_klines(body: &Value) -> Result<Vec<Candle>, BrokerError> {
    let rows = body
        .as_array()
        .ok_or_else(|| BrokerError::Parse("Binance klines: not an array".into()))?;
    rows.iter()
        .map(|r| {
            let bar = (|| {
                Some(Candle {
                    time: int(r.get(0)?)? / 1000,
                    open: num(r.get(1)?)?,
                    high: num(r.get(2)?)?,
                    low: num(r.get(3)?)?,
                    close: num(r.get(4)?)?,
                    volume: num(r.get(5)?)?,
                })
            })();
            bar.ok_or_else(|| BrokerError::Parse(format!("Binance kline row: {r}")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_futures_klines() {
        let body: Value = serde_json::from_str(
            r#"[[1765440300000,"4217.00","4300.00","4100.00","4235.00","11.504",1765440359999,"48643.1",23,"6.1","25800.3","0"]]"#,
        )
        .unwrap();
        let c = parse_klines(&body).unwrap();
        assert_eq!(
            c,
            vec![Candle {
                time: 1_765_440_300,
                open: 4217.0,
                high: 4300.0,
                low: 4100.0,
                close: 4235.0,
                volume: 11.504
            }]
        );
    }

    #[test]
    fn maps_key_errors_to_auth() {
        let body: Value =
            serde_json::from_str(r#"{"code":-2015,"msg":"Invalid API-key, IP, or permissions for action."}"#).unwrap();
        assert!(matches!(
            api_error(&body, reqwest::StatusCode::UNAUTHORIZED),
            BrokerError::Auth { .. }
        ));
    }
}
