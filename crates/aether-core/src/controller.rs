//! Controller: understands the task and produces a plan (spec §2).

use aether_models::{CompletionRequest, Message, ModelProvider};
use anyhow::Result;

pub async fn plan(
    provider: &dyn ModelProvider,
    model: &str,
    task: &str,
    memory_hint: &str,
) -> Result<String> {
    let system = "You are the Controller. Given a user task, produce a concise numbered plan. \
                  The Executor will implement it using tools. Output only the plan, no preamble.";
    let mut msgs = vec![Message {
        role: "system".into(),
        content: system.into(),
        ..Default::default()
    }];
    if !memory_hint.is_empty() {
        msgs.push(Message {
            role: "system".into(),
            content: format!("Relevant memory:\n{memory_hint}"),
            ..Default::default()
        });
    }
    msgs.push(Message {
        role: "user".into(),
        content: format!("TASK: {task}"),
        ..Default::default()
    });
    let req = CompletionRequest {
        model: model.into(),
        messages: msgs,
        ..Default::default()
    };
    let resp = provider.complete(req).await?;
    Ok(resp.content.unwrap_or_default())
}
