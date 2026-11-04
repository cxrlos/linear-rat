use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};

/// How many weeks the Analytics screen looks back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeekRange {
    Four,
    Twelve,
    TwentySix,
}

impl WeekRange {
    /// The number of weeks in the range.
    pub fn weeks(self) -> usize {
        match self {
            WeekRange::Four => 4,
            WeekRange::Twelve => 12,
            WeekRange::TwentySix => 26,
        }
    }
}

/// The weeks analytics are bucketed into, and the moment they are measured
/// at. Weeks start on Monday at midnight in `time_zone` and the last one is
/// the week containing `now`.
#[derive(Debug, Clone)]
pub struct Period {
    now: Timestamp,
    time_zone: TimeZone,
    week_starts: Vec<Date>,
}

impl Period {
    /// The `range` weeks ending with the week containing `now` in `time_zone`.
    pub fn new(now: Timestamp, time_zone: TimeZone, range: WeekRange) -> Self {
        let current = monday_of(now.to_zoned(time_zone.clone()).date());
        let week_starts = (0..range.weeks())
            .rev()
            .map(|back| current.saturating_sub((back as i64).weeks()))
            .collect();
        Self {
            now,
            time_zone,
            week_starts,
        }
    }

    /// The moment issue age, stale issues and the active cycle are measured at.
    pub fn now(&self) -> Timestamp {
        self.now
    }

    /// The time zone weeks and days are counted in.
    pub fn time_zone(&self) -> &TimeZone {
        &self.time_zone
    }

    /// The Monday of every week in the range, oldest first.
    pub fn week_starts(&self) -> &[Date] {
        &self.week_starts
    }

    /// When each week ends, oldest first: midnight at the start of the next
    /// Monday, or now for the current week.
    pub(crate) fn week_ends(&self) -> Vec<Timestamp> {
        self.week_starts
            .iter()
            .map(|start| {
                start
                    .saturating_add(7.days())
                    .to_zoned(self.time_zone.clone())
                    .map_or(self.now, |midnight| midnight.timestamp())
                    .min(self.now)
            })
            .collect()
    }

    /// The calendar day `at` falls on in the period's time zone.
    pub(crate) fn day_of(&self, at: Timestamp) -> Date {
        at.to_zoned(self.time_zone.clone()).date()
    }

    /// The index into [`Period::week_starts`] of the week `at` falls in, or
    /// `None` outside the range.
    pub(crate) fn week_of(&self, at: Timestamp) -> Option<usize> {
        self.week_starts
            .binary_search(&monday_of(self.day_of(at)))
            .ok()
    }

    pub(crate) fn contains(&self, at: Timestamp) -> bool {
        self.week_of(at).is_some()
    }
}

fn monday_of(day: Date) -> Date {
    let since_monday = i64::from(day.weekday().to_monday_zero_offset());
    day.saturating_sub(since_monday.days())
}
