use crate::interfaces::terminal::view::RunViewState;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TuiFocus {
    Transcript,
    #[default]
    Composer,
}

#[derive(Debug)]
pub struct TuiState {
    pub run: RunViewState,
    pub composer: String,
    pub focus: TuiFocus,
    pub terminal_width: u16,
    pub terminal_height: u16,
    pub should_quit: bool,
}

impl Default for TuiState {
    fn default() -> Self {
        Self {
            run: RunViewState::default(),
            composer: String::new(),
            focus: TuiFocus::Composer,
            terminal_width: 80,
            terminal_height: 24,
            should_quit: false,
        }
    }
}
