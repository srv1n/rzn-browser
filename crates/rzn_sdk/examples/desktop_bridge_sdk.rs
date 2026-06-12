use rzn_sdk::prelude::*;
use std::env;

/// Minimal SDK example.
///
/// This runs the planner in "LLM-only" mode (no browser runtime required).
/// To use a real provider, export the appropriate keys (e.g. `OPENAI_API_KEY`)
/// and optionally set `LLM_PROVIDER=openai|gemini|dummy`.
#[tokio::main]
async fn main() -> Result<()> {
    let goal = env::args()
        .nth(1)
        .unwrap_or_else(|| "Summarize what this page is about".to_string());

    let start_url = env::args().nth(2);

    let mut host = Host::from_env().await?;

    let resp = host
        .plan_llm_only(PlanRequest {
            goal,
            start_url,
            parameters: Default::default(),
            save_workflow: false,
            workflow_name: None,
        })
        .await?;

    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}
