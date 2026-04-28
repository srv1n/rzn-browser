//! `rzn_sdk` is an embedding-first facade for RZN Browser Native.
//!
//! Design goals:
//! - Provide a small, stable surface for downstream applications (e.g., Tauri).
//! - Keep iteration velocity high by allowing the engine (`rzn_plan`) to evolve.
//! - Offer an explicit "escape hatch" via the `unstable` feature for internal use.

#[cfg(feature = "native_host")]
pub mod native_host;

#[cfg(feature = "host")]
pub mod host;

#[cfg(feature = "host")]
pub mod session;

#[cfg(feature = "host")]
pub mod tools;

#[cfg(all(feature = "host", feature = "unstable"))]
pub mod unstable;

pub mod prelude;

pub use rzn_contracts as contracts;
pub use rzn_core;

/// SDK-wide result type.
pub type Result<T> = std::result::Result<T, Error>;

/// SDK-wide error type.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[cfg(feature = "host")]
    #[error(transparent)]
    Host(#[from] host::HostError),

    #[cfg(feature = "host")]
    #[error(transparent)]
    Session(#[from] session::Error),

    #[cfg(feature = "native_host")]
    #[error(transparent)]
    NativeHost(#[from] native_host::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
