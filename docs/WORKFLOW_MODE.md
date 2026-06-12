# Workflow Mode - JSON Automation (Legacy)

**Note**: RZN now has **FSM-driven autonomous mode** (`llm-auto`) as the primary automation method. JSON workflows are still supported for deterministic, repeatable tasks.

## Autonomous vs Workflow Modes

### FSM-Driven Autonomous (Recommended)
```bash
# LLM-driven automation with natural language
./target/release/rzn-browser llm-auto "Search Google for OpenAI and extract results"
```
- **Intelligent**: Adapts to changes in page structure  
- **Policy-enforced**: Blocks dangerous patterns like URL construction
- **Self-healing**: Recovers from failures automatically
- **FSM-controlled**: Prevents state drift and loops

### JSON Workflows (Legacy)
```bash  
# Deterministic, scripted automation
./target/release/rzn-browser run workflows/google-search.json
```
- **Deterministic**: Same actions every time
- **Fast**: No LLM calls, direct execution
- **Precise**: Exact selectors and timing
- **Brittle**: Breaks when pages change

## Quick Workflow Example

```json
{
  "version": "1.0",
  "metadata": {
    "name": "Search and Extract",
    "description": "Search Google and get results"
  },
  "browser_automation": {
    "sequences": [{
      "steps": [
        {"type": "navigate_to_url", "url": "https://google.com"},
        {"type": "fill_input_field", "selector": "input[name='q']", "value": "{query}"},
        {"type": "press_special_key", "key": "Enter"},
        {"type": "wait_for_element", "selector": "#search"},
        {"type": "extract_structured_data",
          "item_selector": ".g",
          "fields": [
            {"name": "title", "selector": "h3"},
            {"name": "url", "selector": "a", "attribute": "href"}
          ]
        }
      ]
    }]
  }
}
```

Run it:
```bash
rzn-browser run search.json --param query="RZN automation"
```

## Action Reference

### Navigation

```json
// Load a URL
{"type": "navigate_to_url", "url": "https://example.com"}

// Go back/forward
{"type": "go_back"}
{"type": "go_forward"}

// Refresh
{"type": "refresh_page"}
```

### Clicking

```json
// Basic click
{"type": "click_element", "selector": "#button"}

// With fallbacks
{
  "type": "click_element",
  "selector": "#submit",
  "fallback_selectors": [
    "button[type='submit']",
    "//button[contains(text(), 'Submit')]"
  ]
}

// Force click even if covered
{"type": "click_element", "selector": "#btn", "force": true}
```

### Typing

```json
// Fill input field
{"type": "fill_input_field", "selector": "input[name='email']", "value": "test@example.com"}

// Clear then type
{"type": "fill_input_field", "selector": "#search", "value": "new search", "clear_first": true}

// Type slowly (human-like)
{"type": "fill_input_field", "selector": "#username", "value": "john", "typing_delay": 100}

// Password (won't log value)
{"type": "fill_input_field", "selector": "#password", "value": "{password}", "sensitive": true}
```

### Keyboard

```json
// Special keys
{"type": "press_special_key", "key": "Enter"}
{"type": "press_special_key", "key": "Tab"}
{"type": "press_special_key", "key": "Escape"}

// Key combinations
{"type": "press_key_combination", "keys": ["Control", "a"]}
{"type": "press_key_combination", "keys": ["Command", "v"]}  // Mac
```

### Dropdowns

```json
// Select by value
{"type": "select_option", "selector": "select#country", "value": "US"}

// Select by text
{"type": "select_option", "selector": "#size", "option_text": "Large"}

// Select by index
{"type": "select_option", "selector": "#quantity", "index": 2}
```

### Waiting

```json
// Fixed wait
{"type": "wait", "duration": 2000}

// Wait for element
{"type": "wait_for_element", "selector": ".results", "timeout": 10000}

// Wait for element to disappear
{"type": "wait_for_element_removal", "selector": ".loading", "timeout": 5000}

// Wait for text
{"type": "wait_for_text", "text": "Success", "timeout": 5000}

// Wait for navigation
{"type": "wait_for_navigation", "timeout": 10000}

// Wait for network idle
{"type": "wait_for_network_idle", "idle_time": 500}

// Custom condition
{"type": "wait_for_condition", 
  "condition": "document.querySelectorAll('.item').length >= 10",
  "timeout": 10000
}
```

### Data Extraction

```json
// Get text
{"type": "extract_text", "selector": ".price", "variable_name": "price"}

// Get attribute
{"type": "get_element_attribute", "selector": "img", "attribute": "src", "variable_name": "image_url"}

// Extract structured data
{
  "type": "extract_structured_data",
  "item_selector": ".product-card",
  "fields": [
    {"name": "title", "selector": "h3"},
    {"name": "price", "selector": ".price", "transform": "parse_number"},
    {"name": "image", "selector": "img", "attribute": "src"},
    {"name": "in_stock", "selector": ".stock", "transform": "contains:In Stock"}
  ],
  "variable_name": "products",
  "max_items": 50
}

// Get page source
{"type": "get_page_source", "variable_name": "html"}

// Get DOM snapshot (for LLM)
{"type": "get_dom_snapshot", "options": {"maxElements": 200}}
```

### Screenshots

```json
// Full page
{"type": "take_screenshot", "filename": "page.png"}

// Specific element
{"type": "take_screenshot", "selector": "#chart", "filename": "chart.png"}

// With timestamp
{"type": "take_screenshot", "filename": "screenshot_{timestamp}.png"}
```

### Scrolling

```json
// Scroll to element
{"type": "scroll_element", "selector": "#footer"}

// Scroll by amount
{"type": "scroll", "x": 0, "y": 500}

// Infinite scroll
{"type": "scroll_until",
  "condition": "document.querySelectorAll('.item').length >= 100",
  "max_scrolls": 50,
  "scroll_delay": 1000
}
```

## Selector Strategies

### CSS Selectors
```javascript
"#login-button"              // ID
".submit-btn"                // Class
"button[type='submit']"      // Attribute
"form > button:first-child"  // Hierarchy
"a:contains('Next')"         // Text content (jQuery-style)
```

### XPath
```javascript
"//button[text()='Submit']"                    // Exact text
"//button[contains(text(), 'Submit')]"         // Partial text
"//input[@name='email']/following-sibling::*"  // Relative
"(//div[@class='result'])[3]"                  // Position
```

### Special Selectors
```javascript
"iframe#payment >>> button"     // Inside iframe
"custom-element >>> input"      // Inside shadow DOM
":visible"                       // Only visible elements
":not([disabled])"              // Not disabled
```

## Variables & Control Flow

### Using Variables

```json
// Define variables
{
  "variables": {
    "base_url": "https://example.com",
    "max_price": 100
  }
}

// Use in any string field
{"type": "navigate_to_url", "url": "{base_url}/products"}
{"type": "fill_input_field", "selector": "#max", "value": "{max_price}"}

// Store extracted data
{"type": "extract_text", "selector": ".total", "variable_name": "total_price"}

// Use extracted data
{"type": "log", "message": "Total is {total_price}"}
```

### Conditionals

```json
// If-then-else
{
  "type": "conditional",
  "condition": "document.querySelector('.error')",
  "then": [
    {"type": "log", "message": "Error found"},
    {"type": "take_screenshot", "filename": "error.png"}
  ],
  "else": [
    {"type": "click_element", "selector": "#continue"}
  ]
}

// Check variable
{
  "type": "conditional",
  "condition": "{price} > 100",
  "then": [
    {"type": "log", "message": "Too expensive"}
  ]
}
```

### Loops

```json
// Repeat N times
{
  "type": "loop",
  "iterations": 5,
  "steps": [
    {"type": "click_element", "selector": ".next-page"},
    {"type": "wait", "duration": 1000}
  ]
}

// While condition
{
  "type": "while",
  "condition": "document.querySelector('.next-page')",
  "max_iterations": 10,
  "steps": [
    {"type": "extract_structured_data", "..."},
    {"type": "click_element", "selector": ".next-page"}
  ]
}

// For each item
{
  "type": "foreach",
  "items": "{product_list}",
  "variable_name": "product",
  "steps": [
    {"type": "navigate_to_url", "url": "{product.url}"},
    {"type": "extract_text", "selector": ".price"}
  ]
}
```

## Error Handling

### Try-Catch

```json
{
  "type": "try_catch",
  "try": [
    {"type": "click_element", "selector": "#premium-feature"}
  ],
  "catch": [
    {"type": "log", "message": "Premium not available"},
    {"type": "click_element", "selector": "#basic-feature"}
  ]
}
```

### Retry Configuration

```json
{
  "type": "click_element",
  "selector": "#dynamic-button",
  "retry": {
    "max_attempts": 3,
    "delay": 1000,
    "backoff_factor": 2
  }
}
```

### Global Error Handler

```json
{
  "browser_automation": {
    "on_error": [
      {"type": "take_screenshot", "filename": "error_{timestamp}.png"},
      {"type": "log", "message": "Workflow failed at step {current_step}"}
    ],
    "sequences": [...]
  }
}
```

## Working with iframes

### Same-origin iframe
```json
// Use >>> to traverse into iframe
{"type": "click_element", "selector": "iframe#video >>> button.play"}
```

### Cross-origin iframe (needs CDP)
```json
{
  "type": "click_element",
  "selector": "iframe[src*='stripe.com'] >>> input[name='cardNumber']",
  "use_cdp": true
}
```

### Multiple nested iframes
```json
{
  "type": "fill_input_field",
  "frame_path": ["iframe#outer", "iframe#inner"],
  "selector": "input#field",
  "value": "test"
}
```

## Real-World Examples

### Login Flow
```json
{
  "steps": [
    {"type": "navigate_to_url", "url": "https://example.com/login"},
    {"type": "fill_input_field", "selector": "#email", "value": "{email}"},
    {"type": "fill_input_field", "selector": "#password", "value": "{password}", "sensitive": true},
    {"type": "click_element", "selector": "button[type='submit']"},
    {"type": "wait_for_any", "conditions": [
      {"element": ".dashboard"},
      {"element": ".error-message"}
    ]},
    {"type": "conditional",
      "condition": "document.querySelector('.error-message')",
      "then": [
        {"type": "extract_text", "selector": ".error-message", "variable_name": "error"},
        {"type": "log", "message": "Login failed: {error}", "level": "error"}
      ]
    }
  ]
}
```

### Pagination & Data Collection
```json
{
  "variables": {
    "all_products": []
  },
  "steps": [
    {"type": "navigate_to_url", "url": "https://shop.com/products"},
    {"type": "while",
      "condition": "document.querySelector('.next-page:not(.disabled)')",
      "max_iterations": 20,
      "steps": [
        {"type": "extract_structured_data",
          "item_selector": ".product",
          "fields": [
            {"name": "name", "selector": ".title"},
            {"name": "price", "selector": ".price"}
          ],
          "variable_name": "page_products"
        },
        {"type": "append_to_variable",
          "name": "all_products",
          "value": "{page_products}"
        },
        {"type": "click_element", "selector": ".next-page"},
        {"type": "wait_for_element", "selector": ".products-loaded"}
      ]
    },
    {"type": "log", "message": "Collected {all_products.length} products"}
  ]
}
```

### Form with Dynamic Fields
```json
{
  "steps": [
    {"type": "navigate_to_url", "url": "https://example.com/apply"},
    {"type": "select_option", "selector": "#country", "value": "US"},
    {"type": "wait_for_element", "selector": "#state"},  // Appears after country selection
    {"type": "select_option", "selector": "#state", "value": "CA"},
    {"type": "wait_for_element", "selector": "#city"},   // Appears after state
    {"type": "fill_input_field", "selector": "#city", "value": "San Francisco"},
    {"type": "click_element", "selector": "#submit"}
  ]
}
```

## Performance Tips

### Use specific waits instead of fixed delays
```json
// ❌ Bad
{"type": "wait", "duration": 5000}

// ✅ Good
{"type": "wait_for_element", "selector": ".content-loaded"}
```

### Batch extractions
```json
// ❌ Bad - Multiple extractions
{"type": "extract_text", "selector": ".name", "variable_name": "name"},
{"type": "extract_text", "selector": ".price", "variable_name": "price"},

// ✅ Good - Single extraction
{
  "type": "extract_structured_data",
  "item_selector": "body",
  "fields": [
    {"name": "name", "selector": ".name"},
    {"name": "price", "selector": ".price"}
  ]
}
```

### Use parallel execution for independent tasks
```json
{
  "browser_automation": {
    "parallel_execution": true,
    "sequences": [
      {"name": "search_google", "steps": [...]},
      {"name": "search_bing", "steps": [...]}
    ]
  }
}
```

## Debugging

### Add logging
```json
{"type": "log", "message": "About to click button"},
{"type": "log", "message": "Price is {price}", "level": "debug"}
```

### Take screenshots at key points
```json
{"type": "take_screenshot", "filename": "before_submit.png"},
{"type": "click_element", "selector": "#submit"},
{"type": "take_screenshot", "filename": "after_submit.png"}
```

### Use validation mode
```bash
# Validate without executing
./test/manual/scripts/test.sh --validate workflow.json
```

### Enable debug logging
```bash
RUST_LOG=debug rzn-browser run workflow.json
```

## Common Gotchas

**Dynamic IDs:** Use stable attributes
```json
// ❌ Bad
{"selector": "#btn_12345"}

// ✅ Good
{"selector": "[data-test='submit-button']"}
```

**Timing issues:** Wait for specific conditions
```json
// ❌ Bad
{"type": "click_element", "selector": "#dynamic"}

// ✅ Good
{"type": "wait_for_element", "selector": "#dynamic"},
{"type": "click_element", "selector": "#dynamic"}
```

**Popups:** Handle them explicitly
```json
{"type": "conditional",
  "condition": "document.querySelector('.cookie-banner')",
  "then": [
    {"type": "click_element", "selector": ".cookie-banner .accept"}
  ]
}
```

## Migrating to Autonomous Mode

### When to Use Each Mode

**Use FSM-Driven Autonomous for:**
- Exploratory tasks ("find the cheapest flights")
- Sites that change frequently 
- Complex decision-making
- One-time or rare tasks
- Research and data collection

**Use JSON Workflows for:**
- Repetitive, exact tasks  
- High-performance requirements
- Mission-critical reliability
- Tasks with complex branching logic
- Integration with external systems

### Migration Examples

#### Google Search: Workflow → Autonomous

```json
// Legacy JSON Workflow
{
  "steps": [
    {"type": "navigate_to_url", "url": "https://google.com"},
    {"type": "fill_input_field", "selector": "input[name='q']", "value": "{query}"},
    {"type": "press_special_key", "key": "Enter"},
    {"type": "wait_for_element", "selector": "#search"},
    {"type": "extract_structured_data", "item_selector": ".g", "fields": [...]}
  ]
}
```

```bash
# FSM-Driven Autonomous (equivalent)
./target/release/rzn-browser llm-auto "Search Google for '{query}' and extract top 10 results"
```

#### E-commerce Product Scraping

```json
// Complex JSON workflow with pagination
{
  "variables": {"all_products": []},
  "steps": [
    {"type": "navigate_to_url", "url": "https://shop.com/products"},
    {"type": "while", "condition": "document.querySelector('.next-page')", "steps": [
      {"type": "extract_structured_data", "..."},
      {"type": "click_element", "selector": ".next-page"}
    ]}
  ]
}
```

```bash
# Autonomous mode handles pagination automatically
./target/release/rzn-browser llm-auto "Go to shop.com and extract all product names and prices from all pages"
```

### Hybrid Approach

You can combine both approaches:

```bash
# Use autonomous mode for exploration
./target/release/rzn-browser llm-auto "Find the login form on example.com and determine the field selectors"

# Then create a workflow for repeated logins
./target/release/rzn-browser run login.json --param username="user" --param password="pass"
```

## Autonomous Mode Benefits

✅ **Adapts to page changes** - No broken selectors
✅ **Policy enforcement** - Prevents dangerous patterns  
✅ **Self-healing** - Recovers from failures automatically
✅ **Natural language** - No JSON syntax to learn
✅ **FSM-controlled** - Prevents infinite loops and drift
✅ **Correlation tracking** - Full execution traceability
✅ **CDP integration** - Trusted keyboard/mouse events

The FSM-driven autonomous mode represents the future of browser automation, providing the intelligence and adaptability needed for modern web applications while maintaining the reliability and determinism required for production use.
