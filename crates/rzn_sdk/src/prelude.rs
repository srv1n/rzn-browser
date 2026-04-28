//! Convenience re-exports for common SDK usage.

pub use crate::{Error, Result};

pub use rzn_contracts as contracts;
pub use rzn_core::{Step, StepKind, Workflow};

#[cfg(feature = "host")]
pub use crate::host::{
    Host, HostConfig, HostError, HostErrorCode, PlanRequest, PlanResponse, RunRequest, RunResponse,
    RuntimeTransport,
};

#[cfg(feature = "host")]
pub use crate::session::Session;

#[cfg(feature = "host")]
pub use crate::tools::{BrowserTools, ObserveOptions, ToolError, ToolResult};

#[cfg(feature = "native_host")]
pub use crate::native_host::{
    chrome_native_host_dir, install_rzn_native_host_for_chrome,
    install_rzn_native_host_for_chrome_auto, native_host_manifest_path_for_chrome,
    read_installed_rzn_native_host_manifest_for_chrome, resolve_native_host_executable_path,
    uninstall_rzn_native_host_for_chrome, NativeMessagingHostManifest, RZN_NATIVE_HOST_NAME,
};

#[cfg(feature = "unstable")]
pub use crate::unstable;
