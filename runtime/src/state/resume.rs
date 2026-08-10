use crate::runtime_identity::{
    RuntimeIdentity, RuntimeIdentityEvaluation, evaluate_runtime_identity,
};
use crate::state::reconcile::reconcile_task_state_with_trace;
use crate::state::store::StateStore;
use crate::types::{RunId, TaskState};

pub async fn resolve_resume_state(
    state_store: &StateStore,
    resume: Option<&str>,
) -> anyhow::Result<Option<TaskState>> {
    let Some(value) = resume else {
        return Ok(None);
    };

    let mut state = if value == "latest" {
        match state_store.load_latest_task_state().await? {
            Some(state) => state,
            None => return Ok(None),
        }
    } else {
        let run_id = ulid::Ulid::from_string(value).map_err(|_| {
            anyhow::anyhow!("unsupported --resume value: {value}; expected latest or run_id")
        })?;
        state_store.load_task_state(RunId(run_id)).await?
    };

    reconcile_resume_state(state_store, &mut state).await;
    Ok(Some(state))
}

/// Bring a loaded snapshot up to date with durable trace facts written after
/// its last checkpoint.
///
/// The snapshot is written after the trace line, so a crash between those two
/// writes leaves lifecycle facts only in `trace.jsonl`. Reconciliation is a
/// projection of already-recorded facts and never replays completed work.
///
/// A reconciliation failure is non-fatal: resume proceeds from the durable
/// snapshot, which is a strictly older but internally consistent state.
async fn reconcile_resume_state(state_store: &StateStore, state: &mut TaskState) {
    let run_dir = state_store.run_store.run_dir(&state.run_id);
    match reconcile_task_state_with_trace(&run_dir, state).await {
        Ok(outcome) if outcome.applied_event_count > 0 || outcome.corrupt_line_count > 0 => {
            tracing::info!(
                run_id = %state.run_id,
                applied = outcome.applied_event_count,
                corrupt = outcome.corrupt_line_count,
                changed = outcome.changed,
                last_event_seq = ?outcome.last_event_seq,
                "Reconciled resume snapshot with newer canonical trace facts"
            );
        }
        Ok(_) => {}
        Err(error) => tracing::warn!(
            run_id = %state.run_id,
            %error,
            "Failed to reconcile resume snapshot with trace; resuming from durable snapshot"
        ),
    }
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
    use crate::state::store::StateStore;
    use crate::types::{JobId, RunId, SessionId, TaskState};

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
            agent_profile: None,
            step_ledger: Default::default(),
            execution_lifecycle: Default::default(),
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
    async fn resolve_resume_state_rejects_a_mismatched_artifact_identity() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = StateStore::new(tmp.path());
        let requested = RunId::new();
        let artifact = task_state(RunId::new(), "wrong identity");
        let path = store.run_store.run_dir(&requested).join("task_state.json");
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, serde_json::to_vec(&artifact).unwrap())
            .await
            .unwrap();

        let error = resolve_resume_state(&store, Some(&requested.to_string()))
            .await
            .unwrap_err();
        assert_eq!(
            error
                .chain()
                .find_map(|cause| cause.downcast_ref::<std::io::Error>())
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::InvalidData)
        );
    }

    #[tokio::test]
    async fn resume_claim_is_atomic_and_requires_a_terminal_run() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = StateStore::new(tmp.path());
        let state = task_state(RunId::new(), "claim me");
        store.write_task_state(&state).await.unwrap();

        assert!(
            store
                .index
                .claim_job_for_resume_async(state.job_id, state.run_id)
                .await
                .unwrap()
                .is_none()
        );

        store
            .index
            .record_report(
                state.run_id,
                &store.run_store.run_dir(&state.run_id).join("report.json"),
                "success",
                "final",
            )
            .unwrap();
        let claim = store
            .index
            .claim_job_for_resume_async(state.job_id, state.run_id)
            .await
            .unwrap()
            .expect("terminal job should be claimable");
        assert!(
            store
                .index
                .claim_job_for_resume_async(state.job_id, state.run_id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .index
                .release_job_resume_claim_async(claim)
                .await
                .unwrap()
        );
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
    fn old_task_state_without_runtime_identity_or_step_ledger_deserializes() {
        let state = task_state(RunId::new(), "old");
        let mut value = serde_json::to_value(state).unwrap();
        value.as_object_mut().unwrap().remove("runtime_identity");
        value.as_object_mut().unwrap().remove("step_ledger");

        let state: TaskState = serde_json::from_value(value).unwrap();

        assert!(state.runtime_identity.is_none());
        assert!(state.step_ledger.is_empty());
    }
}
