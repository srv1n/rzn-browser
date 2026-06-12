use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Chrome native messaging host name used by the RZN extension.
pub const RZN_NATIVE_HOST_NAME: &str = "com.rzn.browser.broker";

/// Deterministic unpacked extension ID derived from `extension/src/manifest.base.json` `key`.
pub const RZN_DEV_EXTENSION_ID: &str = "bogjdnehdficgkhklinmnbgiiofbamji";

/// Default development native-messaging allowed origin.
pub const RZN_DEV_EXTENSION_ORIGIN: &str = "chrome-extension://bogjdnehdficgkhklinmnbgiiofbamji/";

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to resolve home directory")]
    MissingHomeDir,

    #[error("unsupported platform for automatic native host installation")]
    UnsupportedPlatform,

    #[error("unsupported native host target: browser={browser}, os={os}")]
    UnsupportedBrowserTarget { browser: String, os: String },

    #[error("native host executable does not exist: {0}")]
    NativeHostExecutableMissing(String),

    #[error("native host executable is not a file: {0}")]
    NativeHostExecutableNotAFile(String),

    #[error("extension id is empty")]
    EmptyExtensionId,

    #[error("invalid extension origin `{origin}`: {reason}")]
    InvalidExtensionOrigin { origin: String, reason: String },

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Chromium-family browser targets supported by native-host registration helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BrowserKind {
    Chrome,
    ChromeForTesting,
    Chromium,
    Edge,
    EdgeBeta,
    EdgeDev,
    EdgeCanary,
}

impl BrowserKind {
    pub const ALL: [Self; 7] = [
        Self::Chrome,
        Self::ChromeForTesting,
        Self::Chromium,
        Self::Edge,
        Self::EdgeBeta,
        Self::EdgeDev,
        Self::EdgeCanary,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::ChromeForTesting => "chrome-for-testing",
            Self::Chromium => "chromium",
            Self::Edge => "edge",
            Self::EdgeBeta => "edge-beta",
            Self::EdgeDev => "edge-dev",
            Self::EdgeCanary => "edge-canary",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Chrome => "Google Chrome",
            Self::ChromeForTesting => "Chrome for Testing",
            Self::Chromium => "Chromium",
            Self::Edge => "Microsoft Edge",
            Self::EdgeBeta => "Microsoft Edge Beta",
            Self::EdgeDev => "Microsoft Edge Dev",
            Self::EdgeCanary => "Microsoft Edge Canary",
        }
    }

    pub fn manifest_directory_identity(self) -> BrowserManifestDirectoryIdentity {
        match self {
            Self::Chrome => BrowserManifestDirectoryIdentity {
                linux_config_dir: "google-chrome",
                macos_application_support_dir: "Google/Chrome",
            },
            Self::ChromeForTesting => BrowserManifestDirectoryIdentity {
                linux_config_dir: "google-chrome-for-testing",
                macos_application_support_dir: "Google/ChromeForTesting",
            },
            Self::Chromium => BrowserManifestDirectoryIdentity {
                linux_config_dir: "chromium",
                macos_application_support_dir: "Chromium",
            },
            Self::Edge => BrowserManifestDirectoryIdentity {
                linux_config_dir: "microsoft-edge",
                macos_application_support_dir: "Microsoft Edge",
            },
            Self::EdgeBeta => BrowserManifestDirectoryIdentity {
                linux_config_dir: "microsoft-edge-beta",
                macos_application_support_dir: "Microsoft Edge Beta",
            },
            Self::EdgeDev => BrowserManifestDirectoryIdentity {
                linux_config_dir: "microsoft-edge-dev",
                macos_application_support_dir: "Microsoft Edge Dev",
            },
            Self::EdgeCanary => BrowserManifestDirectoryIdentity {
                linux_config_dir: "microsoft-edge-canary",
                macos_application_support_dir: "Microsoft Edge Canary",
            },
        }
    }

    pub fn windows_registry_parent_key(self) -> &'static str {
        match self {
            Self::Chrome => "Software\\Google\\Chrome\\NativeMessagingHosts",
            Self::ChromeForTesting => "Software\\Google\\Chrome for Testing\\NativeMessagingHosts",
            Self::Chromium => "Software\\Chromium\\NativeMessagingHosts",
            Self::Edge => "Software\\Microsoft\\Edge\\NativeMessagingHosts",
            Self::EdgeBeta => "Software\\Microsoft\\Edge Beta\\NativeMessagingHosts",
            Self::EdgeDev => "Software\\Microsoft\\Edge Dev\\NativeMessagingHosts",
            Self::EdgeCanary => "Software\\Microsoft\\Edge Canary\\NativeMessagingHosts",
        }
    }

    pub fn is_supported_on_current_os(self) -> bool {
        self.supported_on_os(NativeHostInstallOs::current())
    }

    fn supported_on_os(self, os: NativeHostInstallOs) -> bool {
        match os {
            NativeHostInstallOs::Linux => !matches!(self, Self::EdgeCanary),
            NativeHostInstallOs::Macos | NativeHostInstallOs::Windows => true,
            NativeHostInstallOs::Other(_) => false,
        }
    }
}

impl std::fmt::Display for BrowserKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.slug())
    }
}

impl FromStr for BrowserKind {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let normalized = value.trim().to_ascii_lowercase();
        for kind in Self::ALL {
            if normalized == kind.slug() {
                return Ok(kind);
            }
        }
        Err(format!("unknown browser target: {value}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserManifestDirectoryIdentity {
    pub linux_config_dir: &'static str,
    pub macos_application_support_dir: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserTargetMetadata {
    pub kind: BrowserKind,
    pub display_name: &'static str,
    pub slug: &'static str,
    pub manifest_directory: BrowserManifestDirectoryIdentity,
    pub windows_registry_parent_key: &'static str,
    pub supported_on_current_os: bool,
}

pub fn browser_target_metadata(kind: BrowserKind) -> BrowserTargetMetadata {
    BrowserTargetMetadata {
        kind,
        display_name: kind.display_name(),
        slug: kind.slug(),
        manifest_directory: kind.manifest_directory_identity(),
        windows_registry_parent_key: kind.windows_registry_parent_key(),
        supported_on_current_os: kind.is_supported_on_current_os(),
    }
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
        Self::rzn_native_host_with_origins(native_host_path, [extension_id])
    }

    /// Create a manifest for the RZN native host with multiple extension origins or IDs.
    pub fn rzn_native_host_with_origins<I, S>(
        native_host_path: impl AsRef<Path>,
        allowed_origins: I,
    ) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
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
        let native_host_path = std::fs::canonicalize(native_host_path)?;
        let allowed_origins = normalize_extension_origins(allowed_origins)?;

        Ok(Self {
            name: RZN_NATIVE_HOST_NAME.to_string(),
            description: "RZN Browser Native Host".to_string(),
            path: native_host_path.display().to_string(),
            manifest_type: "stdio".to_string(),
            allowed_origins,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeHostInstallReport {
    pub browser: BrowserKind,
    pub manifest_path: PathBuf,
    pub native_host_path: PathBuf,
    pub allowed_origins: Vec<String>,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeHostUninstallReport {
    pub browser: BrowserKind,
    pub manifest_path: PathBuf,
    pub removed: bool,
}

pub fn normalize_extension_origins<I, S>(origins_or_ids: I) -> Result<Vec<String>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();

    for value in origins_or_ids {
        let origin = normalize_extension_origin(value.as_ref())?;
        if seen.insert(origin.clone()) {
            normalized.push(origin);
        }
    }

    if normalized.is_empty() {
        return Err(Error::EmptyExtensionId);
    }

    Ok(normalized)
}

pub fn normalize_extension_origin(origin_or_id: &str) -> Result<String> {
    let value = origin_or_id.trim();
    if value.is_empty() {
        return Err(Error::EmptyExtensionId);
    }

    if value == "*" {
        return Err(invalid_extension_origin(
            value,
            "wildcard origins are not allowed",
        ));
    }

    if let Some(extension_id) = value.strip_prefix("chrome-extension://") {
        if !extension_id.ends_with('/') {
            return Err(invalid_extension_origin(
                value,
                "chrome-extension origins must end with a trailing slash",
            ));
        }

        let extension_id = &extension_id[..extension_id.len() - 1];
        validate_extension_id(value, extension_id)?;
        return Ok(value.to_string());
    }

    if value.contains("://") {
        return Err(invalid_extension_origin(
            value,
            "only chrome-extension origins are allowed",
        ));
    }

    validate_extension_id(value, value)?;
    Ok(format!("chrome-extension://{value}/"))
}

fn validate_extension_id(original: &str, extension_id: &str) -> Result<()> {
    if extension_id.is_empty() {
        return Err(Error::EmptyExtensionId);
    }

    if extension_id == "*" {
        return Err(invalid_extension_origin(
            original,
            "wildcard extension IDs are not allowed",
        ));
    }

    if extension_id.len() != 32 || !extension_id.bytes().all(|b| (b'a'..=b'p').contains(&b)) {
        return Err(invalid_extension_origin(
            original,
            "extension IDs must be 32 lowercase characters from a through p",
        ));
    }

    Ok(())
}

fn invalid_extension_origin(origin: &str, reason: &str) -> Error {
    Error::InvalidExtensionOrigin {
        origin: origin.to_string(),
        reason: reason.to_string(),
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeHostInstallOs {
    Linux,
    Macos,
    Windows,
    Other(&'static str),
}

impl NativeHostInstallOs {
    fn current() -> Self {
        #[cfg(target_os = "linux")]
        {
            return Self::Linux;
        }

        #[cfg(target_os = "macos")]
        {
            Self::Macos
        }

        #[cfg(target_os = "windows")]
        {
            return Self::Windows;
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            Self::Other(std::env::consts::OS)
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Windows => "windows",
            Self::Other(os) => os,
        }
    }
}

fn unsupported_browser_target(kind: BrowserKind, os: NativeHostInstallOs) -> Error {
    Error::UnsupportedBrowserTarget {
        browser: kind.slug().to_string(),
        os: os.as_str().to_string(),
    }
}

fn native_host_dir_for_browser_on_os(
    kind: BrowserKind,
    home: &Path,
    os: NativeHostInstallOs,
) -> Result<PathBuf> {
    if !kind.supported_on_os(os) {
        return Err(unsupported_browser_target(kind, os));
    }

    let identity = kind.manifest_directory_identity();
    match os {
        NativeHostInstallOs::Linux => Ok(home
            .join(".config")
            .join(identity.linux_config_dir)
            .join("NativeMessagingHosts")),
        NativeHostInstallOs::Macos => Ok(home
            .join("Library")
            .join("Application Support")
            .join(identity.macos_application_support_dir)
            .join("NativeMessagingHosts")),
        NativeHostInstallOs::Windows | NativeHostInstallOs::Other(_) => {
            Err(unsupported_browser_target(kind, os))
        }
    }
}

fn windows_native_host_manifest_path_at_home(home: &Path) -> PathBuf {
    home.join("AppData")
        .join("Local")
        .join("RZN")
        .join("native-hosts")
        .join(format!("{}.json", RZN_NATIVE_HOST_NAME))
}

fn native_host_manifest_path_for_browser_on_os(
    kind: BrowserKind,
    home: &Path,
    os: NativeHostInstallOs,
) -> Result<PathBuf> {
    if !kind.supported_on_os(os) {
        return Err(unsupported_browser_target(kind, os));
    }

    match os {
        NativeHostInstallOs::Windows => Ok(windows_native_host_manifest_path_at_home(home)),
        NativeHostInstallOs::Linux | NativeHostInstallOs::Macos => {
            Ok(native_host_dir_for_browser_on_os(kind, home, os)?
                .join(format!("{}.json", RZN_NATIVE_HOST_NAME)))
        }
        NativeHostInstallOs::Other(_) => Err(unsupported_browser_target(kind, os)),
    }
}

/// Best-effort location for a browser's user-level Native Messaging Hosts directory.
///
/// Notes:
/// - macOS/Linux use filesystem directories.
/// - Windows uses registry-based registration; use
///   [`native_host_registry_key_for_browser`] for the registry key.
pub fn native_host_dir_for_browser(kind: BrowserKind) -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or(Error::MissingHomeDir)?;
    native_host_dir_for_browser_on_os(kind, &home, NativeHostInstallOs::current())
}

/// User-level Windows registry key for this native host under a browser target.
pub fn native_host_registry_key_for_browser(kind: BrowserKind) -> String {
    format!(
        "{}\\{}",
        kind.windows_registry_parent_key(),
        RZN_NATIVE_HOST_NAME
    )
}

/// Path to this native host manifest under a browser target's user-level directory.
pub fn native_host_manifest_path_for_browser(kind: BrowserKind) -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or(Error::MissingHomeDir)?;
    native_host_manifest_path_for_browser_on_os(kind, &home, NativeHostInstallOs::current())
}

/// Best-effort location for Chrome's user-level Native Messaging Hosts directory.
///
/// Notes:
/// - macOS/Linux use filesystem directories.
/// - Windows uses registry-based registration; this helper returns an error.
#[deprecated(note = "use native_host_dir_for_browser(BrowserKind::Chrome)")]
pub fn chrome_native_host_dir() -> Result<PathBuf> {
    native_host_dir_for_browser(BrowserKind::Chrome)
}

/// Install the RZN native host manifest into a browser's user-level directory.
///
/// This writes `${native_host_dir_for_browser(browser)}/com.rzn.browser.broker.json`.
pub fn install_rzn_native_host_for_browser(
    browser: BrowserKind,
    native_host_path: impl AsRef<Path>,
    extension_id: &str,
) -> Result<NativeHostInstallReport> {
    install_rzn_native_host_for_browser_with_origins(browser, native_host_path, [extension_id])
}

pub fn install_rzn_native_host_for_browser_with_origins<I, S>(
    browser: BrowserKind,
    native_host_path: impl AsRef<Path>,
    allowed_origins: I,
) -> Result<NativeHostInstallReport>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let home = dirs::home_dir().ok_or(Error::MissingHomeDir)?;
    install_rzn_native_host_for_browser_with_origins_at_home(
        browser,
        &home,
        NativeHostInstallOs::current(),
        native_host_path,
        allowed_origins,
    )
}

fn install_rzn_native_host_for_browser_with_origins_at_home<I, S>(
    browser: BrowserKind,
    home: &Path,
    os: NativeHostInstallOs,
    native_host_path: impl AsRef<Path>,
    allowed_origins: I,
) -> Result<NativeHostInstallReport>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut manifest = NativeMessagingHostManifest::rzn_native_host_with_origins(
        native_host_path,
        allowed_origins,
    )?;
    let manifest_path = native_host_manifest_path_for_browser_on_os(browser, home, os)?;
    if os == NativeHostInstallOs::Windows {
        manifest.allowed_origins =
            merged_windows_shared_manifest_origins(&manifest_path, manifest.allowed_origins)?;
    }
    let manifest_json = manifest.to_json_pretty()?;
    let changed = match std::fs::read_to_string(&manifest_path) {
        Ok(existing) => existing != manifest_json,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => return Err(error.into()),
    };

    if changed {
        let dir = manifest_path
            .parent()
            .ok_or_else(|| Error::Io(std::io::Error::other("manifest path has no parent")))?;
        std::fs::create_dir_all(dir)?;
        std::fs::write(&manifest_path, manifest_json)?;
    }
    install_windows_native_host_registry(browser, &manifest_path, os)?;

    Ok(NativeHostInstallReport {
        browser,
        manifest_path,
        native_host_path: PathBuf::from(manifest.path),
        allowed_origins: manifest.allowed_origins,
        changed,
    })
}

fn merged_windows_shared_manifest_origins(
    manifest_path: &Path,
    requested_origins: Vec<String>,
) -> Result<Vec<String>> {
    let existing = match std::fs::read_to_string(manifest_path) {
        Ok(contents) => serde_json::from_str::<NativeMessagingHostManifest>(&contents)
            .ok()
            .map(|manifest| manifest.allowed_origins)
            .unwrap_or_default(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
    };

    normalize_extension_origins(
        existing
            .iter()
            .map(String::as_str)
            .chain(requested_origins.iter().map(String::as_str)),
    )
}

/// Install the RZN native host manifest into Chrome's user-level directory.
///
/// This writes `${chrome_native_host_dir()}/com.rzn.browser.broker.json`.
#[deprecated(note = "use install_rzn_native_host_for_browser(BrowserKind::Chrome, ...)")]
pub fn install_rzn_native_host_for_chrome(
    native_host_path: impl AsRef<Path>,
    extension_id: &str,
) -> Result<PathBuf> {
    Ok(
        install_rzn_native_host_for_browser(BrowserKind::Chrome, native_host_path, extension_id)?
            .manifest_path,
    )
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
pub fn install_rzn_native_host_for_browser_auto(
    browser: BrowserKind,
    extension_id: &str,
) -> Result<NativeHostInstallReport> {
    let native_host_path = resolve_native_host_executable_path()?;
    install_rzn_native_host_for_browser(browser, native_host_path, extension_id)
}

/// Install the native host manifest using an auto-resolved executable path.
#[deprecated(note = "use install_rzn_native_host_for_browser_auto(BrowserKind::Chrome, ...)")]
pub fn install_rzn_native_host_for_chrome_auto(extension_id: &str) -> Result<PathBuf> {
    Ok(install_rzn_native_host_for_browser_auto(BrowserKind::Chrome, extension_id)?.manifest_path)
}

/// Remove the RZN native host manifest from a browser's user-level directory.
pub fn uninstall_rzn_native_host_for_browser(
    browser: BrowserKind,
) -> Result<NativeHostUninstallReport> {
    let home = dirs::home_dir().ok_or(Error::MissingHomeDir)?;
    uninstall_rzn_native_host_for_browser_at_home(browser, &home, NativeHostInstallOs::current())
}

fn uninstall_rzn_native_host_for_browser_at_home(
    browser: BrowserKind,
    home: &Path,
    os: NativeHostInstallOs,
) -> Result<NativeHostUninstallReport> {
    let manifest_path = native_host_manifest_path_for_browser_on_os(browser, home, os)?;
    let removed = if os == NativeHostInstallOs::Windows {
        uninstall_windows_native_host_registry(browser, os)?
    } else if manifest_path.exists() {
        std::fs::remove_file(&manifest_path)?;
        true
    } else {
        false
    };

    Ok(NativeHostUninstallReport {
        browser,
        manifest_path,
        removed,
    })
}

#[cfg(target_os = "windows")]
fn install_windows_native_host_registry(
    browser: BrowserKind,
    manifest_path: &Path,
    os: NativeHostInstallOs,
) -> Result<()> {
    if os != NativeHostInstallOs::Windows {
        return Ok(());
    }

    let key = format!("HKCU\\{}", native_host_registry_key_for_browser(browser));
    let status = std::process::Command::new("reg.exe")
        .arg("ADD")
        .arg(&key)
        .arg("/ve")
        .arg("/t")
        .arg("REG_SZ")
        .arg("/d")
        .arg(manifest_path)
        .arg("/f")
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("reg.exe ADD failed for {key} with status {status}"),
        )))
    }
}

#[cfg(not(target_os = "windows"))]
fn install_windows_native_host_registry(
    _browser: BrowserKind,
    _manifest_path: &Path,
    _os: NativeHostInstallOs,
) -> Result<()> {
    Ok(())
}

#[cfg(target_os = "windows")]
fn uninstall_windows_native_host_registry(
    browser: BrowserKind,
    os: NativeHostInstallOs,
) -> Result<bool> {
    if os != NativeHostInstallOs::Windows {
        return Ok(false);
    }

    let key = format!("HKCU\\{}", native_host_registry_key_for_browser(browser));
    let status = std::process::Command::new("reg.exe")
        .arg("DELETE")
        .arg(&key)
        .arg("/f")
        .status()?;
    if status.success() {
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(not(target_os = "windows"))]
fn uninstall_windows_native_host_registry(
    _browser: BrowserKind,
    _os: NativeHostInstallOs,
) -> Result<bool> {
    Ok(false)
}

/// Remove the RZN native host manifest from Chrome's user-level directory.
#[deprecated(note = "use uninstall_rzn_native_host_for_browser(BrowserKind::Chrome)")]
pub fn uninstall_rzn_native_host_for_chrome() -> Result<()> {
    uninstall_rzn_native_host_for_browser(BrowserKind::Chrome).map(|_| ())
}

#[deprecated(note = "use native_host_manifest_path_for_browser(BrowserKind::Chrome)")]
pub fn native_host_manifest_path_for_chrome() -> Result<PathBuf> {
    native_host_manifest_path_for_browser(BrowserKind::Chrome)
}

pub fn read_manifest(path: impl AsRef<Path>) -> Result<NativeMessagingHostManifest> {
    let contents = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&contents)?)
}

pub fn read_installed_rzn_native_host_manifest_for_browser(
    browser: BrowserKind,
) -> Result<Option<NativeMessagingHostManifest>> {
    let path = native_host_manifest_path_for_browser(browser)?;
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(read_manifest(path)?))
}

#[deprecated(note = "use read_installed_rzn_native_host_manifest_for_browser(BrowserKind::Chrome)")]
pub fn read_installed_rzn_native_host_manifest_for_chrome(
) -> Result<Option<NativeMessagingHostManifest>> {
    read_installed_rzn_native_host_manifest_for_browser(BrowserKind::Chrome)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        PathBuf::from("/home/rzn")
    }

    fn temp_native_host_executable(name: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rzn-sdk-native-host-{name}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::write(&path, b"native host").unwrap();
        path
    }

    fn temp_home(name: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "rzn-sdk-home-{name}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn browser_kind_slugs_are_stable_cli_names() {
        assert_eq!(BrowserKind::Chrome.slug(), "chrome");
        assert_eq!(BrowserKind::ChromeForTesting.slug(), "chrome-for-testing");
        assert_eq!(BrowserKind::Chromium.slug(), "chromium");
        assert_eq!(BrowserKind::Edge.slug(), "edge");
        assert_eq!(BrowserKind::EdgeBeta.slug(), "edge-beta");
        assert_eq!(BrowserKind::EdgeDev.slug(), "edge-dev");
        assert_eq!(BrowserKind::EdgeCanary.slug(), "edge-canary");
        assert_eq!("edge-dev".parse::<BrowserKind>(), Ok(BrowserKind::EdgeDev));
    }

    #[test]
    fn linux_manifest_directories_are_browser_specific() {
        assert_eq!(
            native_host_dir_for_browser_on_os(
                BrowserKind::Chrome,
                &home(),
                NativeHostInstallOs::Linux
            )
            .unwrap(),
            PathBuf::from("/home/rzn/.config/google-chrome/NativeMessagingHosts")
        );
        assert_eq!(
            native_host_dir_for_browser_on_os(
                BrowserKind::Chromium,
                &home(),
                NativeHostInstallOs::Linux
            )
            .unwrap(),
            PathBuf::from("/home/rzn/.config/chromium/NativeMessagingHosts")
        );
        assert_eq!(
            native_host_dir_for_browser_on_os(
                BrowserKind::Edge,
                &home(),
                NativeHostInstallOs::Linux
            )
            .unwrap(),
            PathBuf::from("/home/rzn/.config/microsoft-edge/NativeMessagingHosts")
        );
    }

    #[test]
    fn macos_manifest_directories_are_browser_specific() {
        let home = PathBuf::from("/Users/rzn");
        assert_eq!(
            native_host_dir_for_browser_on_os(
                BrowserKind::Chrome,
                &home,
                NativeHostInstallOs::Macos
            )
            .unwrap(),
            PathBuf::from(
                "/Users/rzn/Library/Application Support/Google/Chrome/NativeMessagingHosts"
            )
        );
        assert_eq!(
            native_host_dir_for_browser_on_os(
                BrowserKind::ChromeForTesting,
                &home,
                NativeHostInstallOs::Macos
            )
            .unwrap(),
            PathBuf::from(
                "/Users/rzn/Library/Application Support/Google/ChromeForTesting/NativeMessagingHosts"
            )
        );
        assert_eq!(
            native_host_dir_for_browser_on_os(
                BrowserKind::Chromium,
                &home,
                NativeHostInstallOs::Macos
            )
            .unwrap(),
            PathBuf::from("/Users/rzn/Library/Application Support/Chromium/NativeMessagingHosts")
        );
        assert_eq!(
            native_host_dir_for_browser_on_os(BrowserKind::Edge, &home, NativeHostInstallOs::Macos)
                .unwrap(),
            PathBuf::from(
                "/Users/rzn/Library/Application Support/Microsoft Edge/NativeMessagingHosts"
            )
        );
    }

    #[test]
    fn windows_registry_keys_are_browser_specific() {
        assert_eq!(
            native_host_registry_key_for_browser(BrowserKind::Chrome),
            "Software\\Google\\Chrome\\NativeMessagingHosts\\com.rzn.browser.broker"
        );
        assert_eq!(
            native_host_registry_key_for_browser(BrowserKind::Chromium),
            "Software\\Chromium\\NativeMessagingHosts\\com.rzn.browser.broker"
        );
        assert_eq!(
            native_host_registry_key_for_browser(BrowserKind::Edge),
            "Software\\Microsoft\\Edge\\NativeMessagingHosts\\com.rzn.browser.broker"
        );
    }

    #[test]
    fn unsupported_targets_report_browser_and_os() {
        let err = native_host_dir_for_browser_on_os(
            BrowserKind::EdgeCanary,
            &home(),
            NativeHostInstallOs::Linux,
        )
        .unwrap_err();

        match err {
            Error::UnsupportedBrowserTarget { browser, os } => {
                assert_eq!(browser, "edge-canary");
                assert_eq!(os, "linux");
            }
            other => panic!("expected unsupported browser target, got {other:?}"),
        }
    }

    #[test]
    fn native_host_installer_manifest_filename_is_exact_for_primary_targets() {
        for browser in [
            BrowserKind::Chrome,
            BrowserKind::Chromium,
            BrowserKind::Edge,
        ] {
            let path =
                native_host_dir_for_browser_on_os(browser, &home(), NativeHostInstallOs::Linux)
                    .unwrap()
                    .join(format!("{RZN_NATIVE_HOST_NAME}.json"));

            assert_eq!(
                path.file_name().and_then(|name| name.to_str()),
                Some("com.rzn.browser.broker.json")
            );
        }
    }

    #[test]
    fn native_host_installer_primary_target_paths_cover_supported_oses() {
        let linux_home = PathBuf::from("/home/rzn");
        let macos_home = PathBuf::from("/Users/rzn");

        let cases = [
            (
                BrowserKind::Chrome,
                NativeHostInstallOs::Linux,
                &linux_home,
                "/home/rzn/.config/google-chrome/NativeMessagingHosts",
            ),
            (
                BrowserKind::Chromium,
                NativeHostInstallOs::Linux,
                &linux_home,
                "/home/rzn/.config/chromium/NativeMessagingHosts",
            ),
            (
                BrowserKind::Edge,
                NativeHostInstallOs::Linux,
                &linux_home,
                "/home/rzn/.config/microsoft-edge/NativeMessagingHosts",
            ),
            (
                BrowserKind::Chrome,
                NativeHostInstallOs::Macos,
                &macos_home,
                "/Users/rzn/Library/Application Support/Google/Chrome/NativeMessagingHosts",
            ),
            (
                BrowserKind::Chromium,
                NativeHostInstallOs::Macos,
                &macos_home,
                "/Users/rzn/Library/Application Support/Chromium/NativeMessagingHosts",
            ),
            (
                BrowserKind::Edge,
                NativeHostInstallOs::Macos,
                &macos_home,
                "/Users/rzn/Library/Application Support/Microsoft Edge/NativeMessagingHosts",
            ),
        ];

        for (browser, os, home, expected) in cases {
            assert_eq!(
                native_host_dir_for_browser_on_os(browser, home, os).unwrap(),
                PathBuf::from(expected)
            );
        }

        assert_eq!(
            native_host_registry_key_for_browser(BrowserKind::Chrome),
            "Software\\Google\\Chrome\\NativeMessagingHosts\\com.rzn.browser.broker"
        );
        assert_eq!(
            native_host_registry_key_for_browser(BrowserKind::Chromium),
            "Software\\Chromium\\NativeMessagingHosts\\com.rzn.browser.broker"
        );
        assert_eq!(
            native_host_registry_key_for_browser(BrowserKind::Edge),
            "Software\\Microsoft\\Edge\\NativeMessagingHosts\\com.rzn.browser.broker"
        );
    }

    #[test]
    fn native_host_installer_writes_deduped_multi_origin_manifest_to_temp_home() {
        let home = temp_home("installer-deduped-origins");
        let native_host = temp_native_host_executable("installer-deduped-origins");

        let report = install_rzn_native_host_for_browser_with_origins_at_home(
            BrowserKind::Chromium,
            &home,
            NativeHostInstallOs::Linux,
            &native_host,
            [
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ],
        )
        .unwrap();

        assert_eq!(
            report.manifest_path,
            home.join(".config/chromium/NativeMessagingHosts")
                .join("com.rzn.browser.broker.json")
        );
        assert_eq!(
            report.allowed_origins,
            vec![
                "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/".to_string(),
                "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/".to_string(),
            ]
        );

        let manifest = read_manifest(report.manifest_path).unwrap();
        assert_eq!(manifest.allowed_origins, report.allowed_origins);
    }

    #[test]
    fn windows_shared_manifest_install_unions_allowed_origins_across_browsers() {
        let home = temp_home("installer-windows-shared-origin-union");
        let native_host = temp_native_host_executable("installer-windows-shared-origin-union");

        let chrome = install_rzn_native_host_for_browser_with_origins_at_home(
            BrowserKind::Chrome,
            &home,
            NativeHostInstallOs::Windows,
            &native_host,
            ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
        )
        .unwrap();
        let edge = install_rzn_native_host_for_browser_with_origins_at_home(
            BrowserKind::Edge,
            &home,
            NativeHostInstallOs::Windows,
            &native_host,
            ["bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"],
        )
        .unwrap();

        assert_eq!(chrome.manifest_path, edge.manifest_path);
        assert_eq!(
            edge.allowed_origins,
            vec![
                "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/".to_string(),
                "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/".to_string(),
            ]
        );
        let manifest = read_manifest(edge.manifest_path).unwrap();
        assert_eq!(manifest.allowed_origins, edge.allowed_origins);
    }

    #[test]
    fn native_host_installer_rejects_invalid_origin_before_creating_dirs() {
        let home = temp_home("installer-invalid-origin");
        let native_host = temp_native_host_executable("installer-invalid-origin");

        let err = install_rzn_native_host_for_browser_with_origins_at_home(
            BrowserKind::Edge,
            &home,
            NativeHostInstallOs::Linux,
            &native_host,
            ["https://example.com"],
        )
        .unwrap_err();

        assert!(matches!(err, Error::InvalidExtensionOrigin { .. }));
        assert!(
            !home.exists(),
            "invalid origin must not create temp home dirs"
        );
    }

    #[test]
    fn native_host_installer_uninstall_removes_only_selected_temp_manifest() {
        let home = temp_home("installer-uninstall-selected");
        let native_host = temp_native_host_executable("installer-uninstall-selected");

        let chrome = install_rzn_native_host_for_browser_with_origins_at_home(
            BrowserKind::Chrome,
            &home,
            NativeHostInstallOs::Linux,
            &native_host,
            ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
        )
        .unwrap();
        let chromium = install_rzn_native_host_for_browser_with_origins_at_home(
            BrowserKind::Chromium,
            &home,
            NativeHostInstallOs::Linux,
            &native_host,
            ["bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"],
        )
        .unwrap();

        let uninstall = uninstall_rzn_native_host_for_browser_at_home(
            BrowserKind::Chromium,
            &home,
            NativeHostInstallOs::Linux,
        )
        .unwrap();

        assert_eq!(uninstall.manifest_path, chromium.manifest_path);
        assert!(uninstall.removed);
        assert!(chrome.manifest_path.exists());
        assert!(!chromium.manifest_path.exists());
    }

    #[test]
    fn normalizes_and_deduplicates_extension_origins() {
        let chrome_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let edge_id = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

        assert_eq!(
            normalize_extension_origins([
                chrome_id,
                "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/",
                chrome_id,
            ])
            .unwrap(),
            vec![
                "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/".to_string(),
                "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/".to_string(),
            ]
        );

        assert_eq!(
            normalize_extension_origin(edge_id).unwrap(),
            "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/"
        );
        assert_eq!(
            normalize_extension_origin(RZN_DEV_EXTENSION_ID).unwrap(),
            RZN_DEV_EXTENSION_ORIGIN
        );
    }

    #[test]
    fn rejects_invalid_extension_origins() {
        for value in [
            "",
            "*",
            "chrome-extension://*/",
            "https://example.com",
            "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "chrome-extension://not-valid/",
            "qrstqrstqrstqrstqrstqrstqrstqrst",
        ] {
            assert!(
                normalize_extension_origin(value).is_err(),
                "expected {value:?} to be rejected"
            );
        }
    }

    #[test]
    fn one_origin_manifest_json_matches_compatibility_shape() {
        let native_host = temp_native_host_executable("one-origin");
        let native_host = std::fs::canonicalize(native_host).unwrap();
        let manifest = NativeMessagingHostManifest::rzn_native_host(
            &native_host,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap();
        assert!(Path::new(&manifest.path).is_absolute());

        let path_json = serde_json::to_string(&native_host.display().to_string()).unwrap();
        let expected = format!(
            "{{
  \"name\": \"{RZN_NATIVE_HOST_NAME}\",
  \"description\": \"RZN Browser Native Host\",
  \"path\": {path_json},
  \"type\": \"stdio\",
  \"allowed_origins\": [
    \"chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/\"
  ]
}}"
        );

        assert_eq!(manifest.to_json_pretty().unwrap(), expected);
    }

    #[test]
    fn multi_origin_manifest_json_is_stable() {
        let native_host = temp_native_host_executable("multi-origin");
        let native_host = std::fs::canonicalize(native_host).unwrap();
        let manifest = NativeMessagingHostManifest::rzn_native_host_with_origins(
            &native_host,
            [
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ],
        )
        .unwrap();
        assert!(Path::new(&manifest.path).is_absolute());

        let path_json = serde_json::to_string(&native_host.display().to_string()).unwrap();
        let expected = format!(
            "{{
  \"name\": \"{RZN_NATIVE_HOST_NAME}\",
  \"description\": \"RZN Browser Native Host\",
  \"path\": {path_json},
  \"type\": \"stdio\",
  \"allowed_origins\": [
    \"chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/\",
    \"chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/\"
  ]
}}"
        );

        assert_eq!(manifest.to_json_pretty().unwrap(), expected);
    }

    #[test]
    fn install_report_tracks_changed_state_and_creates_parent_dirs() {
        let home = temp_home("install-idempotent");
        let native_host = temp_native_host_executable("install-idempotent");

        let first = install_rzn_native_host_for_browser_with_origins_at_home(
            BrowserKind::Chrome,
            &home,
            NativeHostInstallOs::Linux,
            &native_host,
            [
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/",
            ],
        )
        .unwrap();
        assert_eq!(first.browser, BrowserKind::Chrome);
        assert!(first.changed);
        assert_eq!(
            first.manifest_path,
            home.join(".config/google-chrome/NativeMessagingHosts")
                .join(format!("{RZN_NATIVE_HOST_NAME}.json"))
        );
        assert!(Path::new(&first.native_host_path).is_absolute());
        assert_eq!(
            first.allowed_origins,
            vec![
                "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/".to_string(),
                "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/".to_string(),
            ]
        );
        assert!(first.manifest_path.exists());

        let second = install_rzn_native_host_for_browser_with_origins_at_home(
            BrowserKind::Chrome,
            &home,
            NativeHostInstallOs::Linux,
            &native_host,
            [
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/",
            ],
        )
        .unwrap();
        assert!(!second.changed);
        assert_eq!(second.manifest_path, first.manifest_path);
        assert_eq!(second.native_host_path, first.native_host_path);
        assert_eq!(second.allowed_origins, first.allowed_origins);
    }

    #[test]
    fn installs_chrome_and_edge_manifests_to_separate_dirs_with_same_binary() {
        let home = temp_home("install-two-browsers");
        let native_host = temp_native_host_executable("install-two-browsers");

        let chrome = install_rzn_native_host_for_browser_with_origins_at_home(
            BrowserKind::Chrome,
            &home,
            NativeHostInstallOs::Macos,
            &native_host,
            ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
        )
        .unwrap();
        let edge = install_rzn_native_host_for_browser_with_origins_at_home(
            BrowserKind::Edge,
            &home,
            NativeHostInstallOs::Macos,
            &native_host,
            ["bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"],
        )
        .unwrap();

        assert_ne!(chrome.manifest_path, edge.manifest_path);
        assert_eq!(
            chrome.manifest_path,
            home.join("Library/Application Support/Google/Chrome/NativeMessagingHosts")
                .join(format!("{RZN_NATIVE_HOST_NAME}.json"))
        );
        assert_eq!(
            edge.manifest_path,
            home.join("Library/Application Support/Microsoft Edge/NativeMessagingHosts")
                .join(format!("{RZN_NATIVE_HOST_NAME}.json"))
        );
        assert_eq!(chrome.native_host_path, edge.native_host_path);
        assert!(chrome.manifest_path.exists());
        assert!(edge.manifest_path.exists());
    }

    #[test]
    fn uninstall_removes_only_target_browser_manifest() {
        let home = temp_home("uninstall-one-browser");
        let native_host = temp_native_host_executable("uninstall-one-browser");

        let chrome = install_rzn_native_host_for_browser_with_origins_at_home(
            BrowserKind::Chrome,
            &home,
            NativeHostInstallOs::Linux,
            &native_host,
            ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
        )
        .unwrap();
        let edge = install_rzn_native_host_for_browser_with_origins_at_home(
            BrowserKind::Edge,
            &home,
            NativeHostInstallOs::Linux,
            &native_host,
            ["bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"],
        )
        .unwrap();

        let uninstall = uninstall_rzn_native_host_for_browser_at_home(
            BrowserKind::Chrome,
            &home,
            NativeHostInstallOs::Linux,
        )
        .unwrap();

        assert_eq!(uninstall.browser, BrowserKind::Chrome);
        assert_eq!(uninstall.manifest_path, chrome.manifest_path);
        assert!(uninstall.removed);
        assert!(!chrome.manifest_path.exists());
        assert!(edge.manifest_path.exists());

        let uninstall_again = uninstall_rzn_native_host_for_browser_at_home(
            BrowserKind::Chrome,
            &home,
            NativeHostInstallOs::Linux,
        )
        .unwrap();
        assert!(!uninstall_again.removed);
    }
}
