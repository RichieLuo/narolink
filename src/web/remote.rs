mod ws;

use reqwest::{Client as AsyncClient, Method, Url, header, redirect::Policy};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Notify, Semaphore, mpsc, watch};
use ws::{ConnectOutcome, OP_BINARY, OP_PING, OP_PONG, OP_TEXT, WsMessage, WsReader, WsWriter};

const MAX_WIRE: u64 = 24 * 1024 * 1024;
const MAX_REQ_BODY: usize = 18 * 1024 * 1024;
const MAX_PENDING_REQ_BYTES: usize = 24 * 1024 * 1024;
const DEFAULT_MAX_CHUNK_BYTES: usize = 64 * 1024;
const MAX_WS_TEXT_BYTES: usize = 64 * 1024;
const WS_BIN_HEADER_LEN: usize = 18;
const WS_BIN_REQ_CHUNK: u8 = 1;
const WS_BIN_RES_CHUNK: u8 = 2;
const WS_BIN_STREAM_CHUNK: u8 = 3;
const WS_BIN_FLAG_EOF: u8 = 0x01;

const DEVICE_MAX_REQUESTS: usize = 8;
const DEVICE_MAX_STREAMS: usize = 8;
const SEEN_ID_CAP: usize = 512;

const DEFAULT_HEARTBEAT_SEC: u64 = 25;
const PONG_TIMEOUT: Duration = Duration::from_secs(15);
const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(65);
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
const WINDOW_WAIT_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_RESPONSE_WINDOW: usize = 2 * 1024 * 1024;
const DEFAULT_STREAM_WINDOW: usize = 256 * 1024;

const BACKOFF_BASE_MS: u64 = 500;
const BACKOFF_MAX_MS: u64 = 30_000;

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
    stop: Option<Arc<StopSignal>>,
}

struct StopSignal {
    flag: AtomicBool,
    notify: watch::Sender<bool>,
}

impl StopSignal {
    fn new() -> Self {
        let (notify, _) = watch::channel(false);
        Self {
            flag: AtomicBool::new(false),
            notify,
        }
    }

    fn stop(&self) {
        self.flag.store(true, Ordering::Release);
        let _ = self.notify.send(true);
    }

    fn is_stopped(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }

    fn subscribe(&self) -> watch::Receiver<bool> {
        self.notify.subscribe()
    }
}

#[derive(Clone, Default)]
pub struct Config {
    pub url: Option<String>,
    pub device_url: Option<String>,
    pub detail: String,
    pub connected: bool,
}

#[derive(Deserialize)]
struct WsInboundText {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    heartbeat_sec: u64,
    #[serde(default)]
    max_chunk_bytes: usize,
    #[serde(default)]
    response_window: usize,
    #[serde(default)]
    stream_window: usize,
    #[serde(default)]
    method: String,
    #[serde(default)]
    path: String,
    origin: Option<String>,
    content_type: Option<String>,
    cookie: Option<String>,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    body_len: usize,
    #[serde(default)]
    bytes: usize,
}

struct WsWork {
    id: String,
    raw_id: [u8; 16],
    method: String,
    path: String,
    origin: Option<String>,
    content_type: Option<String>,
    cookie: Option<String>,
    stream: bool,
    body: Vec<u8>,
}

struct PendingReq {
    work: WsWork,
    expected_len: usize,
}

struct ActiveJob {
    cancel: watch::Sender<bool>,
    window: Arc<SendWindow>,
}

struct SendWindow {
    available: AtomicUsize,
    notify: Notify,
}

impl SendWindow {
    fn new(initial: usize) -> Self {
        Self {
            available: AtomicUsize::new(initial),
            notify: Notify::new(),
        }
    }

    fn release(&self, bytes: usize) {
        if bytes > 0 {
            self.available.fetch_add(bytes, Ordering::AcqRel);
            self.notify.notify_waiters();
        }
    }

    async fn acquire(
        &self,
        bytes: usize,
        job_cancel: &mut watch::Receiver<bool>,
        session_cancel: &mut watch::Receiver<bool>,
    ) -> bool {
        if bytes == 0 {
            return true;
        }
        let deadline = tokio::time::Instant::now() + WINDOW_WAIT_TIMEOUT;
        loop {
            let current = self.available.load(Ordering::Acquire);
            if current >= bytes {
                if self
                    .available
                    .compare_exchange(
                        current,
                        current - bytes,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok()
                {
                    return true;
                }
                continue;
            }
            if *job_cancel.borrow() || *session_cancel.borrow() {
                return false;
            }
            let notified = self.notify.notified();
            tokio::select! {
                _ = notified => {}
                _ = tokio::time::sleep_until(deadline) => return false,
                _ = wait_true(job_cancel) => return false,
                _ = wait_true(session_cancel) => return false,
            }
        }
    }
}

struct OutboundFrame {
    opcode: u8,
    payload: Vec<u8>,
}

struct Outbound {
    high: mpsc::Sender<OutboundFrame>,
    data: mpsc::Sender<OutboundFrame>,
}

impl Outbound {
    async fn send_high(&self, frame: OutboundFrame) -> bool {
        self.high.send(frame).await.is_ok()
    }

    async fn send_data(&self, frame: OutboundFrame) -> bool {
        self.data.send(frame).await.is_ok()
    }

    fn try_send_high(&self, frame: OutboundFrame) -> bool {
        self.high.try_send(frame).is_ok()
    }

    async fn send_json_high(&self, value: &Value) -> bool {
        let Ok(payload) = serde_json::to_vec(value) else {
            return false;
        };
        self.send_high(OutboundFrame {
            opcode: OP_TEXT,
            payload,
        })
        .await
    }
}

struct SeenIds {
    set: HashSet<[u8; 16]>,
    order: VecDeque<[u8; 16]>,
}

impl SeenIds {
    fn new() -> Self {
        Self {
            set: HashSet::with_capacity(SEEN_ID_CAP),
            order: VecDeque::with_capacity(SEEN_ID_CAP),
        }
    }

    fn insert(&mut self, id: [u8; 16]) -> bool {
        if !self.set.insert(id) {
            return false;
        }
        self.order.push_back(id);
        if self.order.len() > SEEN_ID_CAP
            && let Some(oldest) = self.order.pop_front()
        {
            self.set.remove(&oldest);
        }
        true
    }
}

struct Backoff {
    attempt: u32,
}

impl Backoff {
    fn new() -> Self {
        Self { attempt: 0 }
    }

    fn reset(&mut self) {
        self.attempt = 0;
    }

    fn next_delay(&mut self) -> Duration {
        let shift = self.attempt.min(10);
        let cap_ms = BACKOFF_BASE_MS
            .saturating_mul(1u64 << shift)
            .min(BACKOFF_MAX_MS);
        self.attempt = self.attempt.saturating_add(1);
        let rand_bytes = uuid::Uuid::new_v4().into_bytes();
        let rand_u64 = u64::from_le_bytes(rand_bytes[..8].try_into().unwrap());
        let half = (cap_ms / 2).max(1);
        let jitter = rand_u64 % (cap_ms - half + 1);
        Duration::from_millis(half + jitter)
    }
}

#[derive(Clone, Copy)]
struct SessionParams {
    heartbeat_sec: u64,
    max_chunk_bytes: usize,
    response_window: usize,
    stream_window: usize,
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
        let stop = Arc::new(StopSignal::new());
        {
            let mut state = self.0.state.lock().unwrap();
            if let Some(previous) = state.stop.replace(stop.clone()) {
                previous.stop();
            }
            state.config = Config {
                url: Some(origin.clone()),
                device_url: Some(format!("{origin}/devices/{}/", self.0.device_id)),
                detail: "正在连接中继服务器…".into(),
                connected: false,
            };
        }
        let settings = self.clone();
        let bearer = Arc::<str>::from(token);
        let _ = std::thread::Builder::new()
            .name("naro-relay".into())
            .spawn(move || settings.run_coordinator(origin, bearer, stop));
        Ok(())
    }

    pub fn shutdown(&self) {
        if let Some(stop) = self.0.state.lock().unwrap().stop.take() {
            stop.stop();
        }
    }

    fn status_for(&self, stop: &StopSignal, connected: bool, detail: &str) {
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

    fn run_coordinator(&self, origin: String, bearer: Arc<str>, stop: Arc<StopSignal>) {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            self.status_for(&stop, false, "无法初始化异步连接运行时");
            return;
        };
        runtime.block_on(self.coordinator_loop(origin, bearer, stop));
    }

    async fn coordinator_loop(&self, origin: String, bearer: Arc<str>, stop: Arc<StopSignal>) {
        let (Ok(ws_client), Ok(local_client), Ok(stream_local)) = (
            AsyncClient::builder()
                .http1_only()
                .redirect(Policy::none())
                .connect_timeout(Duration::from_secs(7))
                .build(),
            AsyncClient::builder()
                .no_proxy()
                .redirect(Policy::none())
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(28))
                .build(),
            AsyncClient::builder()
                .no_proxy()
                .redirect(Policy::none())
                .connect_timeout(Duration::from_secs(5))
                .build(),
        ) else {
            self.status_for(&stop, false, "无法初始化 HTTP 客户端");
            return;
        };

        let authority = origin
            .split_once("://")
            .map(|(_, host)| host)
            .unwrap_or_default()
            .to_owned();
        let mut backoff = Backoff::new();
        let mut stop_rx = stop.subscribe();

        while !stop.is_stopped() {
            let outcome = tokio::select! {
                outcome = ws::connect(
                    &ws_client,
                    &origin,
                    &self.0.device_id,
                    &self.0.device_key,
                    &bearer,
                ) => outcome,
                _ = wait_true(&mut stop_rx) => return,
            };

            match outcome {
                ConnectOutcome::Connected(reader, writer) => {
                    let healthy = self
                        .run_ws_session(
                            reader,
                            writer,
                            &local_client,
                            &stream_local,
                            &authority,
                            &stop,
                        )
                        .await;
                    if stop.is_stopped() {
                        return;
                    }
                    self.status_for(&stop, false, "正在重连。检查服务器地址和 HTTPS。");
                    if healthy {
                        backoff.reset();
                    }
                }
                ConnectOutcome::AuthRejected => {
                    self.status_for(&stop, false, "服务器拒绝了连接密钥");
                    stop.stop();
                    return;
                }
                ConnectOutcome::UnsupportedByRelay => {
                    self.status_for(&stop, false, "中继不支持 WSS 长连接，请更新中继版本。");
                }
                ConnectOutcome::UpgradeBlocked => {
                    self.status_for(
                        &stop,
                        false,
                        "服务器未完成 WebSocket 升级，请检查反向代理配置",
                    );
                }
                ConnectOutcome::Retryable(detail) => {
                    self.status_for(&stop, false, detail);
                }
            }

            let delay = backoff.next_delay();
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = wait_true(&mut stop_rx) => return,
            }
        }
    }

    async fn run_ws_session(
        &self,
        mut reader: WsReader,
        mut writer: WsWriter,
        local_client: &AsyncClient,
        stream_local: &AsyncClient,
        authority: &str,
        stop: &Arc<StopSignal>,
    ) -> bool {
        let hello = json!({
            "type": "hello",
            "name": &self.0.device_name,
            "streaming": true,
            "protocol": 1,
        });
        let Ok(hello_bytes) = serde_json::to_vec(&hello) else {
            return false;
        };
        if writer.write_frame(OP_TEXT, &hello_bytes).await.is_err() {
            return false;
        }

        let params = match read_hello_ack(&mut reader, &mut writer).await {
            Some(params) => params,
            None => {
                writer.close().await;
                return false;
            }
        };

        self.status_for(stop, true, "中继服务器已连接");
        let connected_at = Instant::now();

        let (high_tx, mut high_rx) = mpsc::channel::<OutboundFrame>(64);
        let (data_tx, mut data_rx) = mpsc::channel::<OutboundFrame>(32);
        let outbound = Arc::new(Outbound {
            high: high_tx,
            data: data_tx,
        });
        let (session_cancel_tx, _) = watch::channel(false);
        let mut writer_cancel = session_cancel_tx.subscribe();

        let writer_cancel_notify = session_cancel_tx.clone();
        let writer_task = tokio::spawn(async move {
            loop {
                let frame = tokio::select! {
                    biased;
                    _ = wait_true(&mut writer_cancel) => break,
                    frame = high_rx.recv() => match frame {
                        Some(frame) => frame,
                        None => break,
                    },
                    frame = data_rx.recv() => match frame {
                        Some(frame) => frame,
                        None => break,
                    },
                };
                if writer.write_frame(frame.opcode, &frame.payload).await.is_err() {
                    let _ = writer_cancel_notify.send(true);
                    break;
                }
            }
            writer.close().await;
        });

        let request_sem = Arc::new(Semaphore::new(DEVICE_MAX_REQUESTS));
        let stream_sem = Arc::new(Semaphore::new(DEVICE_MAX_STREAMS));
        let active_jobs: Arc<Mutex<HashMap<String, ActiveJob>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let mut assembling: HashMap<[u8; 16], PendingReq> = HashMap::new();
        let mut pending_bytes: usize = 0;
        let mut seen_ids = SeenIds::new();

        let max_inbound_frame =
            WS_BIN_HEADER_LEN + params.max_chunk_bytes.max(DEFAULT_MAX_CHUNK_BYTES);
        let heartbeat_interval = Duration::from_secs(params.heartbeat_sec);
        let mut heartbeat_tick = tokio::time::interval_at(
            tokio::time::Instant::now() + heartbeat_interval,
            heartbeat_interval,
        );
        heartbeat_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut liveness_check = tokio::time::interval(Duration::from_secs(5));
        liveness_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        let mut last_rx = Instant::now();
        let mut awaiting_pong_since: Option<Instant> = None;
        let mut ping_seq: u64 = 0;

        let mut stop_rx = stop.subscribe();
        let mut session_watch = session_cancel_tx.subscribe();

        loop {
            tokio::select! {
                _ = wait_true(&mut stop_rx) => break,
                _ = wait_true(&mut session_watch) => break,
                _ = heartbeat_tick.tick() => {
                    if awaiting_pong_since.is_some() {
                        break;
                    }
                    ping_seq = ping_seq.wrapping_add(1);
                    if !outbound.try_send_high(OutboundFrame {
                        opcode: OP_PING,
                        payload: ping_seq.to_be_bytes().to_vec(),
                    }) {
                        break;
                    }
                    awaiting_pong_since = Some(Instant::now());
                }
                _ = liveness_check.tick() => {
                    if last_rx.elapsed() > READ_IDLE_TIMEOUT {
                        break;
                    }
                    if awaiting_pong_since.is_some_and(|sent| sent.elapsed() > PONG_TIMEOUT) {
                        break;
                    }
                }
                message = reader.read_message(max_inbound_frame) => {
                    let Ok(message) = message else {
                        break;
                    };
                    last_rx = Instant::now();
                    match message {
                        WsMessage::Ping(payload) => {
                            let _ = outbound.try_send_high(OutboundFrame {
                                opcode: OP_PONG,
                                payload,
                            });
                        }
                        WsMessage::Pong => {
                            awaiting_pong_since = None;
                        }
                        WsMessage::Close => break,
                        WsMessage::Text(text) => {
                            if text.len() > MAX_WS_TEXT_BYTES {
                                break;
                            }
                            let Ok(msg) = serde_json::from_str::<WsInboundText>(&text) else {
                                continue;
                            };
                            match msg.kind.as_str() {
                                "req_start" => {
                                    let Some(raw_id) = parse_hex_id(&msg.id) else {
                                        continue;
                                    };
                                    if !seen_ids.insert(raw_id) {
                                        continue;
                                    }
                                    if msg.body_len > MAX_REQ_BODY
                                        || pending_bytes.saturating_add(msg.body_len) > MAX_PENDING_REQ_BYTES
                                    {
                                        let out = outbound.clone();
                                        let id = msg.id;
                                        tokio::spawn(async move {
                                            send_error_response(&out, &id, 503, "请求缓冲超限，请稍后重试").await;
                                        });
                                        continue;
                                    }
                                    let work = WsWork {
                                        id: msg.id,
                                        raw_id,
                                        method: msg.method,
                                        path: msg.path,
                                        origin: msg.origin,
                                        content_type: msg.content_type,
                                        cookie: msg.cookie,
                                        stream: msg.stream,
                                        body: Vec::with_capacity(msg.body_len.min(64 * 1024)),
                                    };
                                    if msg.body_len == 0 {
                                        self.spawn_ws_job(
                                            work,
                                            params,
                                            local_client.clone(),
                                            stream_local.clone(),
                                            authority.to_owned(),
                                            outbound.clone(),
                                            request_sem.clone(),
                                            stream_sem.clone(),
                                            active_jobs.clone(),
                                            session_cancel_tx.subscribe(),
                                        );
                                    } else {
                                        pending_bytes = pending_bytes.saturating_add(msg.body_len);
                                        assembling.insert(
                                            raw_id,
                                            PendingReq {
                                                work,
                                                expected_len: msg.body_len,
                                            },
                                        );
                                    }
                                }
                                "req_cancel" => {
                                    if let Some(raw_id) = parse_hex_id(&msg.id)
                                        && let Some(removed) = assembling.remove(&raw_id)
                                    {
                                        pending_bytes = pending_bytes.saturating_sub(removed.expected_len);
                                    }
                                    if let Some(job) = active_jobs.lock().unwrap().get(&msg.id) {
                                        let _ = job.cancel.send(true);
                                    }
                                }
                                "ack" => {
                                    if let Some(job) = active_jobs.lock().unwrap().get(&msg.id) {
                                        job.window.release(msg.bytes);
                                    }
                                }
                                _ => {}
                            }
                        }
                        WsMessage::Binary(payload) => {
                            if payload.len() < WS_BIN_HEADER_LEN {
                                continue;
                            }
                            let kind = payload[0];
                            let flags = payload[1];
                            if kind != WS_BIN_REQ_CHUNK {
                                continue;
                            }
                            let raw_id: [u8; 16] = payload[2..18].try_into().unwrap();
                            let chunk = &payload[WS_BIN_HEADER_LEN..];
                            let eof = (flags & WS_BIN_FLAG_EOF) != 0;
                            let Some(pending) = assembling.get_mut(&raw_id) else {
                                continue;
                            };
                            if pending.work.body.len().saturating_add(chunk.len()) > pending.expected_len {
                                if let Some(removed) = assembling.remove(&raw_id) {
                                    pending_bytes = pending_bytes.saturating_sub(removed.expected_len);
                                    let out = outbound.clone();
                                    let id = removed.work.id;
                                    tokio::spawn(async move {
                                        send_error_response(&out, &id, 400, "请求体长度异常").await;
                                    });
                                }
                                continue;
                            }
                            pending.work.body.extend_from_slice(chunk);
                            if eof {
                                if let Some(completed) = assembling.remove(&raw_id) {
                                    pending_bytes = pending_bytes.saturating_sub(completed.expected_len);
                                    if completed.work.body.len() != completed.expected_len {
                                        let out = outbound.clone();
                                        let id = completed.work.id;
                                        tokio::spawn(async move {
                                            send_error_response(&out, &id, 400, "请求体不完整").await;
                                        });
                                        continue;
                                    }
                                    self.spawn_ws_job(
                                        completed.work,
                                        params,
                                        local_client.clone(),
                                        stream_local.clone(),
                                        authority.to_owned(),
                                        outbound.clone(),
                                        request_sem.clone(),
                                        stream_sem.clone(),
                                        active_jobs.clone(),
                                        session_cancel_tx.subscribe(),
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        let _ = session_cancel_tx.send(true);
        assembling.clear();
        for (_, job) in active_jobs.lock().unwrap().drain() {
            let _ = job.cancel.send(true);
        }
        drop(outbound);
        let _ = tokio::time::timeout(Duration::from_secs(2), writer_task).await;

        connected_at.elapsed() >= Duration::from_secs(2)
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_ws_job(
        &self,
        work: WsWork,
        params: SessionParams,
        local_client: AsyncClient,
        stream_local: AsyncClient,
        authority: String,
        outbound: Arc<Outbound>,
        request_sem: Arc<Semaphore>,
        stream_sem: Arc<Semaphore>,
        active_jobs: Arc<Mutex<HashMap<String, ActiveJob>>>,
        mut session_cancel: watch::Receiver<bool>,
    ) {
        let local_base = self.0.local.clone();
        let (job_cancel_tx, mut job_cancel_rx) = watch::channel(false);
        let initial_window = if work.stream {
            params.stream_window
        } else {
            params.response_window
        };
        let window = Arc::new(SendWindow::new(initial_window));
        active_jobs.lock().unwrap().insert(
            work.id.clone(),
            ActiveJob {
                cancel: job_cancel_tx,
                window: window.clone(),
            },
        );

        tokio::spawn(async move {
            let id = work.id.clone();
            struct JobGuard {
                id: String,
                map: Arc<Mutex<HashMap<String, ActiveJob>>>,
            }
            impl Drop for JobGuard {
                fn drop(&mut self) {
                    self.map.lock().unwrap().remove(&self.id);
                }
            }
            let _guard = JobGuard {
                id: id.clone(),
                map: active_jobs,
            };

            let permit = if work.stream {
                stream_sem.try_acquire_owned()
            } else {
                request_sem.try_acquire_owned()
            };
            let Ok(_permit) = permit else {
                send_error_response(&outbound, &id, 503, "电脑请求较多，请稍后重试").await;
                return;
            };

            let client = if work.stream {
                &stream_local
            } else {
                &local_client
            };
            let Ok(request) = build_async_local_request(&local_base, client, &authority, &work)
            else {
                send_error_response(&outbound, &id, 400, "无效的本地请求").await;
                return;
            };

            let send_fut = request.send();
            let response = tokio::select! {
                res = send_fut => res,
                _ = wait_true(&mut job_cancel_rx) => return,
                _ = wait_true(&mut session_cancel) => return,
            };

            let Ok(mut response) = response else {
                send_unavailable_ws(&outbound, &id).await;
                return;
            };

            let is_sse = work.stream
                && response.status().is_success()
                && response
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| v.starts_with("text/event-stream"));

            if is_sse {
                if !outbound
                    .send_json_high(&json!({
                        "type": "stream_start",
                        "id": &id,
                        "content_type": "text/event-stream",
                    }))
                    .await
                {
                    return;
                }
                loop {
                    let next = tokio::select! {
                        chunk = response.chunk() => chunk,
                        _ = wait_true(&mut job_cancel_rx) => break,
                        _ = wait_true(&mut session_cancel) => break,
                    };
                    match next {
                        Ok(Some(bytes)) => {
                            if bytes.is_empty() {
                                continue;
                            }
                            for piece in bytes.chunks(params.max_chunk_bytes) {
                                if !window
                                    .acquire(piece.len(), &mut job_cancel_rx, &mut session_cancel)
                                    .await
                                {
                                    let eof_frame = encode_bin_frame(
                                        WS_BIN_STREAM_CHUNK,
                                        WS_BIN_FLAG_EOF,
                                        &work.raw_id,
                                        &[],
                                    );
                                    let _ = outbound.send_data(eof_frame).await;
                                    return;
                                }
                                let frame = encode_bin_frame(
                                    WS_BIN_STREAM_CHUNK,
                                    0,
                                    &work.raw_id,
                                    piece,
                                );
                                if !outbound.send_data(frame).await {
                                    return;
                                }
                            }
                        }
                        Ok(None) | Err(_) => {
                            let eof_frame = encode_bin_frame(
                                WS_BIN_STREAM_CHUNK,
                                WS_BIN_FLAG_EOF,
                                &work.raw_id,
                                &[],
                            );
                            let _ = outbound.send_data(eof_frame).await;
                            break;
                        }
                    }
                }
                return;
            }

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
            let empty_body = matches!(status, 204 | 304) || work.method == "HEAD";

            if empty_body {
                let _ = outbound
                    .send_json_high(&json!({
                        "type": "res_start",
                        "id": &id,
                        "status": status,
                        "content_type": content_type,
                        "set_cookie": set_cookie,
                        "eof": true,
                    }))
                    .await;
                return;
            }

            if !outbound
                .send_json_high(&json!({
                    "type": "res_start",
                    "id": &id,
                    "status": status,
                    "content_type": content_type,
                    "set_cookie": set_cookie,
                    "eof": false,
                }))
                .await
            {
                return;
            }

            let mut total: u64 = 0;
            loop {
                let next = tokio::select! {
                    chunk = response.chunk() => chunk,
                    _ = wait_true(&mut job_cancel_rx) => return,
                    _ = wait_true(&mut session_cancel) => return,
                };
                match next {
                    Ok(Some(bytes)) => {
                        if bytes.is_empty() {
                            continue;
                        }
                        total = total.saturating_add(bytes.len() as u64);
                        if total > MAX_WIRE {
                            let _ = outbound
                                .send_json_high(&json!({
                                    "type": "res_abort",
                                    "id": &id,
                                    "error": "响应体过大",
                                }))
                                .await;
                            return;
                        }
                        for piece in bytes.chunks(params.max_chunk_bytes) {
                            if !window
                                .acquire(piece.len(), &mut job_cancel_rx, &mut session_cancel)
                                .await
                            {
                                return;
                            }
                            let frame =
                                encode_bin_frame(WS_BIN_RES_CHUNK, 0, &work.raw_id, piece);
                            if !outbound.send_data(frame).await {
                                return;
                            }
                        }
                    }
                    Ok(None) => {
                        let eof_frame =
                            encode_bin_frame(WS_BIN_RES_CHUNK, WS_BIN_FLAG_EOF, &work.raw_id, &[]);
                        let _ = outbound.send_data(eof_frame).await;
                        return;
                    }
                    Err(_) => {
                        let _ = outbound
                            .send_json_high(&json!({
                                "type": "res_abort",
                                "id": &id,
                                "error": "本地响应中断",
                            }))
                            .await;
                        return;
                    }
                }
            }
        });
    }
}

async fn read_hello_ack(reader: &mut WsReader, writer: &mut WsWriter) -> Option<SessionParams> {
    let deadline = tokio::time::Instant::now() + HELLO_TIMEOUT;
    loop {
        let msg = tokio::time::timeout_at(deadline, reader.read_message(MAX_WS_TEXT_BYTES))
            .await
            .ok()?
            .ok()?;
        match msg {
            WsMessage::Ping(payload) => {
                writer.write_frame(OP_PONG, &payload).await.ok()?;
            }
            WsMessage::Pong => {}
            WsMessage::Text(text) => {
                let ack: WsInboundText = serde_json::from_str(&text).ok()?;
                if ack.kind != "hello_ack" {
                    return None;
                }
                let heartbeat_sec = if ack.heartbeat_sec > 0 {
                    ack.heartbeat_sec.clamp(5, 55)
                } else {
                    DEFAULT_HEARTBEAT_SEC
                };
                let max_chunk_bytes = if ack.max_chunk_bytes > 0 {
                    ack.max_chunk_bytes.clamp(4096, 256 * 1024)
                } else {
                    DEFAULT_MAX_CHUNK_BYTES
                };
                let response_window = if ack.response_window > 0 {
                    ack.response_window.clamp(64 * 1024, 8 * 1024 * 1024)
                } else {
                    DEFAULT_RESPONSE_WINDOW
                };
                let stream_window = if ack.stream_window > 0 {
                    ack.stream_window.clamp(32 * 1024, 2 * 1024 * 1024)
                } else {
                    DEFAULT_STREAM_WINDOW
                };
                return Some(SessionParams {
                    heartbeat_sec,
                    max_chunk_bytes,
                    response_window,
                    stream_window,
                });
            }
            WsMessage::Binary(_) | WsMessage::Close => return None,
        }
    }
}

fn build_async_local_request(
    local_base: &str,
    client: &AsyncClient,
    authority: &str,
    work: &WsWork,
) -> Result<reqwest::RequestBuilder, ()> {
    let method = match work.method.as_str() {
        "GET" => Method::GET,
        "HEAD" => Method::HEAD,
        "POST" => Method::POST,
        _ => return Err(()),
    };
    if !work.path.starts_with("/api/") || work.path.len() > 2048 {
        return Err(());
    }
    let target = Url::parse(&format!("{local_base}{}", work.path)).map_err(|_| ())?;
    if !target.path().starts_with("/api/") {
        return Err(());
    }
    let mut request = client
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
    if !work.body.is_empty() {
        request = request.body(work.body.clone());
    }
    if work.stream {
        request = request.header(header::ACCEPT, "text/event-stream");
    }
    Ok(request)
}

fn encode_bin_frame(kind: u8, flags: u8, raw_id: &[u8; 16], data: &[u8]) -> OutboundFrame {
    let mut payload = Vec::with_capacity(WS_BIN_HEADER_LEN + data.len());
    payload.push(kind);
    payload.push(flags);
    payload.extend_from_slice(raw_id);
    payload.extend_from_slice(data);
    OutboundFrame {
        opcode: OP_BINARY,
        payload,
    }
}

async fn send_unavailable_ws(outbound: &Outbound, id: &str) {
    send_error_response(outbound, id, 502, "Naro local API unavailable").await;
}

async fn send_error_response(outbound: &Outbound, id: &str, status: u16, message: &str) {
    let Some(raw_id) = parse_hex_id(id) else {
        return;
    };
    let body = serde_json::to_vec(&json!({"error": message})).unwrap_or_default();
    if !outbound
        .send_json_high(&json!({
            "type": "res_start",
            "id": id,
            "status": status,
            "content_type": "application/json",
            "eof": false,
        }))
        .await
    {
        return;
    }
    let frame = encode_bin_frame(WS_BIN_RES_CHUNK, WS_BIN_FLAG_EOF, &raw_id, &body);
    let _ = outbound.send_data(frame).await;
}

async fn wait_true(rx: &mut watch::Receiver<bool>) {
    if *rx.borrow() {
        return;
    }
    loop {
        if rx.changed().await.is_err() {
            return;
        }
        if *rx.borrow() {
            return;
        }
    }
}

fn parse_hex_id(id: &str) -> Option<[u8; 16]> {
    if id.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    let bytes = id.as_bytes();
    for i in 0..16 {
        let hi = hex_nibble(bytes[i * 2])?;
        let lo = hex_nibble(bytes[i * 2 + 1])?;
        out[i] = (hi << 4) | lo;
    }
    Some(out)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        extract::State,
        http::StatusCode,
        response::IntoResponse,
        routing::{get, post},
    };
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn backoff_grows_with_jitter_and_caps() {
        let mut backoff = Backoff::new();
        let d0 = backoff.next_delay();
        assert!(d0 >= Duration::from_millis(250) && d0 <= Duration::from_millis(500));
        let d1 = backoff.next_delay();
        assert!(d1 >= Duration::from_millis(500) && d1 <= Duration::from_millis(1000));
        for _ in 0..10 {
            let d = backoff.next_delay();
            assert!(d <= Duration::from_millis(BACKOFF_MAX_MS));
        }
        backoff.reset();
        let reset = backoff.next_delay();
        assert!(reset >= Duration::from_millis(250) && reset <= Duration::from_millis(500));
    }

    #[test]
    fn seen_ids_deduplicates_and_evicts_oldest() {
        let mut seen = SeenIds::new();
        let first = [1u8; 16];
        assert!(seen.insert(first));
        assert!(!seen.insert(first));
        for i in 0..SEEN_ID_CAP {
            let mut id = [0u8; 16];
            id[..8].copy_from_slice(&(i as u64 + 100).to_be_bytes());
            assert!(seen.insert(id));
        }
        assert!(seen.insert(first));
    }

    #[test]
    fn never_uses_legacy_polling_when_relay_lacks_wss() {
        #[derive(Clone, Default)]
        struct OldRelayState {
            ws_hits: Arc<AtomicUsize>,
            hello_hits: Arc<AtomicUsize>,
            poll_hits: Arc<AtomicUsize>,
        }

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let state = OldRelayState::default();
        let app = Router::new()
            .route(
                "/relay/devices/{id}/ws",
                get(|State(s): State<OldRelayState>| async move {
                    s.ws_hits.fetch_add(1, Ordering::AcqRel);
                    StatusCode::METHOD_NOT_ALLOWED
                }),
            )
            .route(
                "/relay/devices/{id}/hello",
                post(|State(s): State<OldRelayState>| async move {
                    s.hello_hits.fetch_add(1, Ordering::AcqRel);
                    StatusCode::NO_CONTENT
                }),
            )
            .route(
                "/relay/devices/{id}/poll",
                post(|State(s): State<OldRelayState>| async move {
                    s.poll_hits.fetch_add(1, Ordering::AcqRel);
                    StatusCode::NO_CONTENT
                }),
            )
            .with_state(state.clone());

        let (addr, shutdown_tx) = rt.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let (tx, rx) = tokio::sync::oneshot::channel::<()>();
            tokio::spawn(async move {
                let _ = axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = rx.await;
                    })
                    .await;
            });
            (addr, tx)
        });

        let remote = Remote::new("http://127.0.0.1:1".into(), None, [7u8; 32]);
        remote
            .connect(&format!("http://{addr}"), "x".repeat(64))
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            rt.block_on(async { tokio::time::sleep(Duration::from_millis(30)).await });
            if state.ws_hits.load(Ordering::Acquire) >= 2 {
                break;
            }
        }
        assert!(state.ws_hits.load(Ordering::Acquire) >= 2);
        assert_eq!(state.hello_hits.load(Ordering::Acquire), 0);
        assert_eq!(state.poll_hits.load(Ordering::Acquire), 0);
        assert!(!remote.status().connected);
        assert!(remote.status().detail.contains("WSS"));

        remote.shutdown();
        let _ = shutdown_tx.send(());
    }

    #[test]
    fn reports_upgrade_blocked_without_falling_back() {
        #[derive(Clone, Default)]
        struct BlockedState {
            ws_hits: Arc<AtomicUsize>,
        }

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let state = BlockedState::default();
        let app = Router::new()
            .route(
                "/relay/devices/{id}/ws",
                get(|State(s): State<BlockedState>| async move {
                    s.ws_hits.fetch_add(1, Ordering::AcqRel);
                    (
                        StatusCode::UPGRADE_REQUIRED,
                        [("X-Naro-Relay-WS", "1"), ("Upgrade", "websocket")],
                    )
                        .into_response()
                }),
            )
            .with_state(state.clone());

        let (addr, shutdown_tx) = rt.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let (tx, rx) = tokio::sync::oneshot::channel::<()>();
            tokio::spawn(async move {
                let _ = axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = rx.await;
                    })
                    .await;
            });
            (addr, tx)
        });

        let remote = Remote::new("http://127.0.0.1:1".into(), None, [9u8; 32]);
        remote
            .connect(&format!("http://{addr}"), "x".repeat(64))
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            rt.block_on(async { tokio::time::sleep(Duration::from_millis(30)).await });
            if state.ws_hits.load(Ordering::Acquire) >= 2 {
                break;
            }
        }
        assert!(state.ws_hits.load(Ordering::Acquire) >= 2);
        assert!(!remote.status().connected);
        assert!(remote.status().detail.contains("WebSocket"));

        remote.shutdown();
        let _ = shutdown_tx.send(());
    }
}
