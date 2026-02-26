use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::collections::HashMap;

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

/// Tracks namespace prefix → URI mappings from xmlns declarations.
/// Supports nested scopes (push/pop), but for PROPFIND we accumulate all.
struct NsContext {
    /// prefix → namespace URI. Empty string key = default namespace.
    map: HashMap<String, String>,
}

impl NsContext {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    /// Extract xmlns declarations from an element's attributes and register them.
    fn register_from_event(&mut self, event: &quick_xml::events::BytesStart) {
        for attr in event.attributes().flatten() {
            let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
            let value = String::from_utf8_lossy(&attr.value).to_string();
            if key == "xmlns" {
                self.map.insert(String::new(), value);
            } else if let Some(prefix) = key.strip_prefix("xmlns:") {
                self.map.insert(prefix.to_string(), value);
            }
        }
    }

    /// Resolve the namespace of an element based on its prefix and current context.
    fn resolve(&self, event: &quick_xml::events::BytesStart) -> String {
        // Check for explicit xmlns on the element itself first
        for attr in event.attributes().flatten() {
            let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
            if key == "xmlns" {
                return String::from_utf8_lossy(&attr.value).to_string();
            }
        }

        // Look up prefix in our accumulated namespace map
        let name = String::from_utf8_lossy(event.name().as_ref()).to_string();
        if let Some((prefix, _)) = name.split_once(':') {
            if let Some(ns) = self.map.get(prefix) {
                return ns.clone();
            }
            // Fallback: well-known prefix conventions
            return match prefix {
                "D" | "d" => super::DAV_NS,
                "C" | "c" | "cal" => super::CALDAV_NS,
                "A" | "IC" => super::APPLE_NS,
                "CS" | "cs" => super::CS_NS,
                _ => super::DAV_NS,
            }
            .to_string();
        }

        // No prefix: check default namespace
        if let Some(ns) = self.map.get("") {
            return ns.clone();
        }

        // Fallback default
        super::DAV_NS.to_string()
    }
}

/// Parse a PROPFIND request body. Returns AllProp if body is empty.
pub fn parse_propfind(body: &[u8]) -> PropfindRequest {
    if body.is_empty() {
        return PropfindRequest::AllProp;
    }

    let mut reader = Reader::from_reader(body);
    reader.config_mut().trim_text(true);

    let mut ns_ctx = NsContext::new();
    let mut in_prop = false;
    let mut props = Vec::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                // Accumulate namespace declarations from every element
                ns_ctx.register_from_event(e);

                let local = String::from_utf8_lossy(e.local_name().as_ref()).to_string();

                match local.as_str() {
                    "allprop" => return PropfindRequest::AllProp,
                    "propname" => return PropfindRequest::PropName,
                    "prop" => {
                        in_prop = true;
                    }
                    _ if in_prop => {
                        let ns = ns_ctx.resolve(e);
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

    let mut ns_ctx = NsContext::new();
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
                ns_ctx.register_from_event(e);
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
                        let ns = ns_ctx.resolve(e);
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

    /// Apple Calendar uses non-standard namespace prefixes (A=DAV, B=CalDAV, etc.).
    /// Our parser must resolve namespaces from xmlns declarations, not prefix guessing.
    #[test]
    fn test_parse_apple_style_namespace_prefixes() {
        let xml = br#"<?xml version="1.0" encoding="utf-8"?>
        <A:propfind xmlns:A="DAV:" xmlns:B="urn:ietf:params:xml:ns:caldav" xmlns:C="http://calendarserver.org/ns/" xmlns:D="http://apple.com/ns/ical/">
            <A:prop>
                <A:displayname/>
                <A:resourcetype/>
                <B:calendar-home-set/>
                <B:calendar-user-address-set/>
                <C:getctag/>
                <D:calendar-color/>
                <D:calendar-order/>
            </A:prop>
        </A:propfind>"#;
        let result = parse_propfind(xml);
        match result {
            PropfindRequest::Props(props) => {
                assert_eq!(props.len(), 7);
                // A: prefix should resolve to DAV: (not Apple NS)
                assert_eq!(props[0].local_name, "displayname");
                assert_eq!(props[0].namespace, "DAV:");
                assert_eq!(props[1].local_name, "resourcetype");
                assert_eq!(props[1].namespace, "DAV:");
                // B: prefix should resolve to CalDAV NS
                assert_eq!(props[2].local_name, "calendar-home-set");
                assert_eq!(props[2].namespace, "urn:ietf:params:xml:ns:caldav");
                assert_eq!(props[3].local_name, "calendar-user-address-set");
                assert_eq!(props[3].namespace, "urn:ietf:params:xml:ns:caldav");
                // C: prefix should resolve to CalendarServer NS
                assert_eq!(props[4].local_name, "getctag");
                assert_eq!(props[4].namespace, "http://calendarserver.org/ns/");
                // D: prefix should resolve to Apple NS
                assert_eq!(props[5].local_name, "calendar-color");
                assert_eq!(props[5].namespace, "http://apple.com/ns/ical/");
                assert_eq!(props[6].local_name, "calendar-order");
                assert_eq!(props[6].namespace, "http://apple.com/ns/ical/");
            }
            _ => panic!("Expected Props variant"),
        }
    }

    #[test]
    fn test_parse_propfind_with_mixed_namespaces() {
        // dataaccessd-style PROPFIND with many properties across namespaces
        let xml = br#"<?xml version="1.0" encoding="utf-8"?>
        <A:propfind xmlns:A="DAV:" xmlns:B="urn:ietf:params:xml:ns:caldav" xmlns:C="http://calendarserver.org/ns/">
            <A:prop>
                <A:current-user-principal/>
                <A:principal-URL/>
                <A:resource-id/>
                <B:schedule-inbox-URL/>
                <B:schedule-outbox-URL/>
                <C:email-address-set/>
                <C:notification-URL/>
            </A:prop>
        </A:propfind>"#;
        let result = parse_propfind(xml);
        match result {
            PropfindRequest::Props(props) => {
                assert_eq!(props.len(), 7);
                assert_eq!(props[0].namespace, "DAV:");
                assert_eq!(props[0].local_name, "current-user-principal");
                assert_eq!(props[3].namespace, "urn:ietf:params:xml:ns:caldav");
                assert_eq!(props[3].local_name, "schedule-inbox-URL");
                assert_eq!(props[5].namespace, "http://calendarserver.org/ns/");
                assert_eq!(props[5].local_name, "email-address-set");
            }
            _ => panic!("Expected Props variant"),
        }
    }
}
