use anyhow::Context;
use rzn_core::{Step, StepKind};
use rzn_plan::broker_client::{BrokerClient, Transport};
use rzn_plan::{DesktopSession, PolicyGate};
use serde_json::json;
use std::env;

fn transport_from_env() -> Transport {
    match env::var("RZN_TRANSPORT")
        .unwrap_or_else(|_| "pipe".to_string())
        .to_lowercase()
        .as_str()
    {
        "tcp" => Transport::Tcp,
        _ => Transport::Pipe,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let url = env::args()
        .nth(1)
        .unwrap_or_else(|| "https://example.com".to_string());

    let broker = BrokerClient::new(transport_from_env());
    let policy = PolicyGate::from_env();
    let mut session = DesktopSession::new(broker, policy);
    session
        .broker_mut()
        .connect()
        .await
        .context("connect broker")?;

    let nav = Step {
        id: "nav".to_string(),
        name: format!("Navigate to {}", url),
        kind: StepKind::NavigateToUrl {
            url,
            wait: Some("domcontentloaded".to_string()),
        },
    };
    session.act_step(&nav).await.context("navigate")?;

    // Deterministic extraction: no arbitrary JS, just a validated plan.
    let plan = json!({
        "version": 1,
        "mode": "single",
        "scope": { "css": "html" },
        "fields": [
            { "name": "title", "selector": "title" },
            { "name": "h1", "selector": "h1", "optional": true }
        ]
    });

    let resp = session
        .extract_extraction_plan(plan)
        .await
        .context("execute extraction plan")?;

    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}
