//! Bybit V5, linear USDT perpetual XAUUSDT.

use serde_json::Value;

use crate::{
    binance::http_client,
    broker::{network, read_json, require, BrokerError, Probe},
    checks::{clock, Checklist},
    market::{int, num, Candle, Timeframe},
    market_depth::{DepthLevel, OrderBookSnapshot},
    sign::{hmac_sha256_hex, now_ms},
};

pub(crate) const SYMBOL: &str = "XAUUSDT";
const VENUE: &str = "Bybit";
const DEFAULT_URL: &str = "https://api.bybit.com";
const RECV_WINDOW: &str = "5000";
const MAX_LIMIT: usize = 1000;

const AFTER_REACH: [(&str, &str); 7] = [
    ("clock", "Clock in sync"),
    ("symbol", "XAUUSDT listed"),
    ("key", "API key accepted"),
    ("uta", "Unified Trading Account"),
    ("perms", "Key permissions"),
    ("ip", "IP binding"),
    ("balance", "Equity"),
];

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

    pub(crate) async fn probe(self) -> Probe<Self> {
        let mut list = Checklist::default();
        let started = now_ms();
        match self.public("/v5/market/time").await {
            Err(e) => {
                list.fail("reach", "Bybit reachable", e.to_string());
                list.skip(&AFTER_REACH);
                return Probe {
                    conn: None,
                    checks: list,
                    account: None,
                };
            }
            Ok(body) => {
                list.ok(
                    "reach",
                    "Bybit reachable",
                    self.base.split("://").nth(1).unwrap_or(&self.base).to_string(),
                );
                let local = (started + now_ms()) / 2;
                let server = int(&body["result"]["timeNano"])
                    .map(|n| n / 1_000_000)
                    .or_else(|| int(&body["time"]));
                clock(&mut list, server.unwrap_or(local) - local);
            }
        }

        match self
            .public(&format!("/v5/market/instruments-info?category=linear&symbol={SYMBOL}"))
            .await
        {
            Ok(body) => match body["result"]["list"][0]["status"].as_str() {
                Some("Trading") => list.ok("symbol", "XAUUSDT listed", "trading · linear perpetual"),
                Some(other) => list.fail("symbol", "XAUUSDT listed", format!("status {other}")),
                None => list.fail("symbol", "XAUUSDT listed", "not offered on this Bybit endpoint"),
            },
            Err(e) => list.warn("symbol", "XAUUSDT listed", format!("could not read instruments: {e}")),
        }

        let info = match self.signed("/v5/user/query-api", "").await {
            Ok(body) => {
                list.ok("key", "API key accepted", "signed request works");
                body["result"].clone()
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
        key_checks(&mut list, &info);

        let equity = match self.signed("/v5/account/wallet-balance", "accountType=UNIFIED").await {
            Ok(body) => num(&body["result"]["list"][0]["totalEquity"]),
            Err(e) => {
                list.warn("balance", "Equity", format!("could not read the wallet: {e}"));
                return Probe {
                    conn: Some(self),
                    checks: list,
                    account: Some("Unified account".into()),
                };
            }
        };
        let equity = equity.unwrap_or(0.0);
        if equity > 0.0 {
            list.ok("balance", "Equity", format!("{equity:.2} USD"));
        } else {
            list.warn("balance", "Equity", "Empty wallet (fine for charts, needed to trade)");
        }
        Probe {
            conn: Some(self),
            checks: list,
            account: Some(format!("Equity {equity:.2} USD")),
        }
    }

    pub(crate) async fn candles(&self, tf: Timeframe, limit: usize) -> Result<Vec<Candle>, BrokerError> {
        let path = format!(
            "/v5/market/kline?category=linear&symbol={SYMBOL}&interval={}&limit={}",
            interval(tf),
            limit.clamp(1, MAX_LIMIT)
        );
        parse_klines(&self.public(&path).await?)
    }

    pub(crate) async fn order_book(&self) -> Result<OrderBookSnapshot, BrokerError> {
        let body = self
            .public(&format!(
                "/v5/market/orderbook?category=linear&symbol={SYMBOL}&limit=200"
            ))
            .await?;
        let result = &body["result"];
        let symbol = result["s"]
            .as_str()
            .ok_or_else(|| BrokerError::Parse("Bybit order book: result.s missing".into()))?;
        let timestamp = int(&result["ts"])
            .ok_or_else(|| BrokerError::Parse("Bybit order book: result.ts missing or invalid".into()))?;
        let timestamp = u64::try_from(timestamp)
            .map_err(|_| BrokerError::Parse("Bybit order book: result.ts must be positive".into()))?;
        let snapshot = OrderBookSnapshot {
            symbol: symbol.to_string(),
            timestamp,
            bids: parse_depth_side(&result["b"], "bids")?,
            asks: parse_depth_side(&result["a"], "asks")?,
        };
        snapshot
            .validate()
            .map_err(|e| BrokerError::Parse(format!("Bybit order book: {e}")))?;
        Ok(snapshot)
    }

    async fn public(&self, path: &str) -> Result<Value, BrokerError> {
        let resp = self
            .http
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .map_err(network)?;
        checked(read_json(VENUE, resp).await?)
    }

    async fn signed(&self, path: &str, query: &str) -> Result<Value, BrokerError> {
        let ts = now_ms();
        let sep = if query.is_empty() { "" } else { "?" };
        let resp = self
            .http
            .get(format!("{}{path}{sep}{query}", self.base))
            .header("X-BAPI-API-KEY", &self.key)
            .header("X-BAPI-TIMESTAMP", ts.to_string())
            .header("X-BAPI-RECV-WINDOW", RECV_WINDOW)
            .header("X-BAPI-SIGN", signature(&self.secret, ts, &self.key, query))
            .send()
            .await
            .map_err(network)?;
        checked(read_json(VENUE, resp).await?)
    }
}

fn parse_depth_side(value: &Value, side: &str) -> Result<Vec<DepthLevel>, BrokerError> {
    value
        .as_array()
        .ok_or_else(|| BrokerError::Parse(format!("Bybit order book: {side} missing or not an array")))?
        .iter()
        .map(|row| {
            let pair = row
                .as_array()
                .filter(|pair| pair.len() == 2)
                .ok_or_else(|| BrokerError::Parse(format!("Bybit order book: invalid {side} level {row}")))?;
            Ok(DepthLevel {
                price: num(&pair[0])
                    .ok_or_else(|| BrokerError::Parse(format!("Bybit order book: invalid price {row}")))?,
                quantity: num(&pair[1])
                    .ok_or_else(|| BrokerError::Parse(format!("Bybit order book: invalid quantity {row}")))?,
            })
        })
        .collect()
}

/// Account type, permissions and IP binding from `/v5/user/query-api`.
fn key_checks(list: &mut Checklist, info: &Value) {
    if info["uta"].as_i64() == Some(1) {
        list.ok("uta", "Unified Trading Account", "yes");
    } else {
        list.fail(
            "uta",
            "Unified Trading Account",
            "classic account: upgrade to UTA in Bybit (Assets → Unified Trading)",
        );
    }

    let has = |group: &str, perm: &str| {
        info["permissions"][group]
            .as_array()
            .is_some_and(|a| a.iter().any(|p| p == perm))
    };
    let mut notes = Vec::new();
    let mut trade = false;
    if info["readOnly"].as_i64() == Some(1) {
        notes.push("read-only: fine now, trading will need Contract → Orders + Positions".to_string());
    } else if (has("ContractTrade", "Order") && has("ContractTrade", "Position"))
        || has("Derivatives", "DerivativesTrade")
    {
        trade = true;
    } else {
        notes.push("no contract trading permission".to_string());
    }
    if has("Wallet", "Withdraw") {
        notes.push("Withdraw is enabled: turn it off for this key".to_string());
    }
    if notes.is_empty() {
        list.ok(
            "perms",
            "Key permissions",
            if trade {
                "Contract Orders + Positions, no withdrawals"
            } else {
                "ok"
            },
        );
    } else {
        list.warn("perms", "Key permissions", notes.join("; "));
    }

    let ips: Vec<&str> = info["ips"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if ips.is_empty() || ips.contains(&"*") {
        let days = info["deadlineDay"]
            .as_i64()
            .map(|d| format!(" (expires in {d} days)"))
            .unwrap_or_default();
        list.warn("ip", "IP binding", format!("not bound to an IP{days}"));
    } else {
        list.ok("ip", "IP binding", format!("bound to {} IP", ips.len()));
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
    use crate::checks::CheckStatus;

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

    fn statuses(info: &str) -> Vec<(String, CheckStatus)> {
        let mut l = Checklist::default();
        key_checks(&mut l, &serde_json::from_str(info).unwrap());
        l.into_vec().into_iter().map(|c| (c.id, c.status)).collect()
    }

    #[test]
    fn key_rules() {
        let good = statuses(
            r#"{"uta":1,"readOnly":0,"ips":["1.2.3.4"],"permissions":{"ContractTrade":["Order","Position"],"Wallet":[]}}"#,
        );
        assert!(good.iter().all(|(_, s)| *s == CheckStatus::Ok), "{good:?}");
        let loose = statuses(r#"{"uta":0,"readOnly":1,"ips":["*"],"permissions":{"Wallet":["Withdraw"]}}"#);
        assert_eq!(
            loose,
            vec![
                ("uta".into(), CheckStatus::Fail),
                ("perms".into(), CheckStatus::Warn),
                ("ip".into(), CheckStatus::Warn)
            ]
        );
    }
}
