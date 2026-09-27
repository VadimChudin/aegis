use std::{collections::BTreeMap, fmt, path::PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    binance::Binance,
    bybit::Bybit,
    checks::{Check, Checklist},
    market::{Candle, Timeframe},
    mt5::Mt5Bridge,
    sign::now_ms,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
    pub hint: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct BrokerInfo {
    pub id: BrokerId,
    pub name: &'static str,
    pub venue: &'static str,
    pub symbol: &'static str,
    pub icon: &'static str,
    /// Where the user creates the key or finds the account.
    pub keys_url: &'static str,
    /// What the account and key must satisfy, shown above the form.
    pub requirements: Vec<&'static str>,
    pub fields: Vec<CredentialField>,
}

const fn field(key: &'static str, label: &'static str, secret: bool, hint: &'static str) -> CredentialField {
    CredentialField {
        key,
        label,
        secret,
        optional: false,
        placeholder: "",
        hint,
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
        match self {
            BrokerId::Binance => BrokerInfo {
                id: self,
                name: self.name(),
                venue: "USDⓈ-M Futures",
                symbol: "XAUUSDT",
                icon: "assets/brokers/binance.svg",
                keys_url: "https://www.binance.com/en/my/settings/api-management",
                requirements: vec![
                    "USDⓈ-M Futures account opened. XAUUSDT is a TradFi perpetual and is not offered in every region.",
                    "API key of type System-generated (HMAC) with Enable Reading.",
                    "Enable Futures only when AEGIS starts trading. Never enable Withdrawals.",
                    "Restrict the key to your IP address: Binance limits trading permissions of unrestricted keys.",
                    "Computer clock in sync (within 1 s).",
                ],
                fields: vec![
                    field("api_key", "API key", false, "API Management → Create API → System generated."),
                    field("api_secret", "Secret key", true, "Shown once when the key is created."),
                ],
            },
            BrokerId::Bybit => BrokerInfo {
                id: self,
                name: self.name(),
                venue: "USDT Perpetual",
                symbol: "XAUUSDT",
                icon: "assets/brokers/bybit.png",
                keys_url: "https://www.bybit.com/app/user/api-management",
                requirements: vec![
                    "Unified Trading Account (UTA).",
                    "System-generated API key (HMAC). Read-only is enough now; trading will need Contract → Orders and Positions.",
                    "No Withdraw permission on this key.",
                    "Bind the key to your IP address, otherwise Bybit expires it after 90 days.",
                    "Computer clock in sync (within 1 s).",
                ],
                fields: vec![
                    field("api_key", "API key", false, "API → Create New Key → System-generated API Keys."),
                    field("api_secret", "API secret", true, "Shown once when the key is created."),
                ],
            },
            BrokerId::Roboforex => BrokerInfo {
                id: self,
                name: self.name(),
                venue: "MetaTrader 5",
                symbol: "XAUUSD",
                icon: "assets/brokers/roboforex.png",
                keys_url: "https://my.roboforex.com/",
                requirements: vec![
                    "Windows with the RoboForex MetaTrader 5 terminal installed and logged in once.",
                    "Python 3.9+ and `pip install MetaTrader5`.",
                    "An MT5 account (ECN, Prime or Pro). MT4 accounts cannot connect.",
                    "Algo Trading enabled on the terminal toolbar (needed for trading later).",
                ],
                fields: vec![
                    field("login", "MT5 login", false, "Account number from the RoboForex Members Area."),
                    field("password", "Password", true, "Trading password (the investor password is read-only)."),
                    CredentialField {
                        placeholder: "RoboForex-ECN",
                        ..field("server", "Server", false, "Exactly as in the terminal login window.")
                    },
                    CredentialField {
                        optional: true,
                        placeholder: r"C:\Program Files\RoboForex MT5 Terminal\terminal64.exe",
                        ..field("terminal_path", "Terminal path", false, "Only if the terminal is not found automatically.")
                    },
                    CredentialField {
                        optional: true,
                        placeholder: "python",
                        ..field("python", "Python", false, "Path to python.exe if it is not on PATH.")
                    },
                ],
            },
        }
    }

    pub fn is_secret(self, key: &str) -> bool {
        self.info().fields.iter().any(|f| f.key == key && f.secret)
    }
}

/// Login data. Kept in memory for the session; `settings` stores it encrypted.
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

    /// Builds credentials from the form fields of one broker.
    pub fn from_fields(broker: BrokerId, fields: &BTreeMap<String, String>) -> Result<Self, BrokerError> {
        let get = |key: &str| fields.get(key).map(|v| v.trim().to_string()).unwrap_or_default();
        let opt = |key: &str| Some(get(key)).filter(|v| !v.is_empty());
        for f in broker.info().fields.iter().filter(|f| !f.optional) {
            if get(f.key).is_empty() {
                return Err(BrokerError::Input(format!("{} is empty", f.label)));
            }
        }
        Ok(match broker {
            BrokerId::Binance => Credentials::Binance {
                api_key: get("api_key"),
                api_secret: get("api_secret"),
            },
            BrokerId::Bybit => Credentials::Bybit {
                api_key: get("api_key"),
                api_secret: get("api_secret"),
            },
            BrokerId::Roboforex => Credentials::Roboforex {
                login: get("login"),
                password: get("password"),
                server: get("server"),
                terminal_path: opt("terminal_path"),
                python: opt("python"),
            },
        })
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

/// Result of a connect attempt: the checklist and whether a session exists.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectReport {
    pub broker: BrokerId,
    /// A session is open and the chart can use this broker.
    pub connected: bool,
    /// Connected and no check failed.
    pub ready: bool,
    pub account: Option<String>,
    pub symbol: Option<String>,
    pub checks: Vec<Check>,
    /// Unix milliseconds.
    pub at: i64,
}

impl ConnectReport {
    pub(crate) fn new(broker: BrokerId, list: Checklist, account: Option<String>, symbol: Option<String>) -> Self {
        let connected = account.is_some();
        let ready = connected && !list.has_fail();
        ConnectReport {
            broker,
            connected,
            ready,
            account,
            symbol,
            checks: list.into_vec(),
            at: now_ms(),
        }
    }

    /// Report for credentials that never reached the venue (empty field, bad login format).
    pub fn input_error(broker: BrokerId, err: &BrokerError) -> Self {
        let mut list = Checklist::default();
        list.fail("input", "Credentials", err.to_string());
        ConnectReport::new(broker, list, None, None)
    }
}

/// Where connectors reach their venue. `None` means the production endpoint.
#[derive(Clone, Debug, Default)]
pub struct ConnectOptions {
    pub binance_url: Option<String>,
    /// Binance spot/SAPI host, used for the key-permission check.
    pub binance_spot_url: Option<String>,
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

/// A connector's connect run: the session when it can be used, plus its checklist.
pub(crate) struct Probe<T> {
    pub conn: Option<T>,
    pub checks: Checklist,
    pub account: Option<String>,
}

/// A logged-in broker.
pub enum Connector {
    Binance(Binance),
    Bybit(Bybit),
    Roboforex(Box<Mt5Bridge>),
}

impl Connector {
    /// Runs the broker's checklist. Returns the session when the key works,
    /// even if some checks only warn (for example read-only keys).
    pub async fn connect(credentials: Credentials, options: &ConnectOptions) -> (Option<Connector>, ConnectReport) {
        let broker = credentials.broker();
        let built = match credentials {
            Credentials::Binance { api_key, api_secret } => {
                match Binance::new(
                    options.binance_url.clone(),
                    options.binance_spot_url.clone(),
                    &api_key,
                    &api_secret,
                ) {
                    Ok(c) => Ok(map(c.probe().await, Connector::Binance)),
                    Err(e) => Err(e),
                }
            }
            Credentials::Bybit { api_key, api_secret } => {
                match Bybit::new(options.bybit_url.clone(), &api_key, &api_secret) {
                    Ok(c) => Ok(map(c.probe().await, Connector::Bybit)),
                    Err(e) => Err(e),
                }
            }
            Credentials::Roboforex {
                login,
                password,
                server,
                terminal_path,
                python,
            } => match options.mt5_bridge_script.clone() {
                None => Err(BrokerError::Bridge(
                    "bridge script is missing from the app bundle".into(),
                )),
                Some(script) => Ok(map(
                    Mt5Bridge::probe(
                        python.as_deref(),
                        &script,
                        &login,
                        &password,
                        &server,
                        terminal_path.as_deref(),
                    )
                    .await,
                    |b| Connector::Roboforex(Box::new(b)),
                )),
            },
        };
        match built {
            Err(e) => (None, ConnectReport::input_error(broker, &e)),
            Ok(probe) => {
                let symbol = probe.conn.as_ref().map(|c| c.symbol().to_string());
                let account = probe.conn.as_ref().and(probe.account);
                let report = ConnectReport::new(broker, probe.checks, account, symbol);
                (probe.conn, report)
            }
        }
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

fn map<T>(p: Probe<T>, wrap: impl FnOnce(T) -> Connector) -> Probe<Connector> {
    Probe {
        conn: p.conn.map(wrap),
        checks: p.checks,
        account: p.account,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn credentials_from_form_fields() {
        let c = Credentials::from_fields(BrokerId::Bybit, &fields(&[("api_key", " k "), ("api_secret", "s")])).unwrap();
        assert!(matches!(c, Credentials::Bybit { ref api_key, .. } if api_key == "k"));
        let c = Credentials::from_fields(
            BrokerId::Roboforex,
            &fields(&[
                ("login", "123"),
                ("password", "p"),
                ("server", "RoboForex-ECN"),
                ("python", " "),
            ]),
        )
        .unwrap();
        assert!(matches!(
            c,
            Credentials::Roboforex {
                python: None,
                terminal_path: None,
                ..
            }
        ));
    }

    #[test]
    fn missing_required_field_is_named() {
        let err = Credentials::from_fields(BrokerId::Binance, &fields(&[("api_key", "k")]))
            .err()
            .unwrap();
        assert_eq!(err.to_string(), "Secret key is empty");
    }

    #[test]
    fn debug_output_never_contains_secrets() {
        let c: Credentials =
            serde_json::from_str(r#"{"broker":"binance","api_key":"KEY123","api_secret":"SECRET456"}"#).unwrap();
        let s = format!("{c:?}");
        assert!(!s.contains("KEY123") && !s.contains("SECRET456"), "{s}");
    }

    #[test]
    fn every_broker_describes_its_form_requirements_and_icon() {
        for id in BrokerId::ALL {
            let info = id.info();
            assert!(!info.fields.is_empty() && !info.requirements.is_empty());
            assert!(info.symbol.starts_with("XAU"));
            assert!(info.icon.starts_with("assets/brokers/"));
            assert!(info.fields.iter().any(|f| f.secret), "{id:?} must have a secret field");
        }
        assert!(BrokerId::Binance.is_secret("api_secret") && !BrokerId::Binance.is_secret("api_key"));
    }

    #[test]
    fn input_error_report_is_not_connected() {
        let r = ConnectReport::input_error(BrokerId::Bybit, &BrokerError::Input("API key is empty".into()));
        assert!(!r.connected && !r.ready);
        assert_eq!(r.checks[0].detail, "API key is empty");
    }
}
