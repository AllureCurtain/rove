//! Tail-first reader for newline-delimited JSON.
//!
//! Resume only ever needs the *end* of a trace: the last N history items and
//! the run's closing lifecycle facts. Reading the whole file to get them makes
//! peak memory scale with total run length, which is the wrong shape for a
//! long-lived session. This scanner walks backwards in fixed chunks, so cost
//! scales with how much tail the caller actually consumes.
//!
//! Two properties matter for durability, and both are deliberate:
//!
//! - A malformed record is reported as [`ScanOutcome::Rejected`] rather than
//!   ending the scan. A crash mid-write leaves exactly one bad record at the
//!   tail, and that must not hide the good history in front of it.
//! - [`ReverseJsonlScanner::new_at`] pins the logical end to a byte offset, so
//!   a scan started while a writer is still appending reads a stable prefix
//!   instead of a moving target.

use std::io::{self, Read, Seek, SeekFrom};

use serde::de::DeserializeOwned;

/// Bytes pulled from the file per backwards step.
pub(crate) const READ_CHUNK_SIZE: usize = 64 * 1024;

/// What one record turned out to be.
#[derive(Debug)]
pub enum ScanOutcome<T> {
    /// Valid JSON for the requested type.
    Parsed(T),
    /// Present but undecodable. The scan continues past it.
    Rejected(serde_json::Error),
}

/// Reads JSONL records from the end of a stream towards the start.
pub struct ReverseJsonlScanner<R> {
    reader: R,
    /// Offset the next backwards read will end at; 0 means the start is reached.
    next_chunk_end: u64,
    /// How much of `chunk` is still unconsumed, counted from its front.
    chunk_position: usize,
    chunk: Vec<u8>,
    /// The record being assembled, held reversed because bytes arrive backwards.
    record_reversed: Vec<u8>,
    max_record_bytes: Option<usize>,
    discarding_oversized_record: bool,
}

impl<R> ReverseJsonlScanner<R>
where
    R: Read + Seek,
{
    /// Scan backwards from the current end of the stream.
    pub fn new(mut reader: R) -> io::Result<Self> {
        let end = reader.seek(SeekFrom::End(0))?;
        Self::new_at(reader, end)
    }

    /// Scan backwards from `end_byte_offset` instead of the true end.
    ///
    /// Lets a reader take a stable view of a file another process is still
    /// appending to: everything written after the offset is invisible.
    pub fn new_at(mut reader: R, end_byte_offset: u64) -> io::Result<Self> {
        let file_len = reader.seek(SeekFrom::End(0))?;
        if end_byte_offset > file_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "reverse JSONL scan end is past the end of the stream",
            ));
        }
        Ok(Self {
            reader,
            next_chunk_end: end_byte_offset,
            chunk_position: 0,
            chunk: vec![0; READ_CHUNK_SIZE],
            record_reversed: Vec::new(),
            max_record_bytes: None,
            discarding_oversized_record: false,
        })
    }

    /// Skip records longer than `max_record_bytes` without buffering them.
    ///
    /// One pathological line (a giant tool payload) must not be able to pull
    /// the whole file into memory just because it sits at the tail.
    pub fn with_max_record_bytes(mut self, max_record_bytes: usize) -> Self {
        self.max_record_bytes = Some(max_record_bytes);
        self
    }

    /// Take the next non-blank record, walking towards the start of the stream.
    ///
    /// `Ok(None)` means the start was reached. I/O failures surface as `Err`;
    /// undecodable records surface as [`ScanOutcome::Rejected`] and leave the
    /// scanner usable.
    pub fn scan_next<T>(&mut self) -> io::Result<Option<ScanOutcome<T>>>
    where
        T: DeserializeOwned,
    {
        loop {
            if self.chunk_position == 0 {
                if self.next_chunk_end == 0 {
                    // The start of the stream terminates whatever is buffered.
                    // An oversized record being discarded simply ends here.
                    if self.discarding_oversized_record {
                        self.discarding_oversized_record = false;
                        return Ok(None);
                    }
                    return Ok(self.finish_record());
                }
                self.read_previous_chunk()?;
            }

            let newline = self.chunk[..self.chunk_position]
                .iter()
                .rposition(|byte| *byte == b'\n');
            match newline {
                // A newline inside the chunk closes the record being assembled.
                Some(newline) => {
                    self.absorb(newline + 1, self.chunk_position);
                    self.chunk_position = newline;
                    if self.discarding_oversized_record {
                        self.discarding_oversized_record = false;
                        continue;
                    }
                    if let Some(outcome) = self.finish_record() {
                        return Ok(Some(outcome));
                    }
                }
                // No newline: the record spans further back than this chunk.
                None => {
                    self.absorb(0, self.chunk_position);
                    self.chunk_position = 0;
                }
            }
        }
    }

    fn read_previous_chunk(&mut self) -> io::Result<()> {
        let read_size = usize::try_from(self.next_chunk_end.min(READ_CHUNK_SIZE as u64))
            .map_err(io::Error::other)?;
        self.next_chunk_end -= read_size as u64;
        self.reader.seek(SeekFrom::Start(self.next_chunk_end))?;
        self.reader.read_exact(&mut self.chunk[..read_size])?;
        self.chunk_position = read_size;
        Ok(())
    }

    /// Prepend `self.chunk[from..to]` to the record under assembly, or start
    /// discarding the record once it exceeds the configured ceiling.
    ///
    /// Takes offsets rather than a slice so the borrow of `self.chunk` ends
    /// before `self.record_reversed` is mutated.
    fn absorb(&mut self, from: usize, to: usize) {
        if self.discarding_oversized_record {
            return;
        }
        let fragment_len = to - from;
        let would_be = self.record_reversed.len().saturating_add(fragment_len);
        if self
            .max_record_bytes
            .is_some_and(|max_record_bytes| would_be > max_record_bytes)
        {
            self.record_reversed.clear();
            self.discarding_oversized_record = true;
            return;
        }
        // Bytes arrive back-to-front, so the buffer is kept reversed and
        // flipped once when the record is complete.
        self.record_reversed
            .extend(self.chunk[from..to].iter().rev().copied());
    }

    fn finish_record<T>(&mut self) -> Option<ScanOutcome<T>>
    where
        T: DeserializeOwned,
    {
        self.record_reversed.reverse();
        let outcome = if self.record_reversed.iter().all(u8::is_ascii_whitespace) {
            // Blank lines are separators, not records.
            None
        } else {
            Some(match serde_json::from_slice::<T>(&self.record_reversed) {
                Ok(value) => ScanOutcome::Parsed(value),
                Err(error) => ScanOutcome::Rejected(error),
            })
        };
        self.record_reversed.clear();
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[derive(Debug, serde::Deserialize, serde::Serialize, PartialEq)]
    struct Record {
        n: u64,
    }

    fn scan_all(content: &str) -> Vec<ScanOutcome<Record>> {
        let mut scanner = ReverseJsonlScanner::new(Cursor::new(content.as_bytes().to_vec())).unwrap();
        let mut seen = Vec::new();
        while let Some(outcome) = scanner.scan_next().unwrap() {
            seen.push(outcome);
        }
        seen
    }

    fn parsed(outcomes: &[ScanOutcome<Record>]) -> Vec<u64> {
        outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                ScanOutcome::Parsed(record) => Some(record.n),
                ScanOutcome::Rejected(_) => None,
            })
            .collect()
    }

    #[test]
    fn records_arrive_in_reverse_file_order() {
        let outcomes = scan_all("{\"n\":1}\n{\"n\":2}\n{\"n\":3}\n");
        assert_eq!(parsed(&outcomes), vec![3, 2, 1]);
    }

    #[test]
    fn a_missing_final_newline_still_yields_the_last_record() {
        // A writer killed after the payload but before its newline.
        let outcomes = scan_all("{\"n\":1}\n{\"n\":2}");
        assert_eq!(parsed(&outcomes), vec![2, 1]);
    }

    /// The signature of a crash mid-write: one unparsable record at the tail.
    /// The good history in front of it must still be reachable.
    #[test]
    fn a_torn_tail_record_is_reported_without_ending_the_scan() {
        let outcomes = scan_all("{\"n\":1}\n{\"n\":2}\n{\"n\":\n");
        assert!(matches!(outcomes.first(), Some(ScanOutcome::Rejected(_))));
        assert_eq!(parsed(&outcomes), vec![2, 1]);
    }

    #[test]
    fn blank_lines_are_separators_rather_than_records() {
        let outcomes = scan_all("{\"n\":1}\n\n\n{\"n\":2}\n\n");
        assert_eq!(parsed(&outcomes), vec![2, 1]);
        assert_eq!(outcomes.len(), 2);
    }

    /// Records longer than one chunk must reassemble correctly, since a single
    /// tool result can easily exceed 64 KiB.
    #[test]
    fn records_spanning_several_chunks_reassemble() {
        let big = "x".repeat(READ_CHUNK_SIZE * 2 + 17);
        let content = format!(
            "{{\"n\":1}}\n{}\n{{\"n\":3}}\n",
            serde_json::json!({ "n": 2, "pad": big })
        );
        let outcomes = scan_all(&content);
        assert_eq!(parsed(&outcomes), vec![3, 2, 1]);
    }

    #[test]
    fn an_oversized_record_is_skipped_without_buffering_it() {
        let big = "x".repeat(200_000);
        let content = format!(
            "{{\"n\":1}}\n{}\n{{\"n\":3}}\n",
            serde_json::json!({ "n": 2, "pad": big })
        );
        let mut scanner = ReverseJsonlScanner::new(Cursor::new(content.into_bytes()))
            .unwrap()
            .with_max_record_bytes(4096);
        let mut seen = Vec::new();
        while let Some(outcome) = scanner.scan_next::<Record>().unwrap() {
            seen.push(outcome);
        }
        // The oversized middle record is gone; its neighbours are intact.
        assert_eq!(parsed(&seen), vec![3, 1]);
    }

    #[test]
    fn scanning_from_a_pinned_offset_ignores_later_appends() {
        let prefix = "{\"n\":1}\n{\"n\":2}\n";
        let content = format!("{prefix}{{\"n\":3}}\n");
        let mut scanner = ReverseJsonlScanner::new_at(
            Cursor::new(content.into_bytes()),
            prefix.len() as u64,
        )
        .unwrap();
        let mut seen = Vec::new();
        while let Some(outcome) = scanner.scan_next::<Record>().unwrap() {
            seen.push(outcome);
        }
        assert_eq!(parsed(&seen), vec![2, 1]);
    }

    #[test]
    fn an_end_offset_past_the_stream_is_refused() {
        let error = match ReverseJsonlScanner::new_at(Cursor::new(b"{}\n".to_vec()), 999) {
            Ok(_) => panic!("an end offset past the stream must be refused"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn an_empty_stream_yields_nothing() {
        let outcomes = scan_all("");
        assert!(outcomes.is_empty());
    }

    /// The point of scanning backwards: taking a bounded tail must not read the
    /// whole file. Asserted on bytes actually pulled through the reader.
    #[test]
    fn taking_a_short_tail_reads_only_a_bounded_slice_of_the_file() {
        let mut content = String::new();
        for n in 0..40_000u64 {
            content.push_str(&format!("{{\"n\":{n},\"pad\":\"{}\"}}\n", "y".repeat(64)));
        }
        let total = content.len();
        assert!(total > 2 * 1024 * 1024, "fixture should be multi-megabyte");

        let counting = CountingReader {
            inner: Cursor::new(content.into_bytes()),
            bytes_read: 0,
        };
        let mut scanner = ReverseJsonlScanner::new(counting).unwrap();
        let mut tail = Vec::new();
        for _ in 0..5 {
            match scanner.scan_next::<Record>().unwrap() {
                Some(ScanOutcome::Parsed(record)) => tail.push(record.n),
                Some(ScanOutcome::Rejected(error)) => panic!("unexpected bad record: {error}"),
                None => panic!("fixture ended early"),
            }
        }

        assert_eq!(tail, vec![39_999, 39_998, 39_997, 39_996, 39_995]);
        let bytes_read = scanner.reader.bytes_read;
        assert!(
            bytes_read <= READ_CHUNK_SIZE,
            "reading a 5-record tail pulled {bytes_read} bytes of a {total}-byte file"
        );
    }

    struct CountingReader<R> {
        inner: R,
        bytes_read: usize,
    }

    impl<R: Read> Read for CountingReader<R> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let read = self.inner.read(buf)?;
            self.bytes_read += read;
            Ok(read)
        }
    }

    impl<R: Seek> Seek for CountingReader<R> {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            self.inner.seek(pos)
        }
    }
}
