//! Minimal JSON-RPC 2.0 framing over an async bytestream.
//!
//! Wire format: newline-delimited JSON, one message per line. Matches the
//! `ndJsonStream` framing in `@zed-industries/agent-client-protocol`'s TS
//! reference implementation.
//!
//! This module deliberately stops at framing + classification. Multiplexing
//! request/response correlation, notification dispatch, and lifecycle live
//! in [`crate::agent`].

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC 2.0 message — covers requests, responses, and notifications in
/// one struct (classified by which fields are present).
///
/// Serializes with `jsonrpc: "2.0"` and omits absent fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawMessage {
    #[serde(default = "default_jsonrpc")]
    pub jsonrpc: String,

    /// Present on requests and responses. Notifications omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,

    /// Present on requests and notifications.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,

    /// Present on success responses (mutually exclusive with `error`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,

    /// Present on error responses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

fn default_jsonrpc() -> String {
    "2.0".to_string()
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Classified view of a [`RawMessage`].
#[derive(Debug)]
pub enum Message {
    Request {
        id: i64,
        method: String,
        params: Option<Value>,
    },
    Response {
        id: i64,
        result: std::result::Result<Value, RpcError>,
    },
    Notification {
        method: String,
        params: Option<Value>,
    },
}

impl RawMessage {
    pub fn new_request(id: i64, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: default_jsonrpc(),
            id: Some(id),
            method: Some(method.into()),
            params,
            result: None,
            error: None,
        }
    }

    pub fn new_notification(method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: default_jsonrpc(),
            id: None,
            method: Some(method.into()),
            params,
            result: None,
            error: None,
        }
    }

    pub fn new_success(id: i64, result: Value) -> Self {
        Self {
            jsonrpc: default_jsonrpc(),
            id: Some(id),
            method: None,
            params: None,
            result: Some(result),
            error: None,
        }
    }

    pub fn new_error(id: i64, error: RpcError) -> Self {
        Self {
            jsonrpc: default_jsonrpc(),
            id: Some(id),
            method: None,
            params: None,
            result: None,
            error: Some(error),
        }
    }

    /// Classify into [`Message`] based on which fields are present.
    pub fn classify(self) -> std::result::Result<Message, ClassifyError> {
        match (self.id, self.method, self.result, self.error) {
            (Some(id), Some(method), None, None) => Ok(Message::Request {
                id,
                method,
                params: self.params,
            }),
            (Some(id), None, Some(result), None) => Ok(Message::Response {
                id,
                result: Ok(result),
            }),
            (Some(id), None, None, Some(error)) => Ok(Message::Response {
                id,
                result: Err(error),
            }),
            (None, Some(method), None, None) => Ok(Message::Notification {
                method,
                params: self.params,
            }),
            // Some peers send responses with both result=null and no error;
            // treat null result as a success with serde_json::Value::Null.
            (Some(id), None, None, None) => Ok(Message::Response {
                id,
                result: Ok(Value::Null),
            }),
            _ => Err(ClassifyError::AmbiguousFields),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClassifyError {
    #[error("ambiguous JSON-RPC message: id/method/result/error combination is invalid")]
    AmbiguousFields,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_request() {
        let raw = RawMessage::new_request(1, "initialize", Some(serde_json::json!({"v": 1})));
        match raw.classify().unwrap() {
            Message::Request { id, method, .. } => {
                assert_eq!(id, 1);
                assert_eq!(method, "initialize");
            }
            other => panic!("expected request, got {other:?}"),
        }
    }

    #[test]
    fn classify_notification() {
        let raw = RawMessage::new_notification("session/update", Some(serde_json::json!({})));
        assert!(matches!(
            raw.classify().unwrap(),
            Message::Notification { .. }
        ));
    }

    #[test]
    fn classify_success_and_error() {
        let ok = RawMessage::new_success(7, serde_json::json!({"protocolVersion": 1}));
        assert!(matches!(
            ok.classify().unwrap(),
            Message::Response { id: 7, result: Ok(_) }
        ));
        let err = RawMessage::new_error(
            8,
            RpcError {
                code: -32603,
                message: "boom".into(),
                data: None,
            },
        );
        assert!(matches!(
            err.classify().unwrap(),
            Message::Response { id: 8, result: Err(_) }
        ));
    }

    #[test]
    fn round_trip_through_json() {
        let raw = RawMessage::new_request(1, "session/prompt", Some(serde_json::json!({})));
        let line = serde_json::to_string(&raw).unwrap();
        let back: RawMessage = serde_json::from_str(&line).unwrap();
        assert_eq!(back.id, Some(1));
        assert_eq!(back.method.as_deref(), Some("session/prompt"));
    }
}
