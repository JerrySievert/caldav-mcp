use super::multistatus::{PropContent, PropValue};
use super::{APPLE_NS, CALDAV_NS, CS_NS, DAV_NS};
use crate::db::models::{Calendar, CalendarObject};

/// Build the standard set of properties for the CalDAV root resource.
pub fn root_props(username: &str) -> Vec<PropValue> {
    vec![
        PropValue {
            name: "resourcetype".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml("<D:collection/>".to_string()),
        },
        PropValue {
            name: "current-user-principal".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml(format!(
                "<D:href>/caldav/users/{username}/</D:href>"
            )),
        },
        PropValue {
            name: "displayname".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Text("CalDAV Server".to_string()),
        },
    ]
}

/// Build properties for the CalDAV root when the user is NOT authenticated.
/// Uses <D:unauthenticated/> for current-user-principal per RFC 5397.
pub fn root_props_unauthenticated() -> Vec<PropValue> {
    vec![
        PropValue {
            name: "resourcetype".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml("<D:collection/>".to_string()),
        },
        PropValue {
            name: "current-user-principal".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml("<D:unauthenticated/>".to_string()),
        },
        PropValue {
            name: "displayname".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Text("CalDAV Server".to_string()),
        },
    ]
}


/// Build the standard set of properties for a calendar-home-set resource.
pub fn calendar_home_props(username: &str) -> Vec<PropValue> {
    vec![
        PropValue {
            name: "resourcetype".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml("<D:collection/>".to_string()),
        },
        PropValue {
            name: "displayname".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Text(format!("{username}'s calendars")),
        },
        PropValue {
            name: "current-user-principal".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml(format!(
                "<D:href>/caldav/users/{username}/</D:href>"
            )),
        },
    ]
}

/// Build minimal discovery properties for the email home URL when the user
/// is NOT authenticated. Returns only structural/capability props needed
/// for `accountsd` to complete account setup without leaking user data.
pub fn email_home_props_unauthenticated(request_path: &str) -> Vec<PropValue> {
    vec![
        PropValue {
            name: "resourcetype".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml("<D:collection/><D:principal/>".to_string()),
        },
        PropValue {
            name: "displayname".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Text("CalDAV Account".to_string()),
        },
        PropValue {
            name: "current-user-principal".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml(format!("<D:href>{request_path}</D:href>")),
        },
        PropValue {
            name: "principal-URL".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml(format!("<D:href>{request_path}</D:href>")),
        },
        PropValue {
            name: "calendar-home-set".to_string(),
            namespace: CALDAV_NS.to_string(),
            value: PropContent::Xml(format!("<D:href>{request_path}</D:href>")),
        },
        PropValue {
            name: "supported-report-set".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml(
                "<D:supported-report><D:report><C:calendar-multiget/></D:report></D:supported-report>\
                 <D:supported-report><D:report><C:calendar-query/></D:report></D:supported-report>\
                 <D:supported-report><D:report><D:sync-collection/></D:report></D:supported-report>"
                    .to_string(),
            ),
        },
    ]
}

/// Build properties for the Apple-proprietary email home URL
/// (/calendar/dav/{email}/user/) when the user IS authenticated.
///
/// `dataaccessd` treats this URL as both the principal and calendar home.
/// We set `current-user-principal`, `principal-URL`, and `calendar-home-set`
/// all to point back to `request_path` (the URL the client is already at),
/// so it never needs to follow a redirect to find the calendar list.
pub fn email_home_props(username: &str, request_path: &str) -> Vec<PropValue> {
    vec![
        PropValue {
            name: "resourcetype".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml("<D:collection/><D:principal/>".to_string()),
        },
        PropValue {
            name: "displayname".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Text(username.to_string()),
        },
        PropValue {
            name: "current-user-principal".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml(format!("<D:href>{request_path}</D:href>")),
        },
        PropValue {
            name: "principal-URL".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml(format!("<D:href>{request_path}</D:href>")),
        },
        PropValue {
            name: "calendar-home-set".to_string(),
            namespace: CALDAV_NS.to_string(),
            value: PropContent::Xml(format!("<D:href>{request_path}</D:href>")),
        },
        PropValue {
            name: "supported-report-set".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml(
                "<D:supported-report><D:report><C:calendar-multiget/></D:report></D:supported-report>\
                 <D:supported-report><D:report><C:calendar-query/></D:report></D:supported-report>\
                 <D:supported-report><D:report><D:sync-collection/></D:report></D:supported-report>"
                    .to_string(),
            ),
        },
        PropValue {
            name: "current-user-privilege-set".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml(
                "<D:privilege><D:read/></D:privilege>\
                 <D:privilege><D:write/></D:privilege>"
                    .to_string(),
            ),
        },
    ]
}

/// Build the properties for a calendar collection.
pub fn calendar_props(username: &str, calendar: &Calendar) -> Vec<PropValue> {
    vec![
        PropValue {
            name: "resourcetype".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml("<D:collection/><C:calendar/>".to_string()),
        },
        PropValue {
            name: "displayname".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Text(calendar.name.clone()),
        },
        PropValue {
            name: "calendar-description".to_string(),
            namespace: CALDAV_NS.to_string(),
            value: PropContent::Text(calendar.description.clone()),
        },
        PropValue {
            name: "calendar-color".to_string(),
            namespace: APPLE_NS.to_string(),
            value: PropContent::Text(calendar.color.clone()),
        },
        PropValue {
            name: "calendar-timezone".to_string(),
            namespace: CALDAV_NS.to_string(),
            value: PropContent::Text(calendar.timezone.clone()),
        },
        PropValue {
            name: "supported-calendar-component-set".to_string(),
            namespace: CALDAV_NS.to_string(),
            value: PropContent::Xml(
                "<C:comp name=\"VEVENT\"/><C:comp name=\"VTODO\"/>".to_string(),
            ),
        },
        PropValue {
            name: "getctag".to_string(),
            namespace: CS_NS.to_string(),
            value: PropContent::Text(calendar.ctag.clone()),
        },
        PropValue {
            name: "sync-token".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Text(calendar.sync_token.clone()),
        },
        PropValue {
            name: "current-user-principal".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml(format!(
                "<D:href>/caldav/users/{username}/</D:href>"
            )),
        },
        PropValue {
            name: "current-user-privilege-set".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml(
                "<D:privilege><D:read/></D:privilege>\
                 <D:privilege><D:write/></D:privilege>\
                 <D:privilege><D:write-content/></D:privilege>"
                    .to_string(),
            ),
        },
        PropValue {
            name: "owner".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml(format!(
                "<D:href>/caldav/users/{username}/</D:href>"
            )),
        },
        PropValue {
            name: "supported-report-set".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml(
                "<D:supported-report><D:report><C:calendar-multiget/></D:report></D:supported-report>\
                 <D:supported-report><D:report><C:calendar-query/></D:report></D:supported-report>\
                 <D:supported-report><D:report><D:sync-collection/></D:report></D:supported-report>"
                    .to_string(),
            ),
        },
    ]
}

/// Build properties for a calendar object (event/todo).
pub fn calendar_object_props(
    _username: &str,
    _calendar_id: &str,
    object: &CalendarObject,
    include_data: bool,
) -> Vec<PropValue> {
    let mut props = vec![
        PropValue {
            name: "getetag".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Text(object.etag.clone()),
        },
        PropValue {
            name: "getcontenttype".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Text("text/calendar; charset=utf-8".to_string()),
        },
    ];

    if include_data {
        props.push(PropValue {
            name: "calendar-data".to_string(),
            namespace: CALDAV_NS.to_string(),
            value: PropContent::Text(object.ical_data.clone()),
        });
    }

    props
}

/// Get the href for a calendar object.
pub fn calendar_object_href(username: &str, calendar_id: &str, uid: &str) -> String {
    format!("/caldav/users/{username}/{calendar_id}/{uid}.ics")
}

/// Get the href for a calendar collection.
pub fn calendar_href(username: &str, calendar_id: &str) -> String {
    format!("/caldav/users/{username}/{calendar_id}/")
}
