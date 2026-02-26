use super::multistatus::{PropContent, PropValue};
use super::{APPLE_NS, CALDAV_NS, CS_NS, DAV_NS};
use crate::caldav::HrefContext;
use crate::caldav::xml::parse::PropfindRequest;
use crate::db::models::{Calendar, CalendarObject};

/// Ensure a sync token is a valid URI (RFC 6578 requirement).
/// Old tokens without a URI scheme get wrapped with `data:,` prefix.
pub fn ensure_sync_token_uri(token: &str) -> String {
    if token.contains(':') {
        token.to_string()
    } else {
        format!("data:,{token}")
    }
}

/// Map a namespace URI to its XML prefix (matching the prefixes declared in multistatus).
fn ns_prefix(namespace: &str) -> &'static str {
    match namespace {
        ns if ns == DAV_NS => "D",
        ns if ns == CALDAV_NS => "C",
        ns if ns == APPLE_NS => "A",
        ns if ns == CS_NS => "CS",
        _ => "D",
    }
}

/// Filter available properties based on what the client requested in a PROPFIND.
///
/// Per RFC 4918 §9.1: when a client requests specific properties, found ones go
/// in a 200 propstat and not-found ones in a 404 propstat. For `AllProp`, return
/// all available properties with no 404 list.
///
/// Returns `(found_props, not_found_prefixed_names)`.
pub fn filter_props(
    request: &PropfindRequest,
    available: Vec<PropValue>,
) -> (Vec<PropValue>, Vec<String>) {
    match request {
        PropfindRequest::AllProp => {
            // Return everything, no 404
            (available, vec![])
        }
        PropfindRequest::PropName => {
            // Return just the names (empty values) for all available props
            let names: Vec<PropValue> = available
                .into_iter()
                .map(|p| PropValue {
                    name: p.name,
                    namespace: p.namespace,
                    value: PropContent::Empty,
                })
                .collect();
            (names, vec![])
        }
        PropfindRequest::Props(requested) => {
            let mut found = Vec::new();
            let mut not_found = Vec::new();

            // Collect properties requested by the client that are available
            for avail in available {
                if requested
                    .iter()
                    .any(|r| r.local_name == avail.name && r.namespace == avail.namespace)
                {
                    found.push(avail);
                }
            }

            // Collect requested properties not available → 404 propstat
            for req in requested {
                let is_found = found
                    .iter()
                    .any(|p| p.name == req.local_name && p.namespace == req.namespace);
                if !is_found {
                    let prefix = ns_prefix(&req.namespace);
                    not_found.push(format!("{prefix}:{}", req.local_name));
                }
            }

            (found, not_found)
        }
    }
}

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
            value: PropContent::Xml(format!("<D:href>/caldav/users/{username}/</D:href>")),
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
            value: PropContent::Xml(format!("<D:href>/caldav/users/{username}/</D:href>")),
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
pub fn email_home_props(username: &str, email: &str, request_path: &str) -> Vec<PropValue> {
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
            name: "calendar-user-address-set".to_string(),
            namespace: CALDAV_NS.to_string(),
            value: PropContent::Xml(format!(
                "<D:href>mailto:{email}</D:href>"
            )),
        },
        PropValue {
            name: "email-address-set".to_string(),
            namespace: CS_NS.to_string(),
            value: PropContent::Xml(format!(
                "<CS:email-address>{email}</CS:email-address>"
            )),
        },
        PropValue {
            name: "schedule-inbox-URL".to_string(),
            namespace: CALDAV_NS.to_string(),
            value: PropContent::Xml(format!("<D:href>{request_path}inbox/</D:href>")),
        },
        PropValue {
            name: "schedule-outbox-URL".to_string(),
            namespace: CALDAV_NS.to_string(),
            value: PropContent::Xml(format!("<D:href>{request_path}outbox/</D:href>")),
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
        // Apple CalendarServer-specific properties
        PropValue {
            name: "notification-URL".to_string(),
            namespace: CS_NS.to_string(),
            value: PropContent::Xml(format!(
                "<D:href>{request_path}notifications/</D:href>"
            )),
        },
        PropValue {
            name: "dropbox-home-URL".to_string(),
            namespace: CS_NS.to_string(),
            value: PropContent::Xml(format!(
                "<D:href>{request_path}dropbox/</D:href>"
            )),
        },
        PropValue {
            name: "principal-collection-set".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml(format!(
                "<D:href>{request_path}</D:href>"
            )),
        },
        PropValue {
            name: "resource-id".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml(format!(
                "<D:href>urn:uuid:{username}</D:href>"
            )),
        },
        PropValue {
            name: "owner".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml(format!("<D:href>{request_path}</D:href>")),
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
            value: PropContent::Text(ensure_sync_token_uri(&calendar.ctag)),
        },
        PropValue {
            name: "sync-token".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Text(ensure_sync_token_uri(&calendar.sync_token)),
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

/// Get the href for a calendar collection using an HrefContext.
/// Uses email-based path when email is available, username-based otherwise.
pub fn calendar_href_for_context(ctx: &HrefContext, calendar_id: &str) -> String {
    match &ctx.email {
        Some(email) => format!("/calendar/dav/{email}/user/{calendar_id}/"),
        None => calendar_href(&ctx.username, calendar_id),
    }
}

/// Get the href for a calendar object using an HrefContext.
pub fn calendar_object_href_for_context(ctx: &HrefContext, calendar_id: &str, uid: &str) -> String {
    match &ctx.email {
        Some(email) => format!("/calendar/dav/{email}/user/{calendar_id}/{uid}.ics"),
        None => calendar_object_href(&ctx.username, calendar_id, uid),
    }
}

/// Build the properties for a calendar collection with context-aware hrefs.
pub fn calendar_props_for_context(ctx: &HrefContext, calendar: &Calendar) -> Vec<PropValue> {
    let principal_href = match &ctx.email {
        Some(email) => format!("/calendar/dav/{email}/user/"),
        None => format!("/caldav/users/{}/", ctx.username),
    };
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
            name: "calendar-order".to_string(),
            namespace: APPLE_NS.to_string(),
            value: PropContent::Text("1".to_string()),
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
            value: PropContent::Text(ensure_sync_token_uri(&calendar.ctag)),
        },
        PropValue {
            name: "sync-token".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Text(ensure_sync_token_uri(&calendar.sync_token)),
        },
        PropValue {
            name: "current-user-principal".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml(format!("<D:href>{principal_href}</D:href>")),
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
            value: PropContent::Xml(format!("<D:href>{principal_href}</D:href>")),
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
        // Apple Calendar-specific properties
        PropValue {
            name: "schedule-calendar-transp".to_string(),
            namespace: CALDAV_NS.to_string(),
            value: PropContent::Xml("<C:opaque/>".to_string()),
        },
        PropValue {
            name: "schedule-default-calendar-URL".to_string(),
            namespace: CALDAV_NS.to_string(),
            value: PropContent::Xml(format!(
                "<D:href>{}</D:href>",
                calendar_href_for_context(ctx, &calendar.id)
            )),
        },
        PropValue {
            name: "getcontenttype".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Text("text/calendar; charset=utf-8".to_string()),
        },
        PropValue {
            name: "resource-id".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Xml(format!(
                "<D:href>urn:uuid:{}</D:href>",
                calendar.id
            )),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caldav::xml::parse::{PropRequest, PropfindRequest};

    #[test]
    fn test_filter_props_allprop_returns_all() {
        let available = vec![
            PropValue {
                name: "displayname".to_string(),
                namespace: DAV_NS.to_string(),
                value: PropContent::Text("Test".to_string()),
            },
            PropValue {
                name: "resourcetype".to_string(),
                namespace: DAV_NS.to_string(),
                value: PropContent::Empty,
            },
        ];

        let (found, not_found) = filter_props(&PropfindRequest::AllProp, available);
        assert_eq!(found.len(), 2);
        assert!(not_found.is_empty());
    }

    #[test]
    fn test_filter_props_specific_found_and_not_found() {
        let available = vec![
            PropValue {
                name: "displayname".to_string(),
                namespace: DAV_NS.to_string(),
                value: PropContent::Text("Test".to_string()),
            },
            PropValue {
                name: "resourcetype".to_string(),
                namespace: DAV_NS.to_string(),
                value: PropContent::Empty,
            },
        ];

        let request = PropfindRequest::Props(vec![
            PropRequest {
                local_name: "displayname".to_string(),
                namespace: DAV_NS.to_string(),
            },
            PropRequest {
                local_name: "quota-available-bytes".to_string(),
                namespace: DAV_NS.to_string(),
            },
            PropRequest {
                local_name: "calendar-color".to_string(),
                namespace: APPLE_NS.to_string(),
            },
        ]);

        let (found, not_found) = filter_props(&request, available);
        // Only displayname should be found (resourcetype not requested)
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "displayname");
        // quota-available-bytes and calendar-color should be 404
        assert_eq!(not_found.len(), 2);
        assert!(not_found.contains(&"D:quota-available-bytes".to_string()));
        assert!(not_found.contains(&"A:calendar-color".to_string()));
    }

    #[test]
    fn test_filter_props_namespace_aware() {
        // Same local name, different namespace — only matching NS should be found
        let available = vec![PropValue {
            name: "calendar-color".to_string(),
            namespace: APPLE_NS.to_string(),
            value: PropContent::Text("#FF0000".to_string()),
        }];

        let request = PropfindRequest::Props(vec![PropRequest {
            local_name: "calendar-color".to_string(),
            namespace: CALDAV_NS.to_string(), // Wrong namespace
        }]);

        let (found, not_found) = filter_props(&request, available);
        assert_eq!(found.len(), 0);
        assert_eq!(not_found.len(), 1);
        assert_eq!(not_found[0], "C:calendar-color");
    }

    #[test]
    fn test_filter_props_propname_returns_empty_values() {
        let available = vec![PropValue {
            name: "displayname".to_string(),
            namespace: DAV_NS.to_string(),
            value: PropContent::Text("Test".to_string()),
        }];

        let (found, not_found) = filter_props(&PropfindRequest::PropName, available);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "displayname");
        assert!(matches!(found[0].value, PropContent::Empty));
        assert!(not_found.is_empty());
    }
}
