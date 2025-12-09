use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use regex::Regex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TimestampError {
    #[error("regex compilation failed: {0}")]
    InvalidRegex(#[from] regex::Error),

    #[error("pattern missing 'ts' capture group")]
    MissingTsGroup,

    #[error("failed to parse timestamp '{value}' with format '{format}': {source}")]
    ParseError {
        value: String,
        format: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

#[derive(Debug, Clone)]
pub enum TimestampFormat {
    Strptime(String),
    Iso8601,
    Epoch,
    EpochMs,
}

#[derive(Debug)]
pub struct TimestampExtractor {
    pattern: Regex,
    format: TimestampFormat,
}

impl TimestampExtractor {
    /// Create a new TimestampExtractor
    ///
    /// # Arguments
    /// * `pattern` - Regex pattern that must contain a named capture group 'ts'
    /// * `format` - One of: strptime format string, 'iso8601', 'epoch', 'epoch_ms'
    pub fn new(pattern: &str, format: &str) -> Result<Self, TimestampError> {
        let regex = Regex::new(pattern)?;

        // Validate that the pattern has a 'ts' capture group
        if regex.capture_names().all(|name| name != Some("ts")) {
            return Err(TimestampError::MissingTsGroup);
        }

        let timestamp_format = match format {
            "iso8601" => TimestampFormat::Iso8601,
            "epoch" => TimestampFormat::Epoch,
            "epoch_ms" => TimestampFormat::EpochMs,
            other => TimestampFormat::Strptime(other.to_string()),
        };

        Ok(Self {
            pattern: regex,
            format: timestamp_format,
        })
    }

    /// Extract timestamp from a log line
    ///
    /// Returns None if the pattern doesn't match the line.
    /// Returns Some(DateTime<Utc>) if matched and successfully parsed.
    pub fn extract(&self, line: &str) -> Result<Option<DateTime<Utc>>, TimestampError> {
        let Some(captures) = self.pattern.captures(line) else {
            return Ok(None);
        };

        let ts_value = captures
            .name("ts")
            .expect("ts capture group must exist")
            .as_str();

        let datetime = match &self.format {
            TimestampFormat::Iso8601 => self.parse_iso8601(ts_value)?,
            TimestampFormat::Epoch => self.parse_epoch(ts_value)?,
            TimestampFormat::EpochMs => self.parse_epoch_ms(ts_value)?,
            TimestampFormat::Strptime(fmt) => self.parse_strptime(ts_value, fmt)?,
        };

        Ok(Some(datetime))
    }

    fn parse_iso8601(&self, value: &str) -> Result<DateTime<Utc>, TimestampError> {
        // Try parsing as RFC3339 (which handles ISO8601 with timezone)
        DateTime::parse_from_rfc3339(value)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|e| TimestampError::ParseError {
                value: value.to_string(),
                format: "iso8601".to_string(),
                source: Box::new(e),
            })
    }

    fn parse_epoch(&self, value: &str) -> Result<DateTime<Utc>, TimestampError> {
        let seconds: i64 =
            value
                .parse()
                .map_err(|e: std::num::ParseIntError| TimestampError::ParseError {
                    value: value.to_string(),
                    format: "epoch".to_string(),
                    source: Box::new(e),
                })?;

        Utc.timestamp_opt(seconds, 0)
            .single()
            .ok_or_else(|| TimestampError::ParseError {
                value: value.to_string(),
                format: "epoch".to_string(),
                source: Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "timestamp out of range",
                )),
            })
    }

    fn parse_epoch_ms(&self, value: &str) -> Result<DateTime<Utc>, TimestampError> {
        let millis: i64 =
            value
                .parse()
                .map_err(|e: std::num::ParseIntError| TimestampError::ParseError {
                    value: value.to_string(),
                    format: "epoch_ms".to_string(),
                    source: Box::new(e),
                })?;

        let seconds = millis / 1000;
        let nanos = ((millis % 1000) * 1_000_000) as u32;

        Utc.timestamp_opt(seconds, nanos)
            .single()
            .ok_or_else(|| TimestampError::ParseError {
                value: value.to_string(),
                format: "epoch_ms".to_string(),
                source: Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "timestamp out of range",
                )),
            })
    }

    fn parse_strptime(&self, value: &str, format: &str) -> Result<DateTime<Utc>, TimestampError> {
        // Check if format contains timezone information
        if format.contains("%z") || format.contains("%Z") || format.contains("%:z") {
            // Parse with timezone and convert to UTC
            DateTime::parse_from_str(value, format)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| TimestampError::ParseError {
                    value: value.to_string(),
                    format: format.to_string(),
                    source: Box::new(e),
                })
        } else {
            // Parse as naive datetime and assume UTC
            NaiveDateTime::parse_from_str(value, format)
                .map(|ndt| Utc.from_utc_datetime(&ndt))
                .map_err(|e| TimestampError::ParseError {
                    value: value.to_string(),
                    format: format.to_string(),
                    source: Box::new(e),
                })
        }
    }
}

// Keep the old function signature for backward compatibility if needed
pub fn extract_timestamp(_line: &str, _pattern: &str, _format: &str) -> Option<DateTime<Utc>> {
    // This is now deprecated in favor of TimestampExtractor
    // Could implement as wrapper if needed
    todo!("use TimestampExtractor instead")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_iso8601_basic() {
        let extractor = TimestampExtractor::new(
            r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z)",
            "iso8601",
        )
        .unwrap();

        let result = extractor
            .extract("2025-12-04T02:42:11.011Z some log message")
            .unwrap()
            .unwrap();

        assert_eq!(result.to_rfc3339(), "2025-12-04T02:42:11.011+00:00");
    }

    #[test]
    fn test_iso8601_with_offset() {
        let extractor = TimestampExtractor::new(
            r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}[+-]\d{2}:\d{2})",
            "iso8601",
        )
        .unwrap();

        let result = extractor
            .extract("2025-12-04T02:42:11+05:30 some log message")
            .unwrap()
            .unwrap();

        // Convert to UTC: 02:42:11 + 05:30 -> should be previous day 21:12:11
        assert_eq!(result.to_rfc3339(), "2025-12-03T21:12:11+00:00");
    }

    #[test]
    fn test_nginx_format() {
        let extractor = TimestampExtractor::new(
            r"\[(?P<ts>\d{2}/\w{3}/\d{4}:\d{2}:\d{2}:\d{2} [+-]\d{4})\]",
            "%d/%b/%Y:%H:%M:%S %z",
        )
        .unwrap();

        let result = extractor
            .extract("[04/Dec/2025:02:42:11 +0000] GET /index.html")
            .unwrap()
            .unwrap();

        assert_eq!(result.to_rfc3339(), "2025-12-04T02:42:11+00:00");
    }

    #[test]
    fn test_epoch_seconds() {
        let extractor = TimestampExtractor::new(r"^(?P<ts>\d{10})", "epoch").unwrap();

        let result = extractor
            .extract("1733280131 log message")
            .unwrap()
            .unwrap();

        assert_eq!(result.timestamp(), 1733280131);
    }

    #[test]
    fn test_epoch_milliseconds() {
        let extractor = TimestampExtractor::new(r"^(?P<ts>\d{13})", "epoch_ms").unwrap();

        let result = extractor
            .extract("1733280131011 log message")
            .unwrap()
            .unwrap();

        assert_eq!(result.timestamp(), 1733280131);
        assert_eq!(result.timestamp_subsec_millis(), 11);
    }

    #[test]
    fn test_custom_strptime_comma_millis() {
        let extractor = TimestampExtractor::new(
            r"^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3})",
            "%Y-%m-%d %H:%M:%S,%3f",
        )
        .unwrap();

        let result = extractor
            .extract("2025-12-04 02:42:11,011 log message")
            .unwrap()
            .unwrap();

        assert_eq!(result.to_rfc3339(), "2025-12-04T02:42:11.011+00:00");
    }

    #[test]
    fn test_no_match_returns_none() {
        let extractor = TimestampExtractor::new(r"^(?P<ts>\d{4}-\d{2}-\d{2})", "iso8601").unwrap();

        let result = extractor.extract("this line has no timestamp").unwrap();

        assert!(result.is_none());
    }

    #[test]
    fn test_missing_ts_group_error() {
        let result = TimestampExtractor::new(r"^\d{4}-\d{2}-\d{2}", "iso8601");

        assert!(matches!(result, Err(TimestampError::MissingTsGroup)));
    }

    #[test]
    fn test_invalid_regex() {
        let result = TimestampExtractor::new(r"(?P<ts>[invalid", "iso8601");

        assert!(matches!(result, Err(TimestampError::InvalidRegex(_))));
    }

    #[test]
    fn test_unparseable_timestamp() {
        let extractor = TimestampExtractor::new(r"^(?P<ts>\S+)", "epoch").unwrap();

        let result = extractor.extract("not_a_number log message");

        assert!(matches!(result, Err(TimestampError::ParseError { .. })));
    }

    #[test]
    fn test_timezone_aware_strptime() {
        let extractor = TimestampExtractor::new(
            r"^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} [+-]\d{4})",
            "%Y-%m-%d %H:%M:%S %z",
        )
        .unwrap();

        let result = extractor
            .extract("2025-12-04 02:42:11 +0530 log message")
            .unwrap()
            .unwrap();

        // Should convert to UTC
        assert_eq!(result.to_rfc3339(), "2025-12-03T21:12:11+00:00");
    }

    #[test]
    fn test_timezone_naive_assumes_utc() {
        let extractor = TimestampExtractor::new(
            r"^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})",
            "%Y-%m-%d %H:%M:%S",
        )
        .unwrap();

        let result = extractor
            .extract("2025-12-04 02:42:11 log message")
            .unwrap()
            .unwrap();

        // Should be interpreted as UTC
        assert_eq!(result.to_rfc3339(), "2025-12-04T02:42:11+00:00");
    }
}
