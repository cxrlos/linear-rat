//! What the due-date prompt accepts: a date, a few words relative to today,
//! or `clear`. Due dates are timeless, so nothing here has a time zone; the
//! caller decides which day "today" is.

use jiff::Span;
use jiff::civil::{Date, Weekday};

/// The forms the prompt takes, for its hint and its errors.
pub const FORMS: &str = "YYYY-MM-DD, +3d, +2w, today, tomorrow, fri or clear";

/// The due date `input` names, counting from `today`, or `None` for
/// `clear`. Case and surrounding spaces do not matter. A weekday is the next
/// one after today, a week ahead when today is that day.
pub fn parse_due_date(input: &str, today: Date) -> Result<Option<Date>, String> {
    let input = input.trim().to_lowercase();
    let unknown = || format!("Use {FORMS}");
    let after = |days: i64| {
        Span::new()
            .try_days(days)
            .and_then(|span| today.checked_add(span))
            .map(Some)
            .map_err(|_| format!("{input} is too far ahead"))
    };
    match input.as_str() {
        "clear" => return Ok(None),
        "today" => return Ok(Some(today)),
        "tomorrow" => return after(1),
        _ => {}
    }
    if let Some(weekday) = weekday(&input) {
        let ahead = (weekday.to_monday_zero_offset() - today.weekday().to_monday_zero_offset())
            .rem_euclid(7);
        return after(i64::from(if ahead == 0 { 7 } else { ahead }));
    }
    if let Some(offset) = input.strip_prefix('+') {
        let (count, unit) = offset.split_at(offset.len().saturating_sub(1));
        let per = match unit {
            "d" => 1,
            "w" => 7,
            _ => return Err(unknown()),
        };
        if count.is_empty() || !count.bytes().all(|b| b.is_ascii_digit()) {
            return Err(unknown());
        }
        let count: i64 = count
            .parse()
            .map_err(|_| format!("{input} is too far ahead"))?;
        return after(count.saturating_mul(per));
    }
    let shape = input.len() == 10
        && input.bytes().enumerate().all(|(index, byte)| match index {
            4 | 7 => byte == b'-',
            _ => byte.is_ascii_digit(),
        });
    if !shape {
        return Err(unknown());
    }
    input
        .parse::<Date>()
        .map(Some)
        .map_err(|_| format!("{input} is not a date"))
}

fn weekday(input: &str) -> Option<Weekday> {
    let weekday = match input {
        "mon" | "monday" => Weekday::Monday,
        "tue" | "tues" | "tuesday" => Weekday::Tuesday,
        "wed" | "wednesday" => Weekday::Wednesday,
        "thu" | "thur" | "thurs" | "thursday" => Weekday::Thursday,
        "fri" | "friday" => Weekday::Friday,
        "sat" | "saturday" => Weekday::Saturday,
        "sun" | "sunday" => Weekday::Sunday,
        _ => return None,
    };
    Some(weekday)
}
