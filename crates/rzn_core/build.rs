use std::env;
use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=../../schema/actions-v1.json");

    let schema_path = Path::new("../../schema/actions-v1.json");
    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("step.rs");

    // Read the schema file
    let schema_content =
        fs::read_to_string(schema_path).expect("Failed to read actions-v1.json schema file");

    // Parse the schema to extract action types
    let schema: serde_json::Value =
        serde_json::from_str(&schema_content).expect("Failed to parse actions-v1.json");

    // Extract action types from enum
    let action_types = schema["properties"]["type"]["enum"]
        .as_array()
        .expect("Failed to find action types in schema")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>();

    // Generate Rust code
    let mut rust_code = String::new();

    // Add imports and derives
    rust_code.push_str("use serde::{Deserialize, Serialize};\n");
    rust_code.push_str("use schemars::JsonSchema;\n\n");

    // Generate StepKind enum
    rust_code.push_str("#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]\n");
    rust_code.push_str("#[serde(tag = \"type\", rename_all = \"snake_case\")]\n");
    rust_code.push_str("pub enum StepKind {\n");

    for action_type in &action_types {
        let variant_name = to_pascal_case(action_type);
        rust_code.push_str(&format!("    #[serde(rename = \"{}\")]\n", action_type));

        // Add fields based on action type
        match *action_type {
            "navigate_to_url" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        url: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        wait: Option<String>,\n");
                rust_code.push_str("    },\n");
            }
            "open_new_tab" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        url: Option<String>,\n");
                rust_code.push_str("    },\n");
            }
            "switch_to_tab" | "close_current_tab" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        tab_identifier: serde_json::Value,\n");
                rust_code.push_str("    },\n");
            }
            "get_current_url" => {
                rust_code.push_str(&format!("    {},\n", variant_name));
            }
            "click_element" | "dbl_click_element" | "hover_element" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        selector: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        frame_id: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        random_offset: Option<bool>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        timeout_ms: Option<u32>,\n");
                rust_code.push_str("    },\n");
            }
            "fill_input_field" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        selector: String,\n");
                rust_code.push_str("        value: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        frame_id: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        clear_first: Option<bool>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        simulate_typing: Option<bool>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        delay_ms: Option<u32>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        timeout_ms: Option<u32>,\n");
                rust_code.push_str("    },\n");
            }
            "fill_and_submit" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        selector: String,\n");
                rust_code.push_str("        value: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        submit_selector: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        submit_label_regex: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        wait_for_increase_selector: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        frame_id: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        clear_first: Option<bool>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        simulate_typing: Option<bool>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        delay_ms: Option<u32>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        timeout_ms: Option<u32>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        wait_timeout_ms: Option<u32>,\n");
                rust_code.push_str("    },\n");
            }
            "type_text" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        selector: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        text: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        value: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        frame_id: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        use_native_input: Option<bool>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        delay_ms: Option<u32>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        typing_speed: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        timeout_ms: Option<u32>,\n");
                rust_code.push_str("    },\n");
            }
            "submit_input" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        selector: String,\n");
                rust_code.push_str("        text: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        frame_id: Option<String>,\n");
                rust_code.push_str("    },\n");
            }
            "press_special_key" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        key: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        selector: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        frame_id: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        timeout_ms: Option<u32>,\n");
                rust_code.push_str("    },\n");
            }
            "select_option_in_dropdown" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        selector: String,\n");
                rust_code.push_str("        value: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        frame_id: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        timeout_ms: Option<u32>,\n");
                rust_code.push_str("    },\n");
            }
            "upload_file" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        selector: String,\n");
                rust_code.push_str("        file_path: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        frame_id: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        timeout_ms: Option<u32>,\n");
                rust_code.push_str("    },\n");
            }
            "drag_and_drop" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        source_selector: String,\n");
                rust_code.push_str("        target_selector: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        frame_id: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        timeout_ms: Option<u32>,\n");
                rust_code.push_str("    },\n");
            }
            "scroll_window_to" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        x: Option<i32>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        y: Option<i32>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        direction: Option<String>,\n");
                rust_code.push_str("    },\n");
            }
            "scroll_element_into_view" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        selector: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        frame_id: Option<String>,\n");
                rust_code.push_str("    },\n");
            }
            "infinite_scroll" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        item_selector: String,\n");
                rust_code.push_str("        target_count: u32,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        frame_id: Option<String>,\n");
                rust_code.push_str("        #[serde(default = \"default_max_cycles\")]\n");
                rust_code.push_str("        max_cycles: u32,\n");
                rust_code.push_str("    },\n");
            }
            "wait_for_timeout" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        timeout_ms: u32,\n");
                rust_code.push_str("    },\n");
            }
            "wait_for_element" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        selector: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        frame_id: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        condition: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        timeout_ms: Option<u32>,\n");
                rust_code.push_str("    },\n");
            }
            "wait_for_navigation" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        url_pattern: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        timeout_ms: Option<u32>,\n");
                rust_code.push_str("    },\n");
            }
            "wait_for_network_idle" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        #[serde(default = \"default_idle_time\")]\n");
                rust_code.push_str("        idle_time_ms: u32,\n");
                rust_code.push_str("        #[serde(default = \"default_max_wait\")]\n");
                rust_code.push_str("        max_wait_ms: u32,\n");
                rust_code.push_str("    },\n");
            }
            "extract_structured_data" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        item_selector: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        limit: Option<u32>,\n");
                rust_code.push_str("        fields: Vec<FieldSpec>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        frame_id: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        extraction_type: Option<String>,\n");
                rust_code.push_str("    },\n");
            }
            "get_element_text" | "get_element_value" | "get_element_count" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        selector: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        frame_id: Option<String>,\n");
                rust_code.push_str("    },\n");
            }
            "get_element_attribute" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        selector: String,\n");
                rust_code.push_str("        attribute: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        frame_id: Option<String>,\n");
                rust_code.push_str("    },\n");
            }
            "take_screenshot" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        full_page: Option<bool>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        annotate: Option<bool>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        annotate_max_labels: Option<u32>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        annotate_max_elements: Option<u32>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        quality: Option<u8>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        format: Option<String>,\n");
                rust_code.push_str("    },\n");
            }
            "get_page_source" => {
                rust_code.push_str(&format!("    {},\n", variant_name));
            }
            "assert_selector_state" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        selector: String,\n");
                rust_code.push_str("        condition: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        frame_id: Option<String>,\n");
                rust_code.push_str("    },\n");
            }
            "assert_text_in_element" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        selector: String,\n");
                rust_code.push_str("        text: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        frame_id: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        match_type: Option<String>,\n");
                rust_code.push_str("    },\n");
            }
            "assert_url_matches" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        url_pattern: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        match_type: Option<String>,\n");
                rust_code.push_str("    },\n");
            }
            "execute_javascript" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        script: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        args: Option<Vec<serde_json::Value>>,\n");
                rust_code.push_str("        #[serde(default = \"default_return_value\")]\n");
                rust_code.push_str("        return_value: bool,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        world: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        timeout_ms: Option<u32>,\n");
                rust_code.push_str("    },\n");
            }
            "eval_main_world" | "eval_isolated_world" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        script: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        args: Option<Vec<serde_json::Value>>,\n");
                rust_code.push_str("        #[serde(default = \"default_return_value\")]\n");
                rust_code.push_str("        return_value: bool,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        timeout_ms: Option<u32>,\n");
                rust_code.push_str("    },\n");
            }
            "inspect_element" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        selector: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        frame_id: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        include_ancestors: Option<bool>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        include_shadow_path: Option<bool>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        timeout_ms: Option<u32>,\n");
                rust_code.push_str("    },\n");
            }
            "inspect_click_surface" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        selector: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        frame_id: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        timeout_ms: Option<u32>,\n");
                rust_code.push_str("    },\n");
            }
            "capture_ui_bundle" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        selector: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        include_dom_snapshot: Option<bool>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        include_screenshot: Option<bool>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        annotate: Option<bool>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        max_elements: Option<u32>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        timeout_ms: Option<u32>,\n");
                rust_code.push_str("    },\n");
            }
            "verify_ui_change" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        selector: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        condition: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        text: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        match_type: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        value_equals: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        value_contains: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        url_includes: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        url_matches: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        active_selector: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        count_at_least: Option<u32>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        count_equals: Option<u32>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        all: Option<Vec<serde_json::Value>>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        any: Option<Vec<serde_json::Value>>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        timeout_ms: Option<u32>,\n");
                rust_code.push_str("    },\n");
            }
            "read_field_value" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        selector: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        frame_id: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        timeout_ms: Option<u32>,\n");
                rust_code.push_str("    },\n");
            }
            "semantic_action" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        action: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        selector: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        value: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        key: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        step: Option<serde_json::Value>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        postcondition: Option<serde_json::Value>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        postcondition_required: Option<bool>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        timeout_ms: Option<u32>,\n");
                rust_code.push_str("    },\n");
            }
            "set_cookie" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        cookie: CookieSpec,\n");
                rust_code.push_str("    },\n");
            }
            "get_cookies" | "clear_cookies" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        domain: Option<String>,\n");
                rust_code.push_str("    },\n");
            }
            "set_local_storage_item" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        storage_key: String,\n");
                rust_code.push_str("        storage_value: String,\n");
                rust_code.push_str("    },\n");
            }
            "get_local_storage_item" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        storage_key: String,\n");
                rust_code.push_str("    },\n");
            }
            "clear_local_storage" => {
                rust_code.push_str(&format!("    {},\n", variant_name));
            }
            "download_images" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        selector: String,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        download_folder: Option<String>,\n");
                rust_code.push_str("    },\n");
            }
            "request_user_intervention" => {
                rust_code.push_str(&format!("    {} {{\n", variant_name));
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        message: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        instructions: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        timeout_ms: Option<u32>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        approval_mode: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        approval_policy: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        continue_on_timeout: Option<bool>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        notification_title: Option<String>,\n");
                rust_code.push_str("        #[serde(default)]\n");
                rust_code.push_str("        notification_message: Option<String>,\n");
                rust_code.push_str("    },\n");
            }
            _ => {
                // Fallback for any unhandled action types
                rust_code.push_str(&format!("    {},\n", variant_name));
            }
        }
    }

    rust_code.push_str("}\n\n");

    // Add supporting types
    rust_code.push_str("#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]\n");
    rust_code.push_str("pub struct FieldSpec {\n");
    rust_code.push_str("    pub name: String,\n");
    rust_code.push_str("    pub selector: String,\n");
    rust_code.push_str("    #[serde(default)]\n");
    rust_code.push_str("    pub attribute: Option<String>,\n");
    rust_code.push_str("    #[serde(default)]\n");
    rust_code.push_str("    pub post_processing: Vec<String>,\n");
    rust_code.push_str("}\n\n");

    rust_code.push_str("#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]\n");
    rust_code.push_str("pub struct CookieSpec {\n");
    rust_code.push_str("    pub name: String,\n");
    rust_code.push_str("    pub value: String,\n");
    rust_code.push_str("    #[serde(default)]\n");
    rust_code.push_str("    pub domain: Option<String>,\n");
    rust_code.push_str("    #[serde(default)]\n");
    rust_code.push_str("    pub path: Option<String>,\n");
    rust_code.push_str("    #[serde(default)]\n");
    rust_code.push_str("    pub secure: Option<bool>,\n");
    rust_code.push_str("    #[serde(default)]\n");
    rust_code.push_str("    pub http_only: Option<bool>,\n");
    rust_code.push_str("    #[serde(default)]\n");
    rust_code.push_str("    pub expiration_date: Option<f64>,\n");
    rust_code.push_str("}\n\n");

    // Add default functions
    rust_code.push_str("fn default_max_cycles() -> u32 { 30 }\n");
    rust_code.push_str("fn default_idle_time() -> u32 { 500 }\n");
    rust_code.push_str("fn default_max_wait() -> u32 { 30000 }\n");
    rust_code.push_str("fn default_return_value() -> bool { true }\n");

    // Write the generated code
    fs::write(&dest_path, rust_code).expect("Failed to write generated step.rs");

    println!("Generated step.rs with {} action types", action_types.len());
}

fn to_pascal_case(snake_case: &str) -> String {
    snake_case
        .split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
            }
        })
        .collect()
}
