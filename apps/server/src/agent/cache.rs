//! The server's cache of a session's terminal output (freeze §11.5; 77c).
//!
//! The host's ring is authoritative; this is a bounded copy with the same
//! addressing. Every byte has a permanent offset, a reader that fell below the
//! floor gets an explicit gap, and the cache accepts bytes **in order only**:
//! - overlap with what it already holds is skipped;
//! - a jump forward starts the ring again at the new offset;
//! - an empty cache takes whatever offset arrives first.
//!
//! The last rule is what makes recovery after a server restart work. The
//! cache comes back empty, `hello` asks the host to replay from 0, and the
//! host's first post, from its own floor, becomes this cache's floor.

use std::collections::VecDeque;

pub const DEFAULT_CAPACITY: usize = 4 * 1024 * 1024;

#[derive(Debug)]
pub struct OutputCache {
    buf: VecDeque<u8>,
    capacity: usize,
    floor: u64,
    initialized: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Read {
    Data {
        from: u64,
        bytes: Vec<u8>,
    },
    Gap {
        from: u64,
        to: u64,
    },
    UpToDate,
    /// Past the end — nothing yet. Not an error here: after a server restart a
    /// client may hold an offset the cache has not been refilled to.
    Ahead,
}

impl OutputCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: VecDeque::new(),
            capacity: capacity.max(1),
            floor: 0,
            initialized: false,
        }
    }

    #[cfg(test)]
    pub fn floor(&self) -> u64 {
        self.floor
    }

    pub fn end(&self) -> u64 {
        self.floor + self.buf.len() as u64
    }

    /// Whether anything has arrived since the cache was created.
    pub fn initialized(&self) -> bool {
        self.initialized
    }

    /// Accepts `bytes` that start at `offset`, in order only (see the module).
    pub fn insert(&mut self, offset: u64, bytes: &[u8]) {
        if !self.initialized || offset > self.end() {
            self.buf.clear();
            self.floor = offset;
            self.initialized = true;
        }
        let end = self.end();
        let last = offset + bytes.len() as u64;
        if last <= end {
            return;
        }
        let fresh = &bytes[(end - offset) as usize..];
        let keep = &fresh[fresh.len().saturating_sub(self.capacity)..];
        let skipped = (fresh.len() - keep.len()) as u64;
        if skipped > 0 {
            self.floor += self.buf.len() as u64 + skipped;
            self.buf.clear();
        }
        let overflow = (self.buf.len() + keep.len()).saturating_sub(self.capacity);
        self.buf.drain(..overflow);
        self.floor += overflow as u64;
        self.buf.extend(keep);
    }

    pub fn read(&self, from: u64, max: usize) -> Read {
        if !self.initialized {
            return if from == 0 {
                Read::UpToDate
            } else {
                Read::Ahead
            };
        }
        let end = self.end();
        if from > end {
            return Read::Ahead;
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

    #[test]
    fn in_order_bytes_append_and_overlap_is_skipped() {
        let mut c = OutputCache::new(64);
        c.insert(0, b"hello ");
        c.insert(3, b"lo world"); // overlaps "lo "
        assert_eq!(c.end(), 11);
        assert_eq!(
            c.read(0, 100),
            Read::Data {
                from: 0,
                bytes: b"hello world".to_vec()
            }
        );
    }

    #[test]
    fn an_empty_cache_takes_the_first_offset_as_its_floor() {
        // After a server restart: the host replays from its own floor.
        let mut c = OutputCache::new(64);
        assert_eq!(c.read(0, 10), Read::UpToDate);
        assert_eq!(c.read(500, 10), Read::Ahead);
        c.insert(1000, b"tail");
        assert_eq!(c.read(0, 10), Read::Gap { from: 0, to: 1000 });
        assert_eq!(
            c.read(1000, 10),
            Read::Data {
                from: 1000,
                bytes: b"tail".to_vec()
            }
        );
    }

    #[test]
    fn a_jump_forward_restarts_the_ring_with_an_explicit_gap() {
        let mut c = OutputCache::new(64);
        c.insert(0, b"abc");
        c.insert(10, b"xyz");
        assert_eq!(c.read(0, 10), Read::Gap { from: 0, to: 10 });
        assert_eq!(c.end(), 13);
    }

    #[test]
    fn capacity_drops_the_oldest() {
        let mut c = OutputCache::new(4);
        c.insert(0, b"0123456789");
        assert_eq!((c.floor(), c.end()), (6, 10));
        c.insert(10, b"ab");
        assert_eq!((c.floor(), c.end()), (8, 12));
    }
}
