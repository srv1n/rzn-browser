use crate::llm_provider::LLMProvider;
use crate::PlanResult;
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct DummyClient {
    model: String,
}

impl DummyClient {
    pub fn new(model: String) -> Self {
        Self { model }
    }

    fn build_reply(&self, messages: &[Value]) -> String {
        // Inspect conversation to decide next action in a deterministic way
        // Strategy:
        // 1) If this is the first turn (contains a Task: ...), navigate to Google
        // 2) If no prior assistant action with cmd=="type", type query into Google box
        // 3) If typed but no press yet, press Enter
        // 4) Else extract links

        let mut saw_type = false;
        let mut saw_press = false;
        let mut instruction: Option<String> = None;

        for msg in messages {
            if let Some(role) = msg.get("role").and_then(|r| r.as_str()) {
                if role == "user" {
                    if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                        if content.contains("Task:") && instruction.is_none() {
                            // Extract after 'Task: '
                            instruction = content
                                .split_once("Task:")
                                .map(|x| x.1)
                                .map(|s| s.trim().to_string());
                        }
                    }
                } else if role == "assistant" {
                    if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                        // assistant content is a JSON string of LLMAction
                        if let Ok(v) = serde_json::from_str::<Value>(content) {
                            let cmd = v
                                .get("action")
                                .and_then(|a| a.get("cmd"))
                                .and_then(|c| c.as_str())
                                .unwrap_or("");
                            match cmd {
                                "type" => saw_type = true,
                                "press" | "press_key" => saw_press = true,
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        // Extract a crude query from the instruction
        let mut query = "test".to_string();
        if let Some(instr) = instruction {
            if let Some(pos) = instr.to_lowercase().find("search google for") {
                let after = &instr[pos + "search google for".len()..].trim();
                // until end or 'and'
                let cut = after.split(['\n', '\r']).next().unwrap_or(after);
                let cut = cut.split(" and ").next().unwrap_or(cut).trim();
                if !cut.is_empty() {
                    query = cut.trim_matches('"').to_string();
                }
            }
        }

        // Decide next action
        if !saw_type {
            // If we never acted, start by navigating to Google
            // We detect first-turn by lack of any assistant action
            let saw_any_assistant = messages
                .iter()
                .any(|m| m.get("role").and_then(|r| r.as_str()) == Some("assistant"));
            if !saw_any_assistant {
                return json!({
                    "thought": "Open Google to begin the search",
                    "action": {"cmd": "navigate", "args": ["https://www.google.com/"]}
                })
                .to_string();
            }
            // Otherwise, type into Google's search box
            return json!({
                "thought": "Type the query into the Google search box",
                "action": {"cmd": "type", "args": ["textarea[name='q']", query]}
            })
            .to_string();
        }

        if !saw_press {
            return json!({
                "thought": "Submit the search by pressing Enter",
                "action": {"cmd": "press_key", "args": ["Enter"]}
            })
            .to_string();
        }

        // Finally, extract links
        json!({
            "thought": "Extract search result links",
            "action": {"cmd": "extract", "args": [{
                "fields": [
                    {"name": "title", "selector": "a h3"},
                    {"name": "href", "selector": "a", "attribute": "href"}
                ]
            }]}
        })
        .to_string()
    }
}

#[async_trait]
impl LLMProvider for DummyClient {
    fn provider_name(&self) -> &str {
        "dummy"
    }
    fn model_name(&self) -> &str {
        &self.model
    }

    async fn chat_completion(
        &self,
        messages: Vec<Value>,
        _temperature: f32,
        _tools: Option<Vec<Value>>,
        _tool_choice: Option<Value>,
        _max_tokens: Option<u32>,
    ) -> PlanResult<Value> {
        let reply = self.build_reply(&messages);
        Ok(json!({
            "choices": [
                {"message": {"content": reply}, "finish_reason": "stop"}
            ]
        }))
    }

    async fn simple_chat(
        &self,
        messages: Vec<Value>,
        _temperature: Option<f32>,
    ) -> PlanResult<String> {
        Ok(self.build_reply(&messages))
    }
}
