use std::path::Path;

use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StageLogRow {
    pub schema_version: u32,
    pub run_id: String,
    pub stage: String,
    pub status: StageStatus,
    pub duration_ms: u128,
    pub input_count: usize,
    pub output_count: usize,
    pub message: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StageStatus {
    Completed,
    Failed,
    Skipped,
}

pub async fn append_stage_log(path: &Path, row: &StageLogRow) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    file.write_all(&serde_json::to_vec(row)?).await?;
    file.write_all(b"\n").await?;
    Ok(())
}
