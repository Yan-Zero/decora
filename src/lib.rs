//! Decora: a multi-dialect, single-tenant gateway that preserves native decision probabilities and strictly enforces capability boundaries.
pub mod adapter;
pub mod config;
pub mod dialect;
pub mod error;
#[cfg(feature = "server")]
pub mod gateway;
pub mod protocol;

#[cfg(feature = "server")]
pub use config::Config;
#[cfg(feature = "server")]
pub use gateway::Gateway;
