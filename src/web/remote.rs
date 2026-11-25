use base64::Engine as _;
use reqwest::{Method, Url, blocking::Client, header, redirect::Policy};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::{Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const MAX_WIRE: u64 = 24 * 1024 * 1024;

#[derive(Clone)]
pub struct Remote(Arc<Shared>);

struct Shared {
    local: String,
    state: Mutex<State>,
}

struct State {
    config: Config,
    stop: Option<Arc<AtomicBool>>,
}

#[derive(Clone, Default)]
pub struct Config {
    pub url: Option<String>,
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
}

#[derive(Serialize)]
struct Answer {
    status: u16,
    content_type: Option<String>,
    set_cookie: Option<String>,
    body: String,
}

impl Remote {
    pub fn new(local: String, configured: Option<String>) -> Self {
        let connected = configured.is_some();
        let detail = if connected {
            "HTTPS 反向代理已配置"
        } else {
            "还未连接中继服务器"
        };
        Self(Arc::new(Shared {
            local,
            state: Mutex::new(State {
                config: Config {
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
                detail: "正在连接中继服务器…".into(),
                connected: false,
            };
        }
        let bearer = Arc::<str>::from(token);
        for _ in 0..4 {
            let settings = self.clone();
            let url = origin.clone();
            let bearer = bearer.clone();
            let stop = stop.clone();
            std::thread::spawn(move || settings.worker(&url, &bearer, &stop));
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

    fn worker(&self, origin: &str, bearer: &str, stop: &AtomicBool) {
        let build = || {
            Client::builder()
                .redirect(Policy::none())
                .connect_timeout(Duration::from_secs(7))
        };
        let (Ok(relay), Ok(local)) = (
            build().timeout(Duration::from_secs(30)).build(),
            build().no_proxy().timeout(Duration::from_secs(28)).build(),
        ) else {
            self.status_for(stop, false, "无法初始化 HTTP 客户端");
            return;
        };
        let authority = origin
            .split_once("://")
            .map(|(_, host)| host)
            .unwrap_or_default();
        if let Ok(reply) = relay
            .post(format!("{origin}/relay/hello"))
            .bearer_auth(bearer)
            .send()
        {
            if reply.status().is_success() {
                self.status_for(stop, true, "电脑已连接中继，可用 Safari 打开网页");
            } else if reply.status().as_u16() == 401 {
                self.status_for(stop, false, "服务器拒绝了连接密钥");
                stop.store(true, Ordering::Release);
                return;
            }
        }
        let poll = format!("{origin}/relay/poll");
        while !stop.load(Ordering::Acquire) {
            match relay.post(&poll).bearer_auth(bearer).send() {
                Ok(response) if response.status().is_success() => {
                    self.status_for(stop, true, "电脑已连接中继，可用 Safari 打开网页");
                    if response.status().as_u16() == 204 {
                        continue;
                    }
                    let Ok(work) = read_work(response) else {
                        continue;
                    };
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    let answer = self.forward(&local, authority, &work);
                    let result = format!("{origin}/relay/answer/{}", work.id);
                    let _ = relay.post(result).bearer_auth(bearer).json(&answer).send();
                    continue;
                }
                Ok(response) if response.status().as_u16() == 401 => {
                    self.status_for(stop, false, "服务器拒绝了连接密钥");
                    stop.store(true, Ordering::Release);
                    return;
                }
                Ok(_) => self.status_for(stop, false, "服务器未接受连接。检查 NaroLink 中继地址。"),
                Err(_) => self.status_for(stop, false, "正在重连。检查服务器地址和 HTTPS。"),
            }
            std::thread::park_timeout(Duration::from_secs(2));
        }
    }

    fn forward(&self, local: &Client, authority: &str, work: &Work) -> Answer {
        match self.fetch(local, authority, work) {
            Ok(answer) => answer,
            Err(()) => Answer {
                status: 502,
                content_type: Some("application/json".into()),
                set_cookie: None,
                body: base64::engine::general_purpose::STANDARD
                    .encode(br#"{"error":"NaroLink local API unavailable"}"#),
            },
        }
    }

    fn fetch(&self, local: &Client, authority: &str, work: &Work) -> Result<Answer, ()> {
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
        let response = request.send().map_err(|_| ())?;
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
        let mut encoder = base64::write::EncoderWriter::new(
            Vec::new(),
            &base64::engine::general_purpose::STANDARD,
        );
        let count =
            std::io::copy(&mut response.take(MAX_WIRE + 1), &mut encoder).map_err(|_| ())?;
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
    let url = Url::parse(value.trim()).map_err(|_| "请输入服务器 HTTPS 地址")?;
    let secure = url.scheme() == "https"
        || (url.scheme() == "http"
            && url
                .host_str()
                .is_some_and(|host| matches!(host, "127.0.0.1" | "[::1]")));
    if !secure
        || !url.has_host()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("服务器地址需要是独立的 HTTPS 地址".into());
    }
    Ok(url.origin().ascii_serialization())
}
