use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiagnosticEvent {
    pub correlation_id: String,
    pub event_type: String,
    pub payload: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiagnosticsBundle {
    pub app_version: String,
    pub runtime_mode: String,
    pub session_snapshot_json: String,
    pub events: Vec<DiagnosticEvent>,
}

impl DiagnosticsBundle {
    pub fn to_redacted_json(&self) -> Result<String, serde_json::Error> {
        let mut cloned = self.clone();
        cloned.session_snapshot_json = redact(&cloned.session_snapshot_json);
        for event in &mut cloned.events {
            event.payload = redact(&event.payload);
        }
        serde_json::to_string_pretty(&cloned)
    }
}

pub fn redact(input: &str) -> String {
    let mut out = input.to_string();

    for marker in ["token", "api_key", "secret"] {
        out = redact_marker(&out, marker);
    }

    out
}

fn redact_marker(input: &str, marker: &str) -> String {
    let mut out = String::new();
    let mut remaining = input;

    while let Some(pos) = remaining.find(marker) {
        let (before, after_marker) = remaining.split_at(pos);
        out.push_str(before);

        if let Some(eq_pos) = after_marker.find('=') {
            let (left, right) = after_marker.split_at(eq_pos + 1);
            out.push_str(left);

            let redacted_tail = right
                .chars()
                .skip_while(|c| *c == ' ')
                .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
                .collect::<String>();

            if redacted_tail.is_empty() {
                out.push_str(right);
                remaining = "";
            } else {
                out.push_str("[REDACTED]");
                let skip_len = right.find(&redacted_tail).unwrap_or(0) + redacted_tail.len();
                remaining = &right[skip_len..];
            }
        } else {
            out.push_str(after_marker);
            remaining = "";
        }
    }

    out.push_str(remaining);
    out
}

#[cfg(test)]
mod tests {
    use super::{DiagnosticEvent, DiagnosticsBundle, redact};

    #[test]
    fn redaction_removes_sensitive_values() {
        let input = "token=abc123 api_key = key123 secret=mysecret";
        let output = redact(input);
        assert!(!output.contains("abc123"));
        assert!(!output.contains("key123"));
        assert!(!output.contains("mysecret"));
        assert!(output.contains("[REDACTED]"));
    }

    #[test]
    fn bundle_json_contains_expected_fields() {
        let bundle = DiagnosticsBundle {
            app_version: "0.1.0".to_string(),
            runtime_mode: "fat_client".to_string(),
            session_snapshot_json: "{\"token\":\"abc123\"}".to_string(),
            events: vec![DiagnosticEvent {
                correlation_id: "corr-1".to_string(),
                event_type: "output.delta".to_string(),
                payload: "api_key=supersecret".to_string(),
            }],
        };

        let json = bundle
            .to_redacted_json()
            .expect("bundle should serialize to JSON");
        assert!(json.contains("app_version"));
        assert!(json.contains("correlation_id"));
        assert!(json.contains("[REDACTED]"));
    }
}
