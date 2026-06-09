use crate::core::types::CallId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalAction {
    SubmitPrompt(String),
    CancelRun,
    ApproveTool { call_id: CallId },
    RejectTool { call_id: CallId },
    SubmitInput { input_id: CallId, answer: String },
    ResumeLatest,
    ResumeRun(String),
    ShowStatus,
    ShowSessions,
    Clear,
    Help,
    Exit,
    Unknown(String),
}

#[cfg(test)]
mod tests {
    use crate::interfaces::terminal::action::TerminalAction;

    #[test]
    fn terminal_action_keeps_resume_target() {
        assert_eq!(
            TerminalAction::ResumeRun("01ABC".to_string()),
            TerminalAction::ResumeRun("01ABC".to_string())
        );
    }
}
