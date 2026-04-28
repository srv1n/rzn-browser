use rzn_sdk::contracts::v1::{ActionV1, TargetV1};
use rzn_sdk::prelude::*;
use std::env;

/// Minimal end-to-end demo for the deterministic SDK substrate.
///
/// Prereqs:
/// - Chrome with the extension installed
/// - Native host runtime running (typically launched by Chrome via native messaging)
/// - `RZN_TRANSPORT=pipe` (default) or `RZN_TRANSPORT=tcp`
///
/// Usage:
///   cargo run -p rzn_sdk --example browser_tools_demo -- "https://www.google.com"
#[tokio::main]
async fn main() -> Result<()> {
    let url = env::args().nth(1);
    let transport = RuntimeTransport::from_env();

    let mut tools = BrowserTools::connect(transport).await?;

    if let Some(url) = url {
        let _ = tools
            .act(ActionV1::NavigateToUrl {
                url,
                wait: Some("domcontentloaded".to_string()),
            })
            .await;
    }

    let snap = tools.observe().await.map_err(|e| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            e.to_string(),
        ))
    })?;
    println!("URL: {}", snap.metadata.url);
    println!("Title: {}", snap.metadata.title);
    println!("Elements: {}", snap.elements.len());

    for el in snap.elements.iter().take(12) {
        println!(
            "- {} {} text={:?} selector={}",
            el.encoded_id, el.tag, el.text, el.selector
        );
    }

    // Example of how a host app would act using encoded ids:
    if let Some(first_input) = snap
        .elements
        .iter()
        .find(|e| e.tag == "input" || e.tag == "textarea")
    {
        let _ = tools
            .act(ActionV1::ClickElement {
                target: TargetV1::from_encoded_id(first_input.encoded_id.clone()),
                random_offset: Some(true),
                timeout_ms: Some(3000),
            })
            .await;
    }

    Ok(())
}
