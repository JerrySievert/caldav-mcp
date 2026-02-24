/// Extracted fields from iCalendar data.
#[derive(Debug, Clone, Default)]
pub struct IcalFields {
    pub uid: Option<String>,
    pub dtstart: Option<String>,
    pub dtend: Option<String>,
    pub summary: Option<String>,
    pub component_type: String,
}

/// Extract key fields from raw iCalendar data.
/// Uses simple line-based parsing to avoid dependency on full iCal parser
/// for field extraction (the raw data is stored as-is).
pub fn extract_fields(ical_data: &str) -> IcalFields {
    let mut fields = IcalFields {
        component_type: "VEVENT".to_string(),
        ..Default::default()
    };

    let mut in_component = false;

    for line in unfold_lines(ical_data) {
        let line = line.trim();

        if line.starts_with("BEGIN:VEVENT") {
            in_component = true;
            fields.component_type = "VEVENT".to_string();
        } else if line.starts_with("BEGIN:VTODO") {
            in_component = true;
            fields.component_type = "VTODO".to_string();
        } else if line.starts_with("END:VEVENT") || line.starts_with("END:VTODO") {
            in_component = false;
        }

        if !in_component && !line.starts_with("UID") {
            continue;
        }

        if let Some(value) = extract_property(line, "UID") {
            fields.uid = Some(value);
        } else if let Some(value) = extract_property(line, "DTSTART") {
            fields.dtstart = Some(value);
        } else if let Some(value) = extract_property(line, "DTEND") {
            fields.dtend = Some(value);
        } else if let Some(value) = extract_property(line, "DUE") {
            // VTODO uses DUE instead of DTEND
            if fields.dtend.is_none() {
                fields.dtend = Some(value);
            }
        } else if let Some(value) = extract_property(line, "SUMMARY") {
            fields.summary = Some(value);
        }
    }

    fields
}

/// Extract a property value, handling parameters (e.g., DTSTART;TZID=...:20260301T090000).
fn extract_property(line: &str, name: &str) -> Option<String> {
    // Match "NAME:" or "NAME;...:"
    if line.starts_with(name) {
        let rest = &line[name.len()..];
        if let Some(stripped) = rest.strip_prefix(':') {
            return Some(stripped.to_string());
        } else if rest.starts_with(';') {
            // Has parameters — find the colon after parameters
            if let Some(colon_pos) = rest.find(':') {
                return Some(rest[colon_pos + 1..].to_string());
            }
        }
    }
    None
}

/// Unfold iCalendar line continuations (lines starting with space or tab).
fn unfold_lines(data: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();

    for raw_line in data.lines() {
        // Strip trailing \r that may remain from \r\n line endings
        let line = raw_line.trim_end_matches('\r');

        if line.starts_with(' ') || line.starts_with('\t') {
            // Continuation line — append without the leading whitespace
            current.push_str(&line[1..]);
        } else {
            if !current.is_empty() {
                result.push(current);
            }
            current = line.to_string();
        }
    }
    if !current.is_empty() {
        result.push(current);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_simple_event() {
        let ical = "BEGIN:VCALENDAR\r\n\
                     VERSION:2.0\r\n\
                     BEGIN:VEVENT\r\n\
                     UID:event-123@example.com\r\n\
                     DTSTART:20260301T090000Z\r\n\
                     DTEND:20260301T100000Z\r\n\
                     SUMMARY:Team Meeting\r\n\
                     END:VEVENT\r\n\
                     END:VCALENDAR";

        let fields = extract_fields(ical);
        assert_eq!(fields.uid.as_deref(), Some("event-123@example.com"));
        assert_eq!(fields.dtstart.as_deref(), Some("20260301T090000Z"));
        assert_eq!(fields.dtend.as_deref(), Some("20260301T100000Z"));
        assert_eq!(fields.summary.as_deref(), Some("Team Meeting"));
        assert_eq!(fields.component_type, "VEVENT");
    }

    #[test]
    fn test_extract_with_parameters() {
        let ical = "BEGIN:VCALENDAR\r\n\
                     BEGIN:VEVENT\r\n\
                     UID:event-456@example.com\r\n\
                     DTSTART;TZID=America/New_York:20260301T090000\r\n\
                     DTEND;TZID=America/New_York:20260301T100000\r\n\
                     SUMMARY;LANGUAGE=en:Lunch Break\r\n\
                     END:VEVENT\r\n\
                     END:VCALENDAR";

        let fields = extract_fields(ical);
        assert_eq!(fields.uid.as_deref(), Some("event-456@example.com"));
        assert_eq!(fields.dtstart.as_deref(), Some("20260301T090000"));
        assert_eq!(fields.summary.as_deref(), Some("Lunch Break"));
    }

    #[test]
    fn test_extract_vtodo() {
        let ical = "BEGIN:VCALENDAR\r\n\
                     BEGIN:VTODO\r\n\
                     UID:todo-1@example.com\r\n\
                     DUE:20260315T170000Z\r\n\
                     SUMMARY:Buy groceries\r\n\
                     END:VTODO\r\n\
                     END:VCALENDAR";

        let fields = extract_fields(ical);
        assert_eq!(fields.uid.as_deref(), Some("todo-1@example.com"));
        assert_eq!(fields.dtend.as_deref(), Some("20260315T170000Z"));
        assert_eq!(fields.component_type, "VTODO");
    }

    #[test]
    fn test_unfold_lines() {
        let data = "SUMMARY:This is a long\r\n summary that wraps\r\n";
        let lines = unfold_lines(data);
        assert!(
            lines.iter().any(|l| l == "SUMMARY:This is a longsummary that wraps"),
            "Expected unfolded line, got: {:?}",
            lines
        );
    }

    #[test]
    fn test_uid_outside_component() {
        // UID can appear at the VCALENDAR level in some implementations
        let ical = "BEGIN:VCALENDAR\r\n\
                     UID:cal-level-uid@example.com\r\n\
                     BEGIN:VEVENT\r\n\
                     DTSTART:20260301T090000Z\r\n\
                     END:VEVENT\r\n\
                     END:VCALENDAR";

        let fields = extract_fields(ical);
        assert_eq!(fields.uid.as_deref(), Some("cal-level-uid@example.com"));
    }
}
