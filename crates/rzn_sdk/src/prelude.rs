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
#[allow(deprecated)]
pub use crate::native_host::{
    browser_target_metadata, chrome_native_host_dir, install_rzn_native_host_for_browser,
    install_rzn_native_host_for_browser_auto, install_rzn_native_host_for_browser_with_origins,
    install_rzn_native_host_for_chrome, install_rzn_native_host_for_chrome_auto,
    native_host_dir_for_browser, native_host_manifest_path_for_browser,
    native_host_manifest_path_for_chrome, native_host_registry_key_for_browser,
    normalize_extension_origin, normalize_extension_origins,
    read_installed_rzn_native_host_manifest_for_browser,
    read_installed_rzn_native_host_manifest_for_chrome, resolve_native_host_executable_path,
    uninstall_rzn_native_host_for_browser, uninstall_rzn_native_host_for_chrome, BrowserKind,
    BrowserManifestDirectoryIdentity, BrowserTargetMetadata, NativeHostInstallReport,
    NativeHostUninstallReport, NativeMessagingHostManifest, RZN_DEV_EXTENSION_ID,
    RZN_DEV_EXTENSION_ORIGIN, RZN_NATIVE_HOST_NAME,
};

#[cfg(feature = "unstable")]
pub use crate::unstable;
