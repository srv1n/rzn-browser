//! Sign in to RZN (backend `docs/plans/browser-fleet/design.md` section 3, BFL-T-0012).
//!
//! The extension's Account page opens the server's own sign-in page (`/oauth/authorize`) as
//! the public client `rzn-browser` with PKCE and hands the code it gets back to the helper,
//! `account.sign_in {server_url, code, code_verifier, redirect_uri}`. The helper:
//!
//! 1. trades the code at `/oauth/token` for the user's access token and the refresh token of
//!    the session the sign-in made (the account's session list shows it);
//! 2. mints a one-time enrollment code with that access token (`POST /v1/fleet/enrollment-codes`)
//!    and redeems it at `POST /v1/fleet/enroll`, so the device token is the user's, with no code
//!    typed. The device works for every tenant the user belongs to, read by the server at each
//!    check-in;
//! 3. keeps the refresh token in `account.json` (0600, beside `fleet_config.json`) so capture
//!    of opened pages pushes to the user's own index with the user's access token. A refresh
//!    rotates the token and the new one replaces the old before it is used.
//!
//! Signing out revokes the device and the session on the server, then forgets both locally.
//! Neither token is ever printed or logged.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{anyhow, bail, Context, Result};
use rzn_contracts::fleet::FleetEnrollRequest;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use super::{
    cli_version, describe_http_error, detect_extension_version, device_capabilities,
    device_platform, fleet_config_path, fleet_http_client, normalize_fleet_server_url,
    perform_enroll, read_fleet_config, write_private, FleetDeviceConfig,
};

/// The OAuth client the extension signs in as (seeded by the backend's migration 0048).
pub(crate) const CLIENT_ID: &str = "rzn-browser";
const SESSION_FILENAME: &str = "account.json";
const SESSION_CONTRACT: &str = "rzn.account.session";
/// An access token is renewed this long before the server says it expires.
const RENEW_EARLY_SECONDS: i64 = 60;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Session {
    version: String,
    server_url: String,
    refresh_token: String,
}

#[derive(Deserialize)]
struct Granted {
    access_token: String,
    #[serde(default)]
    expires_in: i64,
    refresh_token: String,
}

struct Access {
    server_url: String,
    token: String,
    until: Instant,
}

/// Access tokens in memory, per session file. Held across a refresh so two callers never
/// spend the same (rotating) refresh token.
static ACCESS: Mutex<Option<HashMap<PathBuf, Access>>> = Mutex::const_new(None);

fn session_path(config_path: &Path) -> PathBuf {
    config_path.with_file_name(SESSION_FILENAME)
}

fn read_session(path: &Path) -> Result<Option<Session>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(
            serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?,
        )),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("read {}", path.display())),
    }
}

fn write_session(path: &Path, server_url: &str, refresh_token: &str) -> Result<()> {
    let session = Session {
        version: SESSION_CONTRACT.to_string(),
        server_url: server_url.to_string(),
        refresh_token: refresh_token.to_string(),
    };
    write_private(path, &serde_json::to_vec_pretty(&session)?)
}

async fn cache(path: &Path, server_url: &str, granted: &Granted) {
    let seconds = (granted.expires_in - RENEW_EARLY_SECONDS).max(0) as u64;
    ACCESS.lock().await.get_or_insert_with(HashMap::new).insert(
        path.to_path_buf(),
        Access {
            server_url: server_url.to_string(),
            token: granted.access_token.clone(),
            until: Instant::now() + std::time::Duration::from_secs(seconds),
        },
    );
}

/// `POST /oauth/token` with `form`; the server's error text on refusal.
async fn token_request(
    client: &reqwest::Client,
    server_url: &str,
    form: &[(&str, &str)],
) -> Result<Granted> {
    let url = format!("{server_url}/oauth/token");
    let response = client
        .post(&url)
        .form(form)
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("sign-in refused: {}", describe_http_error(status, &body));
    }
    serde_json::from_str(&body).context("decode token response")
}

/// The core of sign-in, against `server_url`, writing beside `config_path`.
async fn sign_in(
    client: &reqwest::Client,
    server_url: &str,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
    config_path: &Path,
) -> Result<FleetDeviceConfig> {
    let granted = token_request(
        client,
        server_url,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", CLIENT_ID),
            ("code_verifier", code_verifier),
        ],
    )
    .await?;
    let path = session_path(config_path);
    write_session(&path, server_url, &granted.refresh_token)?;
    cache(&path, server_url, &granted).await;

    let url = format!("{server_url}/v1/fleet/enrollment-codes");
    let response = client
        .post(&url)
        .bearer_auth(&granted.access_token)
        .json(&json!({}))
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!(
            "could not register this browser: {}",
            describe_http_error(status, &body)
        );
    }
    let minted: Value = serde_json::from_str(&body).context("decode enrollment code")?;
    let one_time = minted
        .get("code")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("the server minted no enrollment code"))?;
    let request = FleetEnrollRequest {
        code: one_time.to_string(),
        device_name: super::default_device_name(),
        platform: device_platform(),
        capabilities: device_capabilities(),
        cli_version: cli_version(),
        extension_version: detect_extension_version().await,
    };
    // Signing in again replaces this browser's device identity.
    perform_enroll(client, server_url, &request, config_path, true).await
}

/// `account.sign_in` RPC.
pub(crate) async fn sign_in_from_rpc(params: &Value) -> Result<Value> {
    let field = |key: &str| {
        params
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow!("missing {key}"))
    };
    let server_url = normalize_fleet_server_url(field("server_url")?)?;
    let config = sign_in(
        &fleet_http_client()?,
        &server_url,
        field("code")?,
        field("code_verifier")?,
        field("redirect_uri")?,
        &fleet_config_path(),
    )
    .await?;
    let loop_started = crate::supervisor_fleet::restart_fleet_loop().await;
    Ok(json!({
        "ok": true, "device_id": config.device_id, "user_id": config.user_id,
        "user_email": config.user_email, "server_url": config.server_url,
        "loop_started": loop_started,
    }))
}

async fn access_token_at(client: &reqwest::Client, config_path: &Path) -> Result<(String, String)> {
    let path = session_path(config_path);
    let mut guard = ACCESS.lock().await;
    let held = guard.get_or_insert_with(HashMap::new);
    if let Some(access) = held.get(&path).filter(|a| a.until > Instant::now()) {
        return Ok((access.server_url.clone(), access.token.clone()));
    }
    let session = read_session(&path)?
        .ok_or_else(|| anyhow!("not signed in to RZN: sign in on the Account page"))?;
    let granted = token_request(
        client,
        &session.server_url,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &session.refresh_token),
            ("client_id", CLIENT_ID),
        ],
    )
    .await?;
    write_session(&path, &session.server_url, &granted.refresh_token)?;
    let seconds = (granted.expires_in - RENEW_EARLY_SECONDS).max(0) as u64;
    held.insert(
        path,
        Access {
            server_url: session.server_url.clone(),
            token: granted.access_token.clone(),
            until: Instant::now() + std::time::Duration::from_secs(seconds),
        },
    );
    Ok((session.server_url, granted.access_token))
}

/// `(server_url, access token)` of the signed-in user, renewed when it is about to expire.
pub(crate) async fn access_token() -> Result<(String, String)> {
    access_token_at(&fleet_http_client()?, &fleet_config_path()).await
}

async fn sign_out_at(client: &reqwest::Client, config_path: &Path) -> Result<Value> {
    let path = session_path(config_path);
    let config = read_fleet_config(config_path)?;
    let mut device_revoked = false;
    let mut session_revoked = false;
    if let Some(session) = read_session(&path)? {
        // Best effort: an offline sign-out still forgets both tokens here.
        if let (Some(config), Ok((server_url, token))) =
            (config.as_ref(), access_token_at(client, config_path).await)
        {
            let url = format!("{server_url}/v1/fleet/devices/{}/revoke", config.device_id);
            device_revoked = client
                .post(&url)
                .bearer_auth(token)
                .json(&json!({}))
                .send()
                .await
                .is_ok_and(|r| r.status().is_success());
        }
        let url = format!("{}/oauth/revoke", session.server_url);
        session_revoked = client
            .post(&url)
            .form(&[("token", session.refresh_token.as_str())])
            .send()
            .await
            .is_ok_and(|r| r.status().is_success());
    }
    if let Some(held) = ACCESS.lock().await.as_mut() {
        held.remove(&path);
    }
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).with_context(|| format!("remove {}", path.display())),
    }
    Ok(json!({"ok": true, "device_revoked": device_revoked, "session_revoked": session_revoked}))
}

/// `account.sign_out` RPC: revoke the device and the session, forget them, stop the loop.
pub(crate) async fn sign_out_from_rpc() -> Result<Value> {
    let mut answer = sign_out_at(&fleet_http_client()?, &fleet_config_path()).await?;
    let unenrolled = super::unenroll_from_rpc()?;
    answer["removed"] = unenrolled["removed"].clone();
    Ok(answer)
}

/// `account.status` RPC: who is signed in and this device, for the Account page. No token.
pub(crate) fn status_rpc() -> Result<Value> {
    let config_path = fleet_config_path();
    let config = read_fleet_config(&config_path)?;
    let session = read_session(&session_path(&config_path))?;
    Ok(json!({
        "signed_in": config.is_some() && session.is_some(),
        "enrolled": config.is_some(),
        "server_url": config.as_ref().map(|c| c.server_url.clone()),
        "user_id": config.as_ref().map(|c| c.user_id.clone()),
        "user_email": config.as_ref().and_then(|c| c.user_email.clone()),
        "device_id": config.as_ref().map(|c| c.device_id.clone()),
        "device_name": super::default_device_name(),
        "fleet": crate::supervisor_fleet::fleet_status_rpc(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Form;
    use axum::http::HeaderMap;
    use axum::routing::post;
    use axum::{Json, Router};
    use std::collections::HashMap as Map;
    use std::sync::{Arc, Mutex as StdMutex};

    async fn spawn(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    fn temp_config() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rzn-signin-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("fleet_config.json")
    }

    /// The server's side of sign-in: the token route (a code once, then rotating refresh
    /// tokens), the code mint (only with the access token) and enroll.
    fn server(seen: Arc<StdMutex<Vec<String>>>) -> Router {
        let token_seen = seen.clone();
        let mint_seen = seen.clone();
        Router::new()
            .route(
                "/oauth/token",
                post(move |Form(form): Form<Map<String, String>>| {
                    let seen = token_seen.clone();
                    async move {
                        let grant = form["grant_type"].clone();
                        seen.lock().unwrap().push(format!(
                            "token:{grant}:{}",
                            form.get("refresh_token").or(form.get("code")).cloned().unwrap_or_default()
                        ));
                        assert_eq!(form["client_id"], CLIENT_ID);
                        let n = seen.lock().unwrap().len();
                        Json(json!({"access_token": format!("acc{n}"), "token_type": "Bearer",
                                    "expires_in": 0, "refresh_token": format!("ref{n}")}))
                    }
                }),
            )
            .route(
                "/v1/fleet/enrollment-codes",
                post(move |headers: HeaderMap| {
                    let seen = mint_seen.clone();
                    async move {
                        let auth = headers["authorization"].to_str().unwrap().to_string();
                        seen.lock().unwrap().push(format!("mint:{auth}"));
                        Json(json!({"code": "ONETIME000", "expires_at_ms": 1}))
                    }
                }),
            )
            .route(
                "/v1/fleet/enroll",
                post(|Json(body): Json<Value>| async move {
                    assert_eq!(body["code"], "ONETIME000");
                    Json(json!({"device_id": "dev_1", "device_token": "fld_x", "user_id": "usr_1",
                                "user_email": "me@example.com", "poll_interval_seconds": 45}))
                }),
            )
    }

    #[tokio::test]
    async fn sign_in_enrolls_with_a_minted_code_and_refresh_rotates() {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let base = spawn(server(seen.clone())).await;
        let config_path = temp_config();
        let client = reqwest::Client::new();

        let config = sign_in(&client, &base, "authcode", "verifier", "https://x/", &config_path)
            .await
            .expect("sign in");
        assert_eq!(config.user_id, "usr_1");
        assert_eq!(config.user_email.as_deref(), Some("me@example.com"));
        assert_eq!(config.device_token, "fld_x");
        let session = read_session(&session_path(&config_path)).unwrap().unwrap();
        assert_eq!(session.refresh_token, "ref1");
        assert_eq!(
            seen.lock().unwrap().clone(),
            vec!["token:authorization_code:authcode".to_string(), "mint:Bearer acc1".to_string()]
        );

        // The first access token expired at once (expires_in 0): the next use renews it, and
        // the rotated refresh token replaces the spent one.
        let (server_url, token) = access_token_at(&client, &config_path).await.unwrap();
        assert_eq!(server_url, base);
        assert_eq!(token, "acc3");
        assert_eq!(seen.lock().unwrap()[2], "token:refresh_token:ref1");
        let session = read_session(&session_path(&config_path)).unwrap().unwrap();
        assert_eq!(session.refresh_token, "ref3");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(session_path(&config_path)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[tokio::test]
    async fn signed_out_there_is_no_access_token() {
        let config_path = temp_config();
        let error = access_token_at(&reqwest::Client::new(), &config_path)
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("not signed in"));
        let answer = sign_out_at(&reqwest::Client::new(), &config_path).await.unwrap();
        assert_eq!(answer["session_revoked"], false);
    }
}
