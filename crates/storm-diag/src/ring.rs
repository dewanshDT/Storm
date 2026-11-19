//! An in-process bounded ring of formatted log lines (decision 162c): the
//! last ~2,000 lines either binary has produced, redacted as they enter it
//! so nothing a bundle later reads out of it was ever held in the clear.
//!
//! Composed as a second `tracing-subscriber` `fmt` layer, writing through
//! [`RingWriter`] instead of stdout or the log file, so it reuses the exact
//! same event formatting as the primary layer rather than re-implementing
//! it — the two must print the same line, and only one of them keeps it.

use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::MakeWriter;

use crate::redact::{self, Sensitive};

const DEFAULT_CAPACITY: usize = 2_000;

struct Inner {
    lines: VecDeque<String>,
    capacity: usize,
    sensitive: Vec<Sensitive>,
}

/// A handle to the ring. Cheap to clone — every clone shares the same
/// buffer — so one lives in the subscriber's writer and another in
/// whatever builds a [`crate::Diag`] bundle.
#[derive(Clone)]
pub struct Ring(Arc<Mutex<Inner>>);

impl Ring {
    pub fn new(capacity: usize) -> Self {
        Self(Arc::new(Mutex::new(Inner {
            lines: VecDeque::with_capacity(capacity.min(DEFAULT_CAPACITY)),
            capacity,
            sensitive: Vec::new(),
        })))
    }

    /// Names this process has since learned are sensitive — a vault
    /// created after boot, say — applied to every line redacted from now
    /// on. Does not touch lines already in the ring.
    pub fn set_sensitive(&self, sensitive: Vec<Sensitive>) {
        let mut inner = self.0.lock().expect("ring lock");
        inner.sensitive = sensitive;
    }

    /// Redacts `line` and pushes it, evicting the oldest line if the ring
    /// is full. `line` is one already-formatted event, newline stripped.
    pub fn push(&self, line: &str) {
        if line.is_empty() {
            return;
        }
        let mut inner = self.0.lock().expect("ring lock");
        let redacted = redact::redact(line, &inner.sensitive);
        if inner.lines.len() >= inner.capacity {
            inner.lines.pop_front();
        }
        inner.lines.push_back(redacted);
    }

    /// The last `n` lines, oldest first — what a [`crate::Diag`] bundle
    /// carries. `n` larger than the ring just returns everything in it.
    pub fn tail(&self, n: usize) -> Vec<String> {
        let inner = self.0.lock().expect("ring lock");
        let skip = inner.lines.len().saturating_sub(n);
        inner.lines.iter().skip(skip).cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.0.lock().expect("ring lock").lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for Ring {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

/// [`MakeWriter`] over a [`Ring`]: each `write` call is one formatted
/// event (`tracing-subscriber`'s `fmt` layer writes a whole line per
/// event), split on `\n` defensively, redacted, and pushed.
#[derive(Clone)]
pub struct RingWriter(Ring);

impl RingWriter {
    pub fn new(ring: Ring) -> Self {
        Self(ring)
    }
}

impl io::Write for RingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        for line in text.split('\n') {
            self.0.push(line.trim_end_matches('\r'));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for RingWriter {
    type Writer = RingWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_the_most_recent_lines() {
        let ring = Ring::new(3);
        for i in 0..5 {
            ring.push(&format!("line {i}"));
        }
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.tail(10), vec!["line 2", "line 3", "line 4"]);
    }

    #[test]
    fn tail_never_exceeds_what_is_held() {
        let ring = Ring::new(100);
        ring.push("only line");
        assert_eq!(ring.tail(5), vec!["only line"]);
    }

    #[test]
    fn lines_are_redacted_on_insertion() {
        let ring = Ring::new(10);
        ring.push("Authorization: Bearer stk_SUPERSECRETVALUE123");
        let tail = ring.tail(1);
        assert_eq!(tail.len(), 1);
        assert!(!tail[0].contains("stk_SUPERSECRETVALUE123"));
    }

    #[test]
    fn empty_lines_are_not_stored() {
        let ring = Ring::new(10);
        ring.push("");
        ring.push("real line");
        assert_eq!(ring.tail(10), vec!["real line"]);
    }

    #[test]
    fn writer_splits_multiple_lines_in_one_write() {
        let ring = Ring::new(10);
        let mut writer = RingWriter::new(ring.clone());
        io::Write::write_all(&mut writer, b"first\nsecond\n").unwrap();
        assert_eq!(ring.tail(10), vec!["first", "second"]);
    }

    #[test]
    fn newly_learned_sensitive_names_apply_to_later_lines_only() {
        let ring = Ring::new(10);
        ring.push("opened My Vault/note.md");
        ring.set_sensitive(vec![Sensitive::vault("My Vault")]);
        ring.push("opened My Vault/note.md again");
        let tail = ring.tail(10);
        assert!(tail[0].contains("My Vault"));
        assert!(!tail[1].contains("My Vault"));
    }
}
