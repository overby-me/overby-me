//! Systemd calendar expression parser and evaluator.
//!
//! Implements the `OnCalendar=` specification from `systemd.time(7)`.
//! Parses expressions like:
//! - Shorthands: `minutely`, `hourly`, `daily`, `weekly`, `monthly`, `yearly`, `quarterly`
//! - Full expressions: `*-*-* 06:00:00`, `Mon *-*-* 00:00:00`, `*-*-1,15 12:00:00`
//! - Ranges and lists: `Mon..Fri *-*-* 09:00`, `*-01,04,07,10-01 00:00:00`
//! - Repetitions: `*-*-* *:*:00/15` (every 15 seconds), `*-*-* 00/2:00:00` (every 2 hours)
//!
//! The key function is [`CalendarSpec::next_elapse`] which computes the next
//! matching wall-clock time after a given reference.

use std::fmt;

/// A parsed systemd calendar specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarSpec {
    /// Original expression string.
    pub original: String,
    /// Day-of-week filter (None = any day). 0=Mon .. 6=Sun.
    pub weekdays: Option<Vec<WeekdayRange>>,
    /// Year component (None = wildcard).
    pub year: Option<CalendarComponent>,
    /// Month component.
    pub month: CalendarComponent,
    /// Day-of-month component.
    pub day: CalendarComponent,
    /// Hour component.
    pub hour: CalendarComponent,
    /// Minute component.
    pub minute: CalendarComponent,
    /// Second component.
    pub second: CalendarComponent,
}

/// A range of weekdays (inclusive), e.g. Mon..Fri or just Wed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeekdayRange {
    /// Start day: 0=Mon, 6=Sun.
    pub start: u8,
    /// End day (inclusive): 0=Mon, 6=Sun.
    pub end: u8,
}

/// A calendar component that describes which values match.
/// For example, `*` matches everything, `1,15` matches 1 and 15,
/// `1..5` matches 1 through 5, `*/2` matches every 2nd value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalendarComponent {
    /// `*` — matches any value.
    Wildcard,
    /// `*/N` — matches values at interval N starting from the range minimum.
    WildcardRepeat(u32),
    /// A list of individual values, ranges, or repeated ranges.
    List(Vec<CalendarValue>),
}

/// A single value or range within a calendar component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalendarValue {
    /// A single exact value.
    Exact(u32),
    /// A range `start..end` (inclusive).
    Range(u32, u32),
    /// A value with repetition: `start/step` — matches start, start+step, start+2*step, ...
    Repeat(u32, u32),
    /// A range with repetition: `start..end/step`.
    RangeRepeat(u32, u32, u32),
}

/// A simple wall-clock time for calendar computations.
/// We avoid pulling in chrono by doing our own civil-date arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DateTime {
    pub year: i32,
    pub month: u32,  // 1..=12
    pub day: u32,    // 1..=31
    pub hour: u32,   // 0..=23
    pub minute: u32, // 0..=59
    pub second: u32, // 0..=59
}

impl CalendarSpec {
    /// Parse a systemd calendar expression.
    pub fn parse(input: &str) -> Result<Self, String> {
        let input = input.trim();
        if input.is_empty() {
            return Err("Empty calendar expression".to_string());
        }

        // Handle well-known shorthands — parse the expanded form but keep
        // the original shorthand name so `as_fixed_interval()` can recognise it.
        let shorthand_expansion = match input.to_lowercase().as_str() {
            "minutely" => Some("*-*-* *:*:00"),
            "hourly" => Some("*-*-* *:00:00"),
            "daily" => Some("*-*-* 00:00:00"),
            "monthly" => Some("*-*-01 00:00:00"),
            "weekly" => Some("Mon *-*-* 00:00:00"),
            "yearly" | "annually" => Some("*-01-01 00:00:00"),
            "quarterly" => Some("*-01,04,07,10-01 00:00:00"),
            "semiannually" | "semi-annually" => Some("*-01,07-01 00:00:00"),
            _ => None,
        };

        if let Some(expanded) = shorthand_expansion {
            let mut spec = Self::parse(expanded)?;
            spec.original = input.to_string();
            return Ok(spec);
        }

        let original = input.to_string();
        let parts: Vec<&str> = input.split_whitespace().collect();

        let (weekdays, date_time_parts) = if !parts.is_empty() && looks_like_weekday(parts[0]) {
            let wd = parse_weekdays(parts[0])?;
            (Some(wd), &parts[1..])
        } else {
            (None, &parts[..])
        };

        let (date_part, time_part) = match date_time_parts.len() {
            0 => {
                // Just weekday — means "00:00:00" on that day
                (None, None)
            }
            1 => {
                let p = date_time_parts[0];
                if p.contains('-') {
                    // date only
                    (Some(p), None)
                } else if p.contains(':') {
                    // time only
                    (None, Some(p))
                } else {
                    // Bare token without `-` or `:` — upstream rejects
                    // these (e.g. `systemd-analyze calendar 1` fails).
                    return Err(format!("Invalid calendar expression: {input}"));
                }
            }
            2 => (Some(date_time_parts[0]), Some(date_time_parts[1])),
            _ => return Err(format!("Too many parts in calendar expression: {input}")),
        };

        // Parse date: YYYY-MM-DD or *-MM-DD or MM-DD etc.
        let (year, month, day) = if let Some(d) = date_part {
            parse_date_part(d)?
        } else {
            (
                None,
                CalendarComponent::Wildcard,
                CalendarComponent::Wildcard,
            )
        };

        // Parse time: HH:MM:SS or HH:MM
        let (hour, minute, second) = if let Some(t) = time_part {
            parse_time_part(t)?
        } else {
            (
                CalendarComponent::List(vec![CalendarValue::Exact(0)]),
                CalendarComponent::List(vec![CalendarValue::Exact(0)]),
                CalendarComponent::List(vec![CalendarValue::Exact(0)]),
            )
        };

        // Validate ranges: systemd rejects calendar expressions whose
        // components fall outside wall-clock bounds (e.g. `*-* 99:*:*`).
        // Upstream: TEST-65-ANALYZE asserts these error out.
        validate_component_range(&month, 1, 12, "month")?;
        validate_component_range(&day, 1, 31, "day")?;
        validate_component_range(&hour, 0, 23, "hour")?;
        validate_component_range(&minute, 0, 59, "minute")?;
        validate_component_range(&second, 0, 60, "second")?;

        Ok(CalendarSpec {
            original,
            weekdays,
            year,
            month,
            day,
            hour,
            minute,
            second,
        })
    }

    /// Compute the next wall-clock time at or after `after` that matches this spec.
    /// Returns `None` if no match can be found (e.g., impossible date).
    ///
    /// The caller converts between `Instant`/system-clock and `DateTime` as needed.
    pub fn next_elapse(&self, after: DateTime) -> Option<DateTime> {
        // Brute-force forward search with early pruning.
        // We iterate year→month→day→hour→minute→second, skipping non-matching values.
        // Safety limit: don't search more than 5 years ahead.
        let max_year = after.year + 5;

        let years = matching_values_from(
            self.year.as_ref().unwrap_or(&CalendarComponent::Wildcard),
            after.year as u32,
            max_year as u32,
        );

        for y in years {
            let y_i32 = y as i32;
            let m_start = if y_i32 == after.year { after.month } else { 1 };

            let months = matching_values_from(&self.month, m_start, 12);

            for m in months {
                let max_day = days_in_month(y_i32, m);
                let d_start = if y_i32 == after.year && m == after.month {
                    after.day
                } else {
                    1
                };

                let days = matching_values_from(&self.day, d_start, max_day);

                for d in days {
                    if d > max_day {
                        break;
                    }

                    // Check weekday filter
                    if let Some(ref wds) = self.weekdays {
                        let wd = weekday(y_i32, m, d);
                        if !wds.iter().any(|r| {
                            if r.start <= r.end {
                                wd >= r.start && wd <= r.end
                            } else {
                                // wrap-around: e.g. Sat..Mon
                                wd >= r.start || wd <= r.end
                            }
                        }) {
                            continue;
                        }
                    }

                    let h_start = if y_i32 == after.year && m == after.month && d == after.day {
                        after.hour
                    } else {
                        0
                    };

                    let hours = matching_values_from(&self.hour, h_start, 23);

                    for h in hours {
                        let min_start = if y_i32 == after.year
                            && m == after.month
                            && d == after.day
                            && h == after.hour
                        {
                            after.minute
                        } else {
                            0
                        };

                        let minutes = matching_values_from(&self.minute, min_start, 59);

                        for mi in minutes {
                            let s_start = if y_i32 == after.year
                                && m == after.month
                                && d == after.day
                                && h == after.hour
                                && mi == after.minute
                            {
                                after.second
                            } else {
                                0
                            };

                            let seconds = matching_values_from(&self.second, s_start, 59);

                            if let Some(&s) = seconds.first() {
                                return Some(DateTime {
                                    year: y_i32,
                                    month: m,
                                    day: d,
                                    hour: h,
                                    minute: mi,
                                    second: s,
                                });
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Produce a normalized representation of the calendar expression.
    pub fn normalized(&self) -> String {
        let mut s = String::new();

        if let Some(ref wds) = self.weekdays {
            let wd_strs: Vec<String> = wds
                .iter()
                .map(|r| {
                    if r.start == r.end {
                        weekday_name(r.start).to_string()
                    } else {
                        format!("{}..{}", weekday_name(r.start), weekday_name(r.end))
                    }
                })
                .collect();
            s.push_str(&wd_strs.join(","));
            s.push(' ');
        }

        // Date part
        if let Some(ref y) = self.year {
            s.push_str(&format_component(y));
            s.push('-');
        } else {
            s.push_str("*-");
        }
        s.push_str(&format_component_padded(&self.month, 2));
        s.push('-');
        s.push_str(&format_component_padded(&self.day, 2));

        s.push(' ');

        // Time part
        s.push_str(&format_component_padded(&self.hour, 2));
        s.push(':');
        s.push_str(&format_component_padded(&self.minute, 2));
        s.push(':');
        s.push_str(&format_component_padded(&self.second, 2));

        s
    }

    /// Convert a `std::time::SystemTime` to a `DateTime`.
    pub fn system_time_to_datetime(t: std::time::SystemTime) -> DateTime {
        let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let secs = dur.as_secs() as i64;
        unix_to_datetime(secs)
    }

    /// Convert a `DateTime` to seconds since the Unix epoch.
    pub fn datetime_to_unix(dt: &DateTime) -> i64 {
        let days = days_from_civil(dt.year, dt.month, dt.day);
        days * 86400 + dt.hour as i64 * 3600 + dt.minute as i64 * 60 + dt.second as i64
    }
}

impl fmt::Display for CalendarSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.normalized())
    }
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

/// Check if a token looks like a weekday specification (e.g. "Mon", "Mon..Fri", "Mon,Wed,Fri").
fn looks_like_weekday(s: &str) -> bool {
    // Split by comma, then check each part
    s.split(',').all(|part| {
        let part = part.trim();
        if part.is_empty() {
            return false;
        }
        // Could be "Mon" or "Mon..Fri"
        let atoms: Vec<&str> = part.split("..").collect();
        atoms.iter().all(|a| parse_weekday_name(a.trim()).is_some())
    })
}

fn parse_weekday_name(s: &str) -> Option<u8> {
    match s.to_lowercase().as_str() {
        "mon" | "monday" => Some(0),
        "tue" | "tuesday" => Some(1),
        "wed" | "wednesday" => Some(2),
        "thu" | "thursday" => Some(3),
        "fri" | "friday" => Some(4),
        "sat" | "saturday" => Some(5),
        "sun" | "sunday" => Some(6),
        _ => None,
    }
}

fn weekday_name(d: u8) -> &'static str {
    match d {
        0 => "Mon",
        1 => "Tue",
        2 => "Wed",
        3 => "Thu",
        4 => "Fri",
        5 => "Sat",
        6 => "Sun",
        _ => "???",
    }
}

fn parse_weekdays(s: &str) -> Result<Vec<WeekdayRange>, String> {
    let mut ranges = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((a, b)) = part.split_once("..") {
            let start =
                parse_weekday_name(a.trim()).ok_or_else(|| format!("Invalid weekday: {a}"))?;
            let end =
                parse_weekday_name(b.trim()).ok_or_else(|| format!("Invalid weekday: {b}"))?;
            ranges.push(WeekdayRange { start, end });
        } else {
            let d = parse_weekday_name(part).ok_or_else(|| format!("Invalid weekday: {part}"))?;
            ranges.push(WeekdayRange { start: d, end: d });
        }
    }
    if ranges.is_empty() {
        return Err("Empty weekday specification".to_string());
    }
    Ok(ranges)
}

/// Parse the date portion: `YYYY-MM-DD`, `*-MM-DD`, `*-*-DD`, `MM-DD`, etc.
fn parse_date_part(
    s: &str,
) -> Result<
    (
        Option<CalendarComponent>,
        CalendarComponent,
        CalendarComponent,
    ),
    String,
> {
    let parts: Vec<&str> = s.split('-').collect();
    match parts.len() {
        3 => {
            // YYYY-MM-DD or *-MM-DD
            let year = if parts[0] == "*" {
                None
            } else {
                Some(parse_component(parts[0])?)
            };
            let month = parse_component(parts[1])?;
            let day = parse_component(parts[2])?;
            Ok((year, month, day))
        }
        2 => {
            // MM-DD (year wildcard implied)
            let month = parse_component(parts[0])?;
            let day = parse_component(parts[1])?;
            Ok((None, month, day))
        }
        1 => {
            // Just a day? Treat as *-*-DD
            let day = parse_component(parts[0])?;
            Ok((None, CalendarComponent::Wildcard, day))
        }
        _ => Err(format!("Invalid date format: {s}")),
    }
}

/// Parse the time portion: `HH:MM:SS` or `HH:MM`.
/// Reject CalendarComponents whose numeric values fall outside `[min, max]`.
/// Wildcard components are always accepted.
fn validate_component_range(
    c: &CalendarComponent,
    min: u32,
    max: u32,
    field: &str,
) -> Result<(), String> {
    match c {
        CalendarComponent::Wildcard => Ok(()),
        CalendarComponent::WildcardRepeat(_) => Ok(()),
        CalendarComponent::List(values) => {
            for v in values {
                match v {
                    CalendarValue::Exact(n) => {
                        if *n < min || *n > max {
                            return Err(format!("{field} {n} out of range {min}..{max}"));
                        }
                    }
                    CalendarValue::Range(start, end) => {
                        if *start < min || *start > max || *end < min || *end > max {
                            return Err(format!(
                                "{field} range {start}..{end} out of range {min}..{max}"
                            ));
                        }
                    }
                    CalendarValue::Repeat(start, _step) => {
                        if *start < min || *start > max {
                            return Err(format!(
                                "{field} repeat start {start} out of range {min}..{max}"
                            ));
                        }
                    }
                    CalendarValue::RangeRepeat(start, end, _step) => {
                        if *start < min || *start > max || *end < min || *end > max {
                            return Err(format!(
                                "{field} range-repeat {start}..{end} out of range {min}..{max}"
                            ));
                        }
                    }
                }
            }
            Ok(())
        }
    }
}

fn parse_time_part(
    s: &str,
) -> Result<(CalendarComponent, CalendarComponent, CalendarComponent), String> {
    let parts: Vec<&str> = s.split(':').collect();
    match parts.len() {
        3 => {
            let hour = parse_component(parts[0])?;
            let minute = parse_component(parts[1])?;
            let second = parse_component(parts[2])?;
            Ok((hour, minute, second))
        }
        2 => {
            let hour = parse_component(parts[0])?;
            let minute = parse_component(parts[1])?;
            Ok((
                hour,
                minute,
                CalendarComponent::List(vec![CalendarValue::Exact(0)]),
            ))
        }
        1 => {
            let hour = parse_component(parts[0])?;
            Ok((
                hour,
                CalendarComponent::List(vec![CalendarValue::Exact(0)]),
                CalendarComponent::List(vec![CalendarValue::Exact(0)]),
            ))
        }
        _ => Err(format!("Invalid time format: {s}")),
    }
}

/// Parse a single calendar component like `*`, `*/2`, `1,15`, `1..5`, `1..5/2`, `00`.
fn parse_component(s: &str) -> Result<CalendarComponent, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("Empty component".to_string());
    }

    if s == "*" {
        return Ok(CalendarComponent::Wildcard);
    }

    // */N — wildcard with repetition
    if let Some(rest) = s.strip_prefix("*/") {
        let step: u32 = rest
            .parse()
            .map_err(|_| format!("Invalid repeat step: {rest}"))?;
        if step == 0 {
            return Err("Repeat step cannot be zero".to_string());
        }
        return Ok(CalendarComponent::WildcardRepeat(step));
    }

    // Comma-separated list of values/ranges
    let mut values = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        values.push(parse_single_value(part)?);
    }

    if values.is_empty() {
        return Err(format!("Empty component: {s}"));
    }

    // Optimize: single exact value
    Ok(CalendarComponent::List(values))
}

/// Parse a single value: `5`, `1..5`, `5/2`, `1..5/2`.
fn parse_single_value(s: &str) -> Result<CalendarValue, String> {
    // Check for /step suffix
    if let Some((base, step_str)) = s.split_once('/') {
        let step: u32 = step_str
            .parse()
            .map_err(|_| format!("Invalid step: {step_str}"))?;
        if step == 0 {
            return Err("Step cannot be zero".to_string());
        }

        // base is either a range "a..b" or a single value
        if let Some((a_str, b_str)) = base.split_once("..") {
            let a: u32 = a_str
                .parse()
                .map_err(|_| format!("Invalid value: {a_str}"))?;
            let b: u32 = b_str
                .parse()
                .map_err(|_| format!("Invalid value: {b_str}"))?;
            Ok(CalendarValue::RangeRepeat(a, b, step))
        } else {
            let v: u32 = base.parse().map_err(|_| format!("Invalid value: {base}"))?;
            Ok(CalendarValue::Repeat(v, step))
        }
    } else if let Some((a_str, b_str)) = s.split_once("..") {
        let a: u32 = a_str
            .parse()
            .map_err(|_| format!("Invalid value: {a_str}"))?;
        let b: u32 = b_str
            .parse()
            .map_err(|_| format!("Invalid value: {b_str}"))?;
        Ok(CalendarValue::Range(a, b))
    } else {
        let v: u32 = s.parse().map_err(|_| format!("Invalid value: {s}"))?;
        Ok(CalendarValue::Exact(v))
    }
}

// ---------------------------------------------------------------------------
// Matching / iteration helpers
// ---------------------------------------------------------------------------

/// Check if a value matches a calendar component.
pub fn component_matches(comp: &CalendarComponent, value: u32) -> bool {
    match comp {
        CalendarComponent::Wildcard => true,
        CalendarComponent::WildcardRepeat(step) => value.is_multiple_of(*step),
        CalendarComponent::List(values) => values.iter().any(|v| value_matches(v, value)),
    }
}

fn value_matches(val: &CalendarValue, v: u32) -> bool {
    match val {
        CalendarValue::Exact(e) => v == *e,
        CalendarValue::Range(a, b) => v >= *a && v <= *b,
        CalendarValue::Repeat(start, step) => {
            if v < *start {
                false
            } else {
                (v - start).is_multiple_of(*step)
            }
        }
        CalendarValue::RangeRepeat(start, end, step) => {
            if v < *start || v > *end {
                false
            } else {
                (v - start).is_multiple_of(*step)
            }
        }
    }
}

/// Generate matching values in `[from..=max]` for a component, in order.
fn matching_values_from(comp: &CalendarComponent, from: u32, max: u32) -> Vec<u32> {
    let mut result = Vec::new();
    match comp {
        CalendarComponent::Wildcard => {
            for v in from..=max {
                result.push(v);
            }
        }
        CalendarComponent::WildcardRepeat(step) => {
            // First matching value >= from
            let first = if from == 0 {
                0
            } else {
                let remainder = from % step;
                if remainder == 0 {
                    from
                } else {
                    from + (step - remainder)
                }
            };
            let mut v = first;
            while v <= max {
                result.push(v);
                v += step;
            }
        }
        CalendarComponent::List(values) => {
            // Collect all matching values in range, then sort and dedup
            for cv in values {
                collect_matching_values(cv, from, max, &mut result);
            }
            result.sort_unstable();
            result.dedup();
        }
    }
    result
}

fn collect_matching_values(val: &CalendarValue, from: u32, max: u32, out: &mut Vec<u32>) {
    match val {
        CalendarValue::Exact(e) => {
            if *e >= from && *e <= max {
                out.push(*e);
            }
        }
        CalendarValue::Range(a, b) => {
            let start = (*a).max(from);
            let end = (*b).min(max);
            for v in start..=end {
                out.push(v);
            }
        }
        CalendarValue::Repeat(start, step) => {
            let mut v = *start;
            // Advance to first value >= from
            if v < from {
                let gap = from - v;
                let skips = gap.div_ceil(*step);
                v += skips * step;
            }
            while v <= max {
                out.push(v);
                v += step;
            }
        }
        CalendarValue::RangeRepeat(start, end, step) => {
            let eff_max = (*end).min(max);
            let mut v = *start;
            if v < from {
                let gap = from - v;
                let skips = gap.div_ceil(*step);
                v += skips * step;
            }
            while v <= eff_max {
                out.push(v);
                v += step;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn format_component(comp: &CalendarComponent) -> String {
    match comp {
        CalendarComponent::Wildcard => "*".to_string(),
        CalendarComponent::WildcardRepeat(step) => format!("*/{step}"),
        CalendarComponent::List(values) => {
            let strs: Vec<String> = values.iter().map(format_value).collect();
            strs.join(",")
        }
    }
}

fn format_component_padded(comp: &CalendarComponent, width: usize) -> String {
    match comp {
        CalendarComponent::Wildcard => "*".to_string(),
        CalendarComponent::WildcardRepeat(step) => format!("*/{step}"),
        CalendarComponent::List(values) => {
            let strs: Vec<String> = values
                .iter()
                .map(|v| format_value_padded(v, width))
                .collect();
            strs.join(",")
        }
    }
}

fn format_value(val: &CalendarValue) -> String {
    match val {
        CalendarValue::Exact(v) => v.to_string(),
        CalendarValue::Range(a, b) => format!("{a}..{b}"),
        CalendarValue::Repeat(start, step) => format!("{start}/{step}"),
        CalendarValue::RangeRepeat(start, end, step) => format!("{start}..{end}/{step}"),
    }
}

fn format_value_padded(val: &CalendarValue, width: usize) -> String {
    match val {
        CalendarValue::Exact(v) => format!("{v:0>width$}"),
        CalendarValue::Range(a, b) => format!("{a:0>width$}..{b:0>width$}"),
        CalendarValue::Repeat(start, step) => format!("{start:0>width$}/{step}"),
        CalendarValue::RangeRepeat(start, end, step) => {
            format!("{start:0>width$}..{end:0>width$}/{step}")
        }
    }
}

// ---------------------------------------------------------------------------
// Date / time arithmetic (no external crate dependency)
// ---------------------------------------------------------------------------

/// Days from civil date (year, month 1-12, day 1-31) to Unix epoch day.
/// Algorithm from Howard Hinnant's date algorithms.
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = y as i64;
    let m = m as i64;
    let d = d as i64;
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64; // [0, 399]
    let m_adj = if m > 2 { m - 3 } else { m + 9 }; // [0, 11]
    let doy = (153 * m_adj as u64 + 2) / 5 + d as u64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe as i64 - 719468
}

/// Convert Unix epoch day to civil date (year, month 1-12, day 1-31).
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d)
}

/// Convert Unix timestamp (seconds) to DateTime.
pub fn unix_to_datetime(secs: i64) -> DateTime {
    let days = secs.div_euclid(86400);
    let day_secs = secs.rem_euclid(86400) as u32;
    let (y, m, d) = civil_from_days(days);
    DateTime {
        year: y,
        month: m,
        day: d,
        hour: day_secs / 3600,
        minute: (day_secs % 3600) / 60,
        second: day_secs % 60,
    }
}

/// Day of week: 0=Mon, 6=Sun.
fn weekday(y: i32, m: u32, d: u32) -> u8 {
    let days = days_from_civil(y, m, d);
    // Unix epoch (1970-01-01) was a Thursday = weekday 3 (0-indexed from Monday).
    ((days.rem_euclid(7) + 3) % 7) as u8
}

/// Public helper: compute weekday (0=Mon .. 6=Sun) from a `DateTime`.
pub fn weekday_from_datetime(dt: &DateTime) -> u8 {
    weekday(dt.year, dt.month, dt.day)
}

fn is_leap_year(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(y) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

impl DateTime {
    /// Create a DateTime one second after this one.
    pub fn add_second(self) -> DateTime {
        let unix = CalendarSpec::datetime_to_unix(&self) + 1;
        unix_to_datetime(unix)
    }
}

// ---------------------------------------------------------------------------
// Convenience: convert between CalendarSpec and the timer scheduler's
// simplified interval model
// ---------------------------------------------------------------------------

/// Try to interpret a calendar spec as a fixed interval (for backwards-compat
/// with the simple shorthand-based scheduler).
/// Returns `None` for complex expressions that need `next_elapse()`.
pub fn as_fixed_interval(spec: &CalendarSpec) -> Option<std::time::Duration> {
    // Only works for the common shorthands that map to uniform intervals
    match spec.original.to_lowercase().as_str() {
        "minutely" => Some(std::time::Duration::from_secs(60)),
        "hourly" => Some(std::time::Duration::from_secs(3600)),
        "daily" => Some(std::time::Duration::from_secs(86400)),
        "weekly" => Some(std::time::Duration::from_secs(7 * 86400)),
        "monthly" => Some(std::time::Duration::from_secs(30 * 86400)),
        "yearly" | "annually" => Some(std::time::Duration::from_secs(365 * 86400)),
        "quarterly" => Some(std::time::Duration::from_secs(90 * 86400)),
        "semiannually" | "semi-annually" => Some(std::time::Duration::from_secs(182 * 86400)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Parsing tests --

    #[test]
    fn test_parse_daily() {
        let spec = CalendarSpec::parse("daily").unwrap();
        assert!(spec.weekdays.is_none());
        assert_eq!(
            spec.hour,
            CalendarComponent::List(vec![CalendarValue::Exact(0)])
        );
        assert_eq!(
            spec.minute,
            CalendarComponent::List(vec![CalendarValue::Exact(0)])
        );
        assert_eq!(
            spec.second,
            CalendarComponent::List(vec![CalendarValue::Exact(0)])
        );
    }

    #[test]
    fn test_parse_hourly() {
        let spec = CalendarSpec::parse("hourly").unwrap();
        assert_eq!(spec.hour, CalendarComponent::Wildcard);
        assert_eq!(
            spec.minute,
            CalendarComponent::List(vec![CalendarValue::Exact(0)])
        );
    }

    #[test]
    fn test_parse_minutely() {
        let spec = CalendarSpec::parse("minutely").unwrap();
        assert_eq!(spec.hour, CalendarComponent::Wildcard);
        assert_eq!(spec.minute, CalendarComponent::Wildcard);
        assert_eq!(
            spec.second,
            CalendarComponent::List(vec![CalendarValue::Exact(0)])
        );
    }

    #[test]
    fn test_parse_weekly() {
        let spec = CalendarSpec::parse("weekly").unwrap();
        assert!(spec.weekdays.is_some());
        let wds = spec.weekdays.unwrap();
        assert_eq!(wds.len(), 1);
        assert_eq!(wds[0].start, 0); // Mon
        assert_eq!(wds[0].end, 0);
    }

    #[test]
    fn test_parse_yearly() {
        let spec = CalendarSpec::parse("yearly").unwrap();
        assert_eq!(
            spec.month,
            CalendarComponent::List(vec![CalendarValue::Exact(1)])
        );
        assert_eq!(
            spec.day,
            CalendarComponent::List(vec![CalendarValue::Exact(1)])
        );
    }

    #[test]
    fn test_parse_quarterly() {
        let spec = CalendarSpec::parse("quarterly").unwrap();
        assert_eq!(
            spec.month,
            CalendarComponent::List(vec![
                CalendarValue::Exact(1),
                CalendarValue::Exact(4),
                CalendarValue::Exact(7),
                CalendarValue::Exact(10),
            ])
        );
    }

    #[test]
    fn test_parse_full_expression() {
        let spec = CalendarSpec::parse("*-*-* 06:00:00").unwrap();
        assert!(spec.weekdays.is_none());
        assert_eq!(spec.month, CalendarComponent::Wildcard);
        assert_eq!(spec.day, CalendarComponent::Wildcard);
        assert_eq!(
            spec.hour,
            CalendarComponent::List(vec![CalendarValue::Exact(6)])
        );
        assert_eq!(
            spec.minute,
            CalendarComponent::List(vec![CalendarValue::Exact(0)])
        );
        assert_eq!(
            spec.second,
            CalendarComponent::List(vec![CalendarValue::Exact(0)])
        );
    }

    #[test]
    fn test_parse_weekday_expression() {
        let spec = CalendarSpec::parse("Mon *-*-* 00:00:00").unwrap();
        let wds = spec.weekdays.as_ref().unwrap();
        assert_eq!(wds.len(), 1);
        assert_eq!(wds[0].start, 0);
        assert_eq!(wds[0].end, 0);
    }

    #[test]
    fn test_parse_weekday_range() {
        let spec = CalendarSpec::parse("Mon..Fri *-*-* 09:00").unwrap();
        let wds = spec.weekdays.as_ref().unwrap();
        assert_eq!(wds.len(), 1);
        assert_eq!(wds[0].start, 0); // Mon
        assert_eq!(wds[0].end, 4); // Fri
    }

    #[test]
    fn test_parse_multiple_weekdays() {
        let spec = CalendarSpec::parse("Mon,Wed,Fri *-*-* 10:00").unwrap();
        let wds = spec.weekdays.as_ref().unwrap();
        assert_eq!(wds.len(), 3);
        assert_eq!(wds[0].start, 0); // Mon
        assert_eq!(wds[1].start, 2); // Wed
        assert_eq!(wds[2].start, 4); // Fri
    }

    #[test]
    fn test_parse_comma_list_day() {
        let spec = CalendarSpec::parse("*-*-1,15 12:00:00").unwrap();
        assert_eq!(
            spec.day,
            CalendarComponent::List(vec![CalendarValue::Exact(1), CalendarValue::Exact(15),])
        );
    }

    #[test]
    fn test_parse_range() {
        let spec = CalendarSpec::parse("*-*-* 09..17:00:00").unwrap();
        assert_eq!(
            spec.hour,
            CalendarComponent::List(vec![CalendarValue::Range(9, 17)])
        );
    }

    #[test]
    fn test_parse_wildcard_repeat() {
        let spec = CalendarSpec::parse("*-*-* */2:00:00").unwrap();
        assert_eq!(spec.hour, CalendarComponent::WildcardRepeat(2));
    }

    #[test]
    fn test_parse_value_with_repeat() {
        let spec = CalendarSpec::parse("*-*-* 00/3:00:00").unwrap();
        assert_eq!(
            spec.hour,
            CalendarComponent::List(vec![CalendarValue::Repeat(0, 3)])
        );
    }

    #[test]
    fn test_parse_time_only() {
        let spec = CalendarSpec::parse("06:00").unwrap();
        assert_eq!(
            spec.hour,
            CalendarComponent::List(vec![CalendarValue::Exact(6)])
        );
        assert_eq!(
            spec.minute,
            CalendarComponent::List(vec![CalendarValue::Exact(0)])
        );
        assert_eq!(
            spec.second,
            CalendarComponent::List(vec![CalendarValue::Exact(0)])
        );
    }

    #[test]
    fn test_parse_date_only() {
        let spec = CalendarSpec::parse("*-12-25").unwrap();
        assert_eq!(
            spec.month,
            CalendarComponent::List(vec![CalendarValue::Exact(12)])
        );
        assert_eq!(
            spec.day,
            CalendarComponent::List(vec![CalendarValue::Exact(25)])
        );
    }

    #[test]
    fn test_parse_year_specific() {
        let spec = CalendarSpec::parse("2025-01-01 00:00:00").unwrap();
        assert_eq!(
            spec.year,
            Some(CalendarComponent::List(vec![CalendarValue::Exact(2025)]))
        );
    }

    #[test]
    fn test_parse_empty_error() {
        assert!(CalendarSpec::parse("").is_err());
    }

    #[test]
    fn test_parse_semiannually() {
        let spec = CalendarSpec::parse("semiannually").unwrap();
        assert_eq!(
            spec.month,
            CalendarComponent::List(vec![CalendarValue::Exact(1), CalendarValue::Exact(7),])
        );
    }

    #[test]
    fn test_parse_range_repeat_second() {
        let spec = CalendarSpec::parse("*-*-* *:*:00/15").unwrap();
        assert_eq!(
            spec.second,
            CalendarComponent::List(vec![CalendarValue::Repeat(0, 15)])
        );
    }

    #[test]
    fn test_parse_range_repeat_full() {
        let spec = CalendarSpec::parse("*-*-* *:*:0..59/10").unwrap();
        assert_eq!(
            spec.second,
            CalendarComponent::List(vec![CalendarValue::RangeRepeat(0, 59, 10)])
        );
    }

    // -- Component matching tests --

    #[test]
    fn test_component_wildcard_matches_all() {
        assert!(component_matches(&CalendarComponent::Wildcard, 0));
        assert!(component_matches(&CalendarComponent::Wildcard, 59));
    }

    #[test]
    fn test_component_wildcard_repeat() {
        let comp = CalendarComponent::WildcardRepeat(5);
        assert!(component_matches(&comp, 0));
        assert!(component_matches(&comp, 5));
        assert!(component_matches(&comp, 10));
        assert!(!component_matches(&comp, 3));
    }

    #[test]
    fn test_component_exact() {
        let comp = CalendarComponent::List(vec![CalendarValue::Exact(15)]);
        assert!(component_matches(&comp, 15));
        assert!(!component_matches(&comp, 14));
    }

    #[test]
    fn test_component_range() {
        let comp = CalendarComponent::List(vec![CalendarValue::Range(9, 17)]);
        assert!(component_matches(&comp, 9));
        assert!(component_matches(&comp, 13));
        assert!(component_matches(&comp, 17));
        assert!(!component_matches(&comp, 8));
        assert!(!component_matches(&comp, 18));
    }

    #[test]
    fn test_component_repeat() {
        let comp = CalendarComponent::List(vec![CalendarValue::Repeat(2, 3)]);
        assert!(component_matches(&comp, 2));
        assert!(component_matches(&comp, 5));
        assert!(component_matches(&comp, 8));
        assert!(!component_matches(&comp, 0));
        assert!(!component_matches(&comp, 3));
    }

    #[test]
    fn test_component_range_repeat() {
        let comp = CalendarComponent::List(vec![CalendarValue::RangeRepeat(0, 59, 15)]);
        assert!(component_matches(&comp, 0));
        assert!(component_matches(&comp, 15));
        assert!(component_matches(&comp, 30));
        assert!(component_matches(&comp, 45));
        assert!(!component_matches(&comp, 10));
        assert!(!component_matches(&comp, 60));
    }

    #[test]
    fn test_component_comma_list() {
        let comp = CalendarComponent::List(vec![CalendarValue::Exact(1), CalendarValue::Exact(15)]);
        assert!(component_matches(&comp, 1));
        assert!(component_matches(&comp, 15));
        assert!(!component_matches(&comp, 2));
    }

    // -- next_elapse tests --

    #[test]
    fn test_next_elapse_daily_same_day() {
        let spec = CalendarSpec::parse("daily").unwrap();
        let after = DateTime {
            year: 2025,
            month: 6,
            day: 15,
            hour: 0,
            minute: 0,
            second: 0,
        };
        let next = spec.next_elapse(after).unwrap();
        assert_eq!(
            next,
            DateTime {
                year: 2025,
                month: 6,
                day: 15,
                hour: 0,
                minute: 0,
                second: 0,
            }
        );
    }

    #[test]
    fn test_next_elapse_daily_after_midnight() {
        let spec = CalendarSpec::parse("daily").unwrap();
        // After midnight — next occurrence is tomorrow
        let after = DateTime {
            year: 2025,
            month: 6,
            day: 15,
            hour: 0,
            minute: 0,
            second: 1,
        };
        let next = spec.next_elapse(after).unwrap();
        assert_eq!(next.year, 2025);
        assert_eq!(next.month, 6);
        assert_eq!(next.day, 16);
        assert_eq!(next.hour, 0);
    }

    #[test]
    fn test_next_elapse_hourly() {
        let spec = CalendarSpec::parse("hourly").unwrap();
        let after = DateTime {
            year: 2025,
            month: 1,
            day: 1,
            hour: 5,
            minute: 30,
            second: 0,
        };
        let next = spec.next_elapse(after).unwrap();
        assert_eq!(next.hour, 6);
        assert_eq!(next.minute, 0);
        assert_eq!(next.second, 0);
    }

    #[test]
    fn test_next_elapse_specific_time() {
        let spec = CalendarSpec::parse("*-*-* 06:00:00").unwrap();
        let after = DateTime {
            year: 2025,
            month: 3,
            day: 10,
            hour: 7,
            minute: 0,
            second: 0,
        };
        // Next 06:00 is tomorrow
        let next = spec.next_elapse(after).unwrap();
        assert_eq!(next.day, 11);
        assert_eq!(next.hour, 6);
    }

    #[test]
    fn test_next_elapse_specific_time_before() {
        let spec = CalendarSpec::parse("*-*-* 06:00:00").unwrap();
        let after = DateTime {
            year: 2025,
            month: 3,
            day: 10,
            hour: 5,
            minute: 0,
            second: 0,
        };
        // Before 06:00 — fires today
        let next = spec.next_elapse(after).unwrap();
        assert_eq!(next.day, 10);
        assert_eq!(next.hour, 6);
    }

    #[test]
    fn test_next_elapse_monthly() {
        let spec = CalendarSpec::parse("monthly").unwrap();
        let after = DateTime {
            year: 2025,
            month: 3,
            day: 2,
            hour: 0,
            minute: 0,
            second: 0,
        };
        // Monthly = *-*-01 00:00:00, so next is April 1
        let next = spec.next_elapse(after).unwrap();
        assert_eq!(next.month, 4);
        assert_eq!(next.day, 1);
    }

    #[test]
    fn test_next_elapse_weekly_monday() {
        let spec = CalendarSpec::parse("weekly").unwrap();
        // 2025-06-15 is a Sunday
        let after = DateTime {
            year: 2025,
            month: 6,
            day: 15,
            hour: 12,
            minute: 0,
            second: 0,
        };
        let next = spec.next_elapse(after).unwrap();
        // Next Monday is June 16
        assert_eq!(next.day, 16);
        assert_eq!(next.hour, 0);
    }

    #[test]
    fn test_next_elapse_weekday_range() {
        let spec = CalendarSpec::parse("Mon..Fri *-*-* 09:00").unwrap();
        // 2025-06-14 is a Saturday
        let after = DateTime {
            year: 2025,
            month: 6,
            day: 14,
            hour: 10,
            minute: 0,
            second: 0,
        };
        let next = spec.next_elapse(after).unwrap();
        // Next Mon..Fri is Monday June 16
        assert_eq!(next.day, 16);
        assert_eq!(next.hour, 9);
    }

    #[test]
    fn test_next_elapse_comma_days() {
        let spec = CalendarSpec::parse("*-*-1,15 12:00:00").unwrap();
        let after = DateTime {
            year: 2025,
            month: 3,
            day: 2,
            hour: 0,
            minute: 0,
            second: 0,
        };
        let next = spec.next_elapse(after).unwrap();
        assert_eq!(next.day, 15);
        assert_eq!(next.hour, 12);
    }

    #[test]
    fn test_next_elapse_every_two_hours() {
        let spec = CalendarSpec::parse("*-*-* */2:00:00").unwrap();
        let after = DateTime {
            year: 2025,
            month: 1,
            day: 1,
            hour: 3,
            minute: 0,
            second: 0,
        };
        let next = spec.next_elapse(after).unwrap();
        // */2 matches 0, 2, 4, 6... so next after hour 3 is hour 4
        assert_eq!(next.hour, 4);
        assert_eq!(next.minute, 0);
    }

    #[test]
    fn test_next_elapse_every_15_seconds() {
        let spec = CalendarSpec::parse("*-*-* *:*:00/15").unwrap();
        let after = DateTime {
            year: 2025,
            month: 1,
            day: 1,
            hour: 10,
            minute: 30,
            second: 20,
        };
        let next = spec.next_elapse(after).unwrap();
        assert_eq!(next.second, 30);
    }

    #[test]
    fn test_next_elapse_year_end_wrap() {
        let spec = CalendarSpec::parse("*-01-01 00:00:00").unwrap();
        let after = DateTime {
            year: 2025,
            month: 6,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        };
        let next = spec.next_elapse(after).unwrap();
        assert_eq!(next.year, 2026);
        assert_eq!(next.month, 1);
        assert_eq!(next.day, 1);
    }

    #[test]
    fn test_next_elapse_feb_29_leap_year() {
        let spec = CalendarSpec::parse("*-02-29 00:00:00").unwrap();
        let after = DateTime {
            year: 2025,
            month: 3,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        };
        let next = spec.next_elapse(after).unwrap();
        assert_eq!(next.year, 2028);
        assert_eq!(next.month, 2);
        assert_eq!(next.day, 29);
    }

    #[test]
    fn test_next_elapse_exact_match() {
        let spec = CalendarSpec::parse("*-*-* 12:00:00").unwrap();
        let after = DateTime {
            year: 2025,
            month: 1,
            day: 1,
            hour: 12,
            minute: 0,
            second: 0,
        };
        // Exact match should return the same time
        let next = spec.next_elapse(after).unwrap();
        assert_eq!(next, after);
    }

    #[test]
    fn test_next_elapse_specific_year_in_past() {
        let spec = CalendarSpec::parse("2020-01-01 00:00:00").unwrap();
        let after = DateTime {
            year: 2025,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        };
        // Specific year 2020 is in the past — no match
        let next = spec.next_elapse(after);
        assert!(next.is_none());
    }

    // -- Date arithmetic tests --

    #[test]
    fn test_days_from_civil_epoch() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
    }

    #[test]
    fn test_days_from_civil_2000() {
        assert_eq!(days_from_civil(2000, 1, 1), 10957);
    }

    #[test]
    fn test_civil_roundtrip() {
        for days in [-1000, -1, 0, 1, 365, 10957, 18000, 20000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(
                days_from_civil(y, m, d),
                days,
                "roundtrip failed for day {days}"
            );
        }
    }

    #[test]
    fn test_weekday_thursday_epoch() {
        // 1970-01-01 was a Thursday = 3
        assert_eq!(weekday(1970, 1, 1), 3);
    }

    #[test]
    fn test_weekday_known_dates() {
        // 2025-06-14 is Saturday = 5
        assert_eq!(weekday(2025, 6, 14), 5);
        // 2025-06-15 is Sunday = 6
        assert_eq!(weekday(2025, 6, 15), 6);
        // 2025-06-16 is Monday = 0
        assert_eq!(weekday(2025, 6, 16), 0);
    }

    #[test]
    fn test_unix_to_datetime_epoch() {
        let dt = unix_to_datetime(0);
        assert_eq!(dt.year, 1970);
        assert_eq!(dt.month, 1);
        assert_eq!(dt.day, 1);
        assert_eq!(dt.hour, 0);
        assert_eq!(dt.minute, 0);
        assert_eq!(dt.second, 0);
    }

    #[test]
    fn test_unix_to_datetime_known() {
        // 2025-01-01 00:00:00 UTC = 1735689600
        let dt = unix_to_datetime(1735689600);
        assert_eq!(dt.year, 2025);
        assert_eq!(dt.month, 1);
        assert_eq!(dt.day, 1);
        assert_eq!(dt.hour, 0);
    }

    #[test]
    fn test_datetime_to_unix_roundtrip() {
        let dt = DateTime {
            year: 2025,
            month: 6,
            day: 15,
            hour: 14,
            minute: 30,
            second: 45,
        };
        let unix = CalendarSpec::datetime_to_unix(&dt);
        let dt2 = unix_to_datetime(unix);
        assert_eq!(dt, dt2);
    }

    #[test]
    fn test_days_in_month_feb_leap() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2025, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
    }

    #[test]
    fn test_is_leap_year() {
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(2025));
        assert!(is_leap_year(2000));
        assert!(!is_leap_year(1900));
    }

    // -- Normalization / display tests --

    #[test]
    fn test_normalized_daily() {
        let spec = CalendarSpec::parse("daily").unwrap();
        assert_eq!(spec.normalized(), "*-*-* 00:00:00");
    }

    #[test]
    fn test_normalized_hourly() {
        let spec = CalendarSpec::parse("hourly").unwrap();
        assert_eq!(spec.normalized(), "*-*-* *:00:00");
    }

    #[test]
    fn test_normalized_minutely() {
        let spec = CalendarSpec::parse("minutely").unwrap();
        assert_eq!(spec.normalized(), "*-*-* *:*:00");
    }

    #[test]
    fn test_normalized_weekly() {
        let spec = CalendarSpec::parse("weekly").unwrap();
        assert_eq!(spec.normalized(), "Mon *-*-* 00:00:00");
    }

    #[test]
    fn test_normalized_quarterly() {
        let spec = CalendarSpec::parse("quarterly").unwrap();
        assert_eq!(spec.normalized(), "*-01,04,07,10-01 00:00:00");
    }

    #[test]
    fn test_normalized_with_weekday_range() {
        let spec = CalendarSpec::parse("Mon..Fri *-*-* 09:00").unwrap();
        assert_eq!(spec.normalized(), "Mon..Fri *-*-* 09:00:00");
    }

    #[test]
    fn test_normalized_preserves_repeat() {
        let spec = CalendarSpec::parse("*-*-* */2:00:00").unwrap();
        assert_eq!(spec.normalized(), "*-*-* */2:00:00");
    }

    // -- matching_values_from tests --

    #[test]
    fn test_matching_values_wildcard() {
        let vals = matching_values_from(&CalendarComponent::Wildcard, 3, 7);
        assert_eq!(vals, vec![3, 4, 5, 6, 7]);
    }

    #[test]
    fn test_matching_values_wildcard_repeat() {
        let vals = matching_values_from(&CalendarComponent::WildcardRepeat(3), 0, 12);
        assert_eq!(vals, vec![0, 3, 6, 9, 12]);
    }

    #[test]
    fn test_matching_values_wildcard_repeat_offset() {
        let vals = matching_values_from(&CalendarComponent::WildcardRepeat(5), 7, 23);
        assert_eq!(vals, vec![10, 15, 20]);
    }

    #[test]
    fn test_matching_values_exact() {
        let vals = matching_values_from(
            &CalendarComponent::List(vec![CalendarValue::Exact(5)]),
            0,
            10,
        );
        assert_eq!(vals, vec![5]);
    }

    #[test]
    fn test_matching_values_exact_out_of_range() {
        let vals = matching_values_from(
            &CalendarComponent::List(vec![CalendarValue::Exact(15)]),
            0,
            10,
        );
        assert!(vals.is_empty());
    }

    #[test]
    fn test_matching_values_range() {
        let vals = matching_values_from(
            &CalendarComponent::List(vec![CalendarValue::Range(3, 7)]),
            5,
            10,
        );
        assert_eq!(vals, vec![5, 6, 7]);
    }

    #[test]
    fn test_matching_values_repeat() {
        let vals = matching_values_from(
            &CalendarComponent::List(vec![CalendarValue::Repeat(0, 15)]),
            0,
            59,
        );
        assert_eq!(vals, vec![0, 15, 30, 45]);
    }

    #[test]
    fn test_matching_values_repeat_offset_start() {
        let vals = matching_values_from(
            &CalendarComponent::List(vec![CalendarValue::Repeat(0, 15)]),
            20,
            59,
        );
        assert_eq!(vals, vec![30, 45]);
    }

    #[test]
    fn test_matching_values_comma_list() {
        let vals = matching_values_from(
            &CalendarComponent::List(vec![
                CalendarValue::Exact(1),
                CalendarValue::Exact(15),
                CalendarValue::Exact(28),
            ]),
            5,
            20,
        );
        assert_eq!(vals, vec![15]);
    }

    #[test]
    fn test_matching_values_range_repeat() {
        let vals = matching_values_from(
            &CalendarComponent::List(vec![CalendarValue::RangeRepeat(0, 59, 10)]),
            0,
            59,
        );
        assert_eq!(vals, vec![0, 10, 20, 30, 40, 50]);
    }

    // -- Integration tests: parse → next_elapse --

    #[test]
    fn test_every_5_minutes() {
        let spec = CalendarSpec::parse("*-*-* *:00/5:00").unwrap();
        let after = DateTime {
            year: 2025,
            month: 1,
            day: 1,
            hour: 10,
            minute: 7,
            second: 0,
        };
        let next = spec.next_elapse(after).unwrap();
        assert_eq!(next.minute, 10);
        assert_eq!(next.second, 0);
    }

    #[test]
    fn test_first_and_fifteenth() {
        let spec = CalendarSpec::parse("*-*-1,15 00:00:00").unwrap();
        let after = DateTime {
            year: 2025,
            month: 3,
            day: 16,
            hour: 0,
            minute: 0,
            second: 0,
        };
        let next = spec.next_elapse(after).unwrap();
        assert_eq!(next.month, 4);
        assert_eq!(next.day, 1);
    }

    #[test]
    fn test_quarterly_months() {
        let spec = CalendarSpec::parse("quarterly").unwrap();
        let after = DateTime {
            year: 2025,
            month: 2,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        };
        let next = spec.next_elapse(after).unwrap();
        assert_eq!(next.month, 4);
        assert_eq!(next.day, 1);
    }

    #[test]
    fn test_saturday_and_sunday() {
        let spec = CalendarSpec::parse("Sat,Sun *-*-* 00:00:00").unwrap();
        // 2025-06-16 is Monday
        let after = DateTime {
            year: 2025,
            month: 6,
            day: 16,
            hour: 0,
            minute: 0,
            second: 1,
        };
        let next = spec.next_elapse(after).unwrap();
        // Next Saturday is June 21
        assert_eq!(next.day, 21);
    }

    #[test]
    fn test_month_end_rollover() {
        let spec = CalendarSpec::parse("*-*-31 00:00:00").unwrap();
        // February doesn't have 31 days, should skip to March
        let after = DateTime {
            year: 2025,
            month: 2,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        };
        let next = spec.next_elapse(after).unwrap();
        assert_eq!(next.month, 3);
        assert_eq!(next.day, 31);
    }

    #[test]
    fn test_datetime_add_second() {
        let dt = DateTime {
            year: 2025,
            month: 12,
            day: 31,
            hour: 23,
            minute: 59,
            second: 59,
        };
        let next = dt.add_second();
        assert_eq!(next.year, 2026);
        assert_eq!(next.month, 1);
        assert_eq!(next.day, 1);
        assert_eq!(next.hour, 0);
        assert_eq!(next.minute, 0);
        assert_eq!(next.second, 0);
    }

    #[test]
    fn test_as_fixed_interval_shorthands() {
        assert_eq!(
            as_fixed_interval(&CalendarSpec::parse("minutely").unwrap()),
            Some(std::time::Duration::from_secs(60))
        );
        assert_eq!(
            as_fixed_interval(&CalendarSpec::parse("hourly").unwrap()),
            Some(std::time::Duration::from_secs(3600))
        );
        assert_eq!(
            as_fixed_interval(&CalendarSpec::parse("daily").unwrap()),
            Some(std::time::Duration::from_secs(86400))
        );
        assert!(as_fixed_interval(&CalendarSpec::parse("*-*-* 06:00:00").unwrap()).is_none());
    }

    // -- Edge case tests --

    #[test]
    fn test_wildcard_all_fields() {
        let spec = CalendarSpec::parse("*-*-* *:*:*").unwrap();
        let after = DateTime {
            year: 2025,
            month: 6,
            day: 15,
            hour: 10,
            minute: 30,
            second: 45,
        };
        // Every second matches — should return the same time
        let next = spec.next_elapse(after).unwrap();
        assert_eq!(next, after);
    }

    #[test]
    fn test_display_format() {
        let spec = CalendarSpec::parse("Mon..Fri *-*-* 09:00:00").unwrap();
        let s = format!("{spec}");
        assert_eq!(s, "Mon..Fri *-*-* 09:00:00");
    }

    #[test]
    fn test_multiple_values_and_ranges_combined() {
        let spec = CalendarSpec::parse("*-*-* 9,12,18:00:00").unwrap();
        assert_eq!(
            spec.hour,
            CalendarComponent::List(vec![
                CalendarValue::Exact(9),
                CalendarValue::Exact(12),
                CalendarValue::Exact(18),
            ])
        );
        let after = DateTime {
            year: 2025,
            month: 1,
            day: 1,
            hour: 10,
            minute: 0,
            second: 0,
        };
        let next = spec.next_elapse(after).unwrap();
        assert_eq!(next.hour, 12);
    }

    #[test]
    fn test_system_time_conversion_roundtrip() {
        let now = std::time::SystemTime::now();
        let dt = CalendarSpec::system_time_to_datetime(now);
        // Verify the result looks sane (year > 2020)
        assert!(dt.year >= 2020);
        assert!((1..=12).contains(&dt.month));
        assert!((1..=31).contains(&dt.day));
        assert!(dt.hour <= 23);
        assert!(dt.minute <= 59);
        assert!(dt.second <= 59);
    }
}
