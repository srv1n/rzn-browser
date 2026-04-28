use crate::broker_client::BrokerClient;
use crate::desktop_tools::{map_plan_error, DesktopToolResult};
use crate::policy_gate::PolicyGate;
use rzn_core::Step;
use serde_json::Value;

/// A desktop-facing runner that enforces policy gates before executing actions.
///
/// Intended usage: the Tauri backend owns this session and provides a PolicyConfirmer
/// implementation for any RequireConfirmation decisions.
pub struct DesktopSession {
    broker: BrokerClient,
    policy: PolicyGate,
}

impl DesktopSession {
    pub fn new(broker: BrokerClient, policy: PolicyGate) -> Self {
        Self { broker, policy }
    }

    pub fn broker_mut(&mut self) -> &mut BrokerClient {
        &mut self.broker
    }

    pub fn policy_mut(&mut self) -> &mut PolicyGate {
        &mut self.policy
    }

    pub async fn observe(
        &mut self,
        instruction: &str,
        scope_selector: Option<&str>,
        max_items: Option<u32>,
    ) -> DesktopToolResult<Value> {
        self.broker
            .observe_desktop(instruction, scope_selector, max_items)
            .await
    }

    pub async fn extract_extraction_plan(&mut self, plan: Value) -> DesktopToolResult<Value> {
        self.broker.execute_extraction_plan_desktop(plan).await
    }

    pub async fn act_step(&mut self, step: &Step) -> DesktopToolResult<Value> {
        let current_url = self.broker.get_current_url();
        self.policy
            .enforce_step(step, None, current_url.as_deref())
            .await
            .map_err(map_plan_error)?;

        self.broker.act_step_desktop(step).await
    }

    pub async fn execute_steps(&mut self, steps: &[Step]) -> DesktopToolResult<Value> {
        let mut out: Vec<Value> = Vec::with_capacity(steps.len());
        for step in steps {
            out.push(self.act_step(step).await?);
        }
        Ok(serde_json::json!({ "success": true, "steps": out }))
    }
}
