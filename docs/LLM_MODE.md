# LLM Mode - FSM-Driven Autonomous Browser Automation

## Consolidated & Revolutionary Architecture: Single LLM Implementation

**Major Simplification (2024)**: The LLM mode has been consolidated into a single, robust system (`llm_autonomous.rs`). This eliminates confusion and provides a unified entry point for all LLM-driven automation.

Flow overview
```
CLI → LLMAutonomousPlanner → Runtime Bridge → Extension (SW) → Content Script → Page
    ←            Results    ←               ←                ←
```

Planner loop (high level)
```
get_page_state → build messages → LLM JSON → parse_llm_response →
policy/FSM gate → execute_step (runtime bridge) → collect result → repeat/finish
```

## Revolutionary Architecture: FSM + Policy + Tool-Only LLM

LLM mode in RZN represents a breakthrough in browser automation reliability through Sam's architectural insights. Instead of free-form LLM responses that can drift and construct URLs, the system uses a Finite State Machine (FSM) with strict policy validation and tool-only LLM calls to ensure deterministic, reliable behavior.

The key innovation is **architectural constraints** rather than prompt engineering. The system prevents bad behavior at the structural level:

- **FSM**: Strict state management (Bootstrap → Search → Results → Complete)
- **Policy Layer**: Blocks Google search URL construction and validates tool usage
- **Tool-Only LLM**: Forces structured output with temperature=0 for determinism
- **CDP Integration**: First-class keyboard input for maximum reliability

Consider the complexity of a seemingly simple task like "find the cheapest flight from New York to London next month." A traditional automation script would need to account for every possible layout variation, every airline's unique interface, different calendar widgets, various price display formats, and countless edge cases. With LLM mode, you simply state your intent, and the system adapts to whatever interface it encounters.

The intelligence comes from combining large language models with real-time page analysis. The system doesn't just blindly follow instructions; it understands context, recognizes patterns, and makes decisions. When it encounters an unexpected popup, it knows whether to dismiss it or interact with it based on your goal. When a page layout changes, it finds alternative ways to accomplish the task. This adaptability makes LLM mode resilient to the constant changes that break traditional automation scripts.

## How FSM-Driven LLM Mode Works

### Core Command: `llm-auto`

```bash
# The new autonomous mode with FSM and policy validation
./target/release/rzn-browser llm-auto "Search Google for OpenAI" --max-steps 10

# Enable debug logging to see FSM transitions
RUST_LOG=debug ./target/release/rzn-browser llm-auto "Your instruction"
```

### FSM-Driven Execution Pipeline

The process follows strict architectural constraints designed to prevent the common failure modes of free-form LLM automation:

### Page Analysis and Understanding

When the LLM analyzer examines a page, it doesn't just see HTML elements; it builds a semantic understanding of the page's purpose and structure. The analyzer identifies the type of page (search, form, listing, article), recognizes common patterns (navigation menus, search boxes, product cards), understands relationships between elements (labels and inputs, headers and content), and infers the page's current state (logged in/out, empty/populated cart, error conditions).

This analysis produces a structured representation that preserves essential information while fitting within LLM token limits. Instead of sending thousands of lines of HTML, the system sends a focused summary that includes interactive elements with their roles and states, form structures with field relationships, content hierarchies and groupings, and any error messages or important notifications.

### 1. FSM State Management

The system starts in **Bootstrap** mode and transitions through states based on actions:

```rust
// FSM States with strict tool restrictions
Bootstrap  → ["navigate", "wait"]                    // Initial navigation only
Search     → ["type", "press_key", "wait"]          // Type in search box + Enter  
Results    → ["extract", "click", "scroll", "wait"] // Extract data or navigate
Form       → ["type", "click", "press_key", "wait"] // Fill forms
Browse     → ["click", "scroll", "extract", "navigate", "wait"] // General browsing
Complete   → ["complete"]                           // Task finished
```

### 2. Tool-Only LLM Planning

Instead of free-form responses, the LLM MUST use structured tool calls:

```json
// Tool-only request (temperature=0, tool_choice="required")
{
  "model": "gpt-5-mini-2025-08-07",
  "temperature": 0.0,
  "tools": [/* filtered by FSM state */],
  "tool_choice": "required"
}

// LLM response (always structured)
{
  "tool_calls": [
    {"function": {"name": "type", "arguments": {"selector": "input[name='q']", "text": "OpenAI"}}},
    {"function": {"name": "press_key", "arguments": {"key": "Enter"}}}
  ]
}
```

### 3. Policy Validation Layer

Before any action executes, the policy validator checks:

```rust
// CRITICAL: Block Google search URL construction
if url.contains("google.com/search?") {
    return Err("POLICY VIOLATION: Never construct Google search URLs. Use type + press_key instead.");
}

// Validate tools are allowed in current FSM state
let allowed_tools = fsm.get_allowed_tools();
if !allowed_tools.contains(&action.cmd) {
    return Err(format!("Tool '{}' not allowed in mode {:?}", action.cmd, fsm.mode));
}
```

### 4. Action Execution with State Transitions

After validation, actions execute and trigger FSM state changes:

```rust
// Execute validated action
let result = broker.execute_step(&action).await?;

// Update FSM state based on action results  
match action.cmd.as_str() {
    "navigate" => {
        let next_mode = fsm.infer_next_mode(&url, &dom_summary);
        fsm.transition(next_mode); // Bootstrap → Search (if google.com)
    }
    "press_key" if key == "Enter" && fsm.mode == Search => {
        fsm.transition(Results); // Search → Results (after pressing Enter)
    }
    "extract" => {
        fsm.transition(Complete); // Results → Complete (data extracted)
    }
    _ => {} // Stay in current state
}
```

### 5. Correlation ID Tracking & Logging

Every action is tracked with correlation IDs for full traceability:

```bash
# View FSM state transitions
tail -f ~/rzn_build.log | grep "State transition"
# [87a21fb2-bd8b-4a00-8abb-17d90a635a5e] State transition: Bootstrap -> Search

# View policy violations  
tail -f ~/rzn_build.log | grep "POLICY VIOLATION"

# View raw LLM calls
cat /tmp/llm_raw_87a21fb2-bd8b-4a00-8abb-17d90a635a5e.jsonl | jq .
```

### 6. CDP-Based Key Input

The `press_key` action uses Chrome DevTools Protocol for maximum reliability:

```typescript
// First-class CDP implementation 
await chrome.debugger.sendCommand(debuggee, 'Input.dispatchKeyEvent', {
  type: 'keyDown',
  key: 'Enter',
  code: 'Enter', 
  windowsVirtualKeyCode: 13,
  // ... trusted events
});
```

### Context Persistence

Throughout execution, the system maintains awareness of the overall goal and progress toward it. This context persistence enables complex multi-step automations that would be difficult to program explicitly.

The context includes not just the current state but also the history of what's been tried, what worked, and what didn't. This prevents the system from getting stuck in loops and helps it make better decisions. If searching for "laptop" returned too many results, it remembers this and might add specifications like "Dell laptop 15 inch" on the next attempt.

## Provider Comparison and Selection

Different LLM providers offer unique strengths, and RZN optimizes its interaction with each to maximize performance and reliability.

### OpenAI GPT-4

GPT-4 excels at understanding complex, nuanced instructions and maintaining context across long interactions. Its strengths make it ideal for sophisticated automation tasks that require reasoning and decision-making.

When using GPT-4, the system leverages its function calling capabilities to ensure structured, predictable output. The prompts are designed to encourage step-by-step reasoning, making the automation process more transparent and debuggable. GPT-4's large context window allows for detailed page representations, enabling better understanding of complex interfaces.

The system sets GPT-4's temperature low (0.1) to ensure consistent, deterministic behavior. This reduces randomness in decision-making, making automation more reliable. For tasks requiring creativity or exploration, the temperature can be adjusted higher.

GPT-4 is particularly effective for tasks involving complex forms with interdependent fields, multi-step workflows requiring memory of previous actions, situations requiring interpretation of ambiguous instructions, and pages with sophisticated JavaScript interactions.

### Google Gemini

Gemini provides exceptional response speed and efficiency, making it ideal for rapid iteration and real-time adaptation. Its visual understanding capabilities add another dimension to page analysis.

The system optimizes prompts for Gemini's architecture, keeping them concise while preserving essential information. Gemini's native JSON mode ensures clean, parseable output without additional processing. The speed advantage becomes particularly valuable during the self-healing process, where multiple attempts might be needed.

Gemini excels at tasks involving visual elements like charts or infographics, rapid exploration of multiple options or paths, situations where response time is critical, and pages with minimal text but rich visual information.

### Anthropic Claude

Claude offers superior reasoning capabilities and better adherence to complex instructions. Its constitutional AI training makes it particularly suitable for automation tasks that require careful decision-making.

The system can use more conversational prompts with Claude while maintaining precision. Claude's larger context window enables even more detailed page representations when needed. Its strong instruction-following makes it ideal for tasks with specific constraints or requirements.

Claude is especially effective for tasks requiring careful data extraction and validation, automation involving sensitive or regulated content, complex decision trees with multiple conditions, and situations requiring explanation of actions taken.

### Provider Selection Strategy

Choosing the right provider depends on your specific needs and constraints. Consider these factors when selecting a provider:

For general-purpose automation with complex requirements, GPT-4 provides the best balance of capabilities. For rapid, iterative tasks where speed matters, Gemini offers the best performance. For tasks requiring careful reasoning and rule adherence, Claude excels. Cost considerations might also influence choice, as providers have different pricing models.

The system can also use multiple providers in a single automation, leveraging each provider's strengths. For example, using Gemini for rapid page exploration, then GPT-4 for complex decision-making, and Claude for final validation.

## Prompt Engineering and Optimization

The quality of LLM automation depends heavily on how prompts are structured and optimized. RZN uses sophisticated prompt engineering to ensure reliable, consistent behavior.

### System Prompts

The system prompt establishes the LLM's role and capabilities. It defines the LLM as a browser automation expert who understands web patterns and can generate reliable action sequences. This prompt includes examples of common patterns, edge cases to watch for, and guidelines for decision-making.

The system prompt is carefully crafted to encourage specific behaviors. It emphasizes accuracy over speed, completeness over brevity, and explicit error handling over assumptions. It includes examples of good and bad automation patterns, helping the LLM avoid common pitfalls.

### Page Context Prompts

Converting a complex web page into an LLM-understandable format requires careful consideration. The page context prompt provides structure while preserving semantic meaning.

The prompt includes hierarchical representation of interactive elements, showing relationships and groupings. Form structures are presented with field relationships and validation requirements. The current state is clearly indicated, including values of inputs, selected options, and visible/hidden elements. Important text like error messages, prices, or status indicators is preserved verbatim.

### Task Prompts

The task prompt combines the user's instruction with additional context to guide the LLM's planning. It includes the original natural language instruction, any clarifications or constraints, history of previous attempts if applicable, and success criteria for the task.

The prompt might also include domain-specific information. For e-commerce tasks, it might include information about common checkout flows. For form filling, it might include typical validation patterns. This domain knowledge helps the LLM make better decisions.

### Optimization Techniques

Several techniques optimize prompt performance and token usage:

**Compression** reduces token consumption while preserving information. Instead of sending full HTML, send structured summaries. Replace repetitive content with templates. Use references instead of duplication.

**Focusing** ensures the LLM attention on relevant information. Highlight elements near the viewport or recently interacted with. Emphasize error messages and status indicators. Reduce detail for decorative or non-interactive elements.

**Chunking** handles pages too large for a single prompt. Divide the page into logical sections. Process sections independently when possible. Maintain context across chunks for coherent planning.

**Caching** improves performance for repeated operations. Cache page analysis for static content. Reuse successful plans for similar tasks. Store element identifiers for faster re-selection.

## Self-Healing and Error Recovery

The self-healing capability sets LLM mode apart from traditional automation. When actions fail, the system doesn't just retry; it understands why they failed and adapts its approach.

### Failure Analysis

When an action fails, the system performs comprehensive analysis to understand the cause. It examines the error message if available, checks the current page state versus expected state, identifies any blocking elements like popups or overlays, and considers whether the page might still be loading.

This analysis categorizes failures into types, each with specific recovery strategies. Element not found might trigger alternative selector strategies. Validation errors lead to input correction. Unexpected navigation might require returning to the previous page. Network errors trigger wait and retry logic.

### Alternative Strategies

The system maintains multiple strategies for accomplishing each action. If clicking a button fails, it might try scrolling the element into view first, using keyboard navigation to reach the element, finding an alternative button with similar text, or executing the button's onclick handler directly.

For form filling, alternative strategies might include trying different input formats (date formats, phone number formats), using dropdown selection instead of typing, clearing and refilling fields that might have auto-complete, or submitting forms programmatically if the button is inaccessible.

### Learning from Failures

Each failure provides learning opportunities that improve future attempts. The system remembers what didn't work to avoid repeating failed approaches. It identifies patterns in failures that might indicate site-specific behaviors. It adjusts timing and waiting strategies based on observed load times.

This learning happens at multiple levels. Within a single session, the system adapts its approach based on what it learns about the specific site. Across sessions, patterns can be identified that improve the system's general web understanding.

### Graceful Degradation

When the system cannot fully complete a task, it attempts graceful degradation rather than complete failure. It might accomplish partial goals when the complete goal is impossible, extract available information even if some is missing, provide alternative suggestions when the requested action cannot be performed, or clearly communicate what was and wasn't accomplished.

## Advanced Patterns and Techniques

LLM mode enables sophisticated automation patterns that would be impractical with traditional scripting.

### Multi-Site Workflows

Automating across multiple websites requires understanding different interfaces while maintaining goal coherence. The system handles this by maintaining high-level goal awareness while adapting to each site's specifics.

For example, comparing prices across e-commerce sites requires understanding that "price" might be displayed differently on each site, search interfaces vary but serve the same purpose, product names might not match exactly across sites, and availability information uses different terminology.

The system abstracts these differences, focusing on semantic meaning rather than specific implementations. It knows that "$29.99", "29.99 USD", and "Price: 29.99" all represent the same concept.

### Conversational Interactions

Some automations benefit from conversational interaction with the user. The system can pause to ask for clarification, present options for the user to choose from, or request additional information when needed.

This interaction model works particularly well for exploratory tasks where the exact goal isn't predetermined, complex decisions requiring human judgment, situations where multiple valid options exist, or when encountering unexpected scenarios requiring user input.

### Exploratory Automation

LLM mode excels at exploratory tasks where the exact path isn't known in advance. "Find interesting articles about quantum computing" or "Discover new restaurants with good vegetarian options" are tasks that require exploration and subjective judgment.

The system approaches these tasks by first exploring broadly to understand available options, then applying filters or criteria to narrow results, evaluating options based on stated or inferred preferences, and presenting findings in a useful format.

### Adaptive Data Extraction

When extracting data from multiple sources with varying formats, LLM mode can adapt to different structures while maintaining consistent output.

The system recognizes equivalent information despite different presentations, handles missing fields gracefully, normalizes data formats automatically, and validates extracted data for consistency.

For example, extracting product specifications from different manufacturer sites requires understanding that "Dimensions", "Size", and "Product Measurements" refer to the same information, "Weight" might be in pounds, kilograms, or ounces, and some sites might omit certain specifications entirely.

## Performance Optimization

While LLM mode is inherently slower than deterministic workflows due to API calls, several optimizations can improve performance significantly.

### Prompt Optimization

Reducing prompt size while maintaining effectiveness directly impacts response time and cost. Remove redundant information from page representations. Use compression techniques for repetitive structures. Focus on elements relevant to the current task. Cache analysis for static page sections.

### Batching Strategies

When possible, batch multiple operations into a single LLM call. Instead of asking for each action individually, request a complete plan. Analyze multiple page sections in one call. Extract all needed data in a single operation.

### Parallel Processing

Some tasks can be parallelized for better performance. Independent page analyses can run concurrently. Multiple sites can be explored simultaneously. Different providers can be used for different subtasks in parallel.

### Caching and Reuse

Intelligent caching reduces redundant LLM calls. Cache page analysis for a reasonable duration. Reuse successful plans for identical tasks. Store extracted selectors for faster re-use. Remember site-specific patterns for future automation.

### Progressive Enhancement

Start with fast, simple approaches and escalate only when needed. Try cached or rule-based solutions first. Use smaller, faster models for simple tasks. Escalate to more capable models for complex scenarios. Combine deterministic and LLM approaches where appropriate.

## Security and Privacy

LLM mode introduces unique security and privacy considerations that must be carefully managed.

### Data Sanitization

Before sending page content to LLM providers, sensitive information must be protected. The system automatically redacts passwords and credit card numbers, removes personally identifiable information when configured, excludes data marked as sensitive, and can operate in a privacy-enhanced mode with stricter filtering.

Users can configure what types of data are shared, with options ranging from full page content to minimal structural information. The system provides clear indicators when data will be sent to external services.

### API Key Management

Secure handling of API keys is critical. Never hard-code keys in workflows or scripts. Use environment variables or secure key stores. Rotate keys regularly. Monitor usage for anomalies. Implement rate limiting to prevent abuse.

### Audit and Compliance

For regulated environments, LLM mode provides comprehensive audit capabilities. Every LLM interaction is logged with request and response. The system tracks what data was sent to external services. Decision rationales can be preserved for review. Compliance modes restrict certain operations.

### Local Processing Options

For maximum privacy, some processing can be done locally. Page analysis and structuring happen in the browser. Certain patterns can be recognized without LLM calls. Local models can be used for simple tasks. Hybrid approaches minimize external data transmission.

## Debugging and Troubleshooting

When LLM automation doesn't behave as expected, systematic debugging helps identify and resolve issues.

### Understanding LLM Decisions

Enable verbose logging to see the LLM's reasoning process. The system can output the prompts sent to the LLM, the raw responses received, the parsed action plans, and decision rationales when available.

This visibility helps identify whether issues stem from page analysis problems, prompt construction issues, LLM misunderstanding, or execution failures.

### Common Issues and Solutions

**Instruction Ambiguity**: If the LLM consistently misunderstands instructions, make them more specific. Instead of "find cheap flights," specify "find flights under $500." Add context about preferences and constraints. Use examples of desired outcomes.

**Page Complexity**: When pages overwhelm the LLM with information, adjust the analysis focus. Reduce the detail level for non-relevant sections. Break complex pages into logical chunks. Use multiple analysis passes for different aspects.

**Provider Limitations**: Different providers have different strengths and weaknesses. If one provider struggles with a task, try another. Adjust prompts for provider-specific optimizations. Consider provider-specific features like vision capabilities.

**Timing Issues**: LLM mode needs to account for page dynamics. Ensure sufficient waiting for content to load. Handle progressive rendering appropriately. Account for API rate limits and throttling.

### Testing Strategies

Comprehensive testing ensures reliable LLM automation. Test with various page states (logged in/out, empty/full cart). Try edge cases like error conditions and validation failures. Verify behavior across different sites with similar functionality. Test with different LLM providers for comparison.

Create test scenarios that cover common patterns, edge cases, error conditions, and performance boundaries. Maintain a regression suite of previously working automations to catch breaking changes.

## Best Practices

Following best practices ensures optimal results from LLM mode automation.

### Instruction Clarity

Write clear, specific instructions that leave little room for misinterpretation. Include context about your current state (logged in, on specific page). Specify exact requirements and constraints. Provide examples when helpful. Indicate preferences for ambiguous situations.

Good: "Find round-trip flights from JFK to LHR, departing March 15-20, returning March 25-30, economy class, under $800 total"

Less effective: "Book a cheap flight to London next month"

### Progressive Complexity

Start with simple tasks and gradually increase complexity. This helps you understand the system's capabilities and limitations. Begin with single-page, single-action tasks. Progress to multi-step workflows on a single site. Then attempt cross-site automation. Finally, tackle complex, exploratory tasks.

### Error Handling

Plan for failures and partial successes. Specify what to do when the primary goal cannot be achieved. Indicate whether partial results are acceptable. Define fallback strategies for common failures. Set clear success criteria.

### Performance Expectations

Set realistic expectations for execution time. LLM mode is slower than deterministic automation due to API calls and analysis time. Plan for 2-5 seconds per significant action. Complex decisions might take longer. Exploration tasks are inherently slower.

### Cost Management

Monitor and manage API token usage. Complex pages consume more tokens. Frequent automation can accumulate costs quickly. Use appropriate models for task complexity. Implement caching to reduce redundant calls. Set usage limits and alerts.

## Real-World Use Cases

LLM mode shines in scenarios where traditional automation would be impractical or impossible.

### Research and Comparison

Automating research tasks across multiple sources demonstrates LLM mode's strengths. The system can gather information from various websites with different structures, understand and correlate related information, handle variations in terminology and presentation, and synthesize findings into useful summaries.

Example: "Research the top 5 electric vehicles of 2024, comparing range, price, and charging time across manufacturer websites and review sites."

The system navigates to each manufacturer's site, finds the relevant model pages, extracts specifications despite different formats, visits review sites for additional information, and compiles a comprehensive comparison.

### Dynamic Form Filling

Complex forms with conditional fields and validation rules are easily handled. The system understands field relationships and dependencies, adapts to validation errors and corrections, handles dynamic field appearance/disappearance, and manages multi-step forms with progress tracking.

Example: "Complete the insurance quote form using the provided customer information, selecting appropriate coverage options based on their needs."

The system interprets customer requirements, makes appropriate selections, corrects errors based on validation feedback, and navigates through multiple form pages.

### Content Monitoring

Tracking changes or updates across websites becomes simple with natural language instructions. The system can check for new content matching criteria, detect changes in specific information, monitor availability or status changes, and aggregate updates from multiple sources.

Example: "Check if any of my watched products on Amazon have dropped in price by more than 20% or have new reviews mentioning defects."

The system navigates to each product, extracts current prices, compares with previous values, scans new reviews for keywords, and reports significant findings.

### Interactive Exploration

Tasks requiring exploration and decision-making showcase LLM mode's intelligence. The system can browse based on subjective criteria, make selections based on preferences, explore unfamiliar interfaces adaptively, and discover information through investigation.

Example: "Find a highly-rated Italian restaurant in downtown Seattle that's open tonight, has outdoor seating, and takes reservations."

The system searches restaurant sites, interprets ratings and reviews, checks availability and features, navigates reservation systems, and presents options meeting all criteria.

### Workflow Migration

Converting manual processes to automation becomes straightforward. Describe the process in natural language rather than programming each step. The system handles variations and exceptions that would require extensive coding in traditional automation.

Example: "Every Monday, check my three supplier websites for new products in the electronics category, extract products under $50 with more than 4-star ratings, and compile them into a spreadsheet."

The system interprets the schedule requirement, navigates to each supplier, applies filters and criteria, handles pagination and loading, extracts relevant data, and formats output appropriately.

## Integration with Workflow Mode

LLM mode and workflow mode can be combined for optimal results, leveraging the strengths of each approach.

### Hybrid Automation

Use deterministic workflows for predictable parts and LLM mode for variable sections. This provides reliability where possible and adaptability where needed.

```json
{
  "steps": [
    {
      "type": "navigate_to_url",
      "url": "https://example.com/search"
    },
    {
      "type": "llm_task",
      "instruction": "Search for the product and find the best deal considering price and shipping",
      "context": {
        "product": "{product_name}",
        "budget": "{max_price}"
      }
    },
    {
      "type": "extract_structured_data",
      "variable_name": "selected_product"
    }
  ]
}
```

### Fallback Strategies

Use LLM mode as a fallback when deterministic approaches fail. If specific selectors break, let the LLM find alternatives. When encountering unexpected page states, use LLM intelligence to recover.

### Planning Assistance

Use LLM mode to generate workflow JSON from natural language descriptions. The system can analyze a manual process and create automation, suggest optimizations for existing workflows, and identify potential failure points and add error handling.

### Dynamic Workflow Generation

LLM mode can generate workflows on-the-fly based on runtime conditions. Analyze the current page and generate appropriate steps. Adapt workflows based on discovered page structure. Create custom workflows for new sites automatically.

## Future Developments

LLM mode continues to evolve with advances in AI and web technologies.

### Model Improvements

Newer LLM models bring enhanced capabilities. Better understanding of visual elements and layouts. Improved reasoning for complex multi-step tasks. Faster inference for reduced latency. Smaller models for cost-effective simple tasks.

### Specialized Training

Domain-specific models could provide better automation for particular industries or use cases. E-commerce optimized models understanding product categorization. Financial services models with regulatory awareness. Healthcare models with HIPAA compliance built-in.

### Local Intelligence

Advances in local AI processing could enable private, fast automation. On-device models for sensitive data handling. Hybrid processing with local and cloud models. Edge computing for distributed automation.

### Autonomous Agents

Future versions could operate more independently. Continuous monitoring and action without constant instruction. Learning from user behavior to anticipate needs. Proactive automation based on patterns and preferences.

## Conclusion

LLM mode transforms browser automation from a rigid, code-based process to an intelligent, adaptive system that understands and achieves goals. By combining the power of large language models with sophisticated page analysis and execution capabilities, RZN enables automation scenarios that were previously impossible or impractical.

The key to success with LLM mode is understanding its strengths and limitations. It excels at handling variation, understanding context, and adapting to changes. It requires clear instructions, appropriate provider selection, and realistic performance expectations. When used effectively, it dramatically reduces the time and expertise required to automate browser tasks.

Whether you're automating research, data extraction, form filling, or exploration tasks, LLM mode provides the intelligence and adaptability to handle real-world web complexity. As AI models continue to improve and web technologies evolve, LLM mode will become even more capable, making browser automation accessible to everyone regardless of technical expertise.

The future of browser automation is not about writing better scripts; it's about describing what you want to accomplish and letting intelligent systems figure out how to do it. RZN's LLM mode is that future, available today.
