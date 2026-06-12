use chrono::Utc;
use rzn_core::runtime_paths;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

const OPENAI_CHAT_COMPLETIONS_URL: &str = "https://api.openai.com/v1/chat/completions";
const DEFAULT_TOOL_MODEL: &str = "gpt-5-mini-2025-08-07";
const REQUEST_TIMEOUT_SECS: u64 = 60;
const CONNECT_TIMEOUT_SECS: u64 = 10;
const RAW_LOG_ENV: &str = "RZN_LLM_DEBUG_LOG";
const HTTP_ERROR_SNIPPET_CHARS: usize = 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: ToolParameters,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolParameters {
    pub required: Vec<String>,
    pub properties: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: Value,
}

pub struct ToolOnlyLLMClient {
    client: reqwest::Client,
    api_key: String,
    model: String,
    api_url: String,
    correlation_id: String,
    log_file: Option<PathBuf>,
}

impl ToolOnlyLLMClient {
    pub fn new(api_key: String, correlation_id: String) -> Self {
        let log_file = Self::default_raw_log_path(&correlation_id);
        Self::new_with_options(
            api_key,
            correlation_id,
            DEFAULT_TOOL_MODEL.to_string(),
            OPENAI_CHAT_COMPLETIONS_URL.to_string(),
            log_file,
        )
    }

    fn new_with_options(
        api_key: String,
        correlation_id: String,
        model: String,
        api_url: String,
        log_file: Option<PathBuf>,
    ) -> Self {
        Self {
            client: Self::build_http_client(),
            api_key,
            model,
            api_url,
            correlation_id,
            log_file,
        }
    }

    fn build_http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .build()
            .expect("tool LLM HTTP client configuration should be valid")
    }

    pub async fn call_with_tools(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        tools: Vec<Tool>,
        allowed_tools: Vec<&str>,
    ) -> Result<Vec<ToolCall>, String> {
        // Filter tools to only allowed ones
        let tools: Vec<Tool> = tools
            .into_iter()
            .filter(|t| allowed_tools.contains(&t.name.as_str()))
            .collect();

        let request_body =
            Self::build_request_body(&self.model, system_prompt, user_prompt, &tools);

        // Log raw request
        self.log_raw("request", &request_body);

        let response = self
            .client
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        let status = response.status();
        let response_text = response
            .text()
            .await
            .map_err(|e| format!("Failed to read response: {}", e))?;

        if !status.is_success() {
            return Err(Self::http_status_error(status, &response_text));
        }

        let response_json: Value = serde_json::from_str(&response_text)
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        // Log raw response
        self.log_raw("response", &response_json);

        // Extract tool calls
        let tool_calls = response_json
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("tool_calls"))
            .and_then(|tc| tc.as_array())
            .ok_or("No tool calls in response")?;

        let mut results = Vec::new();
        for tool_call in tool_calls {
            let name = tool_call
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .ok_or("Missing tool name")?
                .to_string();

            let arguments_str = tool_call
                .get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(|a| a.as_str())
                .ok_or("Missing tool arguments")?;

            let arguments: Value = serde_json::from_str(arguments_str)
                .map_err(|e| format!("Failed to parse tool arguments: {}", e))?;

            results.push(ToolCall { name, arguments });
        }

        Ok(results)
    }

    fn build_request_body(
        model: &str,
        system_prompt: &str,
        user_prompt: &str,
        tools: &[Tool],
    ) -> Value {
        let mut request_body = json!({
            "model": model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt}
            ],
            "tools": tools.iter().map(|t| json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": {
                        "type": "object",
                        "required": t.parameters.required,
                        "properties": t.parameters.properties
                    }
                }
            })).collect::<Vec<_>>(),
            "tool_choice": "required"
        });

        if !Self::model_omits_temperature(model) {
            request_body["temperature"] = json!(0.0);
        }

        if Self::model_uses_max_completion_tokens(model) {
            request_body["max_completion_tokens"] = json!(1000);
        } else {
            request_body["max_tokens"] = json!(1000);
        }

        request_body
    }

    fn model_omits_temperature(model: &str) -> bool {
        model.starts_with("gpt-5")
    }

    fn model_uses_max_completion_tokens(model: &str) -> bool {
        model.starts_with("gpt-5") || model.starts_with("o1-") || model.starts_with("o4-")
    }

    fn http_status_error(status: reqwest::StatusCode, body: &str) -> String {
        format!(
            "OpenAI API request failed with HTTP {}: {}",
            status,
            Self::body_snippet(body)
        )
    }

    fn body_snippet(body: &str) -> String {
        let mut snippet: String = body.chars().take(HTTP_ERROR_SNIPPET_CHARS).collect();
        if body.chars().count() > HTTP_ERROR_SNIPPET_CHARS {
            snippet.push_str("...");
        }
        snippet
    }

    fn log_raw(&self, event_type: &str, data: &Value) {
        let Some(log_file) = &self.log_file else {
            return;
        };

        let log_entry = json!({
            "timestamp": Utc::now().to_rfc3339(),
            "correlation_id": self.correlation_id,
            "event_type": event_type,
            "data": data
        });

        if let Err(err) = Self::append_raw_log(log_file, &log_entry) {
            log::debug!(
                "[{}] failed to write raw LLM {} log: {}",
                self.correlation_id,
                event_type,
                err
            );
            return;
        }

        log::debug!(
            "[{}] raw LLM {} logged to {}",
            self.correlation_id,
            event_type,
            log_file.display()
        );
    }

    fn default_raw_log_path(correlation_id: &str) -> Option<PathBuf> {
        if !Self::raw_logging_enabled() {
            return None;
        }

        Some(Self::raw_log_path_for_base(
            &runtime_paths::default_app_base_dir(),
            correlation_id,
        ))
    }

    fn raw_logging_enabled() -> bool {
        match std::env::var(RAW_LOG_ENV) {
            Ok(value) => matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            ),
            Err(_) => false,
        }
    }

    fn raw_log_path_for_base(base_dir: &Path, correlation_id: &str) -> PathBuf {
        let safe_correlation_id = Self::sanitize_filename_component(correlation_id);
        base_dir
            .join("secure")
            .join("llm-raw")
            .join(format!("llm_raw_{}.jsonl", safe_correlation_id))
    }

    fn sanitize_filename_component(input: &str) -> String {
        let mut sanitized: String = input
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
            .take(96)
            .collect();

        if sanitized.is_empty() {
            sanitized = "unknown".to_string();
        }

        sanitized
    }

    fn append_raw_log(path: &Path, log_entry: &Value) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
            Self::secure_log_dirs(parent)?;
        }

        let mut options = OpenOptions::new();
        options.create(true).append(true);

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        let mut file = options.open(path)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }

        writeln!(file, "{}", log_entry)
    }

    fn secure_log_dirs(parent: &Path) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
            if parent.file_name().and_then(|name| name.to_str()) == Some("llm-raw") {
                if let Some(secure_dir) = parent.parent() {
                    if secure_dir.file_name().and_then(|name| name.to_str()) == Some("secure") {
                        fs::set_permissions(secure_dir, fs::Permissions::from_mode(0o700))?;
                    }
                }
            }
        }

        #[cfg(not(unix))]
        {
            let _ = parent;
        }

        Ok(())
    }

    pub fn get_standard_tools() -> Vec<Tool> {
        vec![
            Tool {
                name: "navigate".to_string(),
                description: "Navigate to a URL".to_string(),
                parameters: ToolParameters {
                    required: vec!["url".to_string()],
                    properties: json!({
                        "url": {"type": "string", "description": "The URL to navigate to"}
                    }),
                },
            },
            Tool {
                name: "type".to_string(),
                description: "Type text into an input field".to_string(),
                parameters: ToolParameters {
                    required: vec!["selector".to_string(), "text".to_string()],
                    properties: json!({
                        "selector": {"type": "string", "description": "CSS selector for the input"},
                        "text": {"type": "string", "description": "Text to type"}
                    }),
                },
            },
            Tool {
                name: "press_key".to_string(),
                description: "Press a keyboard key".to_string(),
                parameters: ToolParameters {
                    required: vec!["key".to_string()],
                    properties: json!({
                        "key": {
                            "type": "string",
                            "enum": ["Enter", "Tab", "Escape", "ArrowUp", "ArrowDown"],
                            "description": "The key to press"
                        }
                    }),
                },
            },
            Tool {
                name: "click".to_string(),
                description: "Click on an element".to_string(),
                parameters: ToolParameters {
                    required: vec!["selector".to_string()],
                    properties: json!({
                        "selector": {"type": "string", "description": "CSS selector for the element"}
                    }),
                },
            },
            Tool {
                name: "extract".to_string(),
                description: "Extract data from the page".to_string(),
                parameters: ToolParameters {
                    required: vec!["fields".to_string()],
                    properties: json!({
                        "fields": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "name": {"type": "string"},
                                    "selector": {"type": "string"}
                                }
                            }
                        }
                    }),
                },
            },
            Tool {
                name: "scroll".to_string(),
                description: "Scroll the page".to_string(),
                parameters: ToolParameters {
                    required: vec!["direction".to_string(), "amount".to_string()],
                    properties: json!({
                        "direction": {"type": "string", "enum": ["up", "down"]},
                        "amount": {"type": "integer", "description": "Pixels to scroll"}
                    }),
                },
            },
            Tool {
                name: "wait".to_string(),
                description: "Wait for a specified time".to_string(),
                parameters: ToolParameters {
                    required: vec!["milliseconds".to_string()],
                    properties: json!({
                        "milliseconds": {"type": "integer", "description": "Time to wait in ms"}
                    }),
                },
            },
            Tool {
                name: "complete".to_string(),
                description: "Mark the task as complete".to_string(),
                parameters: ToolParameters {
                    required: vec![],
                    properties: json!({}),
                },
            },
            Tool {
                name: "batch_actions".to_string(),
                description: "Execute multiple atomic steps in one trusted macro".to_string(),
                parameters: ToolParameters {
                    required: vec!["steps".to_string()],
                    properties: json!({
                        "steps": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 12,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "op": {
                                        "type": "string",
                                        "enum": ["click","insert_text","press_key","wait_selector","scroll_by"],
                                        "description": "The operation to perform"
                                    },
                                    "selector": {
                                        "type": "string",
                                        "description": "CSS or deep selector (>>>). Optional if encodedId provided."
                                    },
                                    "encodedId": {
                                        "type": "string",
                                        "description": "frameId:backendNodeId. Preferred when available."
                                    },
                                    "text": {
                                        "type": "string",
                                        "description": "Text to insert for insert_text operation"
                                    },
                                    "key": {
                                        "type": "string",
                                        "description": "Key to press for press_key operation"
                                    },
                                    "waitSelector": {
                                        "type": "string",
                                        "description": "Selector to wait for in wait_selector operation"
                                    },
                                    "dx": {
                                        "type": "number",
                                        "description": "Horizontal scroll amount for scroll_by"
                                    },
                                    "dy": {
                                        "type": "number",
                                        "description": "Vertical scroll amount for scroll_by"
                                    }
                                },
                                "required": ["op"]
                            }
                        }
                    }),
                },
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use once_cell::sync::Lazy;
    use std::ffi::OsString;
    use std::io::{Read, Write as IoWrite};
    use std::net::TcpListener;
    use std::sync::Mutex;
    use std::thread::JoinHandle;
    use std::time::Duration as StdDuration;

    static ENV_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    struct EnvVarGuard {
        key: &'static str,
        original: Option<OsString>,
    }

    impl EnvVarGuard {
        fn new(key: &'static str) -> Self {
            Self {
                key,
                original: std::env::var_os(key),
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.original {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn simple_tool(name: &str) -> Tool {
        Tool {
            name: name.to_string(),
            description: format!("{name} tool"),
            parameters: ToolParameters {
                required: vec![],
                properties: json!({}),
            },
        }
    }

    fn spawn_http_error_server(
        status_line: &'static str,
        body: &'static str,
    ) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test HTTP server");
        let addr = listener.local_addr().expect("read test HTTP server addr");
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept test request");
            stream
                .set_read_timeout(Some(StdDuration::from_secs(2)))
                .expect("set read timeout");
            let mut request_buf = [0_u8; 4096];
            let _ = stream.read(&mut request_buf);
            let response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("write test response");
        });

        (format!("http://{addr}/v1/chat/completions"), handle)
    }

    #[test]
    fn test_standard_tools() {
        let tools = ToolOnlyLLMClient::get_standard_tools();
        assert!(tools.iter().any(|t| t.name == "navigate"));
        assert!(tools.iter().any(|t| t.name == "press_key"));
        assert!(tools.iter().any(|t| t.name == "type"));
    }

    #[test]
    fn test_tool_filtering() {
        let tools = ToolOnlyLLMClient::get_standard_tools();
        let allowed = ["type", "press_key"];

        let filtered: Vec<Tool> = tools
            .into_iter()
            .filter(|t| allowed.contains(&t.name.as_str()))
            .collect();

        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|t| allowed.contains(&t.name.as_str())));
    }

    #[test]
    fn gpt_5_request_omits_temperature() {
        let body =
            ToolOnlyLLMClient::build_request_body("gpt-5-mini-2025-08-07", "sys", "user", &[]);

        assert!(body.get("temperature").is_none());
        assert_eq!(body["max_completion_tokens"], json!(1000));
        assert!(body.get("max_tokens").is_none());
    }

    #[test]
    fn non_gpt_5_request_keeps_explicit_temperature() {
        let body = ToolOnlyLLMClient::build_request_body("gpt-4o-mini", "sys", "user", &[]);

        assert_eq!(body["temperature"], json!(0.0));
        assert_eq!(body["max_tokens"], json!(1000));
        assert!(body.get("max_completion_tokens").is_none());
    }

    #[tokio::test]
    async fn non_success_http_response_returns_status_and_body() {
        let body = r#"{"error":{"message":"rate limited for test account"}}"#;
        let (api_url, handle) = spawn_http_error_server("429 Too Many Requests", body);
        let client = ToolOnlyLLMClient::new_with_options(
            "sk-test".to_string(),
            "corr-http".to_string(),
            "gpt-5-mini-2025-08-07".to_string(),
            api_url,
            None,
        );

        let err = client
            .call_with_tools(
                "sys",
                "user",
                vec![simple_tool("complete")],
                vec!["complete"],
            )
            .await
            .expect_err("expected HTTP status error");

        handle.join().expect("test HTTP server joined");
        assert!(err.contains("HTTP 429 Too Many Requests"), "{err}");
        assert!(err.contains("rate limited for test account"), "{err}");
        assert!(!err.contains("No tool calls"), "{err}");
    }

    #[test]
    fn raw_logging_requires_env_flag_and_uses_app_base() {
        let _env_lock = ENV_LOCK.lock().expect("env lock");
        let _debug_guard = EnvVarGuard::new(RAW_LOG_ENV);
        let _base_guard = EnvVarGuard::new("RZN_APP_BASE_DIR");
        let app_base = tempfile::tempdir().expect("create app base");

        std::env::remove_var(RAW_LOG_ENV);
        std::env::set_var("RZN_APP_BASE_DIR", app_base.path());
        let disabled = ToolOnlyLLMClient::new("sk-test".to_string(), "corr/../id".to_string());
        assert!(disabled.log_file.is_none());

        std::env::set_var(RAW_LOG_ENV, "1");
        let enabled = ToolOnlyLLMClient::new("sk-test".to_string(), "corr/../id".to_string());
        assert_eq!(
            enabled.log_file.as_deref(),
            Some(
                app_base
                    .path()
                    .join("secure")
                    .join("llm-raw")
                    .join("llm_raw_corrid.jsonl")
                    .as_path()
            )
        );
    }

    #[test]
    fn raw_log_path_sanitizes_correlation_id() {
        let path = ToolOnlyLLMClient::raw_log_path_for_base(
            Path::new("/tmp/rzn-app"),
            "../abc DEF_123-xy!@#",
        );

        assert_eq!(
            path,
            Path::new("/tmp/rzn-app")
                .join("secure")
                .join("llm-raw")
                .join("llm_raw_abcDEF_123-xy.jsonl")
        );
    }

    #[test]
    fn raw_log_file_is_created_private_on_unix() {
        let app_base = tempfile::tempdir().expect("create app base");
        let log_path = ToolOnlyLLMClient::raw_log_path_for_base(app_base.path(), "corr");

        ToolOnlyLLMClient::append_raw_log(&log_path, &json!({"ok": true})).expect("append raw log");

        let written = fs::read_to_string(&log_path).expect("read raw log");
        assert!(written.contains(r#""ok":true"#), "{written}");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                fs::metadata(&log_path)
                    .expect("log metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(log_path.parent().expect("log parent"))
                    .expect("log parent metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
    }
}
