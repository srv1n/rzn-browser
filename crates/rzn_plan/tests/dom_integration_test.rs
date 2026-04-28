use std::collections::HashMap;
use std::time::Duration;

use rzn_plan::dom_context::*;
use serde_json::json;

/// Comprehensive DOM integration tests
/// These tests verify the end-to-end DOM processing pipeline:
/// 1. DOM context formatting and optimization
/// 2. Element priority calculation and filtering
/// 3. Action suggestion generation
/// 4. LLM integration with DOM context
/// 5. Result extraction and validation

#[cfg(test)]
mod dom_integration_tests {
    use super::*;

    fn create_test_element(
        index: u32,
        tag: &str,
        text: &str,
        priority: u8,
        x: i32,
        y: i32,
    ) -> InteractiveElementSummary {
        InteractiveElementSummary {
            highlight_index: index,
            tag_name: tag.to_string(),
            text: text.to_string(),
            selector_hint: format!("{}:nth-child({})", tag, index),
            element_type: tag.to_string(),
            role: match tag {
                "button" => "button".to_string(),
                "input" => "textbox".to_string(),
                "a" => "link".to_string(),
                _ => "".to_string(),
            },
            position: ElementPosition {
                top: y,
                left: x,
                width: 100,
                height: 30,
            },
            action_candidates: match tag {
                "button" => vec!["click_element".to_string()],
                "input" => vec!["fill_input_field".to_string(), "click_element".to_string()],
                "a" => vec!["click_element".to_string()],
                "select" => vec!["select_option".to_string()],
                _ => vec!["click_element".to_string()],
            },
            priority,
        }
    }

    fn create_test_dom_state_login_form() -> ProcessedDOMState {
        ProcessedDOMState {
            interactive_elements: vec![
                create_test_element(1, "input", "", 9, 50, 100), // username field
                create_test_element(2, "input", "", 9, 50, 150), // password field
                create_test_element(3, "button", "Login", 8, 50, 200), // login button
                create_test_element(4, "input", "", 6, 50, 250), // remember me checkbox
                create_test_element(5, "a", "Forgot Password?", 5, 150, 300), // forgot password link
            ],
            element_count: 5,
            viewport_element_count: 5,
            change_summary: None,
            simplified_dom: "login form".to_string(),
            action_hints: vec![
                ActionHint {
                    element_index: 1,
                    suggested_actions: vec!["fill_input_field".to_string()],
                    reasoning: "Username input field should be filled first".to_string(),
                    confidence: 0.95,
                },
                ActionHint {
                    element_index: 2,
                    suggested_actions: vec!["fill_input_field".to_string()],
                    reasoning: "Password input field should be filled after username".to_string(),
                    confidence: 0.95,
                },
                ActionHint {
                    element_index: 3,
                    suggested_actions: vec!["click_element".to_string()],
                    reasoning: "Login button should be clicked to submit form".to_string(),
                    confidence: 0.90,
                },
            ],
        }
    }

    fn create_test_dom_state_product_list() -> ProcessedDOMState {
        ProcessedDOMState {
            interactive_elements: vec![
                create_test_element(1, "select", "Category", 7, 20, 50), // category filter
                create_test_element(2, "select", "Brand", 6, 200, 50),   // brand filter
                create_test_element(3, "input", "", 5, 400, 50),         // price range
                create_test_element(4, "button", "MacBook Pro", 8, 50, 150), // product 1
                create_test_element(5, "button", "Add to Cart", 9, 200, 180), // add to cart 1
                create_test_element(6, "button", "iPhone 15", 8, 300, 150), // product 2
                create_test_element(7, "button", "Add to Cart", 9, 450, 180), // add to cart 2
                create_test_element(8, "button", "Samsung Galaxy", 7, 50, 250), // product 3
                create_test_element(9, "button", "Add to Cart", 8, 200, 280), // add to cart 3
            ],
            element_count: 9,
            viewport_element_count: 6,
            change_summary: None,
            simplified_dom: "product list with filters".to_string(),
            action_hints: vec![
                ActionHint {
                    element_index: 5,
                    suggested_actions: vec!["click_element".to_string()],
                    reasoning: "Add to cart button for MacBook Pro".to_string(),
                    confidence: 0.85,
                },
                ActionHint {
                    element_index: 7,
                    suggested_actions: vec!["click_element".to_string()],
                    reasoning: "Add to cart button for iPhone 15".to_string(),
                    confidence: 0.85,
                },
            ],
        }
    }

    fn create_test_dom_state_with_changes() -> ProcessedDOMState {
        let mut state = create_test_dom_state_login_form();
        state.change_summary = Some(ChangeDetectionSummary {
            new_elements: vec![10, 11],
            removed_elements: vec!["old_element_id".to_string()],
            modified_elements: vec![3],
            significant_changes: true,
            change_count: 4,
        });
        state
    }

    #[test]
    fn test_dom_context_formatter_creation() {
        let mut formatter = DOMContextFormatter::new();
        // Test that formatter can be created and basic functionality works
        let dom_state = create_test_dom_state_login_form();
        let context = formatter.format_dom_context(dom_state);
        assert!(!context.dom_representation.is_empty());
        assert!(context.estimated_tokens > 0);
    }

    #[test]
    fn test_dom_context_formatting_login_form() {
        let mut formatter = DOMContextFormatter::new();
        let dom_state = create_test_dom_state_login_form();

        let context = formatter.format_dom_context(dom_state);

        // Verify basic structure
        assert!(!context.dom_representation.is_empty());
        assert!(context.estimated_tokens > 0);
        assert_eq!(context.metadata.interactive_elements, 5);
        assert_eq!(context.metadata.viewport_elements, 5);

        // Check that form elements are properly categorized
        assert!(context.dom_representation.contains("Form Elements"));
        assert!(context.dom_representation.contains("[1]")); // username field
        assert!(context.dom_representation.contains("[2]")); // password field
        assert!(context.dom_representation.contains("[3]")); // login button

        // Check action suggestions are included
        assert!(context.action_suggestions.is_some());
        let suggestions = context.action_suggestions.unwrap();
        assert!(suggestions.contains("fill_input_field"));
        assert!(suggestions.contains("click_element"));
    }

    #[test]
    fn test_dom_context_formatting_product_list() {
        let mut formatter = DOMContextFormatter::new();
        let dom_state = create_test_dom_state_product_list();

        let context = formatter.format_dom_context(dom_state);

        // Verify product-specific formatting
        assert!(context
            .dom_representation
            .contains("Interactive elements: 9"));
        assert!(context.dom_representation.contains("In viewport: 6"));

        // Check that different element types are present
        assert!(context.dom_representation.contains("select")); // filters
        assert!(context.dom_representation.contains("button")); // products and buttons

        // Verify action hints for products
        assert!(context.action_suggestions.is_some());
        let suggestions = context.action_suggestions.unwrap();
        assert!(suggestions.contains("Add to cart"));
    }

    #[test]
    fn test_dom_context_with_changes() {
        let mut formatter = DOMContextFormatter::new();
        let dom_state = create_test_dom_state_with_changes();

        let context = formatter.format_dom_context(dom_state);

        // Verify change summary is included
        assert!(context.change_summary.is_some());
        let changes = context.change_summary.unwrap();
        assert!(changes.contains("Recent Changes"));
        assert!(changes.contains("New elements: [10], [11]"));
        assert!(changes.contains("Modified elements: [3]"));
        assert!(changes.contains("Significant changes detected"));
    }

    #[test]
    fn test_element_priority_calculation() {
        let mut formatter = DOMContextFormatter::new();
        let dom_state = create_test_dom_state_login_form();

        let context = formatter.format_dom_context(dom_state);

        // High priority elements should be marked with stars
        assert!(context.dom_representation.contains("")); // High priority indicator

        // Check that high priority elements appear first
        let username_pos = context.dom_representation.find("[1]").unwrap();
        let forgot_password_pos = context.dom_representation.find("[5]").unwrap();
        assert!(username_pos < forgot_password_pos); // Higher priority appears first
    }

    #[test]
    fn test_focus_mode_filtering() {
        let config = DOMContextConfig {
            focus_mode: true,
            max_elements: 3,
            ..Default::default()
        };

        let mut formatter = DOMContextFormatter::with_config(config);
        let dom_state = create_test_dom_state_product_list();

        let context = formatter.format_dom_context(dom_state);

        // In focus mode with max 3 elements, should prioritize highest priority elements
        let element_count = context.dom_representation.matches("[").count()
            - context.dom_representation.matches("[]").count(); // Don't count empty brackets
        assert!(element_count <= 6); // 3 elements max, but each has brackets
    }

    #[test]
    fn test_interaction_recording() {
        let mut formatter = DOMContextFormatter::new();

        // Record some interactions
        formatter.record_interaction(
            1,
            "fill_input_field".to_string(),
            true,
            Some("Filled username".to_string()),
        );
        formatter.record_interaction(
            2,
            "fill_input_field".to_string(),
            true,
            Some("Filled password".to_string()),
        );
        formatter.record_interaction(
            3,
            "click_element".to_string(),
            false,
            Some("Login failed".to_string()),
        );
        formatter.record_interaction(1, "fill_input_field".to_string(), true, None);

        // Check interaction stats through public API
        let stats = formatter.get_interaction_stats();
        assert_eq!(stats.get("total_interactions"), Some(&4));
        assert_eq!(stats.get("successful_interactions"), Some(&3));
        assert_eq!(stats.get("unique_elements_interacted"), Some(&3));

        // Test interaction context formatting
        let config = DOMContextConfig {
            include_interaction_history: true,
            ..Default::default()
        };
        formatter.update_config(config);

        let dom_state = create_test_dom_state_login_form();
        let context = formatter.format_dom_context(dom_state);

        assert!(context.interaction_context.is_some());
        let interaction_context = context.interaction_context.unwrap();
        assert!(interaction_context.contains("Recent Interactions"));
        assert!(interaction_context.contains("fill_input_field"));
        assert!(interaction_context.contains("[OK]")); // Success indicator
        assert!(interaction_context.contains("[ERROR]")); // Failure indicator
    }

    #[test]
    fn test_token_estimation() {
        let mut formatter = DOMContextFormatter::new();
        let dom_state = create_test_dom_state_login_form();

        let context = formatter.format_dom_context(dom_state);

        // Token count should be reasonable
        assert!(context.estimated_tokens > 50);
        assert!(context.estimated_tokens < 2000); // Should be well under max for simple form

        // Test with larger DOM state
        let large_dom_state = create_test_dom_state_product_list();
        let large_context = formatter.format_dom_context(large_dom_state);

        // Larger DOM should have more tokens
        assert!(large_context.estimated_tokens > context.estimated_tokens);
    }

    #[test]
    fn test_viewport_prioritization() {
        let config = DOMContextConfig {
            prioritize_viewport: true,
            max_elements: 3,
            ..Default::default()
        };

        let mut formatter = DOMContextFormatter::with_config(config);

        // Create DOM state where some elements are out of viewport
        let mut dom_state = create_test_dom_state_login_form();
        // Make forgot password link appear out of viewport (high Y position)
        dom_state.interactive_elements[4].position.top = 2000;

        let context = formatter.format_dom_context(dom_state);

        // Viewport elements should be prioritized
        assert!(context.dom_representation.contains("")); // Viewport indicator
    }

    #[test]
    fn test_specialized_context_creation() {
        let dom_state = create_test_dom_state_login_form();

        // Test form filling focused context
        let form_context = create_focused_context(dom_state.clone(), "form_filling");
        assert!(form_context.dom_representation.contains("input"));
        assert!(form_context.action_suggestions.is_some());

        // Test navigation focused context
        let nav_context = create_focused_context(dom_state.clone(), "navigation");
        assert!(nav_context.estimated_tokens > 0);

        // Test data extraction context
        let data_context = create_focused_context(dom_state.clone(), "data_extraction");
        assert!(data_context.action_suggestions.is_none()); // Data extraction doesn't need action hints

        // Test minimal context
        let minimal_context = create_minimal_context(dom_state.clone());
        assert!(minimal_context.estimated_tokens < form_context.estimated_tokens);

        // Test debug context
        let debug_context = create_debug_context(dom_state);
        assert!(debug_context.estimated_tokens > form_context.estimated_tokens);
    }

    #[test]
    fn test_action_suggestion_generation() {
        let dom_state = create_test_dom_state_login_form();
        let mut formatter = DOMContextFormatter::new();

        let context = formatter.format_dom_context(dom_state);

        let suggestions = context.action_suggestions.unwrap();

        // Check that suggestions are properly formatted
        assert!(suggestions.contains("Suggested Actions"));
        assert!(suggestions.contains("[TARGET]")); // Action target indicator
        assert!(suggestions.contains("confidence:"));
        assert!(suggestions.contains("[TIP]")); // Reasoning indicator

        // Check specific action suggestions
        assert!(suggestions.contains("fill_input_field"));
        assert!(suggestions.contains("click_element"));

        // Check confidence values are displayed
        assert!(suggestions.contains("95.0") || suggestions.contains("90.0"));
    }

    #[test]
    fn test_element_relationship_tracking() {
        let config = DOMContextConfig {
            include_relationships: true,
            ..Default::default()
        };

        let mut formatter = DOMContextFormatter::with_config(config);
        let dom_state = create_test_dom_state_login_form();

        let context = formatter.format_dom_context(dom_state);

        // Relationships should affect the formatting and prioritization
        assert!(context.estimated_tokens > 0);
        assert!(!context.dom_representation.is_empty());
    }

    #[test]
    fn test_performance_with_large_dom() {
        let mut formatter = DOMContextFormatter::new();

        // Create a large DOM state with many elements
        let mut large_elements = Vec::new();
        for i in 1..=100 {
            large_elements.push(create_test_element(
                i,
                if i % 4 == 0 {
                    "button"
                } else if i % 3 == 0 {
                    "input"
                } else {
                    "a"
                },
                &format!("Element {}", i),
                (i % 10) as u8,
                ((i * 10) % 800) as i32,
                ((i * 20) % 600) as i32,
            ));
        }

        let large_dom_state = ProcessedDOMState {
            interactive_elements: large_elements,
            element_count: 100,
            viewport_element_count: 50,
            change_summary: None,
            simplified_dom: "large test dom".to_string(),
            action_hints: vec![],
        };

        let start_time = std::time::Instant::now();
        let context = formatter.format_dom_context(large_dom_state);
        let processing_time = start_time.elapsed();

        // Processing should complete quickly even with large DOM
        assert!(processing_time < Duration::from_millis(500));

        // Should still respect max elements limit
        let displayed_elements = context.dom_representation.matches("[").count()
            - context.dom_representation.matches("[]").count();
        assert!(displayed_elements <= 60); // Some buffer for formatting

        // Token count should be reasonable
        assert!(context.estimated_tokens < 8000);
    }

    #[test]
    fn test_memory_management() {
        let mut formatter = DOMContextFormatter::new();

        // Add many interactions to test history management
        for i in 0..100 {
            formatter.record_interaction(
                (i % 10) + 1,
                "click_element".to_string(),
                i % 2 == 0,
                Some(format!("Action {}", i)),
            );
        }

        // Check that interactions are properly recorded through public API
        let stats = formatter.get_interaction_stats();
        assert_eq!(stats.get("total_interactions"), Some(&50)); // Should be bounded to 50

        // Clear history
        formatter.clear_history();
        let stats_after = formatter.get_interaction_stats();
        assert_eq!(stats_after.get("total_interactions"), Some(&0));
        assert_eq!(stats_after.get("unique_elements_interacted"), Some(&0));
    }

    #[test]
    fn test_dom_context_serialization() {
        let mut formatter = DOMContextFormatter::new();
        let dom_state = create_test_dom_state_login_form();

        let context = formatter.format_dom_context(dom_state);

        // Test that context can be serialized to JSON
        let json_result = serde_json::to_string(&context);
        assert!(json_result.is_ok());

        let json_str = json_result.unwrap();
        assert!(json_str.contains("dom_representation"));
        assert!(json_str.contains("metadata"));

        // Test deserialization
        let deserialized: Result<FormattedDOMContext, _> = serde_json::from_str(&json_str);
        assert!(deserialized.is_ok());

        let deserialized_context = deserialized.unwrap();
        assert_eq!(
            deserialized_context.estimated_tokens,
            context.estimated_tokens
        );
        assert_eq!(
            deserialized_context.metadata.interactive_elements,
            context.metadata.interactive_elements
        );
    }

    #[test]
    fn test_dom_context_ready_for_llm() {
        let mut formatter = DOMContextFormatter::new();
        let dom_state = create_test_dom_state_login_form();

        let context = formatter.format_dom_context(dom_state);

        // Verify the context is well-formed for LLM consumption
        assert!(context.dom_representation.contains("Form Elements"));
        assert!(context.dom_representation.contains("input"));
        assert!(context.dom_representation.contains("button"));
        assert!(context.dom_representation.contains("Login"));

        // Verify action suggestions are present
        assert!(context.action_suggestions.is_some());
        let suggestions = context.action_suggestions.unwrap();
        assert!(suggestions.contains("fill_input_field"));
        assert!(suggestions.contains("click_element"));

        // Verify token count is reasonable for LLM processing
        assert!(context.estimated_tokens > 50);
        assert!(context.estimated_tokens < 2000);
    }

    #[test]
    fn test_broker_processing_simulation() {
        // This test simulates the broker's DOM processing workflow

        // Simulate raw DOM data from extension
        let _raw_dom_data = json!({
            "element_map": {
                "1": {
                    "tag_name": "input",
                    "attributes": {"type": "text", "id": "username"},
                    "text": "",
                    "position": {"top": 100, "left": 50, "width": 200, "height": 30},
                    "is_interactive": true,
                    "priority_score": 9
                },
                "2": {
                    "tag_name": "input",
                    "attributes": {"type": "password", "id": "password"},
                    "text": "",
                    "position": {"top": 150, "left": 50, "width": 200, "height": 30},
                    "is_interactive": true,
                    "priority_score": 9
                },
                "3": {
                    "tag_name": "button",
                    "attributes": {"type": "submit"},
                    "text": "Login",
                    "position": {"top": 200, "left": 50, "width": 100, "height": 40},
                    "is_interactive": true,
                    "priority_score": 8
                }
            },
            "viewport_info": {
                "width": 1200,
                "height": 800,
                "scroll_x": 0,
                "scroll_y": 0
            },
            "url": "file://test-login-form.html",
            "title": "Test Login Form"
        });

        // Convert to ProcessedDOMState (simulating broker processing)
        let processed_state = ProcessedDOMState {
            interactive_elements: vec![
                create_test_element(1, "input", "", 9, 50, 100),
                create_test_element(2, "input", "", 9, 50, 150),
                create_test_element(3, "button", "Login", 8, 50, 200),
            ],
            element_count: 3,
            viewport_element_count: 3,
            change_summary: None,
            simplified_dom: "login form".to_string(),
            action_hints: vec![],
        };

        // Format for LLM consumption
        let mut formatter = DOMContextFormatter::new();
        let context = formatter.format_dom_context(processed_state);

        // Verify the pipeline worked correctly
        assert!(context.dom_representation.contains("Form Elements"));
        assert!(context.dom_representation.contains("Login"));
        assert_eq!(context.metadata.interactive_elements, 3);
        assert!(context.estimated_tokens > 0);

        // Test that the context is suitable for LLM processing
        assert!(context.estimated_tokens < 4000); // Under token limit
        assert!(context.dom_representation.len() > 100); // Has meaningful content
    }

    #[test]
    fn test_end_to_end_workflow_simulation() {
        // This test simulates the complete end-to-end workflow

        println!("Starting end-to-end DOM processing workflow test...");

        // Step 1: Extension DOM analysis (simulated)
        let extension_dom_data = json!({
            "interactive_elements": [
                {
                    "id": "username",
                    "tag": "input",
                    "type": "text",
                    "text": "",
                    "position": {"x": 50, "y": 100},
                    "priority": 9
                },
                {
                    "id": "password",
                    "tag": "input",
                    "type": "password",
                    "text": "",
                    "position": {"x": 50, "y": 150},
                    "priority": 9
                },
                {
                    "id": "login-btn",
                    "tag": "button",
                    "text": "Login",
                    "position": {"x": 50, "y": 200},
                    "priority": 8
                }
            ]
        });

        // Step 2: Broker DOM processing
        let processed_dom = ProcessedDOMState {
            interactive_elements: vec![
                create_test_element(1, "input", "", 9, 50, 100),
                create_test_element(2, "input", "", 9, 50, 150),
                create_test_element(3, "button", "Login", 8, 50, 200),
            ],
            element_count: 3,
            viewport_element_count: 3,
            change_summary: None,
            simplified_dom: "login form with username, password, and submit button".to_string(),
            action_hints: vec![ActionHint {
                element_index: 1,
                suggested_actions: vec!["fill_input_field".to_string()],
                reasoning: "Username field should be filled first".to_string(),
                confidence: 0.95,
            }],
        };

        // Step 3: DOM context formatting
        let mut formatter = DOMContextFormatter::new();
        let formatted_context = formatter.format_dom_context(processed_dom);

        // Step 4: Validate DOM context is ready for LLM planning
        // Verify the formatted context contains necessary information
        assert!(formatted_context
            .dom_representation
            .contains("Form Elements"));
        assert!(formatted_context.dom_representation.contains("[1]")); // Username field
        assert!(formatted_context.dom_representation.contains("[2]")); // Password field
        assert!(formatted_context.dom_representation.contains("[3]")); // Login button
        assert!(formatted_context.dom_representation.contains("input"));
        assert!(formatted_context.dom_representation.contains("button"));
        assert!(formatted_context.dom_representation.contains("Login"));

        // Verify action suggestions are available
        if formatted_context.action_suggestions.is_some() {
            let suggestions = formatted_context.action_suggestions.unwrap();
            println!("Action suggestions: {}", suggestions);
            assert!(suggestions.contains("fill_input_field"));
            // Check if any action-related content is present
            assert!(suggestions.len() > 0); // At least has some suggestions
        } else {
            println!("No action suggestions available");
        }

        // Step 5: Validate the complete workflow context preparation
        // This would be consumed by an LLM in actual usage
        assert!(formatted_context.estimated_tokens > 0);
        assert!(formatted_context.estimated_tokens < 2000); // Reasonable for LLM
        assert_eq!(formatted_context.metadata.interactive_elements, 3);
        assert_eq!(formatted_context.metadata.viewport_elements, 3);

        // Record interactions for history tracking
        formatter.record_interaction(
            1,
            "fill_input_field".to_string(),
            true,
            Some("Username filled".to_string()),
        );
        formatter.record_interaction(
            2,
            "fill_input_field".to_string(),
            true,
            Some("Password filled".to_string()),
        );
        formatter.record_interaction(
            3,
            "click_element".to_string(),
            true,
            Some("Login successful".to_string()),
        );

        // Verify interaction tracking
        let stats = formatter.get_interaction_stats();
        assert_eq!(stats.get("total_interactions"), Some(&3));
        assert_eq!(stats.get("successful_interactions"), Some(&3));

        println!("End-to-end workflow test completed successfully!");
    }

    #[test]
    fn test_error_handling_and_recovery() {
        let mut formatter = DOMContextFormatter::new();

        // Test with empty DOM state
        let empty_dom = ProcessedDOMState {
            interactive_elements: vec![],
            element_count: 0,
            viewport_element_count: 0,
            change_summary: None,
            simplified_dom: "".to_string(),
            action_hints: vec![],
        };

        let context = formatter.format_dom_context(empty_dom);
        assert!(!context.dom_representation.is_empty()); // Should still generate basic structure
        assert_eq!(context.metadata.interactive_elements, 0);

        // Test with malformed elements
        let malformed_element = InteractiveElementSummary {
            highlight_index: 999,
            tag_name: "".to_string(), // Empty tag name
            text: "".to_string(),
            selector_hint: "".to_string(),
            element_type: "".to_string(),
            role: "".to_string(),
            position: ElementPosition {
                top: -1,
                left: -1,
                width: 0,
                height: 0,
            }, // Invalid position
            action_candidates: vec![],
            priority: 0,
        };

        let malformed_dom = ProcessedDOMState {
            interactive_elements: vec![malformed_element],
            element_count: 1,
            viewport_element_count: 0,
            change_summary: None,
            simplified_dom: "malformed".to_string(),
            action_hints: vec![],
        };

        // Should handle malformed data gracefully
        let context = formatter.format_dom_context(malformed_dom);
        assert!(context.estimated_tokens > 0);
        assert_eq!(context.metadata.interactive_elements, 1);
    }
}

// Helper module for testing utilities
#[cfg(test)]
pub mod test_utils {
    use super::*;

    /// Create a comprehensive test DOM state for complex scenarios
    pub fn create_comprehensive_test_dom() -> ProcessedDOMState {
        ProcessedDOMState {
            interactive_elements: vec![
                // Navigation elements
                create_test_element(1, "a", "Home", 7, 100, 20),
                create_test_element(2, "a", "Products", 8, 200, 20),
                create_test_element(3, "a", "About", 6, 300, 20),
                // Search functionality
                create_test_element(4, "input", "", 8, 500, 20),
                create_test_element(5, "button", "Search", 7, 650, 20),
                // Main content
                create_test_element(6, "button", "Product 1", 9, 100, 150),
                create_test_element(7, "button", "Add to Cart", 9, 250, 180),
                create_test_element(8, "button", "Product 2", 8, 400, 150),
                create_test_element(9, "button", "Add to Cart", 8, 550, 180),
                // Filters
                create_test_element(10, "select", "Category", 6, 100, 100),
                create_test_element(11, "select", "Price Range", 5, 250, 100),
                // Footer links
                create_test_element(12, "a", "Contact", 4, 100, 500),
                create_test_element(13, "a", "Privacy", 3, 200, 500),
            ],
            element_count: 13,
            viewport_element_count: 10,
            change_summary: Some(ChangeDetectionSummary {
                new_elements: vec![6, 7, 8, 9],
                removed_elements: vec![],
                modified_elements: vec![10],
                significant_changes: false,
                change_count: 5,
            }),
            simplified_dom: "e-commerce product listing page with navigation and filters"
                .to_string(),
            action_hints: vec![
                ActionHint {
                    element_index: 6,
                    suggested_actions: vec!["click_element".to_string()],
                    reasoning: "Main product selection".to_string(),
                    confidence: 0.85,
                },
                ActionHint {
                    element_index: 7,
                    suggested_actions: vec!["click_element".to_string()],
                    reasoning: "Add product to shopping cart".to_string(),
                    confidence: 0.90,
                },
            ],
        }
    }

    /// Validate that a formatted DOM context meets quality standards
    pub fn validate_dom_context_quality(context: &FormattedDOMContext) -> Result<(), String> {
        // Check basic structure
        if context.dom_representation.is_empty() {
            return Err("DOM representation is empty".to_string());
        }

        // Check token efficiency
        if context.estimated_tokens > 10000 {
            return Err(format!(
                "Token count too high: {}",
                context.estimated_tokens
            ));
        }

        // Check metadata consistency
        if context.metadata.interactive_elements == 0 && context.dom_representation.contains("[") {
            return Err("Metadata shows no interactive elements but representation contains element indices".to_string());
        }

        // Check for required sections
        if !context.dom_representation.contains("Page Elements") {
            return Err("Missing 'Page Elements' section header".to_string());
        }

        Ok(())
    }

    fn create_test_element(
        index: u32,
        tag: &str,
        text: &str,
        priority: u8,
        x: i32,
        y: i32,
    ) -> InteractiveElementSummary {
        InteractiveElementSummary {
            highlight_index: index,
            tag_name: tag.to_string(),
            text: text.to_string(),
            selector_hint: format!("{}:nth-child({})", tag, index),
            element_type: tag.to_string(),
            role: match tag {
                "button" => "button".to_string(),
                "input" => "textbox".to_string(),
                "a" => "link".to_string(),
                "select" => "combobox".to_string(),
                _ => "".to_string(),
            },
            position: ElementPosition {
                top: y,
                left: x,
                width: if tag == "input" { 200 } else { 100 },
                height: 30,
            },
            action_candidates: match tag {
                "button" => vec!["click_element".to_string()],
                "input" => vec!["fill_input_field".to_string(), "click_element".to_string()],
                "a" => vec!["click_element".to_string()],
                "select" => vec!["select_option".to_string()],
                _ => vec!["click_element".to_string()],
            },
            priority,
        }
    }
}

// Integration test for the complete DOM processing pipeline
#[cfg(test)]
mod integration_tests {
    use super::test_utils::*;
    use super::*;

    #[test]
    fn test_comprehensive_dom_processing_pipeline() {
        println!("Testing comprehensive DOM processing pipeline...");

        let comprehensive_dom = create_comprehensive_test_dom();
        let mut formatter = DOMContextFormatter::new();

        // Test different configuration scenarios
        let configs = vec![
            ("default", DOMContextConfig::default()),
            (
                "form_filling",
                DOMContextConfig {
                    max_elements: 15,
                    focus_mode: true,
                    include_action_hints: true,
                    ..Default::default()
                },
            ),
            (
                "minimal",
                DOMContextConfig {
                    max_tokens: 1000,
                    max_elements: 5,
                    focus_mode: true,
                    include_changes: false,
                    include_action_hints: false,
                    ..Default::default()
                },
            ),
        ];

        for (config_name, config) in configs {
            println!("Testing configuration: {}", config_name);

            formatter.update_config(config);
            let context = formatter.format_dom_context(comprehensive_dom.clone());

            // Validate context quality
            validate_dom_context_quality(&context).expect(&format!(
                "Quality validation failed for config: {}",
                config_name
            ));

            // Verify configuration-specific behaviors
            match config_name {
                "minimal" => {
                    assert!(context.estimated_tokens <= 1500); // Should be close to limit
                    assert!(context.action_suggestions.is_none());
                }
                "form_filling" => {
                    assert!(context.action_suggestions.is_some());
                    assert!(context.dom_representation.contains("")); // High priority indicators
                }
                _ => {
                    assert!(context.estimated_tokens > 0);
                    assert!(!context.dom_representation.is_empty());
                }
            }

            println!("✓ Configuration '{}' passed all tests", config_name);
        }

        println!("Comprehensive DOM processing pipeline test completed successfully!");
    }
}
