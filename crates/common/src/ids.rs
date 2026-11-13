use std::{
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

/// Custom epoch: 2025-01-01T00:00:00Z. Gives ~69 years of 41-bit timestamps.
pub const EPOCH_MS: u64 = 1_735_689_600_000;

const SEQ_BITS: u64 = 12;
const MAX_SEQ: u64 = (1 << SEQ_BITS) - 1;
const MAX_WORKER: u64 = 1023;

/// Snowflake IDs are monotonic per worker, sortable, and usable as a cursor.
///
/// Layout: 41 bits milliseconds since [`EPOCH_MS`], 10 bits worker, 12 bits sequence.
#[derive(Debug)]
pub struct Snowflake {
    worker: u64,
    /// Packed `last_ms << 12 | sequence`.
    state: Mutex<u64>,
}

impl Snowflake {
    pub fn new(worker: u64) -> Self {
        Self {
            worker: worker.min(MAX_WORKER),
            state: Mutex::new(0),
        }
    }

    pub fn next_id(&self) -> i64 {
        let now = now_ms();
        let mut state = match self.state.lock() {
            Ok(guard) => guard,
            // A poisoned mutex still holds a consistent packed integer.
            Err(poisoned) => poisoned.into_inner(),
        };

        let last_ms = *state >> SEQ_BITS;
        let mut ms = now.max(last_ms);
        let mut seq = if ms == last_ms {
            (*state & MAX_SEQ) + 1
        } else {
            0
        };

        if seq > MAX_SEQ {
            // Sequence exhausted within the same millisecond: wait for the next one.
            ms = (last_ms + 1).max(now);
            seq = 0;
        }

        *state = (ms << SEQ_BITS) | seq;
        let id = ((ms - EPOCH_MS) << 22) | (self.worker << SEQ_BITS) | seq;
        id as i64
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(EPOCH_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_monotonic() {
        let ids = Snowflake::new(3);
        let a = ids.next_id();
        let b = ids.next_id();
        assert!(b > a, "{b} should be greater than {a}");
    }

    #[test]
    fn worker_bits_are_encoded() {
        let ids = Snowflake::new(7);
        let id = ids.next_id();
        assert_eq!((id >> 12) & 0x3ff, 7);
    }

    #[test]
    fn worker_is_clamped() {
        let ids = Snowflake::new(99_999);
        let id = ids.next_id();
        assert_eq!((id >> 12) & 0x3ff, 1023);
    }
}
