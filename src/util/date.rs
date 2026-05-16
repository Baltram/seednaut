use chrono::{DateTime, Local, Utc};

fn millis_to_local(millis: u64) -> Option<DateTime<Local>> {
    let millis = i64::try_from(millis).ok()?;
    let utc = DateTime::<Utc>::from_timestamp_millis(millis)?;
    Some(utc.with_timezone(&Local))
}

/// Formats a timestamp (milliseconds since epoch) for display in the UI.
/// Uses the local time zone and the default locale format (%c).
///
/// If the timestamp is out of range, returns a visible placeholder instead of
/// silently falling back to the Unix epoch.
pub fn format_display(millis: u64) -> String {
    millis_to_local(millis)
        .map(|dt| dt.format("%c").to_string())
        .unwrap_or_else(|| "<invalid timestamp>".to_string())
}

/// Formats a timestamp (milliseconds since epoch) for use in filenames.
/// Format: YYYY-MM-DD_HH-MM-SS
///
/// If the timestamp is out of range, returns a visible placeholder instead of
/// silently falling back to the Unix epoch.
pub fn format_filename(millis: u64) -> String {
    millis_to_local(millis)
        .map(|dt| dt.format("%Y-%m-%d_%H-%M-%S").to_string())
        .unwrap_or_else(|| "invalid-timestamp".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_display_invalid_timestamp() {
        assert_eq!(format_display(u64::MAX), "<invalid timestamp>");
    }

    #[test]
    fn test_format_filename_invalid_timestamp() {
        assert_eq!(format_filename(u64::MAX), "invalid-timestamp");
    }
}
