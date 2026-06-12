use std::path::{Path, PathBuf};

pub const SUPERVISOR_SOCKET_FILENAME: &str = "rzn-supervisor.sock";
pub const SUPERVISOR_TOKEN_FILENAME: &str = "rzn-supervisor-token-v1";
pub const SUPERVISOR_SOCKET_ENV_KEYS: &[&str] = &[
    "RZN_LOCAL_RUNTIME_SOCKET_PATH",
    "RZN_SUPERVISOR_SOCKET_PATH",
];
pub const SUPERVISOR_TOKEN_ENV_KEYS: &[&str] =
    &["RZN_LOCAL_RUNTIME_TOKEN_PATH", "RZN_SUPERVISOR_TOKEN_PATH"];
pub const APP_BASE_ENV_KEYS: &[&str] = &[
    "RZN_APP_BASE_DIR",
    "RZN_SUPERVISOR_APP_BASE",
    "RZN_NATIVE_APP_BASE",
    "RZN_APP_BASE",
    "APP_BASE",
];

pub fn env_trimmed(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn first_env_path(keys: &[&str]) -> Option<PathBuf> {
    keys.iter()
        .find_map(|key| env_trimmed(key))
        .map(PathBuf::from)
}

pub fn supervisor_paths_for_app_base(app_base: &Path) -> (PathBuf, PathBuf) {
    (
        app_base.join("run").join(SUPERVISOR_SOCKET_FILENAME),
        app_base.join("secure").join(SUPERVISOR_TOKEN_FILENAME),
    )
}

pub fn infer_app_base_from_executable(exe: &Path) -> Option<PathBuf> {
    let resolved = std::fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());

    if let Some(parent) = resolved.parent() {
        if parent.file_name().and_then(|value| value.to_str()) == Some("bin") {
            return parent.parent().map(Path::to_path_buf);
        }
    }

    for ancestor in resolved.ancestors() {
        if ancestor.file_name().and_then(|value| value.to_str()) == Some("plugins") {
            return ancestor.parent().map(Path::to_path_buf);
        }
    }

    None
}

pub fn infer_current_app_base() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| infer_app_base_from_executable(&exe))
}

pub fn platform_app_base_candidates() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(dir) = dirs::data_local_dir() {
        roots.push(dir);
    }
    if let Some(dir) = dirs::data_dir() {
        if !roots.iter().any(|existing| existing == &dir) {
            roots.push(dir);
        }
    }

    let mut bases = Vec::new();
    for root in roots {
        for candidate in ordered_app_base_candidates_for_root(&root) {
            push_unique(&mut bases, candidate);
        }
    }

    if bases.is_empty() {
        if let Some(home) = dirs::home_dir() {
            push_unique(&mut bases, home.join(".rzn-browser"));
        }
    }

    bases
}

pub fn candidate_app_bases() -> Vec<PathBuf> {
    if let Some(base) = first_env_path(APP_BASE_ENV_KEYS) {
        return vec![base];
    }

    let mut bases = Vec::new();
    if let Some(base) = infer_current_app_base() {
        push_unique(&mut bases, base);
    }
    for base in platform_app_base_candidates() {
        push_unique(&mut bases, base);
    }

    if bases.is_empty() {
        bases.push(PathBuf::from(".rzn-browser"));
    }

    bases
}

pub fn default_app_base_dir() -> PathBuf {
    candidate_app_bases()
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from(".rzn-browser"))
}

fn push_unique(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !paths.iter().any(|existing| existing == &candidate) {
        paths.push(candidate);
    }
}

fn ordered_app_base_candidates_for_root(root: &Path) -> Vec<PathBuf> {
    let mut existing = Vec::new();
    let mut missing = Vec::new();

    for name in app_base_names_by_platform() {
        let candidate = root.join(name);
        if candidate.exists() {
            existing.push(candidate);
        } else {
            missing.push(candidate);
        }
    }

    existing.extend(missing);
    existing
}

fn app_base_names_by_platform() -> &'static [&'static str] {
    if cfg!(target_os = "linux") {
        &["rzn", "RZN", "rzn-browser", "rzn_debug"]
    } else {
        &["RZN", "rzn", "rzn-browser", "rzn_debug"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn infers_app_base_from_installed_bin_layout() {
        let exe = PathBuf::from("/tmp/RZN/bin/rzn-native-host");
        assert_eq!(
            infer_app_base_from_executable(&exe),
            Some(PathBuf::from("/tmp/RZN"))
        );
    }

    #[test]
    fn infers_app_base_from_plugin_bundle_layout() {
        let exe = PathBuf::from(
            "/tmp/RZN/plugins/com.rzn.browser/current/bin/darwin-arm64/rzn-native-host",
        );
        assert_eq!(
            infer_app_base_from_executable(&exe),
            Some(PathBuf::from("/tmp/RZN"))
        );
    }

    #[test]
    fn builds_supervisor_paths_under_run_and_secure_dirs() {
        let (socket, token) = supervisor_paths_for_app_base(Path::new("/tmp/RZN"));
        assert_eq!(socket, PathBuf::from("/tmp/RZN/run/rzn-supervisor.sock"));
        assert_eq!(
            token,
            PathBuf::from("/tmp/RZN/secure/rzn-supervisor-token-v1")
        );
    }

    #[test]
    fn existing_app_base_candidate_beats_case_preference() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock drift")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rzn-runtime-paths-{}", unique));
        fs::create_dir_all(root.join("RZN")).expect("create RZN root");

        let candidates = ordered_app_base_candidates_for_root(&root);

        assert_eq!(candidates.first(), Some(&root.join("RZN")));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn linux_default_preference_matches_installer_layout() {
        if cfg!(target_os = "linux") {
            assert_eq!(app_base_names_by_platform()[0], "rzn");
        }
    }
}
