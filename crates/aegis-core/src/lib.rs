//! AEGIS core: market data types, broker connectors and the strategy registry.
//! The desktop app (`app/src-tauri`) is a thin shell over this crate.

pub mod broker;
pub mod market;
pub mod strategy;

mod binance;
mod bybit;
mod mt5;
mod sign;

pub use broker::{
    AccountSummary, BrokerError, BrokerId, BrokerInfo, ConnectOptions, Connector, CredentialField, Credentials,
};
pub use market::{Candle, Timeframe};
