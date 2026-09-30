//! Binance USDⓈ-M futures: XAUUSDT perpetual (TradFi perps, listed January 2026).

use std::time::Duration;

use serde_json::Value;

use crate::{
    broker::{network, read_json, require, BrokerError, Probe},
    checks::{clock, Checklist},
    market::{int, num, Candle, Timeframe},
    market_depth::{DepthLevel, OrderBookSnapshot},
    sign::{hmac_sha256_hex, now_ms},
};

pub(crate) const SYMBOL: &str = "XAUUSDT";
const VENUE: &str = "Binance";
const DEFAULT_URL: &str = "https://fapi.binance.com";
const DEFAULT_SPOT_URL: &str = "https://api.binance.com";
const MAX_LIMIT: usize = 1500;

const AFTER_REACH: [(&str, &str); 6] = [
    ("clock", "Clock in sync"),
    ("symbol", "XAUUSDT listed"),
    ("key", "API key accepted"),
    ("perms", "Key permissions"),
    ("mode", "Position mode"),
    ("balance", "USDT balance"),
];

pub struct Binance {
    http: reqwest::Client,
    base: String,
    spot: String,
    key: String,
    secret: String,
}

impl Binance {
    pub(crate) fn new(
        base: Option<String>,
        spot: Option<String>,
        key: &str,
        secret: &str,
    ) -> Result<Self, BrokerError> {
        let trim = |u: String| u.trim_end_matches('/').to_string();
        Ok(Self {
            http: http_client()?,
            base: trim(base.unwrap_or_else(|| DEFAULT_URL.into())),
            spot: trim(spot.unwrap_or_else(|| DEFAULT_SPOT_URL.into())),
            key: require(key, "API key")?,
            secret: require(secret, "Secret key")?,
        })
    }

    pub(crate) async fn probe(self) -> Probe<Self> {
        let mut list = Checklist::default();
        let started = now_ms();
        match self.public("/fapi/v1/time").await {
            Err(e) => {
                list.fail("reach", "Binance reachable", e.to_string());
                list.skip(&AFTER_REACH);
                return Probe {
                    conn: None,
                    checks: list,
                    account: None,
                };
            }
            Ok(body) => {
                list.ok("reach", "Binance reachable", host(&self.base));
                let local = (started + now_ms()) / 2;
                clock(&mut list, int(&body["serverTime"]).unwrap_or(local) - local);
            }
        }

        match self.public("/fapi/v1/exchangeInfo").await {
            Ok(body) => symbol_check(&mut list, &body),
            Err(e) => list.warn("symbol", "XAUUSDT listed", format!("could not read exchange info: {e}")),
        }

        let usdt = match self.signed(&self.base, "/fapi/v2/balance", "").await {
            Ok(body) => {
                list.ok("key", "API key accepted", "signed request to the futures account works");
                body.as_array()
                    .and_then(|assets| assets.iter().find(|a| a["asset"] == "USDT"))
                    .and_then(|a| num(&a["balance"]))
                    .unwrap_or(0.0)
            }
            Err(e) => {
                list.fail("key", "API key accepted", e.to_string());
                list.skip(&AFTER_REACH[3..]);
                return Probe {
                    conn: None,
                    checks: list,
                    account: None,
                };
            }
        };

        match self.signed(&self.spot, "/sapi/v1/account/apiRestrictions", "").await {
            Ok(body) => permissions_check(&mut list, &body),
            Err(e) => list.warn("perms", "Key permissions", format!("could not read them: {e}")),
        }

        match self.signed(&self.base, "/fapi/v1/positionSide/dual", "").await {
            Ok(body) if body["dualSidePosition"] == true => list.warn(
                "mode",
                "Position mode",
                "Hedge mode. AEGIS trades one-way: switch in Futures → Preferences → Position Mode",
            ),
            Ok(_) => list.ok("mode", "Position mode", "One-way"),
            Err(e) => list.warn("mode", "Position mode", format!("could not read it: {e}")),
        }

        if usdt > 0.0 {
            list.ok(
                "balance",
                "USDT balance",
                format!("{usdt:.2} USDT in the futures wallet"),
            );
        } else {
            list.warn(
                "balance",
                "USDT balance",
                "No USDT in the futures wallet (fine for charts, needed to trade)",
            );
        }
        Probe {
            conn: Some(self),
            checks: list,
            account: Some(format!("USDT {usdt:.2}")),
        }
    }

    pub(crate) async fn candles(&self, tf: Timeframe, limit: usize) -> Result<Vec<Candle>, BrokerError> {
        let path = format!(
            "/fapi/v1/klines?symbol={SYMBOL}&interval={}&limit={}",
            tf.as_str(),
            limit.clamp(1, MAX_LIMIT)
        );
        parse_klines(&self.public(&path).await?)
    }

    pub(crate) async fn order_book(&self) -> Result<OrderBookSnapshot, BrokerError> {
        let body = self
            .public(&format!("/fapi/v1/depth?symbol={SYMBOL}&limit=500"))
            .await?;
        let snapshot = OrderBookSnapshot {
            symbol: SYMBOL.into(),
            timestamp: body["E"]
                .as_u64()
                .or_else(|| body["T"].as_u64())
                .unwrap_or_else(|| now_ms() as u64),
            bids: parse_depth_side(&body["bids"], "bids")?,
            asks: parse_depth_side(&body["asks"], "asks")?,
        };
        validate_depth(snapshot, "Binance")
    }

    async fn public(&self, path: &str) -> Result<Value, BrokerError> {
        let resp = self
            .http
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .map_err(network)?;
        let (status, body) = read_json(VENUE, resp).await?;
        if !status.is_success() {
            return Err(api_error(&body, status));
        }
        Ok(body)
    }

    async fn signed(&self, base: &str, path: &str, params: &str) -> Result<Value, BrokerError> {
        let sep = if params.is_empty() { "" } else { "&" };
        let query = format!("{params}{sep}recvWindow=5000&timestamp={}", now_ms());
        let signature = hmac_sha256_hex(&self.secret, &query);
        let url = format!("{base}{path}?{query}&signature={signature}");
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
        Ok(body)
    }
}

fn parse_depth_side(value: &Value, side: &str) -> Result<Vec<DepthLevel>, BrokerError> {
    value
        .as_array()
        .ok_or_else(|| BrokerError::Parse(format!("Binance order book: {side} missing or not an array")))?
        .iter()
        .map(|row| {
            let pair = row
                .as_array()
                .filter(|pair| pair.len() == 2)
                .ok_or_else(|| BrokerError::Parse(format!("Binance order book: invalid {side} level {row}")))?;
            Ok(DepthLevel {
                price: num(&pair[0])
                    .ok_or_else(|| BrokerError::Parse(format!("Binance order book: invalid price {row}")))?,
                quantity: num(&pair[1])
                    .ok_or_else(|| BrokerError::Parse(format!("Binance order book: invalid quantity {row}")))?,
            })
        })
        .collect()
}

fn validate_depth(snapshot: OrderBookSnapshot, venue: &str) -> Result<OrderBookSnapshot, BrokerError> {
    snapshot
        .validate()
        .map_err(|e| BrokerError::Parse(format!("{venue} order book: {e}")))?;
    Ok(snapshot)
}

fn host(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url)
        .to_string()
}

fn symbol_check(list: &mut Checklist, body: &Value) {
    let found = body["symbols"]
        .as_array()
        .and_then(|s| s.iter().find(|s| s["symbol"] == SYMBOL));
    match found {
        None => list.fail(
            "symbol",
            "XAUUSDT listed",
            "not offered on this Binance endpoint (region or account type)",
        ),
        Some(s) if s["status"] == "TRADING" => {
            let kind = s["contractType"].as_str().unwrap_or("perpetual");
            list.ok("symbol", "XAUUSDT listed", format!("trading · {kind}"));
        }
        Some(s) => list.fail(
            "symbol",
            "XAUUSDT listed",
            format!("status {}", s["status"].as_str().unwrap_or("unknown")),
        ),
    }
}

fn permissions_check(list: &mut Checklist, body: &Value) {
    if body["enableReading"] != true {
        list.fail("perms", "Key permissions", "Enable Reading is off");
        return;
    }
    let mut notes = Vec::new();
    if body["enableWithdrawals"] == true {
        notes.push("Withdrawals are enabled: turn them off for this key");
    }
    if body["ipRestrict"] != true {
        notes.push("no IP restriction");
    }
    if body["enableFutures"] != true {
        notes.push("Futures trading is off (fine while AEGIS only reads)");
    }
    if notes.is_empty() {
        list.ok(
            "perms",
            "Key permissions",
            "Reading + Futures, IP-restricted, no withdrawals",
        );
    } else {
        list.warn("perms", "Key permissions", notes.join("; "));
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
    use crate::checks::CheckStatus;

    fn run(f: impl FnOnce(&mut Checklist)) -> (CheckStatus, String) {
        let mut l = Checklist::default();
        f(&mut l);
        let c = l.into_vec().remove(0);
        (c.status, c.detail)
    }

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

    #[test]
    fn permission_rules() {
        let v = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        let safe = v(r#"{"enableReading":true,"enableFutures":true,"ipRestrict":true,"enableWithdrawals":false}"#);
        assert_eq!(run(|l| permissions_check(l, &safe)).0, CheckStatus::Ok);
        let risky = v(r#"{"enableReading":true,"enableFutures":true,"ipRestrict":true,"enableWithdrawals":true}"#);
        let (status, detail) = run(|l| permissions_check(l, &risky));
        assert_eq!(status, CheckStatus::Warn);
        assert!(detail.contains("Withdrawals"), "{detail}");
        assert_eq!(
            run(|l| permissions_check(l, &v(r#"{"enableReading":false}"#))).0,
            CheckStatus::Fail
        );
    }

    #[test]
    fn symbol_rules() {
        let v = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        let listed = v(r#"{"symbols":[{"symbol":"XAUUSDT","status":"TRADING","contractType":"TRADIFI_PERPETUAL"}]}"#);
        assert_eq!(run(|l| symbol_check(l, &listed)).0, CheckStatus::Ok);
        assert_eq!(run(|l| symbol_check(l, &v(r#"{"symbols":[]}"#))).0, CheckStatus::Fail);
        let closed = v(r#"{"symbols":[{"symbol":"XAUUSDT","status":"SETTLING"}]}"#);
        assert_eq!(
            run(|l| symbol_check(l, &closed)),
            (CheckStatus::Fail, "status SETTLING".into())
        );
    }
}
