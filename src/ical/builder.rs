use uuid::Uuid;

/// Build a minimal VCALENDAR wrapping a VEVENT.
///
/// If `timezone` is `Some("America/Los_Angeles")` (or any IANA tz name), the
/// DTSTART/DTEND lines are emitted as `DTSTART;TZID=…` and a minimal
/// VTIMEZONE component is included.  When `timezone` is `None` the values are
/// written verbatim (caller is responsible for supplying a UTC `Z`-suffixed
/// value or any other valid iCal datetime string).
pub fn build_vevent(
    uid: &str,
    summary: &str,
    dtstart: &str,
    dtend: &str,
    description: Option<&str>,
    location: Option<&str>,
    timezone: Option<&str>,
) -> String {
    let now = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");

    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//CalDAV Server//EN".to_string(),
    ];

    if let Some(tz) = timezone {
        // Minimal VTIMEZONE — enough for Apple Calendar / RFC 5545 compliance.
        // Using RRULE-based definitions so the component stays compact while
        // correctly representing DST transitions for common US timezones.
        lines.push("BEGIN:VTIMEZONE".to_string());
        lines.push(format!("TZID:{tz}"));
        lines.push("BEGIN:STANDARD".to_string());
        lines.push("DTSTART:19671029T020000".to_string());
        lines.push("RRULE:FREQ=YEARLY;BYDAY=1SU;BYMONTH=11".to_string());
        lines.push(vtimezone_std_offset(tz));
        lines.push(vtimezone_dst_offset(tz));
        lines.push("END:STANDARD".to_string());
        lines.push("BEGIN:DAYLIGHT".to_string());
        lines.push("DTSTART:20070311T020000".to_string());
        lines.push("RRULE:FREQ=YEARLY;BYDAY=2SU;BYMONTH=3".to_string());
        lines.push(vtimezone_dst_offset(tz));
        lines.push(vtimezone_std_offset(tz));
        lines.push("END:DAYLIGHT".to_string());
        lines.push("END:VTIMEZONE".to_string());
    }

    lines.push("BEGIN:VEVENT".to_string());
    lines.push(format!("UID:{uid}"));
    lines.push(format!("DTSTAMP:{now}"));

    if let Some(tz) = timezone {
        lines.push(format!("DTSTART;TZID={tz}:{dtstart}"));
        lines.push(format!("DTEND;TZID={tz}:{dtend}"));
    } else {
        lines.push(format!("DTSTART:{dtstart}"));
        lines.push(format!("DTEND:{dtend}"));
    }

    lines.push(format!("SUMMARY:{summary}"));

    if let Some(desc) = description {
        lines.push(format!("DESCRIPTION:{desc}"));
    }
    if let Some(loc) = location {
        lines.push(format!("LOCATION:{loc}"));
    }

    lines.push("END:VEVENT".to_string());
    lines.push("END:VCALENDAR".to_string());

    lines.join("\r\n") + "\r\n"
}

/// Returns the TZOFFSETFROM line for the standard (winter) period of a timezone.
fn vtimezone_std_offset(tz: &str) -> String {
    let offset = match tz {
        "America/Los_Angeles" | "America/Vancouver" => "-0800",
        "America/Denver" | "America/Phoenix" => "-0700",
        "America/Chicago" => "-0600",
        "America/New_York" | "America/Toronto" => "-0500",
        "Europe/London" => "+0000",
        "Europe/Paris" | "Europe/Berlin" | "Europe/Rome" => "+0100",
        "Asia/Tokyo" => "+0900",
        "Australia/Sydney" => "+1100",
        _ => "+0000",
    };
    format!("TZOFFSETFROM:{offset}")
}

/// Returns the TZOFFSETTO line for the daylight-saving (summer) period of a timezone.
fn vtimezone_dst_offset(tz: &str) -> String {
    let offset = match tz {
        "America/Los_Angeles" | "America/Vancouver" => "-0700",
        "America/Denver" => "-0600",
        "America/Chicago" => "-0500",
        "America/New_York" | "America/Toronto" => "-0400",
        "Europe/London" => "+0100",
        "Europe/Paris" | "Europe/Berlin" | "Europe/Rome" => "+0200",
        // Tokyo and Phoenix don't observe DST — use the same offset
        "Asia/Tokyo" | "America/Phoenix" => "+0900",
        "Australia/Sydney" => "+1100",
        _ => "+0000",
    };
    format!("TZOFFSETTO:{offset}")
}

/// Generate a new unique event UID.
pub fn generate_uid() -> String {
    format!("{}@caldav-server", Uuid::new_v4())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_vevent() {
        let ical = build_vevent(
            "test-uid@example.com",
            "Test Event",
            "20260301T090000Z",
            "20260301T100000Z",
            Some("A description"),
            Some("Room 101"),
            None,
        );

        assert!(ical.contains("BEGIN:VCALENDAR"));
        assert!(ical.contains("END:VCALENDAR"));
        assert!(ical.contains("BEGIN:VEVENT"));
        assert!(ical.contains("UID:test-uid@example.com"));
        assert!(ical.contains("SUMMARY:Test Event"));
        assert!(ical.contains("DTSTART:20260301T090000Z"));
        assert!(ical.contains("DESCRIPTION:A description"));
        assert!(ical.contains("LOCATION:Room 101"));
        assert!(!ical.contains("VTIMEZONE"));
    }

    #[test]
    fn test_build_vevent_minimal() {
        let ical = build_vevent(
            "min-uid@example.com",
            "Minimal",
            "20260301T090000Z",
            "20260301T100000Z",
            None,
            None,
            None,
        );

        assert!(ical.contains("UID:min-uid@example.com"));
        assert!(!ical.contains("DESCRIPTION:"));
        assert!(!ical.contains("LOCATION:"));
    }

    #[test]
    fn test_build_vevent_with_timezone() {
        let ical = build_vevent(
            "tz-uid@example.com",
            "TZ Event",
            "20260301T090000",
            "20260301T100000",
            None,
            None,
            Some("America/Los_Angeles"),
        );

        assert!(ical.contains("BEGIN:VTIMEZONE"));
        assert!(ical.contains("TZID:America/Los_Angeles"));
        assert!(ical.contains("DTSTART;TZID=America/Los_Angeles:20260301T090000"));
        assert!(ical.contains("DTEND;TZID=America/Los_Angeles:20260301T100000"));
        assert!(!ical.contains("DTSTART:20260301")); // should not appear without TZID
    }

    #[test]
    fn test_generate_uid() {
        let uid = generate_uid();
        assert!(uid.contains("@caldav-server"));
        assert!(uid.len() > 20);
    }

    #[test]
    fn test_generate_uid_unique() {
        let uid1 = generate_uid();
        let uid2 = generate_uid();
        assert_ne!(uid1, uid2, "Generated UIDs should be unique");
    }

    #[test]
    fn test_build_vevent_with_eastern_timezone() {
        let ical = build_vevent(
            "tz-east@example.com",
            "Eastern Event",
            "20260301T090000",
            "20260301T100000",
            None,
            None,
            Some("America/New_York"),
        );
        assert!(ical.contains("TZID:America/New_York"));
        assert!(ical.contains("TZOFFSETFROM:-0500"));
        assert!(ical.contains("TZOFFSETTO:-0400"));
    }

    #[test]
    fn test_build_vevent_with_chicago_timezone() {
        let ical = build_vevent(
            "tz-chicago@example.com",
            "Chicago Event",
            "20260301T090000",
            "20260301T100000",
            None,
            None,
            Some("America/Chicago"),
        );
        assert!(ical.contains("TZID:America/Chicago"));
        assert!(ical.contains("TZOFFSETFROM:-0600"));
        assert!(ical.contains("TZOFFSETTO:-0500"));
    }

    #[test]
    fn test_build_vevent_with_london_timezone() {
        let ical = build_vevent(
            "tz-london@example.com",
            "London Event",
            "20260601T090000",
            "20260601T100000",
            None,
            None,
            Some("Europe/London"),
        );
        assert!(ical.contains("TZID:Europe/London"));
        assert!(ical.contains("TZOFFSETFROM:+0000"));
        assert!(ical.contains("TZOFFSETTO:+0100"));
    }

    #[test]
    fn test_build_vevent_with_paris_timezone() {
        let ical = build_vevent(
            "tz-paris@example.com",
            "Paris Event",
            "20260601T090000",
            "20260601T100000",
            None,
            None,
            Some("Europe/Paris"),
        );
        assert!(ical.contains("TZID:Europe/Paris"));
        assert!(ical.contains("TZOFFSETFROM:+0100"));
        assert!(ical.contains("TZOFFSETTO:+0200"));
    }

    #[test]
    fn test_build_vevent_with_tokyo_timezone() {
        let ical = build_vevent(
            "tz-tokyo@example.com",
            "Tokyo Event",
            "20260601T090000",
            "20260601T100000",
            None,
            None,
            Some("Asia/Tokyo"),
        );
        assert!(ical.contains("TZID:Asia/Tokyo"));
        assert!(ical.contains("TZOFFSETFROM:+0900"));
    }

    #[test]
    fn test_build_vevent_with_unknown_timezone() {
        let ical = build_vevent(
            "tz-unknown@example.com",
            "Unknown TZ Event",
            "20260601T090000",
            "20260601T100000",
            None,
            None,
            Some("Pacific/Fake"),
        );
        assert!(ical.contains("TZID:Pacific/Fake"));
        // Unknown TZ falls back to +0000
        assert!(ical.contains("TZOFFSETFROM:+0000"));
    }

    #[test]
    fn test_build_vevent_with_denver_timezone() {
        let ical = build_vevent(
            "tz-denver@example.com",
            "Denver Event",
            "20260301T090000",
            "20260301T100000",
            None,
            None,
            Some("America/Denver"),
        );
        assert!(ical.contains("TZID:America/Denver"));
        assert!(ical.contains("TZOFFSETFROM:-0700"));
        assert!(ical.contains("TZOFFSETTO:-0600"));
    }

    #[test]
    fn test_build_vevent_with_phoenix_timezone() {
        let ical = build_vevent(
            "tz-phoenix@example.com",
            "Phoenix Event",
            "20260601T090000",
            "20260601T100000",
            None,
            None,
            Some("America/Phoenix"),
        );
        assert!(ical.contains("TZID:America/Phoenix"));
        // Phoenix TZOFFSETFROM (standard offset) is -0700
        assert!(ical.contains("TZOFFSETFROM:-0700"));
    }

    #[test]
    fn test_vevent_output_ends_with_crlf() {
        let ical = build_vevent(
            "crlf@test.com",
            "CRLF Test",
            "20260101T000000Z",
            "20260101T010000Z",
            None,
            None,
            None,
        );
        assert!(ical.ends_with("\r\n"), "iCal output must end with CRLF");
    }
}
