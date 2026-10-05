use std::io::{BufRead, Write};
use std::sync::Arc;

use async_trait::async_trait;

use rove_core::ToolError;
use rove_runtime::types::{CallId, PendingUserInput, UserInputProvider, UserInputRequest};

pub struct StdinInputProvider;

#[async_trait]
impl UserInputProvider for StdinInputProvider {
    async fn begin_input(
        &self,
        _input_id: CallId,
        request: UserInputRequest,
    ) -> Result<PendingUserInput, ToolError> {
        Ok(PendingUserInput::new(async move {
            let mut stdin = std::io::stdin().lock();
            let mut stderr = std::io::stderr().lock();
            prompt_for_input(&request, &mut stdin, &mut stderr).map_err(|err| {
                ToolError::ExecutionFailed {
                    reason: err.to_string(),
                }
            })
        }))
    }
}

pub fn stdin_input_provider() -> Arc<dyn UserInputProvider> {
    Arc::new(StdinInputProvider)
}

fn prompt_for_input<R: BufRead, W: Write>(
    request: &UserInputRequest,
    reader: &mut R,
    writer: &mut W,
) -> std::io::Result<String> {
    writeln!(writer, "\n[input needed] {}", request.prompt)?;
    write!(writer, "answer: ")?;
    writer.flush()?;

    let mut input = String::new();
    reader.read_line(&mut input)?;
    Ok(input.trim_end_matches(&['\r', '\n'][..]).to_string())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use rove_runtime::types::UserInputRequest;

    use super::prompt_for_input;

    #[test]
    fn prompt_for_input_returns_answer_without_line_ending() {
        let request = UserInputRequest {
            prompt: "Which branch should I use?".to_string(),
        };
        let mut input = Cursor::new(b"use main\r\n".to_vec());
        let mut output = Vec::new();

        let answer = prompt_for_input(&request, &mut input, &mut output).unwrap();

        assert_eq!(answer, "use main");
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("[input needed] Which branch should I use?"));
        assert!(output.contains("answer: "));
    }
}
