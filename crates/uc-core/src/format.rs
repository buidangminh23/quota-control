//! Number and duration formatting. Port of upstream `MetricFormatter.swift` and the parts of
//! `Formatters.swift` the Rust side needs (chart labels, tray tooltip, CLI).
//!
//! Upstream formats through ICU with the en_US locale. Two rounding rules matter:
//! - percents use `Double.rounded()` (half away from zero);
//! - decimal and compact notation use ICU's default half-even rounding.

use chrono::{DateTime, Datelike, Local, TimeZone};

use crate::model::{MetricKind, MetricValue};

/// Three surfaces, three needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// The tray: shortest. Whole dollars under $1,000, abbreviated above; counts abbreviated.
    Tray,
    /// The popup row: abbreviated like the tray, but money keeps cents.
    Row,
    /// Tooltips and bounded headlines: every digit, grouped.
    Full,
}

/// Round half to even at `decimals` fractional digits, matching ICU's default rounding.
pub fn round_half_even(value: f64, decimals: u32) -> f64 {
    let factor = 10f64.powi(decimals as i32);
    let scaled = value * factor;
    let floor = scaled.floor();
    let diff = scaled - floor;
    let epsilon = 1e-9 * scaled.abs().max(1.0);
    let rounded = if (diff - 0.5).abs() <= epsilon {
        if (floor as i64) % 2 == 0 { floor } else { floor + 1.0 }
    } else {
        scaled.round()
    };
    rounded / factor
}

fn group_thousands(integer: u64) -> String {
    let digits = integer.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// en_US decimal: grouped integer part, `min..=max` fractional digits, half-even rounding.
pub fn decimal(value: f64, min_fraction: u32, max_fraction: u32) -> String {
    let rounded = round_half_even(value, max_fraction);
    let negative = rounded < 0.0 && rounded != 0.0;
    let magnitude = rounded.abs();
    let mut fixed = format!("{magnitude:.prec$}", prec = max_fraction as usize);
    if max_fraction > min_fraction {
        if let Some(dot) = fixed.find('.') {
            let mut end = fixed.len();
            while end > dot + 1 + min_fraction as usize && fixed.as_bytes()[end - 1] == b'0' {
                end -= 1;
            }
            if end == dot + 1 {
                end = dot;
            }
            fixed.truncate(end);
        }
    }
    let (integer, fraction) = match fixed.split_once('.') {
        Some((integer, fraction)) => (integer.to_string(), Some(fraction.to_string())),
        None => (fixed.clone(), None),
    };
    let grouped = group_thousands(integer.parse::<u64>().unwrap_or(0));
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.push_str(&grouped);
    if let Some(fraction) = fraction {
        out.push('.');
        out.push_str(&fraction);
    }
    out
}

/// en_US compact-name notation with up to one fractional digit: `1.2K`, `35.8M`, `2.3B`, `1T`.
pub fn compact(value: f64) -> String {
    const UNITS: [(f64, &str); 4] = [(1e12, "T"), (1e9, "B"), (1e6, "M"), (1e3, "K")];
    let magnitude = value.abs();
    if magnitude < 1000.0 {
        return decimal(value, 0, 1);
    }
    let mut unit_index = UNITS.iter().position(|(threshold, _)| magnitude >= *threshold).unwrap_or(3);
    loop {
        let (threshold, suffix) = UNITS[unit_index];
        let scaled = round_half_even(magnitude / threshold, 1);
        if scaled >= 1000.0 && unit_index > 0 {
            unit_index -= 1;
            continue;
        }
        let sign = if value < 0.0 { "-" } else { "" };
        return format!("{sign}{}{suffix}", decimal(scaled, 0, 1));
    }
}

/// USD currency with a fixed number of fractional digits (`$2,059.07`).
pub fn currency(amount: f64, fraction_digits: u32) -> String {
    let body = decimal(amount.abs(), fraction_digits, fraction_digits);
    if round_half_even(amount, fraction_digits) < 0.0 { format!("-${body}") } else { format!("${body}") }
}

/// Clamp a percent sample into the bounded 0...100 domain.
pub fn clamp_percent(value: f64) -> f64 {
    if value.is_finite() { value.clamp(0.0, 100.0) } else { 0.0 }
}

/// A bare number in the given kind and style (no unit label).
pub fn number(value: f64, kind: MetricKind, style: Style) -> String {
    match kind {
        MetricKind::Percent => format!("{}%", clamp_percent(value).round() as i64),
        MetricKind::Dollars => {
            if value.abs() >= 1000.0 && style != Style::Full {
                return format!("${}", compact(value)).replace("$-", "-$");
            }
            match style {
                Style::Tray => format!("${}", decimal(value, 0, 0)).replace("$-", "-$"),
                Style::Row | Style::Full => currency(value, 2),
            }
        }
        MetricKind::Count => {
            if style != Style::Full && value.abs() >= 1000.0 {
                compact(value)
            } else {
                decimal(value, 0, 1)
            }
        }
    }
}

/// A value with its unit label appended, e.g. "772 credits".
pub fn value_string(value: &MetricValue, style: Style) -> String {
    let text = number(value.number, value.kind, style);
    match value.label.as_deref() {
        Some(label) if !label.is_empty() => format!("{text} {label}"),
        _ => text,
    }
}

/// Compact "Xd Yh" / "Xh Ym" / "Xm" duration. `None` for non-finite or non-positive spans.
pub fn compact_duration(seconds: f64) -> Option<String> {
    if !seconds.is_finite() || seconds <= 0.0 {
        return None;
    }
    let total_minutes = ((seconds / 60.0).ceil() as i64).max(1);
    let days = total_minutes / (24 * 60);
    let hours = (total_minutes % (24 * 60)) / 60;
    let minutes = total_minutes % 60;
    Some(if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        if minutes > 0 { format!("{hours}h {minutes}m") } else { format!("{hours}h") }
    } else {
        format!("{minutes}m")
    })
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// The compact month/day label, e.g. "Jun 21", in the local time zone.
pub fn month_day_label<Tz: TimeZone>(date: &DateTime<Tz>) -> String {
    let local = date.with_timezone(&Local);
    format!("{} {}", MONTHS[local.month0() as usize], local.day())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_counts_match_icu() {
        assert_eq!(compact(35_800_000.0), "35.8M");
        assert_eq!(compact(2_300_000_000.0), "2.3B");
        assert_eq!(compact(738_500.0), "738.5K");
        assert_eq!(compact(1_000.0), "1K");
        assert_eq!(compact(999_960.0), "1M");
        assert_eq!(compact(-1_234.0), "-1.2K");
        assert_eq!(compact(820.6), "820.6");
    }

    #[test]
    fn dollars_follow_each_style() {
        assert_eq!(number(49.85, MetricKind::Dollars, Style::Row), "$49.85");
        assert_eq!(number(2_500.0, MetricKind::Dollars, Style::Row), "$2.5K");
        assert_eq!(number(2_059.07, MetricKind::Dollars, Style::Full), "$2,059.07");
        assert_eq!(number(130.4, MetricKind::Dollars, Style::Tray), "$130");
        assert_eq!(number(13_400.0, MetricKind::Dollars, Style::Tray), "$13.4K");
    }

    #[test]
    fn percent_clamps_and_rounds_half_away() {
        assert_eq!(number(36.5, MetricKind::Percent, Style::Row), "37%");
        assert_eq!(number(104.0, MetricKind::Percent, Style::Row), "100%");
        assert_eq!(number(-3.0, MetricKind::Percent, Style::Tray), "0%");
    }

    #[test]
    fn counts_keep_digits_in_full_style() {
        assert_eq!(number(56_904_995.0, MetricKind::Count, Style::Full), "56,904,995");
        assert_eq!(number(772.0, MetricKind::Count, Style::Row), "772");
    }

    #[test]
    fn value_strings_append_labels() {
        let tokens = MetricValue::count(35_800_000.0, "tokens");
        assert_eq!(value_string(&tokens, Style::Row), "35.8M tokens");
    }

    #[test]
    fn durations_are_compact() {
        assert_eq!(compact_duration(5.0 * 3600.0).as_deref(), Some("5h"));
        assert_eq!(compact_duration(30.0 * 3600.0 + 60.0).as_deref(), Some("1d 6h"));
        assert_eq!(compact_duration(3.0 * 3600.0 + 25.0 * 60.0).as_deref(), Some("3h 25m"));
        assert_eq!(compact_duration(20.0).as_deref(), Some("1m"));
        assert_eq!(compact_duration(0.0), None);
    }

    #[test]
    fn half_even_rounding_matches_icu() {
        assert_eq!(decimal(2.25, 0, 1), "2.2");
        assert_eq!(decimal(2.35, 0, 1), "2.4");
        assert_eq!(currency(0.125, 2), "$0.12");
    }
}
