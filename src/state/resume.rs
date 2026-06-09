use crate::core::runtime_identity::{
    RuntimeIdentity, RuntimeIdentityEvaluation, evaluate_runtime_identity,
};
use crate::core::types::{RunId, TaskState};
use crate::state::store::StateStore;

pub async fn resolve_resume_state(
    state_store: &StateStore,
    resume: Option<&str>,
) -> anyhow::Result<Option<TaskState>> {
    let Some(value) = resume else {
        return Ok(None);
    };

    if value == "latest" {
        return Ok(state_store.load_latest_task_state().await?);
    }

    let run_id = ulid::Ulid::from_string(value).map_err(|_| {
        anyhow::anyhow!("unsupported --resume value: {value}; expected latest or run_id")
    })?;
    Ok(Some(state_store.load_task_state(RunId(run_id)).await?))
}

pub fn evaluate_resume_runtime_identity(
    state: &TaskState,
    current: &RuntimeIdentity,
) -> RuntimeIdentityEvaluation {
    let saved = state
        .checkpoint
        .as_ref()
        .and_then(|checkpoint| checkpoint.runtime_identity.as_ref())
        .or(state.runtime_identity.as_ref());
    evaluate_runtime_identity(saved, current)
}

#[cfg(test)]
mod tests {
    use super::resolve_resume_state;
    use crate::core::types::{JobId, RunId, SessionId, TaskState};
    use crate::state::store::StateStore;

    fn task_state(run_id: RunId, goal: &str) -> TaskState {
        TaskState {
            schema_version: 1,
            session_id: SessionId::new(),
            job_id: JobId::new(),
            run_id,
            goal: goal.to_string(),
            step: 1,
            history: vec![],
            summary: None,
            checkpoint: None,
            plan: None,
            runtime_identity: None,
        }
    }

    #[tokio::test]
    async fn resolve_resume_state_supports_latest() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = StateStore::new(tmp.path());
        let older = task_state(RunId::new(), "older");
        let newer = task_state(RunId::new(), "newer");
        store.write_task_state(&older).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        store.write_task_state(&newer).await.unwrap();

        let state = resolve_resume_state(&store, Some("latest"))
            .await
            .unwrap()
            .unwrap();

        assert_eq!(state.goal, "newer");
    }

    #[tokio::test]
    async fn resolve_resume_state_supports_run_id() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = StateStore::new(tmp.path());
        let target_run_id = RunId::new();
        let target = task_state(target_run_id, "target");
        let other = task_state(RunId::new(), "other");
        store.write_task_state(&target).await.unwrap();
        store.write_task_state(&other).await.unwrap();

        let state = resolve_resume_state(&store, Some(&target_run_id.to_string()))
            .await
            .unwrap()
            .unwrap();

        assert_eq!(state.run_id, target_run_id);
        assert_eq!(state.goal, "target");
    }

    #[tokio::test]
    async fn resolve_resume_state_rejects_invalid_value() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = StateStore::new(tmp.path());

        let err = resolve_resume_state(&store, Some("not-a-run-id"))
            .await
            .unwrap_err();

        assert!(err.to_string().contains("expected latest or run_id"));
    }

    #[test]
    fn old_task_state_without_runtime_identity_deserializes() {
        let state = task_state(RunId::new(), "old");
        let mut value = serde_json::to_value(state).unwrap();
        value.as_object_mut().unwrap().remove("runtime_identity");

        let state: TaskState = serde_json::from_value(value).unwrap();

        assert!(state.runtime_identity.is_none());
    }
}
