use std::str;

use thiserror::Error;

use super::Framing;

const DEFAULT_MAX_LINE_BYTES: usize = 1024 * 1024;
const DEFAULT_MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
const MAX_ALLOWED_LINE_BYTES: usize = 16 * 1024 * 1024;
const MAX_ALLOWED_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// Memory bounds for incremental response framing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramingLimits {
    max_line_bytes: usize,
    max_frame_bytes: usize,
}

impl FramingLimits {
    pub fn new(max_line_bytes: usize, max_frame_bytes: usize) -> Result<Self, FramingError> {
        if max_line_bytes == 0 || max_frame_bytes == 0 {
            return Err(FramingError::InvalidLimits);
        }
        if max_line_bytes > MAX_ALLOWED_LINE_BYTES {
            return Err(FramingError::LimitTooLarge {
                kind: "line",
                max: MAX_ALLOWED_LINE_BYTES,
            });
        }
        if max_frame_bytes > MAX_ALLOWED_FRAME_BYTES {
            return Err(FramingError::LimitTooLarge {
                kind: "frame",
                max: MAX_ALLOWED_FRAME_BYTES,
            });
        }
        Ok(Self {
            max_line_bytes,
            max_frame_bytes,
        })
    }

    pub fn max_line_bytes(&self) -> usize {
        self.max_line_bytes
    }

    pub fn max_frame_bytes(&self) -> usize {
        self.max_frame_bytes
    }
}

impl Default for FramingLimits {
    fn default() -> Self {
        Self {
            max_line_bytes: DEFAULT_MAX_LINE_BYTES,
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
        }
    }
}

/// Incremental, byte-safe SSE or JSONL framer.
///
/// Network chunks are retained as bytes until a complete line is available,
/// so a UTF-8 code point split between chunks is never decoded lossily.
pub struct FrameBuffer {
    framing: Framing,
    limits: FramingLimits,
    pending_line: Vec<u8>,
    sse_data_lines: Vec<String>,
    sse_frame_bytes: usize,
}

impl FrameBuffer {
    pub fn new(framing: Framing) -> Self {
        Self::with_limits(framing, FramingLimits::default())
    }

    pub fn with_limits(framing: Framing, limits: FramingLimits) -> Self {
        Self {
            framing,
            limits,
            pending_line: Vec::new(),
            sse_data_lines: Vec::new(),
            sse_frame_bytes: 0,
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<String>, FramingError> {
        let mut frames = Vec::new();
        let mut remaining = chunk;

        while let Some(line_end) = remaining.iter().position(|byte| *byte == b'\n') {
            self.extend_pending(&remaining[..line_end])?;
            self.process_pending_line(&mut frames)?;
            remaining = &remaining[line_end + 1..];
        }

        self.extend_pending(remaining)?;
        Ok(frames)
    }

    pub fn finish(&mut self) -> Result<Vec<String>, FramingError> {
        let mut frames = Vec::new();
        if !self.pending_line.is_empty() {
            self.process_pending_line(&mut frames)?;
        }
        if self.framing == Framing::ServerSentEvents {
            self.dispatch_sse_frame(&mut frames);
        }
        Ok(frames)
    }

    fn extend_pending(&mut self, bytes: &[u8]) -> Result<(), FramingError> {
        let length = self.pending_line.len().saturating_add(bytes.len());
        if length > self.limits.max_line_bytes {
            return Err(FramingError::LineTooLarge {
                limit: self.limits.max_line_bytes,
            });
        }
        self.pending_line.extend_from_slice(bytes);
        Ok(())
    }

    fn process_pending_line(&mut self, frames: &mut Vec<String>) -> Result<(), FramingError> {
        if self.pending_line.last() == Some(&b'\r') {
            self.pending_line.pop();
        }
        let line = str::from_utf8(&self.pending_line)
            .map_err(|_| FramingError::InvalidUtf8)?
            .to_owned();
        self.pending_line.clear();

        match self.framing {
            Framing::JsonLines => {
                let line = line.trim();
                if !line.is_empty() {
                    if line.len() > self.limits.max_frame_bytes {
                        return Err(FramingError::FrameTooLarge {
                            limit: self.limits.max_frame_bytes,
                        });
                    }
                    frames.push(line.to_owned());
                }
            }
            Framing::ServerSentEvents => self.process_sse_line(&line, frames)?,
        }
        Ok(())
    }

    fn process_sse_line(
        &mut self,
        line: &str,
        frames: &mut Vec<String>,
    ) -> Result<(), FramingError> {
        if line.is_empty() {
            self.dispatch_sse_frame(frames);
            return Ok(());
        }
        if line.starts_with(':') {
            return Ok(());
        }

        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        if field != "data" {
            return Ok(());
        }

        let separator_bytes = usize::from(!self.sse_data_lines.is_empty());
        let frame_bytes = self
            .sse_frame_bytes
            .saturating_add(separator_bytes)
            .saturating_add(value.len());
        if frame_bytes > self.limits.max_frame_bytes {
            return Err(FramingError::FrameTooLarge {
                limit: self.limits.max_frame_bytes,
            });
        }
        self.sse_frame_bytes = frame_bytes;
        self.sse_data_lines.push(value.to_owned());
        Ok(())
    }

    fn dispatch_sse_frame(&mut self, frames: &mut Vec<String>) {
        if !self.sse_data_lines.is_empty() {
            frames.push(self.sse_data_lines.join("\n"));
            self.sse_data_lines.clear();
            self.sse_frame_bytes = 0;
        }
    }
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum FramingError {
    #[error("framing limits must be greater than zero")]
    InvalidLimits,
    #[error("provider stream {kind} limit exceeds the {max}-byte maximum")]
    LimitTooLarge { kind: &'static str, max: usize },
    #[error("provider stream line exceeds the {limit}-byte limit")]
    LineTooLarge { limit: usize },
    #[error("provider stream frame exceeds the {limit}-byte limit")]
    FrameTooLarge { limit: usize },
    #[error("provider stream contains invalid UTF-8")]
    InvalidUtf8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_lines_preserve_utf8_split_across_chunks() {
        let input = "{\"text\":\"你好\"}\n{\"done\":true}".as_bytes();
        let first_non_ascii = input.iter().position(|byte| *byte >= 0x80).unwrap();
        let split = first_non_ascii + 1;
        let mut buffer = FrameBuffer::new(Framing::JsonLines);

        assert!(buffer.push(&input[..split]).unwrap().is_empty());
        let mut frames = buffer.push(&input[split..]).unwrap();
        frames.extend(buffer.finish().unwrap());

        assert_eq!(frames, vec!["{\"text\":\"你好\"}", "{\"done\":true}"]);
    }

    #[test]
    fn json_lines_accept_crlf_and_final_line_without_newline() {
        let mut buffer = FrameBuffer::new(Framing::JsonLines);

        let mut frames = buffer.push(b"  {\"one\":1}  \r\n\r\n{\"two\":2}").unwrap();
        frames.extend(buffer.finish().unwrap());

        assert_eq!(frames, vec!["{\"one\":1}", "{\"two\":2}"]);
    }

    #[test]
    fn sse_handles_comments_crlf_and_multiple_data_lines() {
        let mut buffer = FrameBuffer::new(Framing::ServerSentEvents);
        let chunks: [&[u8]; 3] = [
            b": keep-alive\r\ndata: {\"delta\":\"hel",
            b"lo\"}\r\ndata:{\"second\":true}\r",
            b"\n\r\nevent: ignored\ndata: [DONE]\n\n",
        ];

        let mut frames = Vec::new();
        for chunk in chunks {
            frames.extend(buffer.push(chunk).unwrap());
        }

        assert_eq!(
            frames,
            vec!["{\"delta\":\"hello\"}\n{\"second\":true}", "[DONE]"]
        );
    }

    #[test]
    fn sse_finish_dispatches_a_final_unterminated_event() {
        let mut buffer = FrameBuffer::new(Framing::ServerSentEvents);

        assert!(buffer.push(b"data: final").unwrap().is_empty());

        assert_eq!(buffer.finish().unwrap(), vec!["final"]);
    }

    #[test]
    fn framing_limits_reject_oversized_lines_and_sse_frames() {
        let limits = FramingLimits::new(8, 10).unwrap();
        let mut jsonl = FrameBuffer::with_limits(Framing::JsonLines, limits);
        assert_eq!(
            jsonl.push(b"123456789").unwrap_err(),
            FramingError::LineTooLarge { limit: 8 }
        );

        let limits = FramingLimits::new(16, 5).unwrap();
        let mut sse = FrameBuffer::with_limits(Framing::ServerSentEvents, limits);
        assert_eq!(
            sse.push(b"data: 123456\n").unwrap_err(),
            FramingError::FrameTooLarge { limit: 5 }
        );
    }

    #[test]
    fn complete_invalid_utf8_line_is_rejected() {
        let mut buffer = FrameBuffer::new(Framing::JsonLines);

        assert_eq!(
            buffer.push(&[0xff, b'\n']).unwrap_err(),
            FramingError::InvalidUtf8
        );
    }

    #[test]
    fn zero_framing_limits_are_rejected() {
        assert_eq!(
            FramingLimits::new(0, 1).unwrap_err(),
            FramingError::InvalidLimits
        );
        assert_eq!(
            FramingLimits::new(1, 0).unwrap_err(),
            FramingError::InvalidLimits
        );
    }

    #[test]
    fn framing_limits_have_a_hard_upper_bound() {
        assert_eq!(
            FramingLimits::new(MAX_ALLOWED_LINE_BYTES + 1, 1).unwrap_err(),
            FramingError::LimitTooLarge {
                kind: "line",
                max: MAX_ALLOWED_LINE_BYTES,
            }
        );
        assert_eq!(
            FramingLimits::new(1, MAX_ALLOWED_FRAME_BYTES + 1).unwrap_err(),
            FramingError::LimitTooLarge {
                kind: "frame",
                max: MAX_ALLOWED_FRAME_BYTES,
            }
        );
    }
}
