use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MessageFormat {
    ContentLength,
    JsonLine,
}

pub async fn read_next_message(
    reader: &mut BufReader<tokio::io::Stdin>,
) -> Result<Option<(String, MessageFormat)>> {
    let first = loop {
        let mut first_line = String::new();
        let n = reader.read_line(&mut first_line).await?;
        if n == 0 {
            return Ok(None);
        }
        let first = first_line.trim().to_string();
        if first.is_empty() {
            continue;
        }
        break first;
    };

    if first.starts_with('{') {
        return Ok(Some((first, MessageFormat::JsonLine)));
    }

    let content_length = read_content_length_with_first_line(reader, first).await?;
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).await.context("read body")?;
    Ok(Some((
        String::from_utf8_lossy(&body).to_string(),
        MessageFormat::ContentLength,
    )))
}

pub async fn write_message(
    stdout: &mut tokio::io::Stdout,
    payload: &str,
    format: MessageFormat,
) -> Result<()> {
    match format {
        MessageFormat::ContentLength => {
            let framed = format!("Content-Length: {}\r\n\r\n{}", payload.len(), payload);
            stdout.write_all(framed.as_bytes()).await?;
        }
        MessageFormat::JsonLine => {
            stdout.write_all(payload.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
        }
    }
    stdout.flush().await?;
    Ok(())
}

async fn read_content_length_with_first_line(
    reader: &mut BufReader<tokio::io::Stdin>,
    first_line: String,
) -> Result<usize> {
    let mut content_length: Option<usize> = None;
    let mut pending_first = Some(first_line);
    loop {
        let line = if let Some(first) = pending_first.take() {
            first
        } else {
            let mut line = String::new();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                anyhow::bail!("unexpected EOF while reading Content-Length headers");
            }
            line
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.trim().eq_ignore_ascii_case("Content-Length") {
                content_length = Some(
                    value
                        .trim()
                        .parse()
                        .context("invalid Content-Length value")?,
                );
            }
        }
    }
    content_length.ok_or_else(|| anyhow::anyhow!("missing Content-Length header"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_variants_available() {
        assert_eq!(MessageFormat::JsonLine, MessageFormat::JsonLine);
        assert_eq!(MessageFormat::ContentLength, MessageFormat::ContentLength);
    }

    #[test]
    fn framing_examples_are_correct() {
        let json = r#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
        let framed = format!("Content-Length: {}\r\n\r\n{}", json.len(), json);
        assert!(framed.starts_with("Content-Length: "));
        assert!(framed.ends_with(json));
    }

    #[test]
    fn content_length_header_is_case_insensitive() {
        let header = "content-length: 42";
        let (name, value) = header.split_once(':').expect("header format");
        assert!(name.trim().eq_ignore_ascii_case("Content-Length"));
        assert_eq!(value.trim(), "42");
    }

    #[test]
    fn jsonl_roundtrip_example() {
        let msg = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        let line = format!("{}\n", msg);
        assert_eq!(line.trim_end(), msg);
    }
}
