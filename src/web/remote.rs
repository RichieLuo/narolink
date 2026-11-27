use base64::Engine as _;
use reqwest::{
    Method, Url,
    blocking::{Body, Client, RequestBuilder, Response},
    header,
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;

const MAX_WIRE: u64 = 24 * 1024 * 1024;

#[derive(Clone)]
pub struct Remote(Arc<Shared>);

struct Shared {
    local: String,
    device_id: String,
    device_key: String,
    device_name: String,
    state: Mutex<State>,
}

struct State {
    config: Config,
    stop: Option<Arc<AtomicBool>>,
}

#[derive(Clone, Default)]
pub struct Config {
    pub url: Option<String>,
    pub device_url: Option<String>,
    pub detail: String,
    pub connected: bool,
}

#[derive(Deserialize)]
struct Work {
    id: String,
    method: String,
    path: String,
    origin: Option<String>,
    content_type: Option<String>,
    cookie: Option<String>,
    body: Option<Value>,
    #[serde(default)]
    stream: bool,
}

#[derive(Serialize)]
struct Answer {
    status: u16,
    content_type: Option<String>,
    set_cookie: Option<String>,
    body: String,
}

impl Remote {
    pub fn new(local: String, configured: Option<String>, key: [u8; 32]) -> Self {
        let connected = configured.is_some();
        let detail = if connected {
            "HTTPS 反向代理已配置"
        } else {
            "还未连接中继服务器"
        };
        Self(Arc::new(Shared {
            local,
            device_id: hex(&Sha256::digest(key)[..16]),
            device_key: hex(&key),
            device_name: device_name(),
            state: Mutex::new(State {
                config: Config {
                    device_url: configured.clone(),
                    url: configured,
                    detail: detail.into(),
                    connected,
                },
                stop: None,
            }),
        }))
    }

    pub fn status(&self) -> Config {
        self.0.state.lock().unwrap().config.clone()
    }

    pub fn configure_from_environment(&self) -> Result<(), String> {
        if let Ok(url) = std::env::var("NAROLINK_RELAY_URL")
            && !url.trim().is_empty()
        {
            let token = std::env::var("NAROLINK_RELAY_TOKEN").unwrap_or_default();
            self.connect(&url, token)?;
        }
        Ok(())
    }

    pub fn restore(&self, url: &str) {
        #[cfg(windows)]
        if self.status().url.is_none()
            && let Ok(url) = normalize(url)
            && let Ok(entry) = keyring::Entry::new("NaroLink relay", &url)
            && let Ok(token) = entry.get_password()
        {
            let _ = self.connect(&url, token);
        }
        #[cfg(not(windows))]
        let _ = url;
    }

    pub fn connect_from_desktop(&self, url: &str, token: String) -> Result<(), String> {
        #[cfg(windows)]
        {
            let origin = normalize(url)?;
            self.connect(&origin, token.clone())?;
            keyring::Entry::new("NaroLink relay", &origin)
                .and_then(|entry| entry.set_password(&token))
                .map_err(|_| "已启动连接，系统凭据未能保存".into())
        }
        #[cfg(not(windows))]
        self.connect(url, token)
    }
    pub fn connect(&self, url: &str, token: String) -> Result<(), String> {
        let origin = normalize(url)?;
        if !(32..=512).contains(&token.len()) || token.chars().any(char::is_whitespace) {
            return Err("请输入至少 32 个字符且不含空格的服务器连接密钥".into());
        }
        let stop = Arc::new(AtomicBool::new(false));
        {
            let mut state = self.0.state.lock().unwrap();
            if let Some(previous) = state.stop.replace(stop.clone()) {
                previous.store(true, Ordering::Release);
            }
            state.config = Config {
                url: Some(origin.clone()),
                device_url: Some(format!("{origin}/devices/{}/", self.0.device_id)),
                detail: "正在连接中继服务器…".into(),
                connected: false,
            };
        }
        let bearer = Arc::<str>::from(token);
        let streams = Arc::new(Semaphore::new(8));
        for _ in 0..4 {
            let settings = self.clone();
            let url = origin.clone();
            let bearer = bearer.clone();
            let stop = stop.clone();
            let streams = streams.clone();
            std::thread::spawn(move || settings.worker(&url, &bearer, &stop, &streams));
        }
        Ok(())
    }

    pub fn shutdown(&self) {
        if let Some(stop) = self.0.state.lock().unwrap().stop.take() {
            stop.store(true, Ordering::Release);
        }
    }

    fn status_for(&self, stop: &AtomicBool, connected: bool, detail: &str) {
        let mut state = self.0.state.lock().unwrap();
        if state
            .stop
            .as_deref()
            .is_some_and(|current| std::ptr::eq(current, stop))
        {
            state.config.connected = connected;
            state.config.detail = detail.into();
        }
    }

    fn worker(&self, origin: &str, bearer: &str, stop: &AtomicBool, streams: &Arc<Semaphore>) {
        let build = || {
            Client::builder()
                .redirect(Policy::none())
                .connect_timeout(Duration::from_secs(7))
        };
        let (Ok(relay), Ok(local), Ok(stream_relay), Ok(stream_local)) = (
            build().timeout(Duration::from_secs(30)).build(),
            build().no_proxy().timeout(Duration::from_secs(28)).build(),
            build().timeout(None).build(),
            build().no_proxy().timeout(None).build(),
        ) else {
            self.status_for(stop, false, "无法初始化 HTTP 客户端");
            return;
        };
        let authority = origin
            .split_once("://")
            .map(|(_, host)| host)
            .unwrap_or_default();
        let endpoint = format!("{origin}/relay/devices/{}", self.0.device_id);
        let request = |client: &Client, operation: &str| {
            client
                .post(format!("{endpoint}/{operation}"))
                .bearer_auth(bearer)
                .header("X-Naro-Device-Key", &self.0.device_key)
        };
        let mut registered = false;
        while !stop.load(Ordering::Acquire) {
            let response = if registered {
                request(&relay, "poll").send()
            } else {
                request(&relay, "hello")
                    .json(&json!({"name": &self.0.device_name, "streaming": true}))
                    .send()
            };
            match response {
                Ok(response) if response.status().is_success() => {
                    self.status_for(stop, true, "中继服务器已连接");
                    registered = true;
                    if response.status().as_u16() == 204 {
                        continue;
                    }
                    let Ok(work) = read_work(response) else {
                        continue;
                    };
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    let reply = request(&relay, &format!("answer/{}", work.id));
                    if work.stream {
                        // Keep the ordinary workers free to handle stop and queue requests.
                        if let Ok(permit) = streams.clone().try_acquire_owned() {
                            let local = self.local_request(&stream_local, authority, &work);
                            let upload = request(&stream_relay, &format!("stream/{}", work.id));
                            let fallback = request(&relay, &format!("answer/{}", work.id));
                            let task = std::thread::Builder::new().name("naro-reply-stream".into());
                            if task
                                .spawn(move || {
                                    let _permit = permit;
                                    forward_stream(local, upload, fallback);
                                })
                                .is_ok()
                            {
                                continue;
                            }
                        }
                        let _ = reply.json(&unavailable()).send();
                    } else {
                        let answer = self
                            .local_request(&local, authority, &work)
                            .and_then(|request| request.send().map_err(|_| ()))
                            .and_then(read_answer)
                            .unwrap_or_else(|()| unavailable());
                        let _ = reply.json(&answer).send();
                    }
                    continue;
                }
                Ok(response) if matches!(response.status().as_u16(), 401 | 403) => {
                    self.status_for(stop, false, "服务器拒绝了连接密钥");
                    stop.store(true, Ordering::Release);
                    return;
                }
                Ok(response) if response.status().as_u16() == 404 => {
                    if registered {
                        registered = false;
                        continue;
                    }
                    self.status_for(stop, false, "中继没有设备连接接口，请更新中继版本。");
                }
                Ok(_) => self.status_for(stop, false, "服务器未接受连接。检查 Naro 中继地址。"),
                Err(_) => self.status_for(stop, false, "正在重连。检查服务器地址和 HTTPS。"),
            }
            std::thread::park_timeout(Duration::from_secs(2));
        }
    }

    fn local_request(
        &self,
        local: &Client,
        authority: &str,
        work: &Work,
    ) -> Result<RequestBuilder, ()> {
        let method = match work.method.as_str() {
            "GET" => Method::GET,
            "HEAD" => Method::HEAD,
            "POST" => Method::POST,
            _ => return Err(()),
        };
        if !work.path.starts_with("/api/") || work.path.len() > 2048 {
            return Err(());
        }
        let target = Url::parse(&format!("{}{}", self.0.local, work.path)).map_err(|_| ())?;
        if !target.path().starts_with("/api/") {
            return Err(());
        }
        let mut request = local
            .request(method, target)
            .header(header::HOST, authority);
        if let Some(origin) = &work.origin {
            request = request.header(header::ORIGIN, origin);
        }
        if let Some(cookie) = &work.cookie {
            request = request.header(header::COOKIE, cookie);
        }
        if let Some(content_type) = &work.content_type {
            request = request.header(header::CONTENT_TYPE, content_type);
        }
        if let Some(body) = &work.body {
            request = request.json(body);
        }
        if work.stream {
            request = request.header(header::ACCEPT, "text/event-stream");
        }
        Ok(request)
    }
}

fn forward_stream(
    local: Result<RequestBuilder, ()>,
    upload: RequestBuilder,
    fallback: RequestBuilder,
) {
    let response = local.and_then(|request| request.send().map_err(|_| ()));
    let answer = match response {
        Ok(response)
            if response.status().is_success()
                && response
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .is_some_and(|value| value == "text/event-stream") =>
        {
            if upload
                .header(header::CONTENT_TYPE, "text/event-stream")
                .body(Body::new(response))
                .send()
                .is_ok_and(|response| response.status().is_success())
            {
                return;
            }
            unavailable()
        }
        Ok(response) => read_answer(response).unwrap_or_else(|()| unavailable()),
        Err(()) => unavailable(),
    };
    let _ = fallback.json(&answer).send();
}

fn unavailable() -> Answer {
    Answer {
        status: 502,
        content_type: Some("application/json".into()),
        set_cookie: None,
        body: base64::engine::general_purpose::STANDARD
            .encode(br#"{"error":"Naro local API unavailable"}"#),
    }
}

fn read_answer(response: Response) -> Result<Answer, ()> {
    let status = response.status().as_u16();
    let header_string = |name| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let content_type = header_string(header::CONTENT_TYPE);
    let set_cookie = header_string(header::SET_COOKIE);
    let mut encoder =
        base64::write::EncoderWriter::new(Vec::new(), &base64::engine::general_purpose::STANDARD);
    let count = std::io::copy(&mut response.take(MAX_WIRE + 1), &mut encoder).map_err(|_| ())?;
    if count > MAX_WIRE {
        return Err(());
    }
    encoder.flush().map_err(|_| ())?;
    let body = String::from_utf8(encoder.finish().map_err(|_| ())?).map_err(|_| ())?;
    Ok(Answer {
        status,
        content_type,
        set_cookie,
        body,
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn device_name() -> String {
    std::env::var("NAROLINK_DEVICE_NAME")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            if cfg!(windows) {
                std::env::var("COMPUTERNAME").ok()
            } else {
                std::process::Command::new("/bin/hostname")
                    .output()
                    .ok()
                    .filter(|output| output.status.success())
                    .and_then(|output| String::from_utf8(output.stdout).ok())
            }
        })
        .unwrap_or_else(|| "Naro 电脑".into())
        .trim()
        .chars()
        .take(80)
        .collect()
}

fn read_work(response: reqwest::blocking::Response) -> Result<Work, ()> {
    let mut bytes = Vec::new();
    response
        .take(MAX_WIRE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    if bytes.len() as u64 > MAX_WIRE {
        return Err(());
    }
    let work: Work = serde_json::from_slice(&bytes).map_err(|_| ())?;
    if work.id.len() != 32 || !work.id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(());
    }
    Ok(work)
}

pub fn normalize(value: &str) -> Result<String, String> {
    let url = connection_url(value)
        .filter(|url| url.path() == "/")
        .ok_or("服务器地址需要是独立的 HTTPS 地址")?;
    Ok(url.origin().ascii_serialization())
}

pub fn device_url(value: &str) -> Result<Url, String> {
    connection_url(value)
        .filter(|url| {
            url.path() == "/"
                || url
                    .path()
                    .strip_prefix("/devices/")
                    .and_then(|path| path.strip_suffix('/'))
                    .is_some_and(|id| {
                        id.len() == 32
                            && id
                                .bytes()
                                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
                    })
        })
        .ok_or_else(|| "请粘贴目标电脑上的设备地址".into())
}

fn connection_url(value: &str) -> Option<Url> {
    let url = Url::parse(value.trim()).ok()?;
    let secure = url.scheme() == "https"
        || (url.scheme() == "http"
            && url
                .host_str()
                .is_some_and(|host| matches!(host, "127.0.0.1" | "[::1]")));
    (secure
        && url.has_host()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.username().is_empty()
        && url.password().is_none())
    .then_some(url)
}
