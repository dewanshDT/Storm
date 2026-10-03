//! Terminal output, addressed by absolute byte offset (freeze §11.2, §11.5).
//!
//! A session's output is one byte stream, and every byte has a permanent
//! offset: the first byte the session ever wrote is offset 0. Readers resume
//! by offset rather than by event id, so a resumed stream is exact, and a
//! reader that fell behind what is retained gets an explicit [`Read::Gap`] —
//! never quietly fewer bytes than it asked for. That gap is what the client's
//! `gap` event reports.
//!
//! The host's ring is authoritative; the server keeps a cache of the same
//! shape. Both are bounded (`scrollback_bytes`, default 4 MiB per session),
//! because an agent can print without limit and a host runs several.

use std::collections::VecDeque;

/// The default retained output per session (freeze §5.7). An operational
/// default, not an architectural limit.
pub const DEFAULT_CAPACITY: usize = 4 * 1024 * 1024;

/// A fixed-capacity ring of a session's most recent output.
#[derive(Debug)]
pub struct Scrollback {
    buf: VecDeque<u8>,
    capacity: usize,
    /// The offset of `buf[0]`: everything below it has been dropped.
    floor: u64,
}

/// The answer to a read.
#[derive(Debug, PartialEq, Eq)]
pub enum Read {
    /// Retained output starting at `from`, which is the offset asked for.
    Data { from: u64, bytes: Vec<u8> },
    /// `from..to` is no longer retained. Read again from `to`.
    Gap { from: u64, to: u64 },
    /// Nothing past `from` yet; wait for more output.
    UpToDate,
    /// `from` is past the end of the stream, which no reader of this stream
    /// can have legitimately seen. The caller is talking about some other
    /// stream (a host that restarted, say), and must not be told "up to date".
    Beyond { end: u64 },
}

impl Scrollback {
    /// # Panics
    /// On a zero capacity, which could retain nothing.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "a scrollback must retain something");
        Self {
            buf: VecDeque::new(),
            capacity,
            floor: 0,
        }
    }

    /// The offset of the oldest retained byte.
    pub fn floor(&self) -> u64 {
        self.floor
    }

    /// The offset one past the newest byte: the total ever written.
    pub fn end(&self) -> u64 {
        self.floor + self.buf.len() as u64
    }

    /// Appends output, dropping the oldest bytes beyond the capacity.
    pub fn append(&mut self, bytes: &[u8]) {
        // A single burst larger than the ring keeps only its tail. Offsets
        // still count every byte of it, so the stream's addressing is intact.
        let keep = &bytes[bytes.len().saturating_sub(self.capacity)..];
        let skipped = (bytes.len() - keep.len()) as u64;
        if skipped > 0 {
            self.floor += self.buf.len() as u64 + skipped;
            self.buf.clear();
        }
        let overflow = (self.buf.len() + keep.len()).saturating_sub(self.capacity);
        self.buf.drain(..overflow);
        self.floor += overflow as u64;
        self.buf.extend(keep);
    }

    /// Reads at most `max` bytes starting at offset `from`.
    pub fn read(&self, from: u64, max: usize) -> Read {
        let end = self.end();
        if from > end {
            return Read::Beyond { end };
        }
        if from < self.floor {
            return Read::Gap {
                from,
                to: self.floor,
            };
        }
        if from == end || max == 0 {
            return Read::UpToDate;
        }
        let start = (from - self.floor) as usize;
        let len = max.min(self.buf.len() - start);
        Read::Data {
            from,
            bytes: self.buf.range(start..start + len).copied().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(read: Read) -> (u64, Vec<u8>) {
        match read {
            Read::Data { from, bytes } => (from, bytes),
            other => panic!("expected data, got {other:?}"),
        }
    }

    #[test]
    fn a_read_resumes_exactly_where_it_left_off() {
        let mut sb = Scrollback::new(64);
        sb.append(b"hello ");
        sb.append(b"world");
        assert_eq!(sb.end(), 11);
        assert_eq!(data(sb.read(0, 6)), (0, b"hello ".to_vec()));
        assert_eq!(data(sb.read(6, 100)), (6, b"world".to_vec()));
        assert_eq!(sb.read(11, 100), Read::UpToDate);
    }

    #[test]
    fn dropped_output_is_an_explicit_gap_never_a_short_read() {
        // A reader that fell behind must be told what it missed (the client's
        // `gap` event), not handed whatever happens to be left.
        let mut sb = Scrollback::new(8);
        sb.append(b"0123456789AB");
        assert_eq!(sb.floor(), 4);
        assert_eq!(sb.end(), 12);
        assert_eq!(sb.read(1, 100), Read::Gap { from: 1, to: 4 });
        assert_eq!(data(sb.read(4, 100)), (4, b"456789AB".to_vec()));
    }

    #[test]
    fn appends_past_capacity_drop_the_oldest_bytes() {
        let mut sb = Scrollback::new(4);
        sb.append(b"ab");
        sb.append(b"cd");
        sb.append(b"ef");
        assert_eq!((sb.floor(), sb.end()), (2, 6));
        assert_eq!(data(sb.read(2, 100)), (2, b"cdef".to_vec()));
    }

    #[test]
    fn a_burst_larger_than_the_ring_keeps_its_tail_and_every_offset() {
        let mut sb = Scrollback::new(4);
        sb.append(b"xy");
        sb.append(b"0123456789");
        assert_eq!((sb.floor(), sb.end()), (8, 12));
        assert_eq!(data(sb.read(8, 100)), (8, b"6789".to_vec()));
    }

    #[test]
    fn an_offset_past_the_end_is_not_up_to_date() {
        // A client holding offset 500 against a stream of 3 bytes is talking
        // about another stream; "up to date" would leave it waiting forever.
        let mut sb = Scrollback::new(16);
        sb.append(b"abc");
        assert_eq!(sb.read(500, 10), Read::Beyond { end: 3 });
    }
}
