//! AEGIS core: market data types, broker connectors, settings and the strategy registry.
//! The desktop app (`app/src-tauri`) is a thin shell over this crate.

pub mod bounce;
pub mod broker;
pub mod checks;
pub mod market;
pub mod settings;
pub mod strategy;

mod binance;
mod bybit;
mod mt5;
mod sign;

pub use broker::{
    AccountSummary, BrokerError, BrokerId, BrokerInfo, ConnectOptions, ConnectReport, Connector, CredentialField,
    Credentials,
};
pub use checks::{Check, CheckStatus};
pub use market::{Candle, Timeframe};
pub use settings::{PublicSettings, SettingsStore};
