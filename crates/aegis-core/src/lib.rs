//! AEGIS core: market data types, broker connectors, settings and the strategy registry.
//! The desktop app (`app/src-tauri`) is a thin shell over this crate.

pub mod ai;
pub mod ai_memory;
pub mod ai_provider;
pub mod bounce;
pub mod broker;
pub mod checks;
pub mod density;
pub mod event_archive;
pub mod event_cycle;
pub mod event_scoring;
pub mod event_telemetry;
pub mod live_market;
pub mod local_ai;
pub mod market;
pub mod market_depth;
pub mod observer;
pub mod settings;
pub mod strategy;
pub mod structural;
pub mod trading;

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
pub use market_depth::{BookLevel, DepthLevel, OrderBook, OrderBookSnapshot};
pub use settings::{PublicSettings, SettingsStore};

#[cfg(test)]
mod audit_http_fixture;
