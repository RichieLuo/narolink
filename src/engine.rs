mod antigravity;
pub mod attachments;
mod claude;
mod codex;
mod process;

use process::Process;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError, SyncSender},
    },
    time::{Duration, Instant, SystemTime},
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Codex,
    Claude,
    Antigravity,
}

impl Provider {
    pub const ALL: [Self; 3] = [Self::Codex, Self::Claude, Self::Antigravity];

    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
            Self::Antigravity => "Antigravity",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Session {
    pub provider: Provider,
    pub id: String,
    pub title: String,
    pub cwd: PathBuf,
    pub updated: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Inventory {
    pub sessions: Vec<Session>,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Model {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, Serialize)]
pub struct Message {
    pub role: Role,
    pub text: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Transcript {
    pub messages: Vec<Message>,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TurnRequest {
    pub provider: Provider,
    #[serde(default)]
    pub session_id: Option<String>,
    pub cwd: PathBuf,
    pub model: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub images: Vec<attachments::ImageUpload>,
}

pub enum Event {
    Session(String),
    Text(String),
    Activity(String),
    Approve {
        title: String,
        details: String,
        answer: SyncSender<bool>,
    },
}

pub struct Turn<'a> {
    pub cancel: &'a AtomicBool,
    pub images: &'a [attachments::ImageAttachment],
    pub emit: &'a dyn Fn(Event),
}

impl Turn<'_> {
    pub fn send(&self, event: Event) {
        (self.emit)(event);
    }

    pub fn approval(&self, title: &str, details: String) -> Result<bool, String> {
        let (answer, response) = mpsc::sync_channel(1);
        self.send(Event::Approve {
            title: short(title, 100),
            details: cut(&details, 32_768).to_owned(),
            answer,
        });
        loop {
            check_cancel(self.cancel)?;
            match response.recv_timeout(Duration::from_millis(100)) {
                Ok(allowed) => return Ok(allowed),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(false),
            }
        }
    }
}

pub fn inventory(provider: Provider) -> Result<Inventory, String> {
    let mut result = match provider {
        Provider::Codex => codex::inventory(),
        Provider::Claude => claude::inventory(),
        Provider::Antigravity => antigravity::inventory(),
    }?;
    result
        .sessions
        .sort_by_key(|session| std::cmp::Reverse(session.updated));
    Ok(result)
}

pub fn transcript(session: &Session) -> Result<Transcript, String> {
    match session.provider {
        Provider::Codex => codex::transcript(session),
        Provider::Claude => claude::transcript(session),
        Provider::Antigravity => antigravity::transcript(session),
    }
}

pub fn models(provider: Provider) -> Result<Vec<Model>, String> {
    match provider {
        Provider::Codex => codex::models(),
        Provider::Claude => claude::models(),
        Provider::Antigravity => antigravity::models(),
    }
}

pub fn run(request: &TurnRequest, turn: &Turn<'_>) -> Result<(), String> {
    match request.provider {
        Provider::Codex => codex::run(request, turn),
        Provider::Claude => claude::run(request, turn),
        Provider::Antigravity => antigravity::run(request, turn),
    }
}

pub fn short(text: &str, limit: usize) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|character| !character.is_control())
        .take(limit)
        .collect()
}

pub fn cut(text: &str, limit: usize) -> &str {
    let mut end = limit.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

pub fn likely_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
}

pub fn current_workspace() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().to_owned())
        .filter(|path| path.is_dir())
        .unwrap_or_else(std::env::temp_dir)
}

pub fn unix_time(time: SystemTime) -> i64 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64)
}

pub fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        return Err("已停止".into());
    }
    Ok(())
}

fn receive(
    process: &Process,
    cancel: &AtomicBool,
    until: Option<Instant>,
) -> Result<Value, String> {
    loop {
        check_cancel(cancel)?;
        if until.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err("等待 CLI 响应超时".into());
        }
        match process.lines.recv_timeout(Duration::from_millis(75)) {
            Ok(frame) => return frame,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err("CLI 已断开".into()),
        }
    }
}

struct Rpc {
    process: Process,
    next: u32,
    version: bool,
}

impl Rpc {
    fn start(provider: Provider, cwd: &Path, args: &[&str], version: bool) -> Result<Self, String> {
        let mut command = process::cli_command(provider)?;
        command.args(args);
        Ok(Self {
            process: Process::spawn(&mut command, cwd)?,
            next: 0,
            version,
        })
    }

    fn send(&self, mut frame: Value) -> Result<(), String> {
        if self.version {
            frame["jsonrpc"] = json!("2.0");
        }
        self.process.send(frame)
    }

    fn request(
        &mut self,
        method: &str,
        params: Value,
        cancel: &AtomicBool,
        after: Option<Duration>,
        mut observe: impl FnMut(&Self, Value) -> Result<(), String>,
    ) -> Result<Value, String> {
        self.next += 1;
        let id = self.next;
        self.send(json!({"id":id,"method":method,"params":params}))?;
        let deadline = after.map(|duration| Instant::now() + duration);
        loop {
            let frame = receive(&self.process, cancel, deadline)?;
            if frame["id"] == id && frame.get("method").is_none() {
                return Ok(frame);
            }
            observe(self, frame)?;
        }
    }

    fn control(
        &mut self,
        method: &str,
        params: Value,
        cancel: &AtomicBool,
    ) -> Result<Value, String> {
        let frame = self.request(
            method,
            params,
            cancel,
            Some(Duration::from_secs(18)),
            Self::reject,
        )?;
        rpc_result(frame, method)
    }

    fn reject(&self, frame: Value) -> Result<(), String> {
        if let Some(id) = frame.get("id")
            && frame.get("method").is_some()
        {
            self.send(
                json!({"id":id,"error":{"code":-32601,"message":"Unsupported client request"}}),
            )?;
        }
        Ok(())
    }
}

fn rpc_result(mut frame: Value, method: &str) -> Result<Value, String> {
    if frame["error"]["code"] == -32000
        && frame["error"]["message"]
            .as_str()
            .is_some_and(|text| text.to_lowercase().contains("auth"))
    {
        return Err("CLI 需要登录。请在电脑终端登录后刷新。".into());
    }
    if frame.get("error").is_some() || frame.get("result").is_none() {
        let detail = short(
            frame["error"]["message"].as_str().unwrap_or("未返回结果"),
            220,
        );
        return Err(format!("{method} · {detail}"));
    }
    Ok(frame["result"].take())
}
