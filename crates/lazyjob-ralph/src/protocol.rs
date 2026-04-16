use serde::{Deserialize, Serialize};

use crate::RalphError;
use crate::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerCommand {
    Start {
        loop_type: String,
        params: serde_json::Value,
    },
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerEvent {
    Status {
        phase: String,
        progress: f32,
        message: String,
    },
    Results {
        data: serde_json::Value,
    },
    Error {
        code: String,
        message: String,
    },
    Done {
        success: bool,
    },
}

pub struct NdjsonCodec;

impl NdjsonCodec {
    pub fn encode(cmd: &WorkerCommand) -> String {
        let mut s = serde_json::to_string(cmd).expect("WorkerCommand serialization never fails");
        s.push('\n');
        s
    }

    pub fn decode(line: &str) -> Result<WorkerEvent> {
        serde_json::from_str(line.trim()).map_err(|e| RalphError::Decode(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // learning test: verifies serde tagged enum emits a "type" field
    #[test]
    fn serde_tagged_enum_serializes_type_field() {
        let cmd = WorkerCommand::Cancel;
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"type\""));
        assert!(json.contains("\"cancel\""));
    }

    // learning test: verifies serde_json::Value round-trips through JSON
    #[test]
    fn serde_json_value_roundtrip() {
        let original = json!({"key": "value", "num": 42});
        let serialized = serde_json::to_string(&original).unwrap();
        let deserialized: serde_json::Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn worker_command_start_roundtrip() {
        let cmd = WorkerCommand::Start {
            loop_type: "job_discovery".to_string(),
            params: json!({"company": "Acme"}),
        };
        let serialized = serde_json::to_string(&cmd).unwrap();
        let deserialized: WorkerCommand = serde_json::from_str(&serialized).unwrap();
        assert_eq!(cmd, deserialized);
    }

    #[test]
    fn worker_command_cancel_roundtrip() {
        let cmd = WorkerCommand::Cancel;
        let serialized = serde_json::to_string(&cmd).unwrap();
        let deserialized: WorkerCommand = serde_json::from_str(&serialized).unwrap();
        assert_eq!(cmd, deserialized);
    }

    #[test]
    fn worker_command_start_type_tag() {
        let cmd = WorkerCommand::Start {
            loop_type: "resume_tailor".to_string(),
            params: json!(null),
        };
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&cmd).unwrap()).unwrap();
        assert_eq!(v["type"], "start");
        assert_eq!(v["loop_type"], "resume_tailor");
    }

    #[test]
    fn worker_event_status_roundtrip() {
        let event = WorkerEvent::Status {
            phase: "analyzing".to_string(),
            progress: 0.42,
            message: "Processing jobs".to_string(),
        };
        let serialized = serde_json::to_string(&event).unwrap();
        let deserialized: WorkerEvent = serde_json::from_str(&serialized).unwrap();
        assert_eq!(event, deserialized);
    }

    #[test]
    fn worker_event_results_roundtrip() {
        let event = WorkerEvent::Results {
            data: json!({"jobs_found": 12}),
        };
        let serialized = serde_json::to_string(&event).unwrap();
        let deserialized: WorkerEvent = serde_json::from_str(&serialized).unwrap();
        assert_eq!(event, deserialized);
    }

    #[test]
    fn worker_event_error_roundtrip() {
        let event = WorkerEvent::Error {
            code: "api_timeout".to_string(),
            message: "Request timed out after 30s".to_string(),
        };
        let serialized = serde_json::to_string(&event).unwrap();
        let deserialized: WorkerEvent = serde_json::from_str(&serialized).unwrap();
        assert_eq!(event, deserialized);
    }

    #[test]
    fn worker_event_done_roundtrip() {
        let event = WorkerEvent::Done { success: true };
        let serialized = serde_json::to_string(&event).unwrap();
        let deserialized: WorkerEvent = serde_json::from_str(&serialized).unwrap();
        assert_eq!(event, deserialized);
    }

    #[test]
    fn worker_event_type_tags() {
        let cases: &[(&str, WorkerEvent)] = &[
            (
                "status",
                WorkerEvent::Status {
                    phase: "p".into(),
                    progress: 0.0,
                    message: "m".into(),
                },
            ),
            ("results", WorkerEvent::Results { data: json!(null) }),
            (
                "error",
                WorkerEvent::Error {
                    code: "e".into(),
                    message: "m".into(),
                },
            ),
            ("done", WorkerEvent::Done { success: false }),
        ];
        for (expected_tag, event) in cases {
            let v: serde_json::Value =
                serde_json::from_str(&serde_json::to_string(event).unwrap()).unwrap();
            assert_eq!(v["type"], *expected_tag, "wrong tag for {:?}", event);
        }
    }

    #[test]
    fn ndjson_codec_encode_appends_newline() {
        let cmd = WorkerCommand::Cancel;
        let encoded = NdjsonCodec::encode(&cmd);
        assert!(encoded.ends_with('\n'));
    }

    #[test]
    fn ndjson_codec_encode_start() {
        let cmd = WorkerCommand::Start {
            loop_type: "job_discovery".to_string(),
            params: json!({}),
        };
        let encoded = NdjsonCodec::encode(&cmd);
        assert!(encoded.contains("\"type\":\"start\""));
        assert!(encoded.ends_with('\n'));
    }

    #[test]
    fn ndjson_codec_decode_valid_line() {
        let line = r#"{"type":"done","success":true}"#;
        let event = NdjsonCodec::decode(line).unwrap();
        assert_eq!(event, WorkerEvent::Done { success: true });
    }

    #[test]
    fn ndjson_codec_decode_trims_whitespace() {
        let line = "  {\"type\":\"done\",\"success\":false}  \n";
        let event = NdjsonCodec::decode(line).unwrap();
        assert_eq!(event, WorkerEvent::Done { success: false });
    }

    #[test]
    fn ndjson_codec_decode_invalid_json_returns_error() {
        let result = NdjsonCodec::decode("not json at all");
        assert!(result.is_err());
    }

    #[test]
    fn ndjson_codec_decode_unknown_type_returns_error() {
        let result = NdjsonCodec::decode(r#"{"type":"unknown_variant"}"#);
        assert!(result.is_err());
    }

    #[test]
    fn ndjson_codec_encode_then_decode() {
        let event_json = r#"{"type":"status","phase":"init","progress":0.1,"message":"Starting"}"#;
        let event = NdjsonCodec::decode(event_json).unwrap();
        let cmd = WorkerCommand::Start {
            loop_type: "cover_letter".to_string(),
            params: json!({"job_id": "abc"}),
        };
        let encoded = NdjsonCodec::encode(&cmd);
        let line = encoded.trim_end_matches('\n');
        let decoded: WorkerCommand = serde_json::from_str(line).unwrap();
        assert_eq!(cmd, decoded);
        assert_eq!(
            event,
            WorkerEvent::Status {
                phase: "init".to_string(),
                progress: 0.1,
                message: "Starting".to_string(),
            }
        );
    }
}
