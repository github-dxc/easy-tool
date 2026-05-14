//! Static application constants shared by UI and feature modules.

pub const APP_INSTANCE_ID: &str = "my_unique_easy_tool_app_id";

// Supported timezone identifiers used by the timestamp conversion feature.
pub const TIMEZONES: [&str; 12] = [
    "Asia/Shanghai",
    "Etc/UTC",
    "Asia/Tokyo",
    "Asia/Kolkata",
    "Asia/Singapore",
    "Europe/London",
    "Europe/Paris",
    "America/New_York",
    "America/Chicago",
    "America/Denver",
    "America/Los_Angeles",
    "Australia/Sydney",
];

// Short labels displayed in the timezone switcher.
pub const TIMEZONE_LABELS: [&str; 12] = [
    "CST/+8", "UTC/+0", "JST/+9", "IST/+5.5", "SGT/+8", "BST/+1", "CET/+1", "ET/-5", "CT/-6",
    "MT/-7", "PT/-8", "AET/+10",
];
