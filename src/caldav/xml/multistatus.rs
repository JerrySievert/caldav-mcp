use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use std::io::{Cursor, Write};

/// Builder for WebDAV multistatus XML responses.
pub struct MultistatusBuilder {
    writer: Writer<Cursor<Vec<u8>>>,
}

/// A single property value to include in a response.
pub struct PropValue {
    pub name: String,
    pub namespace: String,
    pub value: PropContent,
}

/// Content of a property — text, XML fragment, or empty.
pub enum PropContent {
    Text(String),
    Xml(String),
    Empty,
}

impl MultistatusBuilder {
    /// Start building a multistatus response.
    pub fn new() -> Self {
        let mut writer = Writer::new(Cursor::new(Vec::new()));

        // XML declaration
        writer
            .write_event(Event::Decl(BytesDecl::new("1.0", Some("utf-8"), None)))
            .unwrap();

        // <D:multistatus> with namespace declarations
        let mut elem = BytesStart::new("D:multistatus");
        elem.push_attribute(("xmlns:D", super::DAV_NS));
        elem.push_attribute(("xmlns:C", super::CALDAV_NS));
        elem.push_attribute(("xmlns:A", super::APPLE_NS));
        elem.push_attribute(("xmlns:CS", super::CS_NS));
        writer.write_event(Event::Start(elem)).unwrap();

        Self { writer }
    }

    /// Add a response entry for a given href with found and not-found properties.
    pub fn add_response(
        &mut self,
        href: &str,
        found_props: Vec<PropValue>,
        not_found_props: Vec<String>,
    ) {
        // <D:response>
        self.writer
            .write_event(Event::Start(BytesStart::new("D:response")))
            .unwrap();

        // <D:href>
        self.writer
            .write_event(Event::Start(BytesStart::new("D:href")))
            .unwrap();
        self.writer
            .write_event(Event::Text(BytesText::new(href)))
            .unwrap();
        self.writer
            .write_event(Event::End(BytesEnd::new("D:href")))
            .unwrap();

        // Found properties: <D:propstat> with 200
        if !found_props.is_empty() {
            self.writer
                .write_event(Event::Start(BytesStart::new("D:propstat")))
                .unwrap();

            // <D:prop>
            self.writer
                .write_event(Event::Start(BytesStart::new("D:prop")))
                .unwrap();

            for prop in &found_props {
                let prefixed = prefix_name(&prop.namespace, &prop.name);
                match &prop.value {
                    PropContent::Text(text) => {
                        self.writer
                            .write_event(Event::Start(BytesStart::new(&prefixed)))
                            .unwrap();
                        self.writer
                            .write_event(Event::Text(BytesText::new(text)))
                            .unwrap();
                        self.writer
                            .write_event(Event::End(BytesEnd::new(&prefixed)))
                            .unwrap();
                    }
                    PropContent::Xml(xml) => {
                        // Write the entire element as raw XML to avoid
                        // Writer buffer/cursor desync when injecting fragments.
                        let raw = format!("<{prefixed}>{xml}</{prefixed}>");
                        self.writer.get_mut().write_all(raw.as_bytes()).unwrap();
                    }
                    PropContent::Empty => {
                        self.writer
                            .write_event(Event::Empty(BytesStart::new(&prefixed)))
                            .unwrap();
                    }
                }
            }

            // </D:prop>
            self.writer
                .write_event(Event::End(BytesEnd::new("D:prop")))
                .unwrap();

            // <D:status>HTTP/1.1 200 OK</D:status>
            self.writer
                .write_event(Event::Start(BytesStart::new("D:status")))
                .unwrap();
            self.writer
                .write_event(Event::Text(BytesText::new("HTTP/1.1 200 OK")))
                .unwrap();
            self.writer
                .write_event(Event::End(BytesEnd::new("D:status")))
                .unwrap();

            // </D:propstat>
            self.writer
                .write_event(Event::End(BytesEnd::new("D:propstat")))
                .unwrap();
        }

        // Not-found properties: <D:propstat> with 404
        if !not_found_props.is_empty() {
            self.writer
                .write_event(Event::Start(BytesStart::new("D:propstat")))
                .unwrap();

            self.writer
                .write_event(Event::Start(BytesStart::new("D:prop")))
                .unwrap();

            for name in &not_found_props {
                self.writer
                    .write_event(Event::Empty(BytesStart::new(name.as_str())))
                    .unwrap();
            }

            self.writer
                .write_event(Event::End(BytesEnd::new("D:prop")))
                .unwrap();

            self.writer
                .write_event(Event::Start(BytesStart::new("D:status")))
                .unwrap();
            self.writer
                .write_event(Event::Text(BytesText::new("HTTP/1.1 404 Not Found")))
                .unwrap();
            self.writer
                .write_event(Event::End(BytesEnd::new("D:status")))
                .unwrap();

            self.writer
                .write_event(Event::End(BytesEnd::new("D:propstat")))
                .unwrap();
        }

        // </D:response>
        self.writer
            .write_event(Event::End(BytesEnd::new("D:response")))
            .unwrap();
    }

    /// Add a sync-token element (used in sync-collection response).
    pub fn add_sync_token(&mut self, token: &str) {
        self.writer
            .write_event(Event::Start(BytesStart::new("D:sync-token")))
            .unwrap();
        self.writer
            .write_event(Event::Text(BytesText::new(token)))
            .unwrap();
        self.writer
            .write_event(Event::End(BytesEnd::new("D:sync-token")))
            .unwrap();
    }

    /// Finish building and return the XML bytes.
    pub fn build(mut self) -> Vec<u8> {
        // </D:multistatus>
        self.writer
            .write_event(Event::End(BytesEnd::new("D:multistatus")))
            .unwrap();

        self.writer.into_inner().into_inner()
    }
}

/// Map a namespace URI + local name to a prefixed element name.
fn prefix_name(namespace: &str, local_name: &str) -> String {
    match namespace {
        ns if ns == super::DAV_NS => format!("D:{local_name}"),
        ns if ns == super::CALDAV_NS => format!("C:{local_name}"),
        ns if ns == super::APPLE_NS => format!("A:{local_name}"),
        ns if ns == super::CS_NS => format!("CS:{local_name}"),
        _ => format!("D:{local_name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_multistatus() {
        let builder = MultistatusBuilder::new();
        let xml = String::from_utf8(builder.build()).unwrap();
        assert!(xml.contains("D:multistatus"));
        assert!(xml.contains("xmlns:D=\"DAV:\""));
        assert!(xml.contains("xmlns:C=\"urn:ietf:params:xml:ns:caldav\""));
    }

    #[test]
    fn test_response_with_text_prop() {
        let mut builder = MultistatusBuilder::new();
        builder.add_response(
            "/caldav/",
            vec![PropValue {
                name: "displayname".to_string(),
                namespace: super::super::DAV_NS.to_string(),
                value: PropContent::Text("My Calendar".to_string()),
            }],
            vec![],
        );
        let xml = String::from_utf8(builder.build()).unwrap();
        assert!(xml.contains("<D:displayname>My Calendar</D:displayname>"));
        assert!(xml.contains("<D:href>/caldav/</D:href>"));
        assert!(xml.contains("HTTP/1.1 200 OK"));
    }

    #[test]
    fn test_response_with_not_found_props() {
        let mut builder = MultistatusBuilder::new();
        builder.add_response("/caldav/", vec![], vec!["D:missing-prop".to_string()]);
        let xml = String::from_utf8(builder.build()).unwrap();
        assert!(xml.contains("D:missing-prop"));
        assert!(xml.contains("HTTP/1.1 404 Not Found"));
    }

    #[test]
    fn test_prefix_name_mapping() {
        assert_eq!(
            prefix_name(super::super::DAV_NS, "displayname"),
            "D:displayname"
        );
        assert_eq!(
            prefix_name(super::super::CALDAV_NS, "calendar-data"),
            "C:calendar-data"
        );
        assert_eq!(
            prefix_name(super::super::APPLE_NS, "calendar-color"),
            "A:calendar-color"
        );
        assert_eq!(prefix_name(super::super::CS_NS, "getctag"), "CS:getctag");
    }
}
