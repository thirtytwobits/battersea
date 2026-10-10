use crate::{view::Delta, Error, Result};
use std::io::Write;

/// Lossless newline-delimited delta capture with explicit finite capacity.
/// After an I/O failure the writer is poisoned: a partial line must never be
/// followed by apparently successful capture into the same stream.
#[derive(Debug)]
pub struct Capture<W> {
    writer: W,
    max_bytes: usize,
    max_records: usize,
    written_bytes: usize,
    written_records: usize,
    failed: bool,
}
impl<W: Write> Capture<W> {
    pub fn new(writer: W, max_bytes: usize, max_records: usize) -> Result<Self> {
        if max_bytes == 0 || max_records == 0 {
            return Err(Error::Invalid("Capture capacity must be positive".into()));
        }
        Ok(Self {
            writer,
            max_bytes,
            max_records,
            written_bytes: 0,
            written_records: 0,
            failed: false,
        })
    }
    pub fn append(&mut self, delta: &Delta) -> Result<()> {
        if self.failed {
            return Err(Error::Delivery("Capture writer failed previously".into()));
        }
        let mut line = serde_json::to_vec(delta).map_err(|e| Error::Invalid(e.to_string()))?;
        line.push(b'\n');
        if self.written_records >= self.max_records
            || self.written_bytes.saturating_add(line.len()) > self.max_bytes
        {
            return Err(Error::Capacity);
        }
        self.writer
            .write_all(&line)
            .and_then(|_| self.writer.flush())
            .map_err(|e| {
                self.failed = true;
                Error::Delivery(e.to_string())
            })?;
        self.written_bytes += line.len();
        self.written_records += 1;
        Ok(())
    }
    pub fn into_inner(self) -> W {
        self.writer
    }
}
