pub mod api;
pub mod cli;
pub(crate) mod runtime;
pub mod terminal;
pub mod tui;

#[cfg(test)]
mod tests {
    #[test]
    fn exports_terminal_module() {
        assert_eq!(
            crate::interfaces::terminal::action::TerminalAction::ShowStatus,
            crate::interfaces::terminal::action::TerminalAction::ShowStatus
        );
    }
}
