use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Chrome native messaging host name used by the RZN extension.
pub const RZN_NATIVE_HOST_NAME: &str = "com.rzn.browser.broker";

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to resolve home directory")]
    MissingHomeDir,

    #[error("unsupported platform for automatic native host installation")]
    UnsupportedPlatform,

    #[error("native host executable does not exist: {0}")]
    NativeHostExecutableMissing(String),

    #[error("native host executable is not a file: {0}")]
    NativeHostExecutableNotAFile(String),

    #[error("extension id is empty")]
    EmptyExtensionId,

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Native messaging host manifest for Chrome/Chromium-based browsers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeMessagingHostManifest {
    pub name: String,
    pub description: String,
    pub path: String,
    #[serde(rename = "type")]
    pub manifest_type: String,
    pub allowed_origins: Vec<String>,
}

impl NativeMessagingHostManifest {
    /// Create a manifest for the RZN native host.
    pub fn rzn_native_host(native_host_path: impl AsRef<Path>, extension_id: &str) -> Result<Self> {
        let extension_id = extension_id.trim();
        if extension_id.is_empty() {
            return Err(Error::EmptyExtensionId);
        }

        let native_host_path = native_host_path.as_ref();
        if !native_host_path.exists() {
            return Err(Error::NativeHostExecutableMissing(
                native_host_path.display().to_string(),
            ));
        }
        if !native_host_path.is_file() {
            return Err(Error::NativeHostExecutableNotAFile(
                native_host_path.display().to_string(),
            ));
        }

        Ok(Self {
            name: RZN_NATIVE_HOST_NAME.to_string(),
            description: "RZN Browser Native Host".to_string(),
            path: native_host_path.display().to_string(),
            manifest_type: "stdio".to_string(),
            allowed_origins: vec![format!("chrome-extension://{}/", extension_id)],
        })
    }

    pub fn filename(&self) -> String {
        format!("{}.json", self.name)
    }

    pub fn to_json_pretty(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn write_to_dir(&self, dir: impl AsRef<Path>) -> Result<PathBuf> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        let path = dir.join(self.filename());
        std::fs::write(&path, self.to_json_pretty()?)?;
        Ok(path)
    }
}

/// Best-effort location for Chrome's user-level Native Messaging Hosts directory.
///
/// Notes:
/// - macOS/Linux use filesystem directories.
/// - Windows uses registry-based registration; this helper returns an error.
pub fn chrome_native_host_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or(Error::MissingHomeDir)?;

    #[cfg(target_os = "macos")]
    {
        return Ok(home.join("Library/Application Support/Google/Chrome/NativeMessagingHosts"));
    }

    #[cfg(target_os = "linux")]
    {
        return Ok(home.join(".config/google-chrome/NativeMessagingHosts"));
    }

    #[cfg(target_os = "windows")]
    {
        let _ = home;
        return Err(Error::UnsupportedPlatform);
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = home;
        Err(Error::UnsupportedPlatform)
    }
}

/// Install the RZN native host manifest into Chrome's user-level directory.
///
/// This writes `${chrome_native_host_dir()}/com.rzn.browser.broker.json`.
pub fn install_rzn_native_host_for_chrome(
    native_host_path: impl AsRef<Path>,
    extension_id: &str,
) -> Result<PathBuf> {
    let dir = chrome_native_host_dir()?;
    let manifest = NativeMessagingHostManifest::rzn_native_host(native_host_path, extension_id)?;
    manifest.write_to_dir(dir)
}

/// Attempt to resolve a usable native host executable path for local development and packaging.
///
/// Resolution order:
/// 1) `RZN_NATIVE_HOST_PATH` env var (explicit)
/// 2) sibling of current executable (e.g. `./rzn-native-host`)
/// 3) repo-local `target/{release,debug}/rzn-native-host` (best-effort)
pub fn resolve_native_host_executable_path() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("RZN_NATIVE_HOST_PATH") {
        let path = PathBuf::from(p);
        if path.exists() {
            return Ok(path);
        }
        return Err(Error::NativeHostExecutableMissing(
            path.display().to_string(),
        ));
    }

    let exe = std::env::current_exe()?;
    let exe_dir = exe.parent().unwrap_or_else(|| Path::new("."));

    #[cfg(target_os = "windows")]
    const BIN_NAME: &str = "rzn-native-host.exe";
    #[cfg(not(target_os = "windows"))]
    const BIN_NAME: &str = "rzn-native-host";

    let sibling = exe_dir.join(BIN_NAME);
    if sibling.exists() {
        return Ok(sibling);
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    for candidate in [
        cwd.join("target").join("release").join(BIN_NAME),
        cwd.join("target").join("debug").join(BIN_NAME),
    ] {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(Error::NativeHostExecutableMissing(BIN_NAME.to_string()))
}

/// Install the native host manifest using an auto-resolved executable path.
pub fn install_rzn_native_host_for_chrome_auto(extension_id: &str) -> Result<PathBuf> {
    let native_host_path = resolve_native_host_executable_path()?;
    install_rzn_native_host_for_chrome(native_host_path, extension_id)
}

/// Remove the RZN native host manifest from Chrome's user-level directory.
pub fn uninstall_rzn_native_host_for_chrome() -> Result<()> {
    let dir = chrome_native_host_dir()?;
    let path = dir.join(format!("{}.json", RZN_NATIVE_HOST_NAME));
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

pub fn native_host_manifest_path_for_chrome() -> Result<PathBuf> {
    Ok(chrome_native_host_dir()?.join(format!("{}.json", RZN_NATIVE_HOST_NAME)))
}

pub fn read_manifest(path: impl AsRef<Path>) -> Result<NativeMessagingHostManifest> {
    let contents = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&contents)?)
}

pub fn read_installed_rzn_native_host_manifest_for_chrome(
) -> Result<Option<NativeMessagingHostManifest>> {
    let path = native_host_manifest_path_for_chrome()?;
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(read_manifest(path)?))
}
