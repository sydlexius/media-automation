use super::super::error::MediaServerError;

#[test]
fn error_display_http() {
    let err = MediaServerError::Http {
        status: 404,
        body: "Not Found".to_string(),
    };
    assert_eq!(err.to_string(), "HTTP 404: Not Found");
}

#[test]
fn error_display_connection() {
    let err = MediaServerError::Connection("timeout".to_string());
    assert_eq!(err.to_string(), "connection error: timeout");
}

#[test]
fn error_display_parse() {
    let err = MediaServerError::Parse("invalid json".to_string());
    assert_eq!(err.to_string(), "parse error: invalid json");
}

#[test]
fn error_display_protocol() {
    let err = MediaServerError::Protocol("no users".to_string());
    assert_eq!(err.to_string(), "protocol error: no users");
}
