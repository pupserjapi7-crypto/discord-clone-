//! Types shared by the API, the gateway and the load tester.
//!
//! The crate is deliberately dependency-light: it must not pull in a database
//! driver or an HTTP stack, so that the load test binary stays small.

pub mod config;
pub mod error;
pub mod ids;
pub mod metrics;
pub mod permissions;
pub mod protocol;
pub mod tokens;
pub mod validate;
