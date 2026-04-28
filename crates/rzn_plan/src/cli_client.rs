use crate::{llm_provider::LLMProvider, PlanError, PlanResult};
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

#[derive(Debug, Clone)]
pub enum CliKind {
    Claude,
    Gemini,
    Codex,
}

impl CliKind {
    fn provider_name(&self) -> &'static str {
        match self {
            CliKind::Claude => "ClaudeCLI",
            CliKind::Gemini => "GeminiCLI",
            CliKind::Codex => "CodexCLI",
        }
    }

    fn binary_name(&self) -> &'static str {
        match self {
            CliKind::Claude => "claude",
            CliKind::Gemini => "gemini",
            CliKind::Codex => "codex",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CliClient {
    kind: CliKind,
    model: String,
    timeout_seconds: u64,
}

impl CliClient {
    pub fn new(kind: CliKind, model: String, timeout_seconds: u64) -> Self {
        Self {
            kind,
            model,
            timeout_seconds,
        }
    }

    fn flatten_messages(messages: &[Value]) -> String {
        // Keep this deterministic and readable for external CLIs.
        // The upstream prompts already include JSON-only constraints where needed.
        let mut out = String::new();
        for m in messages {
            let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            let content = m
                .get("content")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .trim();
            if content.is_empty() {
                continue;
            }
            out.push_str(&format!("{}\n", role.to_uppercase()));
            out.push_str(content);
            out.push_str("\n\n");
        }
        out
    }

    fn extract_first_json_object(s: &str) -> Option<Value> {
        // Gemini CLI (and others) may emit logs before the JSON payload. This attempts
        // to locate the first valid JSON object in the output.
        //
        // We do a simple brace-balance scan with string/escape handling.
        let bytes = s.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if bytes[i] != b'{' {
                i += 1;
                continue;
            }

            let start = i;
            let mut depth = 0i32;
            let mut in_string = false;
            let mut escaped = false;
            while i < bytes.len() {
                let b = bytes[i];
                if in_string {
                    if escaped {
                        escaped = false;
                    } else if b == b'\\' {
                        escaped = true;
                    } else if b == b'"' {
                        in_string = false;
                    }
                } else if b == b'"' {
                    in_string = true;
                } else if b == b'{' {
                    depth += 1;
                } else if b == b'}' {
                    depth -= 1;
                    if depth == 0 {
                        let end = i;
                        if let Ok(v) = serde_json::from_slice::<Value>(&bytes[start..=end]) {
                            return Some(v);
                        }
                        break;
                    }
                }
                i += 1;
            }

            // If we got here, this '{' didn't lead to a valid JSON object; continue scanning.
            i = start + 1;
        }
        None
    }

    fn parse_gemini_json_response(raw_stdout: &str) -> Option<String> {
        let v = Self::extract_first_json_object(raw_stdout)?;
        v.get("response")
            .and_then(|r| r.as_str())
            .map(|s| s.to_string())
    }

    async fn run_cli(&self, prompt: &str) -> PlanResult<String> {
        let mut cmd = Command::new(self.kind.binary_name());
        // Ensure external CLIs don't accidentally ingest the current repo as a "project context".
        cmd.current_dir(std::env::temp_dir());

        match self.kind {
            CliKind::Claude => {
                // Non-interactive. Disable tools for safety/predictability.
                cmd.arg("-p")
                    .arg("--output-format")
                    .arg("text")
                    .arg("--tools")
                    .arg("")
                    .arg("--model")
                    .arg(self.model.clone())
                    .arg(prompt);
            }
            CliKind::Gemini => {
                // Non-interactive, JSON envelope for reliable parsing.
                // Use yolo to avoid hanging on confirmations even if the CLI decides to invoke tools.
                cmd.arg("--output-format").arg("json");
                cmd.arg("--approval-mode").arg("yolo");
                if !self.model.trim().is_empty() && self.model.trim() != "auto" {
                    cmd.arg("--model").arg(self.model.clone());
                }
                cmd.arg(prompt);
            }
            CliKind::Codex => {
                // Codex is an agentic CLI; keep it in a safe non-mutating mode.
                // We ask for a single JSON response and rely on sandbox=read-only to prevent edits.
                cmd.arg("exec")
                    .arg("--sandbox")
                    .arg("read-only")
                    .arg("--skip-git-repo-check")
                    .arg("--model")
                    .arg(self.model.clone())
                    .arg(prompt);
            }
        }

        let timeout_dur = Duration::from_secs(self.timeout_seconds.max(5));
        let output = timeout(timeout_dur, cmd.output())
            .await
            .map_err(|_| PlanError::LLMError(format!("{} timed out", self.kind.provider_name())))?
            .map_err(|e| {
                PlanError::LLMError(format!("{} exec failed: {}", self.kind.provider_name(), e))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            return Err(PlanError::LLMError(format!(
                "{} returned non-zero exit. stderr='{}' stdout='{}'",
                self.kind.provider_name(),
                stderr,
                stdout
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        match self.kind {
            CliKind::Gemini => Ok(Self::parse_gemini_json_response(&stdout).unwrap_or(stdout)),
            _ => Ok(stdout),
        }
    }
}

#[async_trait]
impl LLMProvider for CliClient {
    fn provider_name(&self) -> &str {
        self.kind.provider_name()
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    async fn chat_completion(
        &self,
        messages: Vec<Value>,
        _temperature: f32,
        tools: Option<Vec<Value>>,
        tool_choice: Option<Value>,
        _max_tokens: Option<u32>,
    ) -> PlanResult<Value> {
        // CLI providers are intentionally "text-in/text-out". We don't support tool calling here.
        // (The autonomous planner uses JSON-in-plain-text prompts and doesn't need tool calls.)
        if tools.is_some() || tool_choice.is_some() {
            return Err(PlanError::LLMError(format!(
                "{} does not support tool calls; use API providers for tool calling",
                self.kind.provider_name()
            )));
        }

        let prompt = Self::flatten_messages(&messages);
        let content = self.run_cli(&prompt).await?;

        Ok(json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": content
                }
            }]
        }))
    }

    async fn simple_chat(
        &self,
        messages: Vec<Value>,
        _temperature: Option<f32>,
    ) -> PlanResult<String> {
        let prompt = Self::flatten_messages(&messages);
        self.run_cli(&prompt).await
    }
}

#[cfg(test)]
mod tests {
    use super::CliClient;

    #[test]
    fn gemini_parses_json_response_with_preamble_logs() {
        let raw = r#"Loaded cached credentials.
[STARTUP] Some log line
{
  "response": "{\"ok\":true,\"value\":3}",
  "stats": { "models": {} }
}"#;
        let extracted = CliClient::parse_gemini_json_response(raw).expect("expected response");
        assert_eq!(extracted, r#"{"ok":true,"value":3}"#);
    }
}
