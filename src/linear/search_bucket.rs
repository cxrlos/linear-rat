//! Search's own budget. Linear allows 30 searches a minute on top of the
//! hourly budgets the governor keeps, so searches also draw from a token
//! bucket that holds 30 and refills one every two seconds. A search beyond
//! it is not refused: it takes the next token still to come and waits for
//! it, so a burst goes out in order as the bucket refills.

use std::time::Duration;

use jiff::{SignedDuration, Timestamp};

/// How many searches Linear allows a minute, and so the bucket's size.
const SEARCHES_PER_MINUTE: i32 = 30;

/// How long the bucket takes to win back one token.
const REFILL: SignedDuration = SignedDuration::from_secs(60 / SEARCHES_PER_MINUTE as i64);

#[derive(Debug, Default)]
pub(crate) struct SearchBucket {
    /// When the bucket is full again if nothing more is taken. A search may
    /// go out once at most a full bucket's worth of refills are still to come.
    full_at: Option<Timestamp>,
}

impl SearchBucket {
    /// Takes a token for one search at `now` and says how long that search
    /// must wait before it goes out.
    pub(crate) fn take(&mut self, now: Timestamp) -> Duration {
        let from = self.full_at.map_or(now, |full_at| full_at.max(now));
        let full_at = from.saturating_add(REFILL).unwrap_or(Timestamp::MAX);
        self.full_at = Some(full_at);
        let goes_out = full_at
            .saturating_sub(REFILL * SEARCHES_PER_MINUTE)
            .unwrap_or(Timestamp::MIN);
        Duration::try_from(goes_out.duration_since(now)).unwrap_or(Duration::ZERO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = Timestamp::constant(1_793_696_400, 0);

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_second(NOW.as_second() + seconds).unwrap()
    }

    #[test]
    fn thirty_searches_in_a_burst_go_out_at_once() {
        let mut bucket = SearchBucket::default();

        for _ in 0..30 {
            assert_eq!(bucket.take(NOW), Duration::ZERO);
        }
    }

    #[test]
    fn searches_beyond_thirty_wait_two_seconds_each_in_order() {
        let mut bucket = SearchBucket::default();
        for _ in 0..30 {
            bucket.take(NOW);
        }

        assert_eq!(bucket.take(NOW), Duration::from_secs(2));
        assert_eq!(bucket.take(NOW), Duration::from_secs(4));
        assert_eq!(bucket.take(NOW), Duration::from_secs(6));
    }

    #[test]
    fn the_bucket_refills_one_token_every_two_seconds() {
        let mut bucket = SearchBucket::default();
        for _ in 0..30 {
            bucket.take(NOW);
        }

        assert_eq!(bucket.take(at(2)), Duration::ZERO);
        assert_eq!(bucket.take(at(2)), Duration::from_secs(2));
    }

    #[test]
    fn a_minute_of_quiet_fills_the_bucket_again() {
        let mut bucket = SearchBucket::default();
        for _ in 0..35 {
            bucket.take(NOW);
        }

        for _ in 0..30 {
            assert_eq!(bucket.take(at(200)), Duration::ZERO);
        }
        assert_eq!(bucket.take(at(200)), Duration::from_secs(2));
    }
}
