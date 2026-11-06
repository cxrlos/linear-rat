//! The budget governor: keeps lrat inside Linear's hourly request and
//! complexity budgets, as Linear reports them in response headers.
//!
//! The rule, applied to each budget separately and the longest wait wins:
//!
//! - A tenth of each budget's limit is a reserve that only urgent requests
//!   (the viewer is waiting on them) may spend. A non-urgent request (every
//!   page after the first, follow-up lookups) that would dip into the reserve
//!   waits until the budget resets.
//! - Below a quarter of the limit, non-urgent requests are paced: each waits
//!   `time to reset × cost / (remaining − reserve)`, which spreads what is left
//!   above the reserve evenly until the reset. A request costs 1 against the
//!   request budget and the last `X-Complexity` seen against the complexity
//!   budget.
//! - After a rate-limited response every request waits, urgent or not, for
//!   `Retry-After` seconds, or else until the exhausted budget resets, or else
//!   a minute.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use reqwest::header::HeaderMap;

/// How the client tells the time and waits. The real one is [`SystemTimer`];
/// tests use a clock that moves only when the client sleeps.
pub trait Timer: Send + Sync + fmt::Debug {
    /// The current time.
    fn now(&self) -> Timestamp;
    /// A future that finishes after `duration`.
    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}

/// The system clock and tokio's timer.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemTimer;

impl Timer for SystemTimer {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }

    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(tokio::time::sleep(duration))
    }
}

/// Whether the viewer is waiting on a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    /// The viewer is waiting: the first page of a load, and writes. Never
    /// slowed for budget, and may spend the reserve.
    Urgent,
    /// Every later page and follow-up lookup, and a detail that only follows
    /// the list's focus. Slowed when the budget runs low.
    Background,
}

/// What lrat knows of its budgets, from the last response that reported them.
#[derive(Debug, Default)]
pub(crate) struct Governor {
    requests: Option<Budget>,
    complexity: Option<Budget>,
    /// Complexity points the last query cost, from `X-Complexity`.
    last_cost: Option<u64>,
    /// No request goes out before this, after a rate-limited response.
    blocked_until: Option<Timestamp>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Budget {
    remaining: u64,
    limit: u64,
    reset: Timestamp,
}

const DEFAULT_BLOCK: Duration = Duration::from_secs(60);

impl Governor {
    /// Takes in the budget headers of a response. Headers that are missing
    /// or unreadable leave what was known before.
    pub(crate) fn observe(&mut self, headers: &HeaderMap) {
        if let Some(budget) = Budget::from_headers(headers, "requests") {
            self.requests = Some(budget);
        }
        if let Some(budget) = Budget::from_headers(headers, "complexity") {
            self.complexity = Some(budget);
        }
        if let Some(cost) = number(headers, "x-complexity") {
            self.last_cost = Some(cost);
        }
    }

    /// Linear refused a request for its rate limit; `retry_after` is its
    /// `Retry-After`, when it sent one.
    pub(crate) fn rate_limited(&mut self, now: Timestamp, retry_after: Option<Duration>) {
        let exhausted_reset = [self.requests, self.complexity]
            .into_iter()
            .flatten()
            .filter(|budget| budget.remaining == 0 && budget.reset > now)
            .map(|budget| budget.reset)
            .min();
        let until = match (retry_after, exhausted_reset) {
            (Some(wait), _) => add(now, wait),
            (None, Some(reset)) => reset,
            (None, None) => add(now, DEFAULT_BLOCK),
        };
        self.blocked_until = Some(until);
    }

    /// How long a request of `urgency` must wait before it goes out at `now`.
    pub(crate) fn wait(&self, urgency: Urgency, now: Timestamp) -> Duration {
        let blocked = self
            .blocked_until
            .map_or(Duration::ZERO, |until| until_then(now, until));
        if urgency == Urgency::Urgent {
            return blocked;
        }
        let cost = self.last_cost.unwrap_or(1).max(1);
        [
            self.requests.map(|budget| budget.wait(1, now)),
            self.complexity.map(|budget| budget.wait(cost, now)),
        ]
        .into_iter()
        .flatten()
        .fold(blocked, Duration::max)
    }
}

impl Budget {
    fn from_headers(headers: &HeaderMap, kind: &str) -> Option<Self> {
        let header = |part: &str| number(headers, &format!("x-ratelimit-{kind}-{part}"));
        let reset_ms = i64::try_from(header("reset")?).ok()?;
        Some(Self {
            remaining: header("remaining")?,
            limit: header("limit")?,
            reset: Timestamp::from_millisecond(reset_ms).ok()?,
        })
    }

    /// The wait for a non-urgent request costing `cost` at `now`.
    fn wait(self, cost: u64, now: Timestamp) -> Duration {
        let to_reset = until_then(now, self.reset);
        let reserve = self.limit / 10;
        if self.remaining < reserve.saturating_add(cost) {
            return to_reset;
        }
        if self.remaining >= self.limit / 4 {
            return Duration::ZERO;
        }
        let spendable = self.remaining - reserve;
        to_reset.mul_f64(cost as f64 / spendable as f64)
    }
}

fn number(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers.get(name)?.to_str().ok()?.trim().parse().ok()
}

fn until_then(now: Timestamp, then: Timestamp) -> Duration {
    Duration::try_from(then.duration_since(now)).unwrap_or(Duration::ZERO)
}

fn add(now: Timestamp, wait: Duration) -> Timestamp {
    SignedDuration::try_from(wait)
        .ok()
        .and_then(|wait| now.checked_add(wait).ok())
        .unwrap_or(Timestamp::MAX)
}

#[cfg(test)]
mod tests {
    use reqwest::header::{HeaderName, HeaderValue};

    use super::*;

    const NOW: Timestamp = Timestamp::constant(1_793_696_400, 0);

    fn headers(pairs: &[(&str, String)]) -> HeaderMap {
        pairs
            .iter()
            .map(|(name, value)| {
                (
                    HeaderName::from_bytes(name.as_bytes()).unwrap(),
                    HeaderValue::from_str(value).unwrap(),
                )
            })
            .collect()
    }

    fn requests(remaining: u64, limit: u64, reset_in_secs: i64) -> HeaderMap {
        let reset = (NOW.as_second() + reset_in_secs) * 1000;
        headers(&[
            ("x-ratelimit-requests-remaining", remaining.to_string()),
            ("x-ratelimit-requests-limit", limit.to_string()),
            ("x-ratelimit-requests-reset", reset.to_string()),
        ])
    }

    #[test]
    fn nothing_waits_before_linear_reports_a_budget() {
        let governor = Governor::default();

        assert_eq!(governor.wait(Urgency::Background, NOW), Duration::ZERO);
    }

    #[test]
    fn a_healthy_budget_slows_nothing() {
        let mut governor = Governor::default();
        governor.observe(&requests(1000, 1500, 600));

        assert_eq!(governor.wait(Urgency::Background, NOW), Duration::ZERO);
    }

    #[test]
    fn below_a_quarter_background_requests_share_what_is_left_until_the_reset() {
        let mut governor = Governor::default();
        // Reserve 150, so 150 requests are left to spread over 600 s.
        governor.observe(&requests(300, 1500, 600));

        assert_eq!(
            governor.wait(Urgency::Background, NOW),
            Duration::from_secs(4)
        );
        assert_eq!(governor.wait(Urgency::Urgent, NOW), Duration::ZERO);
    }

    #[test]
    fn at_the_reserve_background_requests_wait_for_the_reset() {
        let mut governor = Governor::default();
        governor.observe(&requests(150, 1500, 600));

        assert_eq!(
            governor.wait(Urgency::Background, NOW),
            Duration::from_secs(600)
        );
        assert_eq!(governor.wait(Urgency::Urgent, NOW), Duration::ZERO);
    }

    #[test]
    fn the_complexity_budget_paces_by_the_cost_of_the_last_query() {
        let mut governor = Governor::default();
        let reset = (NOW.as_second() + 600) * 1000;
        // Reserve 25 000, so 5 000 points are left: ten 500-point queries.
        governor.observe(&headers(&[
            ("x-ratelimit-complexity-remaining", "30000".to_owned()),
            ("x-ratelimit-complexity-limit", "250000".to_owned()),
            ("x-ratelimit-complexity-reset", reset.to_string()),
            ("x-complexity", "500".to_owned()),
        ]));

        assert_eq!(
            governor.wait(Urgency::Background, NOW),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn a_rate_limit_blocks_urgent_requests_too_for_retry_after() {
        let mut governor = Governor::default();
        governor.rate_limited(NOW, Some(Duration::from_secs(7)));

        assert_eq!(governor.wait(Urgency::Urgent, NOW), Duration::from_secs(7));
        let later = add(NOW, Duration::from_secs(7));
        assert_eq!(governor.wait(Urgency::Urgent, later), Duration::ZERO);
    }

    #[test]
    fn without_retry_after_a_rate_limit_waits_for_the_exhausted_budget_to_reset() {
        let mut governor = Governor::default();
        governor.observe(&requests(0, 1500, 90));
        governor.rate_limited(NOW, None);

        assert_eq!(governor.wait(Urgency::Urgent, NOW), Duration::from_secs(90));
    }

    #[test]
    fn without_any_hint_a_rate_limit_waits_a_minute() {
        let mut governor = Governor::default();
        governor.rate_limited(NOW, None);

        assert_eq!(governor.wait(Urgency::Urgent, NOW), DEFAULT_BLOCK);
    }
}
