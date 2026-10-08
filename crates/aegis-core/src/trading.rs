use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountState {
    pub login: u64,
    pub server: String,
    pub currency: String,
    pub balance: f64,
    pub equity: f64,
    pub profit: f64,
    pub margin: f64,
    pub margin_free: f64,
    pub trade_allowed: bool,
    pub demo: bool,
    pub time_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrokerPosition {
    pub ticket: u64,
    pub symbol: String,
    pub side: String,
    pub volume_lots: f64,
    pub quantity_oz: f64,
    pub entry: f64,
    pub current: f64,
    pub stop: f64,
    pub target: f64,
    pub profit: f64,
    pub magic: u64,
    pub comment: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TradingState {
    pub account: AccountState,
    pub positions: Vec<BrokerPosition>,
    pub contract_size: f64,
    pub volume_min: f64,
    pub volume_max: f64,
    pub volume_step: f64,
    pub hedging: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TradeRequest {
    pub request_id: String,
    pub side: String,
    pub risk_pct: f64,
    pub stop: f64,
    pub target: f64,
    pub max_spread: f64,
    pub max_positions: usize,
    pub daily_loss_limit_pct: f64,
    pub confirm_real: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TradeResult {
    pub request_id: String,
    pub status: String,
    pub retcode: i64,
    pub message: String,
    pub order: u64,
    pub deal: u64,
    pub volume_lots: f64,
    pub price: f64,
}
