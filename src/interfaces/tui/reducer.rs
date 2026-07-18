use crate::interfaces::terminal::action::TerminalAction;
use crate::interfaces::tui::action::TuiAction;
use crate::interfaces::tui::effect::TuiEffect;
use crate::interfaces::tui::state::{TuiFocus, TuiState};

pub fn reduce(state: &mut TuiState, action: TuiAction) -> Vec<TuiEffect> {
    match action {
        TuiAction::Terminal(TerminalAction::Exit) => {
            state.should_quit = true;
            vec![TuiEffect::Exit]
        }
        TuiAction::Terminal(action) => vec![TuiEffect::Dispatch(action)],
        TuiAction::InsertChar(ch) => {
            state.composer.push(ch);
            Vec::new()
        }
        TuiAction::Backspace => {
            state.composer.pop();
            Vec::new()
        }
        TuiAction::SubmitComposer => {
            let message = state.composer.trim().to_string();
            if message.is_empty() {
                Vec::new()
            } else {
                state.composer.clear();
                vec![TuiEffect::Dispatch(TerminalAction::SubmitPrompt(message))]
            }
        }
        TuiAction::FocusNext => {
            state.focus = match state.focus {
                TuiFocus::Transcript => TuiFocus::Composer,
                TuiFocus::Composer => TuiFocus::Transcript,
            };
            Vec::new()
        }
        TuiAction::Resize { width, height } => {
            state.terminal_width = width;
            state.terminal_height = height;
            Vec::new()
        }
        TuiAction::ScrollUp(_) | TuiAction::ScrollDown(_) | TuiAction::Tick => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use crate::interfaces::terminal::action::TerminalAction;
    use crate::interfaces::tui::action::TuiAction;
    use crate::interfaces::tui::effect::TuiEffect;
    use crate::interfaces::tui::state::TuiState;

    use super::reduce;

    #[test]
    fn submitting_composer_produces_typed_terminal_effect() {
        let mut state = TuiState {
            composer: "  hello  ".to_string(),
            ..TuiState::default()
        };

        let effects = reduce(&mut state, TuiAction::SubmitComposer);

        assert_eq!(state.composer, "");
        assert_eq!(
            effects,
            vec![TuiEffect::Dispatch(TerminalAction::SubmitPrompt(
                "hello".to_string()
            ))]
        );
    }
}
