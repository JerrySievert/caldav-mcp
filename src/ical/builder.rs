use uuid::Uuid;

/// Build a minimal VCALENDAR wrapping a VEVENT.
pub fn build_vevent(
    uid: &str,
    summary: &str,
    dtstart: &str,
    dtend: &str,
    description: Option<&str>,
    location: Option<&str>,
) -> String {
    let now = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//CalDAV Server//EN".to_string(),
        "BEGIN:VEVENT".to_string(),
        format!("UID:{uid}"),
        format!("DTSTAMP:{now}"),
        format!("DTSTART:{dtstart}"),
        format!("DTEND:{dtend}"),
        format!("SUMMARY:{summary}"),
    ];

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
        );

        assert!(ical.contains("BEGIN:VCALENDAR"));
        assert!(ical.contains("END:VCALENDAR"));
        assert!(ical.contains("BEGIN:VEVENT"));
        assert!(ical.contains("UID:test-uid@example.com"));
        assert!(ical.contains("SUMMARY:Test Event"));
        assert!(ical.contains("DTSTART:20260301T090000Z"));
        assert!(ical.contains("DESCRIPTION:A description"));
        assert!(ical.contains("LOCATION:Room 101"));
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
        );

        assert!(ical.contains("UID:min-uid@example.com"));
        assert!(!ical.contains("DESCRIPTION:"));
        assert!(!ical.contains("LOCATION:"));
    }

    #[test]
    fn test_generate_uid() {
        let uid = generate_uid();
        assert!(uid.contains("@caldav-server"));
        assert!(uid.len() > 20);
    }
}
