#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunViewUpdate {
    RunStarted { user_message: String },
}

#[cfg(test)]
mod tests {
    use crate::interfaces::terminal::action::TerminalAction;
    use crate::interfaces::terminal::view::RunViewUpdate;

    #[test]
    fn terminal_module_exports_view_and_action_types() {
        let update = RunViewUpdate::RunStarted {
            user_message: "hello".to_string(),
        };
        assert_eq!(
            update,
            RunViewUpdate::RunStarted {
                user_message: "hello".to_string()
            }
        );
        assert_eq!(TerminalAction::ShowStatus, TerminalAction::ShowStatus);
    }
}
