//! Decora：保留原生决策概率、严格检查能力边界的多方言单租户网关。
pub mod adapter;
pub mod config;
pub mod dialect;
pub mod error;
pub mod gateway;
pub mod protocol;

pub use config::Config;
pub use gateway::Gateway;
