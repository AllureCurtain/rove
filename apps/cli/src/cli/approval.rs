use std::io::{BufRead, Write};
use std::sync::Arc;

use async_trait::async_trait;

use rove_core::ToolError;
use rove_runtime::types::{
    ApprovalDecision, PendingToolApproval, ToolApprovalProvider, ToolApprovalRequest,
};

pub struct StdinApprovalProvider;

#[async_trait]
impl ToolApprovalProvider for StdinApprovalProvider {
    async fn begin_approval(
        &self,
        request: ToolApprovalRequest,
    ) -> Result<PendingToolApproval, ToolError> {
        Ok(PendingToolApproval::new(async move {
            let mut stdin = std::io::stdin().lock();
            let mut stderr = std::io::stderr().lock();
            prompt_for_approval(&request, &mut stdin, &mut stderr)
                .unwrap_or(ApprovalDecision::Reject)
        }))
    }
}

pub fn stdin_approval_provider() -> Arc<dyn ToolApprovalProvider> {
    Arc::new(StdinApprovalProvider)
}

fn prompt_for_approval<R: BufRead, W: Write>(
    request: &ToolApprovalRequest,
    reader: &mut R,
    writer: &mut W,
) -> std::io::Result<ApprovalDecision> {
    writeln!(
        writer,
        "\n[approval needed] {}: {}",
        request.name, request.reason
    )?;
    writeln!(writer, "  args: {}", request.args)?;
    write!(writer, "approve? (y/N): ")?;
    writer.flush()?;

    let mut input = String::new();
    reader.read_line(&mut input)?;
    if matches!(input.trim(), "y" | "Y" | "yes" | "YES" | "Yes") {
        Ok(ApprovalDecision::Approve)
    } else {
        Ok(ApprovalDecision::Reject)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use rove_runtime::types::CallId;

    use super::{ApprovalDecision, ToolApprovalRequest, prompt_for_approval};

    fn request() -> ToolApprovalRequest {
        ToolApprovalRequest {
            call_id: CallId::new(),
            name: "fs_write".to_string(),
            args: serde_json::json!({"path":"approved.txt"}),
            reason: "destructive tool requires explicit approval".to_string(),
        }
    }

    #[test]
    fn accepts_yes_input() {
        let mut input = Cursor::new(b"y\n".to_vec());
        let mut output = Vec::new();
        let decision = prompt_for_approval(&request(), &mut input, &mut output).unwrap();
        assert_eq!(decision, ApprovalDecision::Approve);
    }

    #[test]
    fn rejects_no_input() {
        let mut input = Cursor::new(b"n\n".to_vec());
        let mut output = Vec::new();
        let decision = prompt_for_approval(&request(), &mut input, &mut output).unwrap();
        assert_eq!(decision, ApprovalDecision::Reject);
    }
}
