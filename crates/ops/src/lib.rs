//! EPO OPS client: authentication, throttling, parsing (SPEC section 5.4).

pub mod auth;
pub mod biblio;
pub mod client;
pub mod error;
pub mod retry;
pub mod throttle;

pub use error::OpsError;
