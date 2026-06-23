//! Daily-rotating gzip JSONL writer.
//!
//! Output layout: `<out>/<slug>/<YYYY-MM-DD>/{events,metrics}.jsonl.gz`.
//! One `RotatingGzWriter` per stream (events, metrics). Lines are appended to a
//! gzip stream; the encoder is flushed on a cadence and finished on rotation /
//! shutdown so the file is always a valid `.gz`.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use flate2::write::GzEncoder;
use flate2::Compression;

pub struct RotatingGzWriter {
    base_dir: PathBuf,
    slug: String,
    file_stem: String, // "events" | "metrics"
    current_day: String,
    enc: Option<GzEncoder<File>>,
    lines_since_flush: u64,
}

impl RotatingGzWriter {
    pub fn new(base_dir: PathBuf, slug: &str, file_stem: &str) -> Self {
        Self {
            base_dir,
            slug: slug.to_string(),
            file_stem: file_stem.to_string(),
            current_day: String::new(),
            enc: None,
            lines_since_flush: 0,
        }
    }

    fn day_str() -> String {
        chrono::Utc::now().format("%Y-%m-%d").to_string()
    }

    /// Open (or rotate to) today's file. Idempotent within the same UTC day.
    fn ensure_open(&mut self) -> Result<()> {
        let day = Self::day_str();
        if day == self.current_day && self.enc.is_some() {
            return Ok(());
        }
        // Rotate: finish the old encoder cleanly.
        if let Some(enc) = self.enc.take() {
            let _ = enc.finish();
        }
        let dir = self.base_dir.join(&self.slug).join(&day);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.jsonl.gz", self.file_stem));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        tracing::info!("[WRITER] open {}", path.display());
        self.enc = Some(GzEncoder::new(file, Compression::new(4)));
        self.current_day = day;
        Ok(())
    }

    /// Append one JSON value as a line.
    pub fn write_line(&mut self, line: &str) -> Result<()> {
        self.ensure_open()?;
        if let Some(enc) = self.enc.as_mut() {
            enc.write_all(line.as_bytes())?;
            enc.write_all(b"\n")?;
            self.lines_since_flush += 1;
        }
        Ok(())
    }

    /// Flush the gzip stream to disk (does not close it).
    pub fn flush(&mut self) {
        if let Some(enc) = self.enc.as_mut() {
            if self.lines_since_flush > 0 {
                let _ = enc.flush();
                self.lines_since_flush = 0;
            }
        }
    }

    /// Finish the gzip stream — call on shutdown.
    pub fn finish(&mut self) {
        if let Some(enc) = self.enc.take() {
            let _ = enc.finish();
        }
    }
}

impl Drop for RotatingGzWriter {
    fn drop(&mut self) {
        self.finish();
    }
}
