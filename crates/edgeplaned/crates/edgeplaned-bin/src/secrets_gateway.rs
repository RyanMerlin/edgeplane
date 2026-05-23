/// Secrets broker gateway — Unix socket server.
///
/// Agents receive `EP_SECRETS_SOCKET` (path to this socket) and
/// `EP_SECRETS_SESSION` (a UUID session ID) as environment variables instead of
/// raw credential values. They request individual values at runtime:
///
/// Request  (newline-delimited JSON): `{"op":"get","session":"<id>","name":"<KEY>"}`
/// Response (newline-delimited JSON): `{"ok":true,"value":"..."}` or
///                                    `{"ok":false,"error":"..."}`
///
/// The socket is created at `~/.ep/secrets.sock` with mode 0600.
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use edgeplaned_secrets::SessionStore;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

pub struct SecretsGateway {
    sessions: Arc<SessionStore>,
    socket_path: PathBuf,
}

impl SecretsGateway {
    pub fn new(sessions: Arc<SessionStore>, socket_path: PathBuf) -> Self {
        Self { sessions, socket_path }
    }

    pub async fn run(self) -> Result<()> {
        let _ = std::fs::remove_file(&self.socket_path);
        let listener = UnixListener::bind(&self.socket_path)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                &self.socket_path,
                std::fs::Permissions::from_mode(0o600),
            )?;
        }

        tracing::info!(
            path = %self.socket_path.display(),
            "secrets gateway listening"
        );

        loop {
            let (stream, _) = listener.accept().await?;
            let sessions = Arc::clone(&self.sessions);
            tokio::spawn(async move {
                if let Err(e) = handle_connection(stream, sessions).await {
                    tracing::debug!("secrets gateway connection: {e}");
                }
            });
        }
    }
}

async fn handle_connection(
    stream: tokio::net::UnixStream,
    sessions: Arc<SessionStore>,
) -> Result<()> {
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut lines = BufReader::new(read_half).lines();

    while let Some(line) = lines.next_line().await? {
        let resp = match serde_json::from_str::<serde_json::Value>(&line) {
            Err(e) => {
                serde_json::json!({"ok": false, "error": format!("invalid JSON: {e}")})
            }
            Ok(req) => match req.get("op").and_then(|v| v.as_str()) {
                Some("get") => {
                    let session = req.get("session").and_then(|v| v.as_str()).unwrap_or("");
                    let name = req.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    match sessions.get(session, name) {
                        Some(value) => serde_json::json!({"ok": true, "value": value}),
                        None => serde_json::json!({
                            "ok": false,
                            "error": format!("secret '{name}' not found in session")
                        }),
                    }
                }
                Some(op) => serde_json::json!({"ok": false, "error": format!("unknown op: {op}")}),
                None => serde_json::json!({"ok": false, "error": "missing op field"}),
            },
        };

        write_half
            .write_all(format!("{resp}\n").as_bytes())
            .await?;
    }

    Ok(())
}
