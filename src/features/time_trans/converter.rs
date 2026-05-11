use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use regex::Regex;

pub fn trans_string_timestamp(
    input: &str,
    unit: bool,
    zone: &str,
) -> (Result<String, String>, Option<bool>) {
    let tz: Tz = zone.parse().unwrap_or_default();

    if Regex::new(r"^[12]\d{9}$").unwrap().is_match(input) {
        let timestamp = input.parse().unwrap();
        let dt = Utc.timestamp_opt(timestamp, 0).single().unwrap();
        return (
            Ok(dt
                .with_timezone(&tz)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()),
            Some(false),
        );
    }

    if Regex::new(r"^[12]\d{12}$").unwrap().is_match(input) {
        let timestamp = input.parse::<i64>().unwrap() / 1000;
        let dt = Utc.timestamp_opt(timestamp, 0).single().unwrap();
        return (
            Ok(dt
                .with_timezone(&tz)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()),
            Some(true),
        );
    }

    if Regex::new(r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$")
        .unwrap()
        .is_match(input)
    {
        if let Some(timestamp) = local_time_to_timestamp(input, "%Y-%m-%d %H:%M:%S", tz, unit) {
            return (Ok(timestamp), None);
        }
    }

    if Regex::new(r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$")
        .unwrap()
        .is_match(input)
    {
        let input = format!("{input}:00");
        if let Some(timestamp) = local_time_to_timestamp(&input, "%Y-%m-%d %H:%M:%S", tz, unit) {
            return (Ok(timestamp), None);
        }
    }

    if Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}[+-]\d{2}:\d{2}$")
        .unwrap()
        .is_match(input)
    {
        if let Ok(dt) = DateTime::parse_from_rfc3339(input) {
            return (Ok(scale_timestamp(dt.timestamp(), unit).to_string()), None);
        }
    }

    if Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap().is_match(input) {
        let input = format!("{input} 00:00:00");
        if let Some(timestamp) = local_time_to_timestamp(&input, "%Y-%m-%d %H:%M:%S", tz, unit) {
            return (Ok(timestamp), None);
        }
    }

    if Regex::new(r"^[A-Za-z]{3}, \d{1,2} [A-Za-z]{3} \d{4} \d{2}:\d{2}:\d{2} [+-]\d{4}$")
        .unwrap()
        .is_match(input)
    {
        if let Ok(dt) = DateTime::parse_from_rfc2822(input) {
            return (Ok(scale_timestamp(dt.timestamp(), unit).to_string()), None);
        }
    }

    (Err("Error".to_string()), None)
}

fn local_time_to_timestamp(input: &str, format: &str, tz: Tz, unit: bool) -> Option<String> {
    let naive_dt = NaiveDateTime::parse_from_str(input, format).ok()?;
    let dt = tz.from_local_datetime(&naive_dt).single()?;
    Some(scale_timestamp(dt.timestamp(), unit).to_string())
}

fn scale_timestamp(timestamp: i64, unit: bool) -> i64 {
    if unit { timestamp * 1000 } else { timestamp }
}
