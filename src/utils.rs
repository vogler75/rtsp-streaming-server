use chrono::Duration;
use crate::errors::{Result, StreamError};

/// Parse a duration string like "10m", "5h", "7d" into a chrono::Duration
pub fn parse_duration(s: &str) -> Result<Duration> {
    if s.is_empty() {
        return Err(StreamError::config("Empty duration string"));
    }
    
    let (number_part, unit_part) = s.split_at(s.len() - 1);
    
    let value: i64 = number_part.parse()
        .map_err(|_| StreamError::config(&format!("Invalid number in duration: {}", number_part)))?;
    
    if value <= 0 {
        return Err(StreamError::config("Duration must be positive"));
    }
    
    let duration = match unit_part {
        "m" => Duration::minutes(value),
        "h" => Duration::hours(value),
        "d" => Duration::days(value),
        _ => return Err(StreamError::config(&format!("Invalid duration unit '{}'. Use 'm' for minutes, 'h' for hours, or 'd' for days", unit_part))),
    };
    
    Ok(duration)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("10m").unwrap(), Duration::minutes(10));
        assert_eq!(parse_duration("5h").unwrap(), Duration::hours(5));
        assert_eq!(parse_duration("7d").unwrap(), Duration::days(7));
        assert_eq!(parse_duration("30d").unwrap(), Duration::days(30));
        assert_eq!(parse_duration("1h").unwrap(), Duration::hours(1));
        
        assert!(parse_duration("").is_err());
        assert!(parse_duration("10").is_err());
        assert!(parse_duration("m").is_err());
        assert!(parse_duration("-5h").is_err());
        assert!(parse_duration("10x").is_err());
    }
}