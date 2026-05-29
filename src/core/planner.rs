use futures::StreamExt;
use thiserror::Error;

use crate::core::types::{Message, PlanStep, TaskPlan};
use crate::models::traits::{ModelClient, ModelEvent};

pub const DEFAULT_PLANNER_PROMPT: &str = r#"You are the planner for rove.
Return JSON only:
{
  "goal": "string",
  "steps": [
    { "id": "1", "title": "string" }
  ]
}
"#;

#[derive(Debug, Error)]
pub enum PlannerError {
    #[error("model error while drafting plan: {0}")]
    Model(String),
    #[error("planner returned invalid JSON: {0}")]
    InvalidJson(String),
    #[error("planner returned no steps")]
    EmptyPlan,
}

pub struct Planner {
    prompt: String,
}

impl Planner {
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
        }
    }

    pub fn with_default_prompt() -> Self {
        Self::new(DEFAULT_PLANNER_PROMPT)
    }

    pub async fn draft(
        &self,
        model: &dyn ModelClient,
        goal: &str,
        history: &[Message],
    ) -> Result<TaskPlan, PlannerError> {
        let mut messages = vec![
            Message::system(self.prompt.clone()),
            Message::user(format!("Goal: {goal}")),
        ];
        messages.extend_from_slice(history);

        let mut full_response = String::new();
        let mut stream = model.stream(&messages, &[]);
        while let Some(event) = stream.next().await {
            let event = event.map_err(|err| PlannerError::Model(err.to_string()))?;
            match event {
                ModelEvent::TextDelta { text } => full_response.push_str(&text),
                ModelEvent::Done => break,
                ModelEvent::ThinkingDelta { .. }
                | ModelEvent::ToolUseStart { .. }
                | ModelEvent::ToolUseDelta { .. }
                | ModelEvent::ToolUseDone { .. }
                | ModelEvent::Usage { .. } => {}
            }
        }

        parse_plan(&full_response)
    }
}

impl Default for Planner {
    fn default() -> Self {
        Self::with_default_prompt()
    }
}

fn parse_plan(raw: &str) -> Result<TaskPlan, PlannerError> {
    #[derive(serde::Deserialize)]
    struct RawPlan {
        goal: String,
        steps: Vec<RawStep>,
    }

    #[derive(serde::Deserialize)]
    struct RawStep {
        id: String,
        title: String,
    }

    let json = extract_json_object(raw)
        .ok_or_else(|| PlannerError::InvalidJson("planner returned no JSON object".to_string()))?;
    let raw_plan: RawPlan =
        serde_json::from_str(json).map_err(|err| PlannerError::InvalidJson(err.to_string()))?;
    if raw_plan.steps.is_empty() {
        return Err(PlannerError::EmptyPlan);
    }

    Ok(TaskPlan {
        goal: raw_plan.goal,
        steps: raw_plan
            .steps
            .into_iter()
            .map(|step| PlanStep {
                id: step.id,
                title: step.title,
                done: false,
            })
            .collect(),
        current_step: 0,
    })
}

fn extract_json_object(raw: &str) -> Option<&str> {
    let start = raw.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (offset, ch) in raw[start..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let end = start + offset + ch.len_utf8();
                    return Some(&raw[start..end]);
                }
            }
            _ => {}
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::parse_plan;

    #[test]
    fn parse_plan_accepts_json_surrounded_by_prose() {
        let plan = parse_plan(
            "Here is the plan:\n{\"goal\":\"fix docs\",\"steps\":[{\"id\":\"1\",\"title\":\"inspect\"}]}\nDone.",
        )
        .unwrap();

        assert_eq!(plan.goal, "fix docs");
        assert_eq!(plan.steps[0].id, "1");
        assert_eq!(plan.steps[0].title, "inspect");
    }
}
