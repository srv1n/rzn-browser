use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};
use std::collections::{BTreeMap, HashSet};

pub const WORKFLOW_CONTRACT_VERSION: &str = "rzn.workflow_manifest";
pub const RUN_ENVELOPE_VERSION: &str = "rzn.run_envelope.v1";
pub const RUN_RESULT_VERSION: &str = "rzn.run_result.v2";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowManifestV2 {
    pub schema_version: String,
    pub id: String,
    pub name: String,
    pub version: String,
    pub system: String,
    pub capability: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub params: ParamSchemaV2,
    #[serde(default)]
    pub side_effects: Vec<SideEffectDeclarationV2>,
    #[serde(default)]
    pub runtime: RuntimeRequirementsV1,
    #[serde(default)]
    pub steps: Vec<StepV2>,
    #[serde(default)]
    pub result: ResultContractV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<HelpBlockV2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

impl WorkflowManifestV2 {
    pub fn validate(&self) -> Result<(), Vec<ContractValidationIssueV2>> {
        let mut issues = Vec::new();

        if self.schema_version != WORKFLOW_CONTRACT_VERSION {
            issues.push(ContractValidationIssueV2::new(
                "schema_version",
                format!("expected {WORKFLOW_CONTRACT_VERSION}"),
            ));
        }
        require_non_empty(&mut issues, "id", &self.id);
        require_non_empty(&mut issues, "name", &self.name);
        require_non_empty(&mut issues, "version", &self.version);
        require_non_empty(&mut issues, "system", &self.system);
        require_non_empty(&mut issues, "capability", &self.capability);

        self.params.validate_into("params", &mut issues);
        self.runtime.validate_into("runtime", &mut issues);

        let allowed_side_effects = self
            .side_effects
            .iter()
            .map(|effect| effect.class)
            .collect::<HashSet<_>>();
        let mut step_ids = HashSet::new();
        for (index, step) in self.steps.iter().enumerate() {
            let base = format!("steps[{index}]");
            require_non_empty(&mut issues, format!("{base}.id"), &step.id);
            if !step_ids.insert(step.id.clone()) {
                issues.push(ContractValidationIssueV2::new(
                    format!("{base}.id"),
                    format!("duplicate step id `{}`", step.id),
                ));
            }
            if step.action.kind == ActionKindV2::Custom {
                if step
                    .action
                    .custom_kind
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .is_empty()
                {
                    issues.push(ContractValidationIssueV2::new(
                        format!("{base}.action.custom_kind"),
                        "custom actions must declare custom_kind",
                    ));
                }
            }
            step.retry
                .validate_into(format!("{base}.retry"), &mut issues);

            for class in &step.action.side_effects {
                if !allowed_side_effects.contains(class) {
                    issues.push(ContractValidationIssueV2::new(
                        format!("{base}.action.side_effects"),
                        format!("step declares undeclared side effect `{}`", class.as_str()),
                    ));
                }
            }
        }

        if let Some(selector) = &self.result.output_selector {
            require_non_empty(
                &mut issues,
                "result.output_selector.step_id",
                &selector.step_id,
            );
            if !self.steps.is_empty() && !step_ids.contains(&selector.step_id) {
                issues.push(ContractValidationIssueV2::new(
                    "result.output_selector.step_id",
                    format!("references unknown step id `{}`", selector.step_id),
                ));
            }
        }

        result_from_issues(issues)
    }

    pub fn normalize_params(
        &self,
        input: &Value,
    ) -> Result<Map<String, Value>, Vec<ParamValidationIssueV2>> {
        self.params.normalize(input)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HelpBlockV2 {
    pub summary: String,
    #[serde(default)]
    pub parameters: BTreeMap<String, String>,
    #[serde(default)]
    pub examples: Vec<Value>,
    #[serde(default)]
    pub returns: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRequirementsV1 {
    #[serde(default = "default_actor")]
    pub actor: RuntimeActorV1,
    #[serde(default)]
    pub requires_cdp: bool,
    #[serde(default)]
    pub requires_existing_session: bool,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_path: Option<String>,
}

impl Default for RuntimeRequirementsV1 {
    fn default() -> Self {
        Self {
            actor: default_actor(),
            requires_cdp: false,
            requires_existing_session: false,
            timeout_ms: None,
            workflow_ref: None,
            workflow_path: None,
        }
    }
}

impl RuntimeRequirementsV1 {
    fn validate_into(&self, path: impl Into<String>, issues: &mut Vec<ContractValidationIssueV2>) {
        let path = path.into();
        if let Some(workflow_ref) = &self.workflow_ref {
            require_non_empty(issues, format!("{path}.workflow_ref"), workflow_ref);
        }
        if let Some(workflow_path) = &self.workflow_path {
            require_non_empty(issues, format!("{path}.workflow_path"), workflow_path);
        }
        if self.workflow_ref.is_some() && self.workflow_path.is_some() {
            issues.push(ContractValidationIssueV2::new(
                format!("{path}.workflow_ref"),
                "declare either workflow_ref or workflow_path, not both",
            ));
        }
    }
}

fn default_actor() -> RuntimeActorV1 {
    RuntimeActorV1::Extension
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeActorV1 {
    Extension,
    Supervisor,
    Cloud,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct ParamSchemaV2 {
    #[serde(default)]
    pub properties: BTreeMap<String, ParamDefV2>,
    #[serde(default)]
    pub additional_params: bool,
}

impl ParamSchemaV2 {
    pub fn validate_into(
        &self,
        path: impl Into<String>,
        issues: &mut Vec<ContractValidationIssueV2>,
    ) {
        let path = path.into();
        for (name, def) in &self.properties {
            if name.trim().is_empty() {
                issues.push(ContractValidationIssueV2::new(
                    format!("{path}.properties"),
                    "parameter names must not be empty",
                ));
            }
            if let Some(default) = &def.default {
                if let Err(param_issues) = def.validate_value(name, default) {
                    for issue in param_issues {
                        issues.push(ContractValidationIssueV2::new(
                            format!("{path}.properties.{name}.default"),
                            issue.message,
                        ));
                    }
                }
            }
        }
    }

    pub fn normalize(
        &self,
        input: &Value,
    ) -> Result<Map<String, Value>, Vec<ParamValidationIssueV2>> {
        let Some(input) = input.as_object() else {
            return Err(vec![ParamValidationIssueV2::new(
                "",
                "input must be a JSON object",
            )]);
        };

        let mut output = Map::new();
        let mut issues = Vec::new();

        for (name, def) in &self.properties {
            match input.get(name) {
                Some(raw) if !raw.is_null() => match def.coerce_value(name, raw) {
                    Ok(value) => {
                        if let Err(mut validation_issues) = def.validate_value(name, &value) {
                            issues.append(&mut validation_issues);
                        } else {
                            output.insert(name.clone(), value);
                        }
                    }
                    Err(issue) => issues.push(issue),
                },
                _ => {
                    if let Some(default) = &def.default {
                        output.insert(name.clone(), default.clone());
                    } else if def.required {
                        issues.push(ParamValidationIssueV2::new(
                            name,
                            "missing required parameter",
                        ));
                    }
                }
            }
        }

        if self.additional_params {
            for (name, value) in input {
                output.entry(name.clone()).or_insert_with(|| value.clone());
            }
        } else {
            for name in input.keys() {
                if !self.properties.contains_key(name) {
                    issues.push(ParamValidationIssueV2::new(name, "unknown parameter"));
                }
            }
        }

        result_from_issues_with_value(output, issues)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ParamDefV2 {
    pub kind: ParamKindV2,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub sensitive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enum_values: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
}

impl ParamDefV2 {
    fn coerce_value(&self, field: &str, raw: &Value) -> Result<Value, ParamValidationIssueV2> {
        match self.kind {
            ParamKindV2::String => match raw {
                Value::String(_) => Ok(raw.clone()),
                Value::Bool(value) => Ok(Value::String(value.to_string())),
                Value::Number(value) => Ok(Value::String(value.to_string())),
                _ => Err(ParamValidationIssueV2::new(field, "expected string")),
            },
            ParamKindV2::Integer => {
                coerce_integer(field, raw).map(|value| Value::Number(value.into()))
            }
            ParamKindV2::Number => coerce_number(field, raw).map(Value::Number),
            ParamKindV2::Boolean => coerce_bool(field, raw).map(Value::Bool),
            ParamKindV2::Object => {
                if raw.is_object() {
                    Ok(raw.clone())
                } else {
                    Err(ParamValidationIssueV2::new(field, "expected object"))
                }
            }
            ParamKindV2::Array => {
                if raw.is_array() {
                    Ok(raw.clone())
                } else {
                    Err(ParamValidationIssueV2::new(field, "expected array"))
                }
            }
        }
    }

    fn validate_value(
        &self,
        field: &str,
        value: &Value,
    ) -> Result<(), Vec<ParamValidationIssueV2>> {
        let mut issues = Vec::new();

        if !self.enum_values.is_empty() && !self.enum_values.iter().any(|item| item == value) {
            issues.push(ParamValidationIssueV2::new(
                field,
                "value is not in enum_values",
            ));
        }

        match self.kind {
            ParamKindV2::String => {
                if let Some(value) = value.as_str() {
                    if let Some(min) = self.min_length {
                        if value.chars().count() < min {
                            issues.push(ParamValidationIssueV2::new(
                                field,
                                format!("string shorter than min_length {min}"),
                            ));
                        }
                    }
                    if let Some(max) = self.max_length {
                        if value.chars().count() > max {
                            issues.push(ParamValidationIssueV2::new(
                                field,
                                format!("string longer than max_length {max}"),
                            ));
                        }
                    }
                } else {
                    issues.push(ParamValidationIssueV2::new(field, "expected string"));
                }
            }
            ParamKindV2::Integer => {
                validate_i64_range(field, value.as_i64(), self.min, self.max, &mut issues)
            }
            ParamKindV2::Number => {
                validate_f64_range(field, value.as_f64(), self.min, self.max, &mut issues)
            }
            ParamKindV2::Boolean if !value.is_boolean() => {
                issues.push(ParamValidationIssueV2::new(field, "expected boolean"));
            }
            ParamKindV2::Object if !value.is_object() => {
                issues.push(ParamValidationIssueV2::new(field, "expected object"));
            }
            ParamKindV2::Array if !value.is_array() => {
                issues.push(ParamValidationIssueV2::new(field, "expected array"));
            }
            _ => {}
        }

        result_from_issues(issues)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ParamKindV2 {
    String,
    Integer,
    Number,
    Boolean,
    Object,
    Array,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StepV2 {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub action: ActionV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub retry: RetryPolicyV1,
    #[serde(default)]
    pub continue_on_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ActionV2 {
    pub kind: ActionKindV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<TargetV2>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub inputs: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub options: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub side_effects: Vec<SideEffectClassV2>,
}

impl ActionV2 {
    pub fn new(kind: ActionKindV2) -> Self {
        Self {
            kind,
            custom_kind: None,
            target: None,
            inputs: Map::new(),
            options: Map::new(),
            side_effects: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionKindV2 {
    Navigate,
    NavigateToUrl,
    Click,
    ClickElement,
    FillInputField,
    TypeText,
    PressKey,
    PressSpecialKey,
    Wait,
    WaitForElement,
    WaitForTimeout,
    Scroll,
    ScrollElementIntoView,
    ScrollWindowTo,
    SelectOption,
    SubmitInput,
    UploadFile,
    Extract,
    ExtractStructuredData,
    GetElementText,
    Download,
    DownloadImages,
    ExecuteJavascript,
    EvalMainWorld,
    EvalIsolatedWorld,
    DismissPopups,
    DetectPopups,
    WaitForNoPopups,
    SameOriginRequest,
    AssertSelectorState,
    AssertTextInElement,
    AssertUrlMatches,
    InfiniteScroll,
    RequestUserIntervention,
    TakeScreenshot,
    OpenNewTab,
    SwitchToTab,
    CloseCurrentTab,
    WaitForNavigation,
    WaitForNetworkIdle,
    GetCurrentUrl,
    GetPageSource,
    FillAndSubmit,
    SubmitTextQuery,
    SelectResult,
    SelectOptionInDropdown,
    HoverElement,
    DblClickElement,
    DragAndDrop,
    Observe,
    GetElementAttribute,
    GetElementValue,
    GetElementCount,
    ReadFieldValue,
    ExecuteExtractionPlan,
    ExtractPageAssets,
    CaptureUiBundle,
    InspectElement,
    InspectClickSurface,
    VerifyUiChange,
    SemanticAction,
    ApplyFilterByText,
    DateSetRange,
    WaitForAuth,
    WaitForTotp,
    WaitForVerification,
    HandleCaptcha,
    ConfigureCaptchaSolver,
    SimulateHumanBehavior,
    ClearCookies,
    GetCookies,
    SetCookie,
    ClearLocalStorage,
    GetLocalStorageItem,
    SetLocalStorageItem,
    ClearEnhancedCaches,
    GetPerformanceStats,
    DownloadFile,
    Custom,
}

impl ActionKindV2 {
    pub fn engine_step_type(self) -> Option<&'static str> {
        match self {
            Self::Navigate => Some("navigate"),
            Self::NavigateToUrl => Some("navigate_to_url"),
            Self::Click => Some("click"),
            Self::ClickElement => Some("click_element"),
            Self::FillInputField => Some("fill_input_field"),
            Self::TypeText => Some("type_text"),
            Self::PressKey => Some("press_key"),
            Self::PressSpecialKey => Some("press_special_key"),
            Self::Wait => Some("wait"),
            Self::WaitForElement => Some("wait_for_element"),
            Self::WaitForTimeout => Some("wait_for_timeout"),
            Self::Scroll => Some("scroll"),
            Self::ScrollElementIntoView => Some("scroll_element_into_view"),
            Self::ScrollWindowTo => Some("scroll_window_to"),
            Self::SelectOption => Some("select_option"),
            Self::SubmitInput => Some("submit_input"),
            Self::UploadFile => Some("upload_file"),
            Self::Extract => Some("extract"),
            Self::ExtractStructuredData => Some("extract_structured_data"),
            Self::GetElementText => Some("get_element_text"),
            Self::Download => Some("download"),
            Self::DownloadImages => Some("download_images"),
            Self::ExecuteJavascript => Some("execute_javascript"),
            Self::EvalMainWorld => Some("eval_main_world"),
            Self::EvalIsolatedWorld => Some("eval_isolated_world"),
            Self::DismissPopups => Some("dismiss_popups"),
            Self::DetectPopups => Some("detect_popups"),
            Self::WaitForNoPopups => Some("wait_for_no_popups"),
            Self::SameOriginRequest => Some("same_origin_request"),
            Self::AssertSelectorState => Some("assert_selector_state"),
            Self::AssertTextInElement => Some("assert_text_in_element"),
            Self::AssertUrlMatches => Some("assert_url_matches"),
            Self::InfiniteScroll => Some("infinite_scroll"),
            Self::RequestUserIntervention => Some("request_user_intervention"),
            Self::TakeScreenshot => Some("take_screenshot"),
            Self::OpenNewTab => Some("open_new_tab"),
            Self::SwitchToTab => Some("switch_to_tab"),
            Self::CloseCurrentTab => Some("close_current_tab"),
            Self::WaitForNavigation => Some("wait_for_navigation"),
            Self::WaitForNetworkIdle => Some("wait_for_network_idle"),
            Self::GetCurrentUrl => Some("get_current_url"),
            Self::GetPageSource => Some("get_page_source"),
            Self::FillAndSubmit => Some("fill_and_submit"),
            Self::SubmitTextQuery => Some("submit_text_query"),
            Self::SelectResult => Some("select_result"),
            Self::SelectOptionInDropdown => Some("select_option_in_dropdown"),
            Self::HoverElement => Some("hover_element"),
            Self::DblClickElement => Some("dbl_click_element"),
            Self::DragAndDrop => Some("drag_and_drop"),
            Self::Observe => Some("observe"),
            Self::GetElementAttribute => Some("get_element_attribute"),
            Self::GetElementValue => Some("get_element_value"),
            Self::GetElementCount => Some("get_element_count"),
            Self::ReadFieldValue => Some("read_field_value"),
            Self::ExecuteExtractionPlan => Some("execute_extraction_plan"),
            Self::ExtractPageAssets => Some("extract_page_assets"),
            Self::CaptureUiBundle => Some("capture_ui_bundle"),
            Self::InspectElement => Some("inspect_element"),
            Self::InspectClickSurface => Some("inspect_click_surface"),
            Self::VerifyUiChange => Some("verify_ui_change"),
            Self::SemanticAction => Some("semantic_action"),
            Self::ApplyFilterByText => Some("apply_filter_by_text"),
            Self::DateSetRange => Some("date_set_range"),
            Self::WaitForAuth => Some("wait_for_auth"),
            Self::WaitForTotp => Some("wait_for_totp"),
            Self::WaitForVerification => Some("wait_for_verification"),
            Self::HandleCaptcha => Some("handle_captcha"),
            Self::ConfigureCaptchaSolver => Some("configure_captcha_solver"),
            Self::SimulateHumanBehavior => Some("simulate_human_behavior"),
            Self::ClearCookies => Some("clear_cookies"),
            Self::GetCookies => Some("get_cookies"),
            Self::SetCookie => Some("set_cookie"),
            Self::ClearLocalStorage => Some("clear_local_storage"),
            Self::GetLocalStorageItem => Some("get_local_storage_item"),
            Self::SetLocalStorageItem => Some("set_local_storage_item"),
            Self::ClearEnhancedCaches => Some("clear_enhanced_caches"),
            Self::GetPerformanceStats => Some("get_performance_stats"),
            Self::DownloadFile => Some("download_file"),
            Self::Custom => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TargetV2 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoded_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RetryPolicyV1 {
    /// Total delivery attempts for this step, including the initial try.
    ///
    /// `max_attempts = 1` means "try once and do not retry"; the retry budget is
    /// therefore `max_attempts - 1`.
    #[serde(default)]
    pub max_attempts: u8,
    #[serde(default)]
    pub backoff_ms: Option<u64>,
}

impl Default for RetryPolicyV1 {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            backoff_ms: None,
        }
    }
}

impl RetryPolicyV1 {
    fn validate_into(&self, path: impl Into<String>, issues: &mut Vec<ContractValidationIssueV2>) {
        let path = path.into();
        if self.max_attempts == 0 {
            issues.push(ContractValidationIssueV2::new(
                format!("{path}.max_attempts"),
                "max_attempts is total attempts including the initial try and must be at least 1",
            ));
        }
    }

    pub fn retry_budget(self) -> u8 {
        self.max_attempts.saturating_sub(1)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SideEffectDeclarationV2 {
    pub class: SideEffectClassV2,
    #[serde(default)]
    pub idempotency: IdempotencyPolicyV1,
    #[serde(default)]
    pub confirmation_required: bool,
    #[serde(default)]
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectClassV2 {
    ReadOnly,
    ExternalRead,
    NetworkAccess,
    BrowserState,
    FileWrite,
    Download,
    ExternalWrite,
    Auth,
    Destructive,
}

impl SideEffectClassV2 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::ExternalRead => "external_read",
            Self::NetworkAccess => "network_access",
            Self::BrowserState => "browser_state",
            Self::FileWrite => "file_write",
            Self::Download => "download",
            Self::ExternalWrite => "external_write",
            Self::Auth => "auth",
            Self::Destructive => "destructive",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum IdempotencyPolicyV1 {
    #[default]
    SafeRetry,
    SingleDelivery,
    CallerProvidedKey,
    NeverRetry,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RunEnvelopeV1 {
    pub version: String,
    pub run_id: String,
    pub workflow_id: String,
    pub workflow_version: String,
    pub system: String,
    pub capability: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<u64>,
    #[serde(default)]
    pub params: Map<String, Value>,
    #[serde(default)]
    pub policy: RunPolicyV1,
}

impl RunEnvelopeV1 {
    pub fn validate_for_manifest(
        &self,
        manifest: &WorkflowManifestV2,
    ) -> Result<(), Vec<ContractValidationIssueV2>> {
        let mut issues = Vec::new();
        if self.version != RUN_ENVELOPE_VERSION {
            issues.push(ContractValidationIssueV2::new(
                "version",
                format!("expected {RUN_ENVELOPE_VERSION}"),
            ));
        }
        if self.workflow_id != manifest.id {
            issues.push(ContractValidationIssueV2::new(
                "workflow_id",
                "does not match manifest id",
            ));
        }
        if self.workflow_version != manifest.version {
            issues.push(ContractValidationIssueV2::new(
                "workflow_version",
                "does not match manifest version",
            ));
        }
        if self.system != manifest.system {
            issues.push(ContractValidationIssueV2::new(
                "system",
                "does not match manifest system",
            ));
        }
        if self.capability != manifest.capability {
            issues.push(ContractValidationIssueV2::new(
                "capability",
                "does not match manifest capability",
            ));
        }

        if let Err(param_issues) = manifest
            .params
            .normalize(&Value::Object(self.params.clone()))
        {
            for issue in param_issues {
                issues.push(ContractValidationIssueV2::new(
                    format!("params.{}", issue.field),
                    issue.message,
                ));
            }
        }

        result_from_issues(issues)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct RunPolicyV1 {
    #[serde(default)]
    pub allow_side_effects: Vec<SideEffectClassV2>,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RunResultV2 {
    pub version: String,
    pub run_id: String,
    pub workflow_id: String,
    pub status: RunStatusV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<RunWarningV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<StepRunResultV2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug: Option<DebugBundleV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RunErrorV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatusV2 {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    PolicyBlocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct ResultContractV2 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_selector: Option<OutputSelectorV1>,
    #[serde(default)]
    pub artifact_policy: ArtifactPolicyV1,
    #[serde(default)]
    pub include_debug: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OutputSelectorV1 {
    pub step_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct ArtifactPolicyV1 {
    #[serde(default)]
    pub max_inline_bytes: Option<u64>,
    #[serde(default)]
    pub prefer_downloads: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactV1 {
    pub id: String,
    pub kind: ArtifactKindV1,
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKindV1 {
    File,
    Download,
    Screenshot,
    Json,
    Text,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RunWarningV1 {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StepRunResultV2 {
    pub step_id: String,
    pub status: RunStatusV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RunErrorV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<RunWarningV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DebugBundleV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<DebugEventV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DebugEventV1 {
    pub at_ms: u64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RunErrorV1 {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContractValidationIssueV2 {
    pub field: String,
    pub message: String,
}

impl ContractValidationIssueV2 {
    pub fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ParamValidationIssueV2 {
    pub field: String,
    pub message: String,
}

impl ParamValidationIssueV2 {
    pub fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

pub fn validate_manifest_value(
    value: &Value,
) -> Result<WorkflowManifestV2, Vec<ContractValidationIssueV2>> {
    let manifest = serde_json::from_value::<WorkflowManifestV2>(value.clone()).map_err(|err| {
        vec![ContractValidationIssueV2::new(
            "",
            format!("invalid manifest JSON: {err}"),
        )]
    })?;
    manifest.validate()?;
    Ok(manifest)
}

pub fn validate_run_envelope_value(
    manifest: &WorkflowManifestV2,
    value: &Value,
) -> Result<RunEnvelopeV1, Vec<ContractValidationIssueV2>> {
    let envelope = serde_json::from_value::<RunEnvelopeV1>(value.clone()).map_err(|err| {
        vec![ContractValidationIssueV2::new(
            "",
            format!("invalid run envelope JSON: {err}"),
        )]
    })?;
    envelope.validate_for_manifest(manifest)?;
    Ok(envelope)
}

fn coerce_integer(field: &str, raw: &Value) -> Result<i64, ParamValidationIssueV2> {
    match raw {
        Value::Number(value) => value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
            .ok_or_else(|| ParamValidationIssueV2::new(field, "expected integer")),
        Value::String(value) => value
            .trim()
            .parse::<i64>()
            .map_err(|_| ParamValidationIssueV2::new(field, "expected integer")),
        _ => Err(ParamValidationIssueV2::new(field, "expected integer")),
    }
}

fn coerce_number(field: &str, raw: &Value) -> Result<Number, ParamValidationIssueV2> {
    match raw {
        Value::Number(value) => Ok(value.clone()),
        Value::String(value) => value
            .trim()
            .parse::<f64>()
            .ok()
            .and_then(Number::from_f64)
            .ok_or_else(|| ParamValidationIssueV2::new(field, "expected number")),
        _ => Err(ParamValidationIssueV2::new(field, "expected number")),
    }
}

fn coerce_bool(field: &str, raw: &Value) -> Result<bool, ParamValidationIssueV2> {
    match raw {
        Value::Bool(value) => Ok(*value),
        Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Ok(true),
            "false" | "0" | "no" => Ok(false),
            _ => Err(ParamValidationIssueV2::new(field, "expected boolean")),
        },
        _ => Err(ParamValidationIssueV2::new(field, "expected boolean")),
    }
}

fn validate_i64_range(
    field: &str,
    value: Option<i64>,
    min: Option<i64>,
    max: Option<i64>,
    issues: &mut Vec<ParamValidationIssueV2>,
) {
    let Some(value) = value else {
        issues.push(ParamValidationIssueV2::new(field, "expected integer"));
        return;
    };
    if let Some(min) = min {
        if value < min {
            issues.push(ParamValidationIssueV2::new(
                field,
                format!("value below min {min}"),
            ));
        }
    }
    if let Some(max) = max {
        if value > max {
            issues.push(ParamValidationIssueV2::new(
                field,
                format!("value above max {max}"),
            ));
        }
    }
}

fn validate_f64_range(
    field: &str,
    value: Option<f64>,
    min: Option<i64>,
    max: Option<i64>,
    issues: &mut Vec<ParamValidationIssueV2>,
) {
    let Some(value) = value else {
        issues.push(ParamValidationIssueV2::new(field, "expected number"));
        return;
    };
    if let Some(min) = min {
        if value < min as f64 {
            issues.push(ParamValidationIssueV2::new(
                field,
                format!("value below min {min}"),
            ));
        }
    }
    if let Some(max) = max {
        if value > max as f64 {
            issues.push(ParamValidationIssueV2::new(
                field,
                format!("value above max {max}"),
            ));
        }
    }
}

fn require_non_empty(
    issues: &mut Vec<ContractValidationIssueV2>,
    field: impl Into<String>,
    value: &str,
) {
    if value.trim().is_empty() {
        issues.push(ContractValidationIssueV2::new(field, "must not be empty"));
    }
}

fn result_from_issues<E>(issues: Vec<E>) -> Result<(), Vec<E>> {
    if issues.is_empty() {
        Ok(())
    } else {
        Err(issues)
    }
}

fn result_from_issues_with_value<T, E>(value: T, issues: Vec<E>) -> Result<T, Vec<E>> {
    if issues.is_empty() {
        Ok(value)
    } else {
        Err(issues)
    }
}
