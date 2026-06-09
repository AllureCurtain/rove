pub mod api;
pub mod cli;
pub mod terminal;

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
