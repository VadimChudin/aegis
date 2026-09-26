use std::{fmt, path::PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    binance::Binance,
    bybit::Bybit,
    market::{Candle, Timeframe},
    mt5::Mt5Bridge,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BrokerId {
    Binance,
    Bybit,
    Roboforex,
}

/// One input in the broker's login form.
#[derive(Clone, Debug, Serialize)]
pub struct CredentialField {
    pub key: &'static str,
    pub label: &'static str,
    pub secret: bool,
    pub optional: bool,
    pub placeholder: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct BrokerInfo {
    pub id: BrokerId,
    pub name: &'static str,
    pub venue: &'static str,
    pub symbol: &'static str,
    pub note: &'static str,
    pub fields: Vec<CredentialField>,
}

const fn field(key: &'static str, label: &'static str, secret: bool) -> CredentialField {
    CredentialField {
        key,
        label,
        secret,
        optional: false,
        placeholder: "",
    }
}

impl BrokerId {
    pub const ALL: [BrokerId; 3] = [BrokerId::Binance, BrokerId::Bybit, BrokerId::Roboforex];

    pub fn name(self) -> &'static str {
        match self {
            BrokerId::Binance => "Binance",
            BrokerId::Bybit => "Bybit",
            BrokerId::Roboforex => "RoboForex",
        }
    }

    pub fn info(self) -> BrokerInfo {
        let api_fields = vec![
            field("api_key", "API key", false),
            field("api_secret", "API secret", true),
        ];
        match self {
            BrokerId::Binance => BrokerInfo {
                id: self,
                name: self.name(),
                venue: "USDⓈ-M Futures",
                symbol: "XAUUSDT",
                note: "A read-only API key is enough. AEGIS does not place orders yet.",
                fields: api_fields,
            },
            BrokerId::Bybit => BrokerInfo {
                id: self,
                name: self.name(),
                venue: "USDT Perpetual",
                symbol: "XAUUSDT",
                note: "A read-only API key of a Unified Trading Account is enough. AEGIS does not place orders yet.",
                fields: api_fields,
            },
            BrokerId::Roboforex => BrokerInfo {
                id: self,
                name: self.name(),
                venue: "MetaTrader 5",
                symbol: "XAUUSD",
                note: "Windows only: needs the RoboForex MT5 terminal, Python 3 and `pip install MetaTrader5`.",
                fields: vec![
                    field("login", "MT5 login", false),
                    field("password", "Password", true),
                    CredentialField {
                        placeholder: "RoboForex-ECN",
                        ..field("server", "Server", false)
                    },
                    CredentialField {
                        optional: true,
                        placeholder: r"C:\Program Files\RoboForex MT5 Terminal\terminal64.exe",
                        ..field("terminal_path", "Terminal path", false)
                    },
                    CredentialField {
                        optional: true,
                        placeholder: "python",
                        ..field("python", "Python", false)
                    },
                ],
            },
        }
    }
}

/// Login data sent by the window. Kept in memory only.
#[derive(Clone, Deserialize)]
#[serde(tag = "broker", rename_all = "lowercase")]
pub enum Credentials {
    Binance {
        api_key: String,
        api_secret: String,
    },
    Bybit {
        api_key: String,
        api_secret: String,
    },
    Roboforex {
        login: String,
        password: String,
        server: String,
        #[serde(default)]
        terminal_path: Option<String>,
        #[serde(default)]
        python: Option<String>,
    },
}

impl Credentials {
    pub fn broker(&self) -> BrokerId {
        match self {
            Credentials::Binance { .. } => BrokerId::Binance,
            Credentials::Bybit { .. } => BrokerId::Bybit,
            Credentials::Roboforex { .. } => BrokerId::Roboforex,
        }
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Credentials({:?}, <redacted>)", self.broker())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct AccountSummary {
    pub broker: BrokerId,
    pub name: &'static str,
    pub symbol: String,
    pub account: String,
}

/// Where connectors reach their venue. `None` means the production endpoint.
#[derive(Clone, Debug, Default)]
pub struct ConnectOptions {
    pub binance_url: Option<String>,
    pub bybit_url: Option<String>,
    pub mt5_bridge_script: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum BrokerError {
    #[error("{0}")]
    Input(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("{0} blocks access from this region (use a VPN or proxy exiting in an allowed country)")]
    Geo(&'static str),
    #[error("{venue}: invalid API key, secret or permissions ({message})")]
    Auth { venue: &'static str, message: String },
    #[error("{venue}: {message} (code {code})")]
    Api {
        venue: &'static str,
        code: i64,
        message: String,
    },
    #[error("unexpected response: {0}")]
    Parse(String),
    #[error("MT5 bridge: {0}")]
    Bridge(String),
}

pub(crate) fn network(err: reqwest::Error) -> BrokerError {
    BrokerError::Network(err.without_url().to_string())
}

/// Reads a JSON body, turning geo-blocks and non-JSON pages into readable errors.
pub(crate) async fn read_json(
    venue: &'static str,
    resp: reqwest::Response,
) -> Result<(reqwest::StatusCode, Value), BrokerError> {
    let status = resp.status();
    let text = resp.text().await.map_err(network)?;
    let blocked = status.as_u16() == 451
        || (status.as_u16() == 403 && (text.contains("CloudFront") || text.contains("restricted location")));
    if blocked {
        return Err(BrokerError::Geo(venue));
    }
    match serde_json::from_str(&text) {
        Ok(v) => Ok((status, v)),
        Err(_) => {
            let head: String = text.chars().take(160).collect();
            Err(BrokerError::Parse(format!("{venue} HTTP {status}: {head}")))
        }
    }
}

pub(crate) fn require(value: &str, what: &str) -> Result<String, BrokerError> {
    let v = value.trim();
    if v.is_empty() {
        Err(BrokerError::Input(format!("{what} is empty")))
    } else {
        Ok(v.to_string())
    }
}

/// A logged-in broker. The chart only ever reads from the active connector.
pub enum Connector {
    Binance(Binance),
    Bybit(Bybit),
    Roboforex(Box<Mt5Bridge>),
}

impl Connector {
    /// Verifies the credentials against the venue and returns the ready connector.
    pub async fn connect(
        credentials: Credentials,
        options: &ConnectOptions,
    ) -> Result<(Connector, AccountSummary), BrokerError> {
        let broker = credentials.broker();
        let (connector, account) = match credentials {
            Credentials::Binance { api_key, api_secret } => {
                let c = Binance::new(options.binance_url.clone(), &api_key, &api_secret)?;
                let account = c.verify().await?;
                (Connector::Binance(c), account)
            }
            Credentials::Bybit { api_key, api_secret } => {
                let c = Bybit::new(options.bybit_url.clone(), &api_key, &api_secret)?;
                let account = c.verify().await?;
                (Connector::Bybit(c), account)
            }
            Credentials::Roboforex {
                login,
                password,
                server,
                terminal_path,
                python,
            } => {
                let script = options
                    .mt5_bridge_script
                    .clone()
                    .ok_or_else(|| BrokerError::Bridge("bridge script is missing from the app bundle".into()))?;
                let (bridge, account) = Mt5Bridge::connect(
                    python.as_deref(),
                    &script,
                    &login,
                    &password,
                    &server,
                    terminal_path.as_deref(),
                )
                .await?;
                (Connector::Roboforex(Box::new(bridge)), account)
            }
        };
        let summary = AccountSummary {
            broker,
            name: broker.name(),
            symbol: connector.symbol().to_string(),
            account,
        };
        Ok((connector, summary))
    }

    pub fn id(&self) -> BrokerId {
        match self {
            Connector::Binance(_) => BrokerId::Binance,
            Connector::Bybit(_) => BrokerId::Bybit,
            Connector::Roboforex(_) => BrokerId::Roboforex,
        }
    }

    pub fn symbol(&self) -> &str {
        match self {
            Connector::Binance(_) => crate::binance::SYMBOL,
            Connector::Bybit(_) => crate::bybit::SYMBOL,
            Connector::Roboforex(b) => b.symbol(),
        }
    }

    /// Latest `limit` bars, oldest first. The last bar may still be forming.
    pub async fn candles(&self, timeframe: Timeframe, limit: usize) -> Result<Vec<Candle>, BrokerError> {
        match self {
            Connector::Binance(c) => c.candles(timeframe, limit).await,
            Connector::Bybit(c) => c.candles(timeframe, limit).await,
            Connector::Roboforex(c) => c.candles(timeframe, limit).await,
        }
    }

    /// Releases the venue session (logs the MT5 terminal out and stops the bridge).
    pub async fn close(&self) {
        if let Connector::Roboforex(c) = self {
            c.shutdown().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_parse_from_the_window_payload() {
        let c: Credentials = serde_json::from_str(r#"{"broker":"bybit","api_key":"k","api_secret":"s"}"#).unwrap();
        assert_eq!(c.broker(), BrokerId::Bybit);
        let c: Credentials =
            serde_json::from_str(r#"{"broker":"roboforex","login":"123","password":"p","server":"RoboForex-ECN"}"#)
                .unwrap();
        assert_eq!(c.broker(), BrokerId::Roboforex);
    }

    #[test]
    fn debug_output_never_contains_secrets() {
        let c: Credentials =
            serde_json::from_str(r#"{"broker":"binance","api_key":"KEY123","api_secret":"SECRET456"}"#).unwrap();
        let s = format!("{c:?}");
        assert!(!s.contains("KEY123") && !s.contains("SECRET456"), "{s}");
    }

    #[test]
    fn every_broker_describes_its_login_form() {
        for id in BrokerId::ALL {
            let info = id.info();
            assert!(!info.fields.is_empty());
            assert!(info.symbol.starts_with("XAU"));
        }
    }
}
