pub mod multistatus;
pub mod parse;
pub mod properties;

/// DAV: namespace
pub const DAV_NS: &str = "DAV:";
/// CalDAV namespace
pub const CALDAV_NS: &str = "urn:ietf:params:xml:ns:caldav";
/// Apple iCal namespace (for calendar-color, calendar-order, etc.)
pub const APPLE_NS: &str = "http://apple.com/ns/ical/";
/// CalendarServer namespace (for getctag)
pub const CS_NS: &str = "http://calendarserver.org/ns/";
