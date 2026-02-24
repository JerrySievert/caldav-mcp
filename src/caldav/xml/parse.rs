use quick_xml::events::Event;
use quick_xml::reader::Reader;

/// Parsed PROPFIND request body.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum PropfindRequest {
    /// Client wants all properties
    AllProp,
    /// Client wants just property names
    PropName,
    /// Client wants specific properties
    Props(Vec<PropRequest>),
}

/// A single requested property with namespace and local name.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PropRequest {
    pub namespace: String,
    pub local_name: String,
}

/// Parse a PROPFIND request body. Returns AllProp if body is empty.
pub fn parse_propfind(body: &[u8]) -> PropfindRequest {
    if body.is_empty() {
        return PropfindRequest::AllProp;
    }

    let mut reader = Reader::from_reader(body);
    reader.config_mut().trim_text(true);

    let mut in_prop = false;
    let mut props = Vec::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let local = String::from_utf8_lossy(e.local_name().as_ref()).to_string();

                match local.as_str() {
                    "allprop" => return PropfindRequest::AllProp,
                    "propname" => return PropfindRequest::PropName,
                    "prop" => {
                        in_prop = true;
                    }
                    _ if in_prop => {
                        let ns = resolve_namespace(&reader, e);
                        props.push(PropRequest {
                            namespace: ns,
                            local_name: local,
                        });
                    }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                let local = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                if local == "prop" {
                    in_prop = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => return PropfindRequest::AllProp,
            _ => {}
        }
        buf.clear();
    }

    if props.is_empty() {
        PropfindRequest::AllProp
    } else {
        PropfindRequest::Props(props)
    }
}

/// Resolve the namespace of an element from its prefix.
fn resolve_namespace<R: std::io::BufRead>(
    _reader: &Reader<R>,
    event: &quick_xml::events::BytesStart,
) -> String {
    // Check for explicit xmlns on the element
    for attr in event.attributes().flatten() {
        let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
        if key == "xmlns" {
            return String::from_utf8_lossy(&attr.value).to_string();
        }
    }

    // Try to match based on prefix
    let name = String::from_utf8_lossy(event.name().as_ref()).to_string();
    if let Some((prefix, _)) = name.split_once(':') {
        match prefix {
            "D" | "d" => return super::DAV_NS.to_string(),
            "C" | "c" | "cal" => return super::CALDAV_NS.to_string(),
            "A" | "IC" => return super::APPLE_NS.to_string(),
            "CS" | "cs" => return super::CS_NS.to_string(),
            _ => {}
        }
    }

    // Default to DAV:
    super::DAV_NS.to_string()
}

/// Parsed REPORT request body.
#[derive(Debug, Clone)]
pub enum ReportRequest {
    CalendarMultiget {
        props: Vec<PropRequest>,
        hrefs: Vec<String>,
    },
    CalendarQuery {
        props: Vec<PropRequest>,
        time_range: Option<(String, String)>,
    },
    SyncCollection {
        props: Vec<PropRequest>,
        sync_token: String,
    },
}

/// Parse a REPORT request body.
pub fn parse_report(body: &[u8]) -> Option<ReportRequest> {
    if body.is_empty() {
        return None;
    }

    let mut reader = Reader::from_reader(body);
    reader.config_mut().trim_text(true);

    let mut buf = Vec::new();
    let mut report_type: Option<String> = None;
    let mut in_prop = false;
    let mut _in_filter = false;
    let mut props = Vec::new();
    let mut hrefs = Vec::new();
    let mut time_start = String::new();
    let mut time_end = String::new();
    let mut sync_token = String::new();
    let mut in_sync_token = false;
    let mut in_href = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let local = String::from_utf8_lossy(e.local_name().as_ref()).to_string();

                match local.as_str() {
                    "calendar-multiget" => report_type = Some("multiget".to_string()),
                    "calendar-query" => report_type = Some("query".to_string()),
                    "sync-collection" => report_type = Some("sync".to_string()),
                    "prop" => in_prop = true,
                    "filter" | "comp-filter" => _in_filter = true,
                    "href" => in_href = true,
                    "sync-token" => in_sync_token = true,
                    "time-range" => {
                        for attr in e.attributes().flatten() {
                            let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                            let val = String::from_utf8_lossy(&attr.value).to_string();
                            match key.as_str() {
                                "start" => time_start = val,
                                "end" => time_end = val,
                                _ => {}
                            }
                        }
                    }
                    _ if in_prop => {
                        let ns = resolve_namespace(&reader, e);
                        props.push(PropRequest {
                            namespace: ns,
                            local_name: local,
                        });
                    }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                let local = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                match local.as_str() {
                    "prop" => in_prop = false,
                    "filter" | "comp-filter" => _in_filter = false,
                    "href" => in_href = false,
                    "sync-token" => in_sync_token = false,
                    _ => {}
                }
            }
            Ok(Event::Text(ref e)) => {
                let text = e.unescape().unwrap_or_default().to_string();
                if in_href {
                    hrefs.push(text);
                } else if in_sync_token {
                    sync_token = text;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }

    match report_type.as_deref() {
        Some("multiget") => Some(ReportRequest::CalendarMultiget { props, hrefs }),
        Some("query") => {
            let time_range = if !time_start.is_empty() && !time_end.is_empty() {
                Some((time_start, time_end))
            } else {
                None
            };
            Some(ReportRequest::CalendarQuery { props, time_range })
        }
        Some("sync") => Some(ReportRequest::SyncCollection { props, sync_token }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty_propfind() {
        let result = parse_propfind(b"");
        assert!(matches!(result, PropfindRequest::AllProp));
    }

    #[test]
    fn test_parse_allprop() {
        let xml = br#"<?xml version="1.0" encoding="utf-8"?>
        <D:propfind xmlns:D="DAV:">
            <D:allprop/>
        </D:propfind>"#;
        let result = parse_propfind(xml);
        assert!(matches!(result, PropfindRequest::AllProp));
    }

    #[test]
    fn test_parse_specific_props() {
        let xml = br#"<?xml version="1.0" encoding="utf-8"?>
        <D:propfind xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
            <D:prop>
                <D:displayname/>
                <D:resourcetype/>
                <C:calendar-home-set/>
            </D:prop>
        </D:propfind>"#;
        let result = parse_propfind(xml);
        match result {
            PropfindRequest::Props(props) => {
                assert_eq!(props.len(), 3);
                assert_eq!(props[0].local_name, "displayname");
                assert_eq!(props[1].local_name, "resourcetype");
                assert_eq!(props[2].local_name, "calendar-home-set");
            }
            _ => panic!("Expected Props variant"),
        }
    }

    #[test]
    fn test_parse_calendar_multiget() {
        let xml = br#"<?xml version="1.0" encoding="utf-8"?>
        <C:calendar-multiget xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
            <D:prop>
                <D:getetag/>
                <C:calendar-data/>
            </D:prop>
            <D:href>/caldav/users/alice/work/event1.ics</D:href>
            <D:href>/caldav/users/alice/work/event2.ics</D:href>
        </C:calendar-multiget>"#;
        let result = parse_report(xml).unwrap();
        match result {
            ReportRequest::CalendarMultiget { props, hrefs } => {
                assert_eq!(props.len(), 2);
                assert_eq!(hrefs.len(), 2);
                assert!(hrefs[0].contains("event1.ics"));
            }
            _ => panic!("Expected CalendarMultiget"),
        }
    }

    #[test]
    fn test_parse_calendar_query_with_time_range() {
        let xml = br#"<?xml version="1.0" encoding="utf-8"?>
        <C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
            <D:prop>
                <D:getetag/>
                <C:calendar-data/>
            </D:prop>
            <C:filter>
                <C:comp-filter name="VCALENDAR">
                    <C:comp-filter name="VEVENT">
                        <C:time-range start="20260301T000000Z" end="20260401T000000Z"/>
                    </C:comp-filter>
                </C:comp-filter>
            </C:filter>
        </C:calendar-query>"#;
        let result = parse_report(xml).unwrap();
        match result {
            ReportRequest::CalendarQuery { props, time_range } => {
                assert_eq!(props.len(), 2);
                let (start, end) = time_range.unwrap();
                assert_eq!(start, "20260301T000000Z");
                assert_eq!(end, "20260401T000000Z");
            }
            _ => panic!("Expected CalendarQuery"),
        }
    }

    #[test]
    fn test_parse_sync_collection() {
        let xml = br#"<?xml version="1.0" encoding="utf-8"?>
        <D:sync-collection xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
            <D:sync-token>sync-abc123</D:sync-token>
            <D:prop>
                <D:getetag/>
            </D:prop>
        </D:sync-collection>"#;
        let result = parse_report(xml).unwrap();
        match result {
            ReportRequest::SyncCollection { props, sync_token } => {
                assert_eq!(props.len(), 1);
                assert_eq!(sync_token, "sync-abc123");
            }
            _ => panic!("Expected SyncCollection"),
        }
    }
}
