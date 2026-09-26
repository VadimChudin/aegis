//! Bybit V5, linear USDT perpetual XAUUSDT.

use serde_json::Value;

use crate::{
    binance::http_client,
    broker::{network, read_json, require, BrokerError},
    market::{int, num, Candle, Timeframe},
    sign::{hmac_sha256_hex, now_ms},
};

pub(crate) const SYMBOL: &str = "XAUUSDT";
const VENUE: &str = "Bybit";
const DEFAULT_URL: &str = "https://api.bybit.com";
const RECV_WINDOW: &str = "5000";
const MAX_LIMIT: usize = 1000;

pub struct Bybit {
    http: reqwest::Client,
    base: String,
    key: String,
    secret: String,
}

fn interval(tf: Timeframe) -> &'static str {
    match tf {
        Timeframe::M1 => "1",
        Timeframe::M5 => "5",
        Timeframe::M15 => "15",
        Timeframe::H1 => "60",
        Timeframe::H4 => "240",
        Timeframe::D1 => "D",
    }
}

/// V5 GET signature: HMAC-SHA256 of `timestamp + apiKey + recvWindow + queryString`.
pub(crate) fn signature(secret: &str, timestamp: i64, key: &str, query: &str) -> String {
    hmac_sha256_hex(secret, &format!("{timestamp}{key}{RECV_WINDOW}{query}"))
}

impl Bybit {
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

    pub(crate) async fn verify(&self) -> Result<String, BrokerError> {
        let query = "accountType=UNIFIED";
        let ts = now_ms();
        let resp = self
            .http
            .get(format!("{}/v5/account/wallet-balance?{query}", self.base))
            .header("X-BAPI-API-KEY", &self.key)
            .header("X-BAPI-TIMESTAMP", ts.to_string())
            .header("X-BAPI-RECV-WINDOW", RECV_WINDOW)
            .header("X-BAPI-SIGN", signature(&self.secret, ts, &self.key, query))
            .send()
            .await
            .map_err(network)?;
        let body = checked(read_json(VENUE, resp).await?)?;
        let equity = num(&body["result"]["list"][0]["totalEquity"]);
        Ok(match equity {
            Some(e) => format!("Equity {e:.2} USD"),
            None => "unified account".into(),
        })
    }

    pub(crate) async fn candles(&self, tf: Timeframe, limit: usize) -> Result<Vec<Candle>, BrokerError> {
        let url = format!(
            "{}/v5/market/kline?category=linear&symbol={SYMBOL}&interval={}&limit={}",
            self.base,
            interval(tf),
            limit.clamp(1, MAX_LIMIT)
        );
        let resp = self.http.get(url).send().await.map_err(network)?;
        parse_klines(&checked(read_json(VENUE, resp).await?)?)
    }
}

fn checked((status, body): (reqwest::StatusCode, Value)) -> Result<Value, BrokerError> {
    let code = body["retCode"]
        .as_i64()
        .unwrap_or(if status.is_success() { 0 } else { status.as_u16() as i64 });
    if code == 0 {
        return Ok(body);
    }
    let message = body["retMsg"].as_str().unwrap_or("request failed").to_string();
    Err(match code {
        10003 | 10004 | 10005 | 10009 | 33004 => BrokerError::Auth { venue: VENUE, message },
        10002 => BrokerError::Api {
            venue: VENUE,
            code,
            message: format!("{message}; sync the computer clock"),
        },
        _ => BrokerError::Api {
            venue: VENUE,
            code,
            message,
        },
    })
}

/// `result.list` holds `[startTime, open, high, low, close, volume, turnover]`, newest first.
pub(crate) fn parse_klines(body: &Value) -> Result<Vec<Candle>, BrokerError> {
    let rows = body["result"]["list"]
        .as_array()
        .ok_or_else(|| BrokerError::Parse("Bybit kline: result.list missing".into()))?;
    let mut out = rows
        .iter()
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
            bar.ok_or_else(|| BrokerError::Parse(format!("Bybit kline row: {r}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    out.reverse();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_klines_oldest_first() {
        let body: Value = serde_json::from_str(
            r#"{"retCode":0,"retMsg":"OK","result":{"category":"linear","symbol":"XAUUSDT","list":[
                ["1790208900000","4293.1","4295.0","4290.2","4294.4","12.5","53671.2"],
                ["1790208000000","4290.0","4293.5","4289.0","4293.1","20.1","86272.3"]]}}"#,
        )
        .unwrap();
        let c = parse_klines(&checked((reqwest::StatusCode::OK, body)).unwrap()).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].time, 1_790_208_000);
        assert_eq!(c[1].close, 4294.4);
    }

    #[test]
    fn signature_covers_timestamp_key_window_and_query() {
        let expected = hmac_sha256_hex("secret", "1700000000000key5000accountType=UNIFIED");
        assert_eq!(
            signature("secret", 1_700_000_000_000, "key", "accountType=UNIFIED"),
            expected
        );
    }

    #[test]
    fn maps_key_errors_to_auth() {
        let body: Value = serde_json::from_str(r#"{"retCode":10003,"retMsg":"API key is invalid."}"#).unwrap();
        assert!(matches!(
            checked((reqwest::StatusCode::OK, body)),
            Err(BrokerError::Auth { .. })
        ));
    }

    #[test]
    fn every_timeframe_has_an_interval() {
        for tf in Timeframe::ALL {
            assert!(!interval(tf).is_empty());
        }
    }
}
