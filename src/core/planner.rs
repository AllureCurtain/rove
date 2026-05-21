use futures::StreamExt;
use thiserror::Error;

use crate::core::types::{Message, PlanStep, Role, TaskPlan};
use crate::models::traits::ModelClient;

const DEFAULT_PLANNER_PROMPT: &str = r#"You are the planner for rove.
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
    pub fn new() -> Self {
        Self {
            prompt: std::fs::read_to_string("prompts/planner.md")
                .unwrap_or_else(|_| DEFAULT_PLANNER_PROMPT.to_string()),
        }
    }

    pub async fn draft(
        &self,
        model: &dyn ModelClient,
        goal: &str,
        history: &[Message],
    ) -> Result<TaskPlan, PlannerError> {
        let mut messages = vec![
            Message {
                role: Role::System,
                content: self.prompt.clone(),
            },
            Message {
                role: Role::User,
                content: format!("Goal: {goal}"),
            },
        ];
        messages.extend_from_slice(history);

        let mut full_response = String::new();
        let mut stream = model.stream(&messages, &[]);
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|err| PlannerError::Model(err.to_string()))?;
            full_response.push_str(&chunk.delta);
        }

        parse_plan(&full_response)
    }
}

impl Default for Planner {
    fn default() -> Self {
        Self::new()
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

    let raw_plan: RawPlan = serde_json::from_str(raw.trim())
        .map_err(|err| PlannerError::InvalidJson(err.to_string()))?;
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
