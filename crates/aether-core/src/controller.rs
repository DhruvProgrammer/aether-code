//! Controller: understands the task and produces a plan (spec §2).

use aether_models::{CompletionRequest, Message, ModelProvider};
use anyhow::Result;

use crate::mode::{KARPATHY_POLICY, BUILD_MODE_PROMPT, PLAN_MODE_PROMPT, Mode};

pub async fn plan(
    provider: &dyn ModelProvider,
    model: &str,
    task: &str,
    memory_hint: &str,
    mode: Mode,
) -> Result<String> {
    let (mode_label, mode_prompt) = if mode.is_plan() {
        ("PLAN MODE", PLAN_MODE_PROMPT)
    } else {
        ("BUILD MODE", BUILD_MODE_PROMPT)
    };
    let system = format!(
        "{core}\n\n---\n\nYou are the Controller. {mode_prompt}\n{mode_label}\n{KARPATHY_POLICY}\n\
         Given a user task, produce a concise, actionable response. \
         In BUILD MODE output a numbered implementation plan for the Executor. \
         In PLAN MODE output the structured PLAN document with these sections: Goal, Current \
         Understanding, Relevant Files, Existing Architecture, Facts, Assumptions, Unknowns, \
         Hypotheses (with confidence + evidence), Recommended Approach, Implementation Steps, \
         Verification, Risks, Files Expected to Change, Questions/Decisions Required. \
         Output only the plan, no preamble.",
        core = crate::prompt::AETHER_CORE_SYSTEM_PROMPT,
    );
    let mut msgs = vec![Message {
        role: "system".into(),
        content: system,
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
