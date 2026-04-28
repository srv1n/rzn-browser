use crate::element_ref::TargetSpec;
use crate::PlanError;
use async_trait::async_trait;
use log::{info, warn};
use rzn_core::{Step, StepKind};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDecisionKind {
    Allow,
    RequireConfirmation,
    Block,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub decision: PolicyDecisionKind,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRequest {
    pub reason: String,
    pub current_url: Option<String>,
    pub step: Step,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<TargetSpec>,
}

#[async_trait]
pub trait PolicyConfirmer: Send + Sync {
    async fn confirm(&self, request: PolicyRequest) -> bool;
}

#[derive(Clone)]
pub struct PolicyGate {
    /// When set, skips confirmations for RequireConfirmation decisions.
    ///
    /// This is intended for local dev/testing only; production desktop apps should
    /// use `confirmer` to implement explicit user confirmation UX.
    pub auto_approve: bool,
    pub confirmer: Option<Arc<dyn PolicyConfirmer>>,
}

impl PolicyGate {
    pub fn from_env() -> Self {
        let auto_approve = std::env::var("RZN_POLICY_AUTO_APPROVE")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        Self {
            auto_approve,
            confirmer: None,
        }
    }

    pub fn with_confirmer(mut self, confirmer: Arc<dyn PolicyConfirmer>) -> Self {
        self.confirmer = Some(confirmer);
        self
    }

    pub fn classify_step(
        &self,
        step: &Step,
        target: Option<&TargetSpec>,
        current_url: Option<&str>,
    ) -> PolicyDecision {
        let kind = &step.kind;

        // Default allow; special-case only when we have strong generic risk signals.
        let mut decision = PolicyDecision {
            decision: PolicyDecisionKind::Allow,
            reason: "allowed".to_string(),
        };

        // Hard blocks (no safe unattended execution).
        if matches!(
            kind,
            StepKind::ExecuteJavascript { .. }
                | StepKind::HandleCaptcha { .. }
                | StepKind::ConfigureCaptchaSolver { .. }
        ) {
            return PolicyDecision {
                decision: PolicyDecisionKind::Block,
                reason: "high-risk step type blocked by policy".to_string(),
            };
        }

        // Require explicit confirmation for actions that can exfiltrate/modify local or session state.
        if matches!(
            kind,
            StepKind::UploadFile { .. }
                | StepKind::DownloadImages { .. }
                | StepKind::SameOriginRequest { .. }
                | StepKind::SetCookie { .. }
                | StepKind::GetCookies { .. }
                | StepKind::ClearCookies { .. }
                | StepKind::SetLocalStorageItem { .. }
                | StepKind::GetLocalStorageItem { .. }
                | StepKind::ClearLocalStorage { .. }
        ) {
            return PolicyDecision {
                decision: PolicyDecisionKind::RequireConfirmation,
                reason: "step modifies sensitive state or transfers data".to_string(),
            };
        }

        // Auth-ish typing: require confirmation if selector/text hints at passwords/OTP.
        if let StepKind::FillInputField {
            selector, value, ..
        } = kind
        {
            let sel = selector.to_lowercase();
            if sel.contains("password") || sel.contains("otp") || sel.contains("2fa") {
                let has_value = !value.trim().is_empty();
                if has_value {
                    return PolicyDecision {
                        decision: PolicyDecisionKind::RequireConfirmation,
                        reason: "typing into auth-related field".to_string(),
                    };
                }
            }
        }

        // Destructive-ish clicks: only generic keyword heuristics (no per-site logic).
        if let StepKind::ClickElement { selector, .. } = kind {
            let sel = selector.to_lowercase();
            let destructive = ["delete", "remove", "unsubscribe", "cancel", "terminate"];
            if destructive.iter().any(|k| sel.contains(k)) {
                return PolicyDecision {
                    decision: PolicyDecisionKind::RequireConfirmation,
                    reason: "potentially destructive click".to_string(),
                };
            }
        }

        // Navigation to checkout/payment-ish URLs: generic keyword heuristic.
        if let StepKind::NavigateToUrl { url, .. } = kind {
            let u = url.to_lowercase();
            let sensitive = ["checkout", "payment", "pay", "bank", "billing"];
            if sensitive.iter().any(|k| u.contains(k)) {
                return PolicyDecision {
                    decision: PolicyDecisionKind::RequireConfirmation,
                    reason: "navigating to a potentially sensitive flow".to_string(),
                };
            }
        }

        // TargetSpec text hints can raise risk (generic).
        if let Some(t) = target {
            let near = t.text_near.as_deref().unwrap_or("").to_lowercase();
            if near.contains("delete") || near.contains("remove") {
                decision = PolicyDecision {
                    decision: PolicyDecisionKind::RequireConfirmation,
                    reason: "target hint suggests destructive action".to_string(),
                };
            }
        }

        // Include current URL only for better logging (not for decisions).
        let _ = current_url;
        decision
    }

    pub async fn enforce_step(
        &self,
        step: &Step,
        target: Option<&TargetSpec>,
        current_url: Option<&str>,
    ) -> Result<(), PlanError> {
        let decision = self.classify_step(step, target, current_url);

        match decision.decision {
            PolicyDecisionKind::Allow => {
                info!(
                    "[policy] allow step={} kind={:?} reason={}",
                    step.id, step.kind, decision.reason
                );
                Ok(())
            }
            PolicyDecisionKind::Block => {
                warn!(
                    "[policy] block step={} kind={:?} reason={}",
                    step.id, step.kind, decision.reason
                );
                Err(PlanError::PolicyBlocked(decision.reason))
            }
            PolicyDecisionKind::RequireConfirmation => {
                let url = current_url.map(|s| s.to_string());
                let target_owned = target.cloned();
                let request = PolicyRequest {
                    reason: decision.reason.clone(),
                    current_url: url,
                    step: step.clone(),
                    target: target_owned,
                };

                if self.auto_approve {
                    warn!(
                        "[policy] auto-approve step={} kind={:?} reason={}",
                        step.id, step.kind, decision.reason
                    );
                    return Ok(());
                }

                if let Some(confirmer) = &self.confirmer {
                    let ok = confirmer.confirm(request).await;
                    if ok {
                        info!(
                            "[policy] confirmed step={} kind={:?} reason={}",
                            step.id, step.kind, decision.reason
                        );
                        Ok(())
                    } else {
                        warn!(
                            "[policy] denied step={} kind={:?} reason={}",
                            step.id, step.kind, decision.reason
                        );
                        Err(PlanError::PolicyBlocked(decision.reason))
                    }
                } else {
                    warn!(
                        "[policy] block (no confirmer) step={} kind={:?} reason={}",
                        step.id, step.kind, decision.reason
                    );
                    Err(PlanError::PolicyBlocked(decision.reason))
                }
            }
        }
    }
}
