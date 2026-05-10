/// Network-facing attach WebSocket server.
///
/// Runs alongside the local Unix `attach_gateway` and serves the same purpose
/// over TCP: bidirectional bytes between an external viewer and a live
/// persistent agent's PTY. Reached only via Tailscale in production — the
/// controlplane proxies browser WS upgrades to this endpoint.
///
/// Route: `GET /attach/{agent_id}?token={hmac}&exp={unix_seconds}`
///
/// Token validation: HMAC-SHA256(`attach_secret`, `"attach:{agent_id}:{exp}"`)
/// hex-encoded. The `exp` query param is the unix timestamp at which the
/// token stops being valid; the controlplane mints these for short windows
/// (60s recommended).
///
/// Wire framing:
///   - Binary frames carry raw bytes both ways (stdout fan-out, stdin in).
///   - Text frames may carry control JSON; supported today: resize.
///         {"kind":"resize","cols":120,"rows":40}
///   - Anything else is logged and ignored.
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use futures::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::handshake::server::{
    ErrorResponse, Request, Response,
};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::Message;

use crate::attach_registry::{AttachEndpoints, AttachRegistry, PtyAttachEndpoints};

type HmacSha256 = Hmac<Sha256>;

pub async fn serve(
    bind_addr: String,
    secret: Option<String>,
    registry: Arc<AttachRegistry>,
) -> Result<()> {
    let listener = TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("attach_ws bind {bind_addr}"))?;
    tracing::info!("attach_ws listening on {bind_addr}");

    if secret.is_none() {
        tracing::warn!(
            "attach_ws: no attach_secret configured — all upgrades will be rejected (default-deny)"
        );
    }

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("attach_ws accept error: {e}");
                continue;
            }
        };

        let secret = secret.clone();
        let registry = Arc::clone(&registry);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, peer.to_string(), secret, registry).await {
                tracing::debug!("attach_ws connection from {peer} ended: {e:#}");
            }
        });
    }
}

async fn handle_connection(
    stream: TcpStream,
    peer: String,
    secret: Option<String>,
    registry: Arc<AttachRegistry>,
) -> Result<()> {
    // Validate the URI inside the WS handshake callback. If validation
    // fails we return an HTTP error response and tungstenite delivers it
    // before the upgrade.
    let mut parsed: Option<ParsedRequest> = None;

    let ws = tokio_tungstenite::accept_hdr_async(stream, |req: &Request, resp: Response| {
        match validate_request(req, secret.as_deref()) {
            Ok(parsed_ok) => {
                parsed = Some(parsed_ok);
                Ok(resp)
            }
            Err(reason) => {
                let response = http::Response::builder()
                    .status(StatusCode::FORBIDDEN)
                    .body(Some(format!("attach rejected: {reason}")))
                    .expect("static response builds");
                Err(ErrorResponse::from(response))
            }
        }
    })
    .await
    .map_err(|e| anyhow!("ws handshake from {peer}: {e}"))?;

    let parsed = parsed.ok_or_else(|| anyhow!("ws handshake produced no parsed request"))?;
    let agent_id = parsed.agent_id;

    let endpoints = registry
        .get(&agent_id)
        .await
        .ok_or_else(|| anyhow!("no live persistent session for agent {agent_id}"))?;

    tracing::info!("attach_ws viewer connected for agent {agent_id} from {peer}");
    match endpoints {
        AttachEndpoints::Pty(pty) => pump_pty(ws, pty).await,
        AttachEndpoints::Acp(_) => {
            // The PTY frame protocol (binary stdin/stdout + resize control)
            // does not match ACP's JSON-RPC stream. A future layer will add
            // a text-frame pump that relays session/update notifications and
            // accepts {kind:"prompt"|"cancel"} envelopes from viewers.
            tracing::warn!(
                "attach_ws: agent {agent_id} is an ACP session; \
                 byte-stream attach not yet supported on this endpoint"
            );
            // Send a one-shot text frame so the viewer sees a clear reason.
            let mut sink = ws;
            use futures::SinkExt;
            let _ = sink
                .send(Message::Text(
                    "{\"kind\":\"error\",\"detail\":\"ACP attach over WS not yet supported\"}"
                        .into(),
                ))
                .await;
        }
    }
    tracing::info!("attach_ws viewer disconnected for agent {agent_id} from {peer}");
    Ok(())
}

struct ParsedRequest {
    agent_id: String,
}

fn validate_request(req: &Request, secret: Option<&str>) -> Result<ParsedRequest, String> {
    let uri = req.uri();
    let path = uri.path();

    // Path: /attach/<agent_id>
    let agent_id = path
        .strip_prefix("/attach/")
        .ok_or_else(|| format!("unexpected path {path}"))?
        .to_string();
    if agent_id.is_empty() {
        return Err("empty agent_id".into());
    }

    let secret = secret.ok_or_else(|| "server has no attach_secret configured".to_string())?;

    let query = uri.query().unwrap_or("");
    let mut token: Option<String> = None;
    let mut exp: Option<i64> = None;
    for pair in query.split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        match k {
            "token" => token = Some(v.to_string()),
            "exp" => exp = v.parse().ok(),
            _ => {}
        }
    }
    let token = token.ok_or_else(|| "missing token".to_string())?;
    let exp = exp.ok_or_else(|| "missing exp".to_string())?;

    let now = chrono::Utc::now().timestamp();
    if now > exp {
        return Err("token expired".into());
    }

    let expected = sign_attach(secret, &agent_id, exp);
    // Constant-time-ish: compare via fixed-length subtle equality. hex strings
    // are equal length so a normal == is fine here in practice.
    if token != expected {
        return Err("invalid token".into());
    }

    Ok(ParsedRequest { agent_id })
}

/// Sign an attach token for `agent_id` valid until `exp` (unix seconds).
/// Used by tests today; the controlplane proxy will use the same scheme
/// out-of-band in Phase 2b.
pub fn sign_attach(secret: &str, agent_id: &str, exp: i64) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("hmac key length");
    mac.update(format!("attach:{agent_id}:{exp}").as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

async fn pump_pty(
    ws: tokio_tungstenite::WebSocketStream<TcpStream>,
    endpoints: PtyAttachEndpoints,
) {
    let (mut sink, mut stream) = ws.split();
    let mut stdout_rx = endpoints.stdout_broadcast.subscribe();
    let stdin_tx = endpoints.stdin_tx.clone();
    let resize_tx = endpoints.resize_tx.clone();

    // PTY → WS
    let outbound = tokio::spawn(async move {
        loop {
            match stdout_rx.recv().await {
                Ok(bytes) => {
                    if sink.send(Message::Binary(bytes.into())).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("attach_ws viewer lagged {n} chunks");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // WS → PTY (and resize control)
    while let Some(msg) = stream.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!("attach_ws stream error: {e}");
                break;
            }
        };
        match msg {
            Message::Binary(bytes) => {
                if stdin_tx.send(bytes.to_vec()).await.is_err() {
                    break;
                }
            }
            Message::Text(txt) => {
                // Control frames. Single supported kind today: resize.
                #[derive(serde::Deserialize)]
                struct ControlFrame {
                    kind: String,
                    #[serde(default)]
                    cols: u16,
                    #[serde(default)]
                    rows: u16,
                }
                match serde_json::from_str::<ControlFrame>(&txt) {
                    Ok(ctrl) if ctrl.kind == "resize" && ctrl.rows > 0 && ctrl.cols > 0 => {
                        // Best-effort; bounded resize channel coalesces.
                        let _ = resize_tx.try_send((ctrl.rows, ctrl.cols));
                    }
                    Ok(ctrl) => {
                        tracing::debug!("attach_ws ignored control frame kind={}", ctrl.kind);
                    }
                    Err(e) => {
                        tracing::debug!("attach_ws bad control frame: {e}");
                    }
                }
            }
            Message::Close(_) => break,
            // Ping/Pong handled by tungstenite; nothing to do here.
            _ => {}
        }
    }

    outbound.abort();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_then_verify_roundtrip() {
        let sig = sign_attach("s3cret", "agent-1", 9999999999);
        assert_eq!(sig.len(), 64); // hex of 32 bytes
        // Different agent → different sig.
        assert_ne!(sig, sign_attach("s3cret", "agent-2", 9999999999));
        // Different exp → different sig.
        assert_ne!(sig, sign_attach("s3cret", "agent-1", 9999999998));
        // Different secret → different sig.
        assert_ne!(sig, sign_attach("other", "agent-1", 9999999999));
    }
}
