//! Product ownership recorded in the run directory that the run belongs to.
//!
//! Codex alignment Phase 5: the runtime index is rebuildable because every fact
//! it holds is also on disk — `trace.jsonl` carries the events and the identity
//! header, `task_state.json` the checkpoints, `report.json` the outcome. The
//! product catalog had no such property. `product_session_id`, the workspace a
//! session runs in, and the session title existed **only** as rows in
//! `product.sqlite`, so losing that file lost the session list permanently
//! while every run's transcript sat intact next to it.
//!
//! So each run directory also records who owns it. The file is small, written
//! once at bind time, and never read on the hot path: it exists so a cold start
//! with a missing catalog can put the sessions back.
//!
//! > 分歧记录（§0.3 规则）: the plan merges the two SQLite files and adds a
//! > `rollouts` table to carry this association. rove keeps them apart — the
//! > runtime index is per-workspace (`<workspace>/.rove/state.sqlite`) and the
//! > product catalog is global (`~/.rove/product.sqlite`), so merging either
//! > direction destroys one of the two properties that separation buys. A
//! > per-run sidecar gets the durability the `rollouts` table was for without
//! > the merge: rove 产品语义 > codex 机制.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rove_runtime::{JobId, RunId, SessionId};
use serde::{Deserialize, Serialize};

use crate::product::{
    ProductOwnershipRecovery, ProductSessionId, ProductSessionRecovery, ProductSessionStatus,
    ProductStore, ProductWorkspaceId, ProductWorkspaceKind, RecoverProductRun,
    RecoverProductSessionOwnership,
};

/// File name inside a run directory. Sits beside `trace.jsonl`.
pub(crate) const OWNERSHIP_FILE_NAME: &str = "product_owner.json";

/// Everything needed to reinsert one run's product rows.
///
/// Deliberately self-contained rather than a set of ids to look up elsewhere:
/// the whole point is to survive the loss of the database that those lookups
/// would go to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProductRunOwnership {
    pub product_session_id: ProductSessionId,
    pub workspace_id: ProductWorkspaceId,
    /// Canonical absolute root, as the catalog stores it. The canonical key is
    /// derived from this on recovery rather than stored, so the two can never
    /// disagree.
    pub workspace_root: PathBuf,
    pub workspace_kind: ProductWorkspaceKind,
    pub workspace_display_name: String,
    pub session_title: String,
    pub ordinal: u64,
    pub runtime_session_id: SessionId,
    pub runtime_job_id: JobId,
    pub runtime_run_id: RunId,
    /// What this run resumed, as it actually happened. Recovery does not replay
    /// it — the chain is relinked from the records that survived, so a lost run
    /// does not break the ones after it — but the recorded link is the only
    /// evidence of the original shape and is worth keeping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed_from_run_id: Option<RunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<ProductSessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_point_run_id: Option<RunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_point_seq: Option<u64>,
    /// When the session was created, so a recovered list sorts as it did.
    pub session_created_at: String,
    pub bound_at: String,
}

impl ProductRunOwnership {
    /// The status a recovered session gets.
    ///
    /// Always non-terminal-looking: the sidecar says who owns a run, never how
    /// it ended, and claiming `running` for a process that is long gone would be
    /// worse than admitting we do not know.
    pub(crate) fn recovered_status(&self) -> ProductSessionStatus {
        ProductSessionStatus::Idle
    }
}

pub(crate) fn ownership_path(run_dir: &Path) -> PathBuf {
    run_dir.join(OWNERSHIP_FILE_NAME)
}

/// Write the sidecar, replacing whatever was there.
///
/// Rewriting rather than skipping-if-present is intentional: a resumed session
/// may be renamed or rebound between runs, and the newest binding is the one
/// worth keeping. Written atomically via a temp file in the same directory so a
/// crash mid-write cannot leave a half-parsed record.
pub(crate) fn write_ownership(
    run_dir: &Path,
    ownership: &ProductRunOwnership,
) -> std::io::Result<()> {
    let payload = serde_json::to_vec_pretty(ownership).map_err(std::io::Error::other)?;
    let final_path = ownership_path(run_dir);
    let temp_path = run_dir.join(format!("{OWNERSHIP_FILE_NAME}.tmp"));
    std::fs::write(&temp_path, &payload)?;
    match std::fs::rename(&temp_path, &final_path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = std::fs::remove_file(&temp_path);
            Err(error)
        }
    }
}

/// Read the sidecar, if the run has one.
///
/// A missing file is `Ok(None)` — runs predating this and runs started outside
/// the product surface legitimately have none. A corrupt file is also
/// `Ok(None)` with a warning: one unreadable run must not abort a recovery
/// sweep over all the others.
pub(crate) fn read_ownership(run_dir: &Path) -> Option<ProductRunOwnership> {
    let path = ownership_path(run_dir);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::warn!(path = %path.display(), "product ownership record is unreadable: {error}");
            return None;
        }
    };
    match serde_json::from_slice::<ProductRunOwnership>(&bytes) {
        Ok(ownership) => Some(ownership),
        Err(error) => {
            tracing::warn!(path = %path.display(), "product ownership record is corrupt: {error}");
            None
        }
    }
}

/// Every `runs` directory the API could recover ownership from.
///
/// Under the contract layout each workspace has its own runtime directory under
/// `<data_root>/workspaces/<storage_key>/`, so recovery sweeps them all: a cold
/// start in one workspace still restores the sessions of the others, which is
/// what the cross-workspace session list needs. Without a data root there is
/// exactly one candidate — the workspace this process was pointed at.
pub(crate) fn candidate_runs_dirs(data_root: Option<&Path>, state_dir: &Path) -> Vec<PathBuf> {
    let Some(data_root) = data_root else {
        return vec![state_dir.join("runs")];
    };
    let mut dirs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(data_root.join("workspaces")) {
        for entry in entries.filter_map(Result::ok) {
            let runs_dir = entry.path().join("runs");
            if runs_dir.is_dir() {
                dirs.push(runs_dir);
            }
        }
    }
    // The active workspace may sit outside the contract layout (an explicit
    // `state_dir` in config), so it is included either way.
    let own = state_dir.join("runs");
    if !dirs.contains(&own) && own.is_dir() {
        dirs.push(own);
    }
    dirs
}

/// Collect every ownership record under a `runs` directory, oldest first.
///
/// Ordering by `(session, ordinal)` matters: recovery inserts bindings in
/// ordinal order and takes the last one as the session's latest, so an
/// arbitrary directory-listing order would leave `latest_run_id` pointing at
/// whichever run the filesystem happened to name first.
pub(crate) fn collect_ownership(runs_dir: &Path) -> Vec<ProductRunOwnership> {
    let Ok(entries) = std::fs::read_dir(runs_dir) else {
        return Vec::new();
    };
    let mut records: Vec<ProductRunOwnership> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| read_ownership(&entry.path()))
        .collect();
    records.sort_by(|left, right| {
        left.product_session_id
            .to_string()
            .cmp(&right.product_session_id.to_string())
            .then(left.ordinal.cmp(&right.ordinal))
    });
    records
}

/// Group a session's records into the store's recovery input.
///
/// A session is recovered whole rather than run by run, because every read of
/// `product_session_runs` validates the chain: contiguous ordinals from 1, each
/// run resuming the one before it. Feeding runs in one at a time can leave rows
/// that exist but cannot be read.
///
/// Session-level fields come from the **newest** record. A session can be
/// renamed or moved between runs, and each record is a snapshot from its own
/// bind time, so the last one written is the closest thing on disk to current
/// truth. The workspace's canonical key is recomputed from the recorded root by
/// the same function the create path uses, so a recovered workspace collides
/// with an existing registration of the same root instead of duplicating it.
///
/// Returns `None` for an empty group, which callers never produce.
pub(crate) fn to_store_input(
    mut records: Vec<ProductRunOwnership>,
) -> Option<RecoverProductSessionOwnership> {
    records.sort_by_key(|record| record.ordinal);
    let newest = records.last()?;
    let canonical_root_text = newest.workspace_root.to_string_lossy().to_string();
    Some(RecoverProductSessionOwnership {
        canonical_key: crate::product::store::canonical_workspace_key(&canonical_root_text),
        canonical_root_text,
        status: newest.recovered_status(),
        product_session_id: newest.product_session_id.clone(),
        workspace_id: newest.workspace_id.clone(),
        workspace_kind: newest.workspace_kind,
        workspace_display_name: newest.workspace_display_name.clone(),
        session_title: newest.session_title.clone(),
        // The oldest record carries the session's own creation time; they should
        // all agree, but the first binding is the one that observed it.
        session_created_at: records
            .first()
            .map(|record| record.session_created_at.clone())
            .unwrap_or_default(),
        runs: records
            .iter()
            .map(|record| RecoverProductRun {
                recorded_ordinal: record.ordinal,
                runtime_session_id: record.runtime_session_id,
                runtime_job_id: record.runtime_job_id,
                runtime_run_id: record.runtime_run_id,
                bound_at: record.bound_at.clone(),
            })
            .collect(),
    })
}

/// Group records by the session that owns them, sessions in id order.
///
/// Records from different `runs` directories can in principle name the same
/// session, so grouping happens across the whole sweep rather than per
/// directory.
fn group_by_session(
    records: Vec<ProductRunOwnership>,
) -> BTreeMap<String, Vec<ProductRunOwnership>> {
    let mut grouped: BTreeMap<String, Vec<ProductRunOwnership>> = BTreeMap::new();
    for record in records {
        grouped
            .entry(record.product_session_id.to_string())
            .or_default()
            .push(record);
    }
    grouped
}

/// Put back every session whose runs are on disk but absent from the catalog.
///
/// Codex alignment Phase 5 acceptance: deleting the catalog and cold-starting
/// brings the session list back. Each session is recovered as a whole chain, and
/// a session that fails is counted rather than propagated — one unusable record
/// must not cost the user every other session.
pub(crate) async fn recover_product_ownership(
    store: &Arc<dyn ProductStore>,
    runs_dirs: &[PathBuf],
) -> ProductOwnershipRecovery {
    let mut summary = ProductOwnershipRecovery::default();
    let mut records = Vec::new();
    for runs_dir in runs_dirs {
        let runs_dir = runs_dir.clone();
        let Ok(found) = tokio::task::spawn_blocking(move || collect_ownership(&runs_dir)).await
        else {
            continue;
        };
        records.extend(found);
    }
    summary.records_found = records.len();

    for (session_id, group) in group_by_session(records) {
        let Some(input) = to_store_input(group) else {
            continue;
        };
        summary.sessions_found += 1;
        match store.recover_session_ownership(input).await {
            Ok(ProductSessionRecovery::Recovered { runs }) => {
                summary.sessions_recovered += 1;
                summary.runs_recovered += runs;
            }
            Ok(ProductSessionRecovery::AlreadyPresent | ProductSessionRecovery::Skipped) => {}
            Err(error) => {
                summary.sessions_failed += 1;
                tracing::warn!(
                    product_session_id = %session_id,
                    "product session ownership could not be recovered: {error}"
                );
            }
        }
    }
    summary
}
