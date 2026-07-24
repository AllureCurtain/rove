use crate::memory::durable::{recall_durable_memory_from_dir_sync, recall_durable_memory_sync};
use crate::memory::paths::MemoryPaths;
use crate::memory::session::{read_session_summary_from_dir_sync, read_session_summary_sync};
use crate::{SessionId, Workspace};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PromptMemory {
    pub durable_index: Option<String>,
    pub session_summary: Option<String>,
}

pub fn load_prompt_memory_sync(
    workspace: &Workspace,
    session_id: SessionId,
    resume_summary: Option<&str>,
    query: &str,
    durable_recall_limit: usize,
) -> std::io::Result<PromptMemory> {
    let durable_index = match recall_durable_memory_sync(workspace, query, durable_recall_limit) {
        Ok(index) => index,
        Err(err) => {
            tracing::warn!(error = %err, "failed to recall durable memory");
            None
        }
    };

    let session_summary = if let Some(summary) = resume_summary {
        Some(summary.to_string())
    } else {
        match read_session_summary_sync(workspace, session_id) {
            Ok(summary) => summary,
            Err(err) => {
                tracing::warn!(error = %err, "failed to read session memory");
                None
            }
        }
    };

    Ok(PromptMemory {
        durable_index,
        session_summary,
    })
}

pub fn load_prompt_memory_from_paths_sync(
    memory_paths: &MemoryPaths,
    session_id: SessionId,
    resume_summary: Option<&str>,
    query: &str,
) -> std::io::Result<PromptMemory> {
    let durable_index = match recall_durable_memory_from_dir_sync(
        &memory_paths.durable_dir,
        query,
        memory_paths.recall_limit,
    ) {
        Ok(index) => index,
        Err(err) => {
            tracing::warn!(error = %err, "failed to recall durable memory");
            None
        }
    };

    let session_summary = if let Some(summary) = resume_summary {
        Some(summary.to_string())
    } else {
        match read_session_summary_from_dir_sync(&memory_paths.session_dir, session_id) {
            Ok(summary) => summary,
            Err(err) => {
                tracing::warn!(error = %err, "failed to read session memory");
                None
            }
        }
    };

    Ok(PromptMemory {
        durable_index,
        session_summary,
    })
}
