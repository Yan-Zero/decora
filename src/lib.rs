//! Decora: a multi-dialect, single-tenant gateway that preserves native decision probabilities and strictly enforces capability boundaries.
pub mod adapter;
pub mod config;
pub mod dialect;
pub mod error;
pub mod gateway;
pub mod protocol;

pub use config::Config;
pub use gateway::Gateway;
