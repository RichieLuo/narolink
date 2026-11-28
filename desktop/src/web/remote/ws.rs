use base64::Engine as _;
use reqwest::{Client, StatusCode, Upgraded, header};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};

pub const OP_CONT: u8 = 0x0;
pub const OP_TEXT: u8 = 0x1;
pub const OP_BINARY: u8 = 0x2;
pub const OP_CLOSE: u8 = 0x8;
pub const OP_PING: u8 = 0x9;
pub const OP_PONG: u8 = 0xA;

const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const WRITE_TIMEOUT: Duration = Duration::from_secs(20);

pub enum ConnectOutcome {
    Connected(WsReader, WsWriter),
    AuthRejected,
    UnsupportedByRelay,
    UpgradeBlocked,
    Retryable(&'static str),
}

pub enum WsMessage {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong,
    Close,
}

pub struct WsReader {
    inner: ReadHalf<Upgraded>,
}

pub struct WsWriter {
    inner: WriteHalf<Upgraded>,
}

pub async fn connect(
    client: &Client,
    origin: &str,
    device_id: &str,
    device_key: &str,
    bearer: &str,
) -> ConnectOutcome {
    let key_bytes = uuid::Uuid::new_v4().into_bytes();
    let key = base64::engine::general_purpose::STANDARD.encode(key_bytes);
    let expected_accept = ws_accept_key(&key);
    let url = format!("{origin}/relay/devices/{device_id}/ws");
    let request = client
        .get(&url)
        .bearer_auth(bearer)
        .header("X-Naro-Device-Key", device_key)
        .header(header::CONNECTION, "Upgrade")
        .header(header::UPGRADE, "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", &key);

    let response = match tokio::time::timeout(HANDSHAKE_TIMEOUT, request.send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(_)) | Err(_) => {
            return ConnectOutcome::Retryable("正在重连。检查服务器地址和 HTTPS。");
        }
    };

    let status = response.status();
    let has_ws_header = response
        .headers()
        .get("X-Naro-Relay-WS")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim() == "1");

    match status {
        StatusCode::SWITCHING_PROTOCOLS => {
            let upgrade_ok = header_has_token(response.headers(), header::UPGRADE, "websocket");
            let conn_ok = header_has_token(response.headers(), header::CONNECTION, "upgrade");
            let accept_ok = response
                .headers()
                .get("Sec-WebSocket-Accept")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.trim() == expected_accept);
            if !upgrade_ok || !conn_ok || !accept_ok {
                return ConnectOutcome::Retryable("服务器返回的 WebSocket 握手无效。");
            }
            match tokio::time::timeout(HANDSHAKE_TIMEOUT, response.upgrade()).await {
                Ok(Ok(upgraded)) => {
                    let (reader, writer) = tokio::io::split(upgraded);
                    ConnectOutcome::Connected(
                        WsReader { inner: reader },
                        WsWriter { inner: writer },
                    )
                }
                Ok(Err(_)) | Err(_) => {
                    ConnectOutcome::Retryable("正在重连。检查服务器地址和 HTTPS。")
                }
            }
        }
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ConnectOutcome::AuthRejected,
        StatusCode::UPGRADE_REQUIRED => ConnectOutcome::UpgradeBlocked,
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED | StatusCode::NOT_IMPLEMENTED => {
            if has_ws_header {
                ConnectOutcome::UpgradeBlocked
            } else {
                ConnectOutcome::UnsupportedByRelay
            }
        }
        _ if has_ws_header => ConnectOutcome::UpgradeBlocked,
        _ => ConnectOutcome::Retryable("服务器未接受连接。检查 Naro 中继地址。"),
    }
}

impl WsReader {
    pub async fn read_message(&mut self, max_len: usize) -> std::io::Result<WsMessage> {
        let mut msg_op: Option<u8> = None;
        let mut assembled: Vec<u8> = Vec::new();
        loop {
            let mut hdr = [0u8; 2];
            self.inner.read_exact(&mut hdr).await?;
            let fin = (hdr[0] & 0x80) != 0;
            let rsv = hdr[0] & 0x70;
            let op = hdr[0] & 0x0f;
            let masked = (hdr[1] & 0x80) != 0;
            let mut len = (hdr[1] & 0x7f) as u64;

            if rsv != 0 || masked {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid server frame flags",
                ));
            }
            if len == 126 {
                let mut ext = [0u8; 2];
                self.inner.read_exact(&mut ext).await?;
                len = u16::from_be_bytes(ext) as u64;
            } else if len == 127 {
                let mut ext = [0u8; 8];
                self.inner.read_exact(&mut ext).await?;
                if (ext[0] & 0x80) != 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid 64-bit frame length",
                    ));
                }
                len = u64::from_be_bytes(ext);
            }

            let is_control = op >= 0x8;
            if is_control {
                if !fin || len > 125 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid control frame",
                    ));
                }
            } else if len > max_len as u64
                || (assembled.len() as u64).saturating_add(len) > max_len as u64
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "frame exceeds size limit",
                ));
            }

            let mut payload = vec![0u8; len as usize];
            if len > 0 {
                self.inner.read_exact(&mut payload).await?;
            }

            if is_control {
                match op {
                    OP_PING => return Ok(WsMessage::Ping(payload)),
                    OP_PONG => return Ok(WsMessage::Pong),
                    OP_CLOSE => return Ok(WsMessage::Close),
                    _ => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "unknown control opcode",
                        ));
                    }
                }
            }

            match op {
                OP_TEXT | OP_BINARY => {
                    if msg_op.is_some() {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "unexpected data frame during fragmentation",
                        ));
                    }
                    if fin {
                        return finish_data(op, payload);
                    }
                    msg_op = Some(op);
                    assembled = payload;
                }
                OP_CONT => {
                    let Some(initial) = msg_op else {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "unexpected continuation frame",
                        ));
                    };
                    assembled.extend_from_slice(&payload);
                    if fin {
                        return finish_data(initial, assembled);
                    }
                }
                _ => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "unknown data opcode",
                    ));
                }
            }
        }
    }
}

fn finish_data(op: u8, payload: Vec<u8>) -> std::io::Result<WsMessage> {
    match op {
        OP_TEXT => String::from_utf8(payload)
            .map(WsMessage::Text)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid utf-8")),
        OP_BINARY => Ok(WsMessage::Binary(payload)),
        _ => unreachable!(),
    }
}

impl WsWriter {
    pub async fn write_frame(&mut self, opcode: u8, payload: &[u8]) -> std::io::Result<()> {
        let mask_src = uuid::Uuid::new_v4().into_bytes();
        let mask = [mask_src[0], mask_src[1], mask_src[2], mask_src[3]];
        let len = payload.len();
        let mut frame = Vec::with_capacity(14 + len);
        frame.push(0x80 | (opcode & 0x0f));
        if len <= 125 {
            frame.push(0x80 | (len as u8));
        } else if len <= 65535 {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
        frame.extend_from_slice(&mask);
        let start = frame.len();
        frame.extend_from_slice(payload);
        for (i, byte) in frame[start..].iter_mut().enumerate() {
            *byte ^= mask[i & 3];
        }
        tokio::time::timeout(WRITE_TIMEOUT, async {
            self.inner.write_all(&frame).await?;
            self.inner.flush().await
        })
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "ws write timeout"))?
    }

    pub async fn close(&mut self) {
        let _ = self.write_frame(OP_CLOSE, &[0x03, 0xe8]).await;
        let _ = tokio::time::timeout(Duration::from_secs(1), self.inner.shutdown()).await;
    }
}

fn header_has_token(headers: &header::HeaderMap, name: header::HeaderName, token: &str) -> bool {
    headers.get_all(name).iter().any(|value| {
        value.to_str().ok().is_some_and(|text| {
            text.split(',')
                .any(|part| part.trim().eq_ignore_ascii_case(token))
        })
    })
}

pub fn ws_accept_key(client_key: &str) -> String {
    let mut input = Vec::with_capacity(client_key.len() + WS_GUID.len());
    input.extend_from_slice(client_key.as_bytes());
    input.extend_from_slice(WS_GUID.as_bytes());
    let digest = sha1(&input);
    base64::engine::general_purpose::STANDARD.encode(digest)
}

/// Pure-Rust SHA-1 (RFC 3174), used solely for RFC 6455 Sec-WebSocket-Accept verification.
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x67452301;
    let mut h1: u32 = 0xEFCDAB89;
    let mut h2: u32 = 0x98BADCFE;
    let mut h3: u32 = 0x10325476;
    let mut h4: u32 = 0xC3D2E1F0;

    let bit_len = (data.len() as u64) * 8;
    let mut padded = Vec::with_capacity(data.len() + 72);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    let mut w = [0u32; 80];
    for chunk in padded.chunks_exact(64) {
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h0, h1, h2, h3, h4);
        for (i, &word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDCu32),
                _ => (b ^ c ^ d, 0xCA62C1D6u32),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut out = [0u8; 20];
    out[0..4].copy_from_slice(&h0.to_be_bytes());
    out[4..8].copy_from_slice(&h1.to_be_bytes());
    out[8..12].copy_from_slice(&h2.to_be_bytes());
    out[12..16].copy_from_slice(&h3.to_be_bytes());
    out[16..20].copy_from_slice(&h4.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6455_accept_key_vector() {
        // RFC 6455 Section 1.3 example key
        assert_eq!(
            ws_accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }
}
