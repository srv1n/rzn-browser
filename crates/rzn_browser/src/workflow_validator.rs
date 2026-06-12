use colored::*;
use serde_json::{Value, Map};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::fs;
use regex::Regex;
use url::Url;

#[derive(Debug)]
pub struct ValidationIssue {
    pub level: IssueLevel,
    pub location: String,
    pub message: String,
    pub suggestion: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum IssueLevel {
    Error,
    Warning,
    Info,
}

pub struct WorkflowValidator {
    workflow: Value,
    issues: Vec<ValidationIssue>,
    action_schema: HashMap<String, ActionSchema>,
}

#[derive(Debug)]
struct ActionSchema {
    required_fields: Vec<String>,
    optional_fields: Vec<String>,
    field_types: HashMap<String, String>,
}

impl WorkflowValidator {
    pub fn new(workflow_path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(workflow_path)?;
        let workflow: Value = serde_json::from_str(&content)?;
        
        Ok(Self {
            workflow,
            issues: Vec::new(),
            action_schema: Self::build_action_schema(),
        })
    }
    
    pub fn from_json(workflow: Value) -> Self {
        Self {
            workflow,
            issues: Vec::new(),
            action_schema: Self::build_action_schema(),
        }
    }
    
    fn build_action_schema() -> HashMap<String, ActionSchema> {
        let mut schema = HashMap::new();
        
        // Navigation actions
        schema.insert("navigate_to_url".to_string(), ActionSchema {
            required_fields: vec!["url".to_string()],
            optional_fields: vec!["strict".to_string()],
            field_types: HashMap::from([
                ("url".to_string(), "string".to_string()),
                ("strict".to_string(), "boolean".to_string()),
            ]),
        });
        
        // Click actions
        schema.insert("click_element".to_string(), ActionSchema {
            required_fields: vec!["selectors".to_string()],
            optional_fields: vec!["wait_before_ms".to_string(), "wait_after_ms".to_string(), "strict".to_string()],
            field_types: HashMap::from([
                ("selectors".to_string(), "object".to_string()),
                ("wait_before_ms".to_string(), "number".to_string()),
                ("wait_after_ms".to_string(), "number".to_string()),
                ("strict".to_string(), "boolean".to_string()),
            ]),
        });
        
        // Input actions
        schema.insert("fill_input_field".to_string(), ActionSchema {
            required_fields: vec!["selectors".to_string(), "value".to_string()],
            optional_fields: vec!["clear_first".to_string(), "press_enter_after".to_string(), "wait_before_ms".to_string(), "wait_after_ms".to_string()],
            field_types: HashMap::from([
                ("selectors".to_string(), "object".to_string()),
                ("value".to_string(), "string".to_string()),
                ("clear_first".to_string(), "boolean".to_string()),
                ("press_enter_after".to_string(), "boolean".to_string()),
            ]),
        });

        schema.insert("type_text".to_string(), ActionSchema {
            required_fields: vec!["selector".to_string()],
            optional_fields: vec![
                "text".to_string(),
                "value".to_string(),
                "use_native_input".to_string(),
                "delay_ms".to_string(),
                "typing_speed".to_string(),
                "timeout_ms".to_string(),
            ],
            field_types: HashMap::from([
                ("selector".to_string(), "string".to_string()),
                ("text".to_string(), "string".to_string()),
                ("value".to_string(), "string".to_string()),
                ("use_native_input".to_string(), "boolean".to_string()),
                ("delay_ms".to_string(), "number".to_string()),
                ("typing_speed".to_string(), "string".to_string()),
                ("timeout_ms".to_string(), "number".to_string()),
            ]),
        });
        
        // Wait actions
        schema.insert("wait_for_element".to_string(), ActionSchema {
            required_fields: vec!["selectors".to_string()],
            optional_fields: vec!["timeout_ms".to_string(), "visible".to_string()],
            field_types: HashMap::from([
                ("selectors".to_string(), "object".to_string()),
                ("timeout_ms".to_string(), "number".to_string()),
                ("visible".to_string(), "boolean".to_string()),
            ]),
        });
        
        schema.insert("wait_for_timeout".to_string(), ActionSchema {
            required_fields: vec!["milliseconds".to_string()],
            optional_fields: vec![],
            field_types: HashMap::from([
                ("milliseconds".to_string(), "number".to_string()),
            ]),
        });
        
        // Add more action schemas as needed...
        
        schema
    }
    
    pub fn validate(&mut self) -> &Vec<ValidationIssue> {
        self.issues.clear();
        
        // Validate top-level structure
        self.validate_metadata();
        self.validate_parameters();
        self.validate_actions();
        
        // Semantic validation
        self.validate_variable_usage();
        self.validate_selectors();
        self.validate_workflow_flow();
        
        // Performance and best practices
        self.check_performance_issues();
        self.check_best_practices();
        
        // V1 to V2 migration suggestions
        self.check_legacy_patterns();
        
        &self.issues
    }
    
    fn validate_metadata(&mut self) {
        let required_fields = ["system_id", "id", "name", "version"];
        
        for field in required_fields {
            if self.workflow.get(field).is_none() {
                self.add_issue(
                    IssueLevel::Error,
                    "metadata",
                    &format!("Missing required field: {}", field),
                    Some(format!("Add '{}' field to workflow metadata", field)),
                );
            }
        }
        
        // Validate version format
        if let Some(version) = self.workflow.get("version").and_then(|v| v.as_str()) {
            if !Regex::new(r"^\d+\.\d+\.\d+$").unwrap().is_match(version) {
                self.add_issue(
                    IssueLevel::Warning,
                    "metadata.version",
                    "Version should follow semantic versioning (x.y.z)",
                    Some("Use format like '1.0.0'".to_string()),
                );
            }
        }
        
        // Check for description
        if self.workflow.get("description").is_none() {
            self.add_issue(
                IssueLevel::Info,
                "metadata",
                "Consider adding a description field",
                Some("Add 'description' to explain workflow purpose".to_string()),
            );
        }
    }
    
    fn validate_parameters(&mut self) {
        if let Some(params) = self.workflow.get("parameters").and_then(|p| p.as_array()) {
            let mut param_names = HashSet::new();
            
            for (i, param) in params.iter().enumerate() {
                let location = format!("parameters[{}]", i);
                
                // Check required fields
                if param.get("name").is_none() {
                    self.add_issue(
                        IssueLevel::Error,
                        &location,
                        "Parameter missing 'name' field",
                        None,
                    );
                }
                
                if param.get("type").is_none() {
                    self.add_issue(
                        IssueLevel::Error,
                        &location,
                        "Parameter missing 'type' field",
                        Some("Add type: 'string', 'number', 'boolean', etc.".to_string()),
                    );
                }
                
                // Check for duplicate names
                if let Some(name) = param.get("name").and_then(|n| n.as_str()) {
                    if !param_names.insert(name.to_string()) {
                        self.add_issue(
                            IssueLevel::Error,
                            &location,
                            &format!("Duplicate parameter name: {}", name),
                            None,
                        );
                    }
                }
                
                // Validate parameter type
                if let Some(param_type) = param.get("type").and_then(|t| t.as_str()) {
                    let valid_types = ["string", "number", "boolean", "array", "object"];
                    if !valid_types.contains(&param_type) {
                        self.add_issue(
                            IssueLevel::Warning,
                            &format!("{}.type", location),
                            &format!("Invalid parameter type: {}", param_type),
                            Some(format!("Use one of: {}", valid_types.join(", "))),
                        );
                    }
                }
            }
        }
    }
    
    fn validate_actions(&mut self) {
        let actions = if let Some(actions) = self.workflow.get("actions").and_then(|a| a.as_array()) {
            actions
        } else if let Some(automation) = self.workflow.get("browser_automation").and_then(|b| b.as_object()) {
            // Legacy v1 format
            self.add_issue(
                IssueLevel::Warning,
                "browser_automation",
                "Using legacy v1 workflow format",
                Some("Migrate to v2 format with 'actions' array".to_string()),
            );
            return;
        } else {
            self.add_issue(
                IssueLevel::Error,
                "root",
                "Missing 'actions' array",
                None,
            );
            return;
        };
        
        let mut action_ids = HashSet::new();
        
        for (i, action) in actions.iter().enumerate() {
            let location = format!("actions[{}]", i);
            
            // Validate action ID
            if let Some(id) = action.get("id").and_then(|id| id.as_str()) {
                if !action_ids.insert(id.to_string()) {
                    self.add_issue(
                        IssueLevel::Error,
                        &location,
                        &format!("Duplicate action ID: {}", id),
                        Some("Each action must have a unique ID".to_string()),
                    );
                }
            } else {
                self.add_issue(
                    IssueLevel::Error,
                    &location,
                    "Action missing 'id' field",
                    None,
                );
            }
            
            // Validate action type
            let action_type = self.get_action_type(action);
            if action_type.is_none() {
                self.add_issue(
                    IssueLevel::Error,
                    &location,
                    "Action has no recognized type",
                    Some("Add a valid action type (e.g., navigate_to_url, click_element)".to_string()),
                );
                continue;
            }
            
            // Validate action schema
            if let Some(action_type) = action_type {
                self.validate_action_schema(&location, &action_type, action);
            }
        }
    }
    
    fn get_action_type(&self, action: &Value) -> Option<String> {
        let action_obj = action.as_object()?;
        
        // Find the action type key (excluding metadata fields)
        let metadata_fields = ["id", "name", "description", "condition", "retry", "timeout", "store_result", "error_handler"];
        
        for (key, value) in action_obj {
            if !metadata_fields.contains(&key.as_str()) && value.is_object() {
                return Some(key.clone());
            }
        }
        
        None
    }
    
    fn validate_action_schema(&mut self, location: &str, action_type: &str, action: &Value) {
        if let Some(schema) = self.action_schema.get(action_type) {
            if let Some(action_data) = action.get(action_type).and_then(|a| a.as_object()) {
                // Check required fields
                for required in &schema.required_fields {
                    if action_data.get(required).is_none() {
                        self.add_issue(
                            IssueLevel::Error,
                            &format!("{}.{}", location, action_type),
                            &format!("Missing required field: {}", required),
                            None,
                        );
                    }
                }
                
                // Check for unknown fields
                for field in action_data.keys() {
                    if !schema.required_fields.contains(&field.to_string()) && 
                       !schema.optional_fields.contains(&field.to_string()) {
                        self.add_issue(
                            IssueLevel::Warning,
                            &format!("{}.{}.{}", location, action_type, field),
                            &format!("Unknown field: {}", field),
                            Some("Check documentation for valid fields".to_string()),
                        );
                    }
                }
            }
        } else {
            self.add_issue(
                IssueLevel::Info,
                location,
                &format!("Unknown action type: {}", action_type),
                Some("This might be a custom action or typo".to_string()),
            );
        }
    }
    
    fn validate_variable_usage(&mut self) {
        let mut defined_vars = HashSet::new();
        let mut used_vars = HashSet::new();
        
        // Collect parameter names
        if let Some(params) = self.workflow.get("parameters").and_then(|p| p.as_array()) {
            for param in params {
                if let Some(name) = param.get("name").and_then(|n| n.as_str()) {
                    defined_vars.insert(name.to_string());
                }
            }
        }
        
        // Collect store_result variables and find template variables
        if let Some(actions) = self.workflow.get("actions").and_then(|a| a.as_array()) {
            for action in actions {
                if let Some(store_result) = action.get("store_result").and_then(|s| s.as_str()) {
                    defined_vars.insert(store_result.to_string());
                }
                
                // Find template variables in action
                self.find_template_variables(action, &mut used_vars);
            }
        }
        
        // Check for undefined variables
        for var in &used_vars {
            if !defined_vars.contains(var) && !var.contains('.') {
                self.add_issue(
                    IssueLevel::Warning,
                    "variables",
                    &format!("Undefined variable: {}", var),
                    Some("Define as parameter or store from previous action".to_string()),
                );
            }
        }
        
        // Check for unused parameters
        for var in &defined_vars {
            if !used_vars.contains(var) {
                self.add_issue(
                    IssueLevel::Info,
                    "variables",
                    &format!("Unused variable: {}", var),
                    None,
                );
            }
        }
    }
    
    fn find_template_variables(&self, value: &Value, vars: &mut HashSet<String>) {
        match value {
            Value::String(s) => {
                let re = Regex::new(r"\{([^}]+)\}").unwrap();
                for cap in re.captures_iter(s) {
                    vars.insert(cap[1].to_string());
                }
            }
            Value::Array(arr) => {
                for item in arr {
                    self.find_template_variables(item, vars);
                }
            }
            Value::Object(obj) => {
                for (_, val) in obj {
                    self.find_template_variables(val, vars);
                }
            }
            _ => {}
        }
    }
    
    fn validate_selectors(&mut self) {
        if let Some(actions) = self.workflow.get("actions").and_then(|a| a.as_array()) {
            for (i, action) in actions.iter().enumerate() {
                let action_type = self.get_action_type(action);
                if let Some(action_type) = action_type {
                    if let Some(action_data) = action.get(&action_type) {
                        if let Some(selectors) = action_data.get("selectors").and_then(|s| s.as_object()) {
                            self.validate_selector_object(&format!("actions[{}].{}.selectors", i, action_type), selectors);
                        }
                    }
                }
            }
        }
    }
    
    fn validate_selector_object(&mut self, location: &str, selectors: &Map<String, Value>) {
        if selectors.is_empty() {
            self.add_issue(
                IssueLevel::Error,
                location,
                "Empty selectors object",
                Some("Add at least one selector (css, xpath, or text)".to_string()),
            );
            return;
        }
        
        // Validate CSS selectors
        if let Some(css) = selectors.get("css").and_then(|c| c.as_str()) {
            self.validate_css_selector(location, css);
        }
        
        // Validate XPath selectors
        if let Some(xpath) = selectors.get("xpath").and_then(|x| x.as_str()) {
            self.validate_xpath_selector(location, xpath);
        }
        
        // Check for best practices
        if selectors.len() == 1 {
            self.add_issue(
                IssueLevel::Info,
                location,
                "Consider adding fallback selectors",
                Some("Multiple selector types improve reliability".to_string()),
            );
        }
    }
    
    fn validate_css_selector(&mut self, location: &str, selector: &str) {
        // Check for common CSS selector issues
        if selector.contains("//") {
            self.add_issue(
                IssueLevel::Error,
                &format!("{}.css", location),
                "CSS selector contains XPath syntax",
                Some("Use proper CSS selector syntax".to_string()),
            );
        }
        
        if selector.starts_with("/") {
            self.add_issue(
                IssueLevel::Error,
                &format!("{}.css", location),
                "CSS selector looks like XPath",
                Some("Move to 'xpath' field instead".to_string()),
            );
        }
        
        // Check for overly specific selectors
        let parts: Vec<&str> = selector.split_whitespace().collect();
        if parts.len() > 5 {
            self.add_issue(
                IssueLevel::Warning,
                &format!("{}.css", location),
                "CSS selector is very specific and may be fragile",
                Some("Consider using simpler, more robust selectors".to_string()),
            );
        }
        
        // Check for performance issues
        if selector.starts_with("*") || selector.contains(" * ") {
            self.add_issue(
                IssueLevel::Warning,
                &format!("{}.css", location),
                "Universal selector (*) can be slow",
                Some("Use more specific selectors for better performance".to_string()),
            );
        }
    }
    
    fn validate_xpath_selector(&mut self, location: &str, selector: &str) {
        if !selector.starts_with("//") && !selector.starts_with("/") {
            self.add_issue(
                IssueLevel::Warning,
                &format!("{}.xpath", location),
                "XPath selector should start with / or //",
                None,
            );
        }
        
        // Check for common XPath mistakes
        if selector.contains("@class=") && !selector.contains("'") && !selector.contains("\"") {
            self.add_issue(
                IssueLevel::Error,
                &format!("{}.xpath", location),
                "XPath attribute values must be quoted",
                Some("Use [@class='value'] or [@class=\"value\"]".to_string()),
            );
        }
    }
    
    fn validate_workflow_flow(&mut self) {
        if let Some(actions) = self.workflow.get("actions").and_then(|a| a.as_array()) {
            let mut has_navigation = false;
            let mut wait_count = 0;
            
            for (i, action) in actions.iter().enumerate() {
                let action_type = self.get_action_type(action);
                
                match action_type.as_deref() {
                    Some("navigate_to_url") => has_navigation = true,
                    Some("wait_for_timeout") => wait_count += 1,
                    Some("click_element") | Some("fill_input_field") | Some("type_text") => {
                        if !has_navigation && i == 0 {
                            self.add_issue(
                                IssueLevel::Warning,
                                &format!("actions[{}]", i),
                                "Interaction before navigation",
                                Some("Add navigate_to_url as first action".to_string()),
                            );
                        }
                    }
                    _ => {}
                }
            }
            
            if wait_count > actions.len() / 3 {
                self.add_issue(
                    IssueLevel::Warning,
                    "workflow",
                    "Excessive use of wait_for_timeout",
                    Some("Consider using wait_for_element instead".to_string()),
                );
            }
        }
    }
    
    fn check_performance_issues(&mut self) {
        if let Some(actions) = self.workflow.get("actions").and_then(|a| a.as_array()) {
            let mut total_wait_time = 0;
            
            for action in actions {
                // Check for excessive timeouts
                if let Some(timeout) = action.get("timeout").and_then(|t| t.as_u64()) {
                    if timeout > 30000 {
                        self.add_issue(
                            IssueLevel::Warning,
                            "performance",
                            &format!("Very long timeout: {}ms", timeout),
                            Some("Consider reducing timeout or splitting action".to_string()),
                        );
                    }
                }
                
                // Sum up explicit waits
                if let Some(wait) = action.get("wait_for_timeout") {
                    if let Some(ms) = wait.get("milliseconds").and_then(|m| m.as_u64()) {
                        total_wait_time += ms;
                    }
                }
            }
            
            if total_wait_time > 10000 {
                self.add_issue(
                    IssueLevel::Info,
                    "performance",
                    &format!("Total explicit wait time: {}ms", total_wait_time),
                    Some("Consider reducing wait times for faster execution".to_string()),
                );
            }
        }
    }
    
    fn check_best_practices(&mut self) {
        // Check for error handling
        let has_error_handling = self.workflow.get("error_handling").is_some();
        let actions_with_handlers = if let Some(actions) = self.workflow.get("actions").and_then(|a| a.as_array()) {
            actions.iter().filter(|a| a.get("error_handler").is_some()).count()
        } else {
            0
        };
        
        if !has_error_handling && actions_with_handlers == 0 {
            self.add_issue(
                IssueLevel::Info,
                "workflow",
                "No error handling configured",
                Some("Add error_handler to critical actions".to_string()),
            );
        }
        
        // Check for retries on critical actions
        if let Some(actions) = self.workflow.get("actions").and_then(|a| a.as_array()) {
            for (i, action) in actions.iter().enumerate() {
                if let Some(action_type) = self.get_action_type(action) {
                    if ["click_element", "fill_input_field", "type_text", "extract_structured_data"].contains(&action_type.as_str()) {
                        if action.get("retry").is_none() {
                            self.add_issue(
                                IssueLevel::Info,
                                &format!("actions[{}]", i),
                                "Consider adding retry configuration",
                                Some("Critical actions benefit from retry logic".to_string()),
                            );
                        }
                    }
                }
            }
        }
    }
    
    fn check_legacy_patterns(&mut self) {
        // Check for v1 patterns
        if self.workflow.get("browser_automation").is_some() {
            self.add_issue(
                IssueLevel::Warning,
                "migration",
                "Workflow uses v1 format",
                Some("Run migration tool to convert to v2".to_string()),
            );
        }
        
        // Check for old action patterns
        if let Some(actions) = self.workflow.get("actions").and_then(|a| a.as_array()) {
            for action in actions {
                if action.get("type").is_some() && action.get("action").is_some() {
                    self.add_issue(
                        IssueLevel::Warning,
                        "migration",
                        "Action uses legacy type/action pattern",
                        Some("Use new action format (e.g., click_element: {...})".to_string()),
                    );
                }
            }
        }
    }
    
    fn add_issue(&mut self, level: IssueLevel, location: &str, message: &str, suggestion: Option<String>) {
        self.issues.push(ValidationIssue {
            level,
            location: location.to_string(),
            message: message.to_string(),
            suggestion,
        });
    }
    
    pub fn print_report(&self) {
        if self.issues.is_empty() {
            println!("{}", "[OK] Workflow validation passed!".green().bold());
            return;
        }
        
        let errors: Vec<_> = self.issues.iter().filter(|i| i.level == IssueLevel::Error).collect();
        let warnings: Vec<_> = self.issues.iter().filter(|i| i.level == IssueLevel::Warning).collect();
        let info: Vec<_> = self.issues.iter().filter(|i| i.level == IssueLevel::Info).collect();
        
        println!("{}", "Workflow Validation Report".bold().underline());
        println!();
        
        if !errors.is_empty() {
            println!("{} {} found:", "[ERROR] Errors".red().bold(), errors.len());
            for issue in &errors {
                self.print_issue(issue);
            }
            println!();
        }
        
        if !warnings.is_empty() {
            println!("{} {} found:", "[WARNING]  Warnings".yellow().bold(), warnings.len());
            for issue in &warnings {
                self.print_issue(issue);
            }
            println!();
        }
        
        if !info.is_empty() {
            println!("{} {} suggestions:", "[INFO] Info".blue().bold(), info.len());
            for issue in &info {
                self.print_issue(issue);
            }
            println!();
        }
        
        // Summary
        println!("{}", "Summary:".bold());
        println!("  {} errors", errors.len().to_string().red());
        println!("  {} warnings", warnings.len().to_string().yellow());
        println!("  {} suggestions", info.len().to_string().blue());
        
        if !errors.is_empty() {
            println!();
            println!("{}", "[ERROR] Validation failed! Fix errors before running workflow.".red().bold());
        }
    }
    
    fn print_issue(&self, issue: &ValidationIssue) {
        let icon = match issue.level {
            IssueLevel::Error => "  [ERROR]",
            IssueLevel::Warning => "  [WARNING] ",
            IssueLevel::Info => "  [INFO] ",
        };
        
        println!("{} {} {}", 
            icon,
            format!("[{}]", issue.location).bright_black(),
            issue.message
        );
        
        if let Some(suggestion) = &issue.suggestion {
            println!("     {} {}", "->".green(), suggestion.bright_black());
        }
    }
    
    pub fn suggest_optimizations(&self) -> Vec<String> {
        let mut suggestions = Vec::new();
        
        // Analyze selector usage
        let mut selector_types = HashMap::new();
        if let Some(actions) = self.workflow.get("actions").and_then(|a| a.as_array()) {
            for action in actions {
                if let Some(action_type) = self.get_action_type(action) {
                    if let Some(selectors) = action.get(&action_type)
                        .and_then(|a| a.get("selectors"))
                        .and_then(|s| s.as_object()) {
                        for selector_type in selectors.keys() {
                            *selector_types.entry(selector_type.clone()).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
        
        // Suggest selector optimizations
        if let Some(css_count) = selector_types.get("css") {
            if *css_count < selector_types.values().sum::<usize>() / 2 {
                suggestions.push(
                    "Consider using more CSS selectors for better performance".to_string()
                );
            }
        }
        
        // Check for parallel execution opportunities
        suggestions.push(
            "Consider using parallel execution for independent data extraction steps".to_string()
        );
        
        suggestions
    }
}

// CLI command for validation
pub fn validate_workflow_command(workflow_path: &str, check_live: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut validator = WorkflowValidator::new(workflow_path)?;
    let issues = validator.validate();
    
    validator.print_report();
    
    if check_live {
        println!();
        println!("{}", "Checking selectors against live page...".cyan());
        // TODO: Implement live selector checking
        println!("{}", "Live checking not yet implemented".yellow());
    }
    
    if issues.iter().any(|i| i.level == IssueLevel::Error) {
        std::process::exit(1);
    }
    
    Ok(())
}
