use thiserror::Error;

pub type Result<T> = std::result::Result<T, AcpError>;

#[derive(Debug, Error)]
pub enum AcpError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    /// JSON-RPC peer returned an error response for a request we sent.
    #[error("rpc error {code}: {message}")]
    RpcError {
        code: i64,
        message: String,
        data: Option<serde_json::Value>,
    },

    /// Connection closed before the response we were waiting for arrived.
    #[error("connection closed before response (request id={request_id:?})")]
    ConnectionClosed { request_id: Option<i64> },

    /// Agent process exited before we finished talking to it.
    #[error("agent process exited (status={status:?})")]
    AgentExited { status: Option<i32> },

    /// We asked the agent to do something it doesn't have a handler for.
    #[error("unsupported method: {0}")]
    UnsupportedMethod(String),

    /// The agent responded but the response shape did not match the schema.
    #[error("malformed response for {method}: {detail}")]
    MalformedResponse { method: String, detail: String },

    #[error("{0}")]
    Other(String),
}

impl AcpError {
    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }
}
