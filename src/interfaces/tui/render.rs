use ratatui::Frame;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::interfaces::tui::state::TuiState;

pub fn render(frame: &mut Frame<'_>, state: &TuiState) {
    let content = if state.composer.is_empty() {
        "rove tui foundation"
    } else {
        state.composer.as_str()
    };
    frame.render_widget(
        Paragraph::new(content).block(Block::default().borders(Borders::ALL).title("rove")),
        frame.area(),
    );
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::interfaces::tui::state::TuiState;

    #[test]
    fn foundation_renderer_draws_with_test_backend() {
        let backend = TestBackend::new(40, 12);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| super::render(frame, &TuiState::default()))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let rendered = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("rove tui foundation"));
    }
}
