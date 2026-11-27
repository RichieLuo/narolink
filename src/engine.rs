mod antigravity;
pub mod attachments;
mod claude;
mod codex;
mod operations;
mod process;
pub use operations::{SessionAction, occurrences, search, session_action};
pub fn codex_session(id: &str) -> Result<Session, String> {
    codex::read_session(id)
}
pub mod title;
pub mod usage;
pub mod work;

use process::Process;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
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

    pub fn supports_steering(self) -> bool {
        matches!(self, Self::Codex | Self::Claude | Self::Antigravity)
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Quota {
    pub limits: Vec<QuotaLimit>,
    pub credit_balance: Option<String>,
    pub unlimited_credits: bool,
    pub reset_credits: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct QuotaLimit {
    pub name: String,
    pub windows: Vec<QuotaWindow>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct QuotaWindow {
    pub label: String,
    pub remaining_percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_amount: Option<f64>,
    pub resets_at: Option<i64>,
}

pub fn quota(provider: Provider) -> Result<Quota, String> {
    match provider {
        Provider::Codex => codex::quota(),
        Provider::Claude => claude::quota(),
        Provider::Antigravity => antigravity::quota(),
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CodexQuotaResetOutcome {
    Reset,
    AlreadyRedeemed,
    NothingToReset,
    NoCredit,
}

impl CodexQuotaResetOutcome {
    pub fn message(self) -> &'static str {
        match self {
            Self::Reset | Self::AlreadyRedeemed => "Codex 额度已重置。",
            Self::NothingToReset => "当前没有可重置的 Codex 额度。",
            Self::NoCredit => "当前没有可用的重置次数。",
        }
    }
}

pub fn reset_codex_quota(idempotency_key: uuid::Uuid) -> Result<CodexQuotaResetOutcome, String> {
    codex::reset_quota(idempotency_key)
}

fn quota_reset(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| {
            chrono::DateTime::parse_from_rfc3339(value.as_str()?)
                .ok()
                .map(|time| time.timestamp())
        })
        .filter(|timestamp| *timestamp > 0)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Session {
    pub provider: Provider,
    pub id: String,
    pub title: String,
    #[serde(skip)]
    pub named: bool,
    pub cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub updated: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    #[serde(skip)]
    pub rollout: Option<PathBuf>,
}

impl Session {
    pub fn revision(&self) -> Option<String> {
        let metadata = self.rollout.as_ref()?.metadata().ok()?;
        let modified = metadata
            .modified()
            .ok()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?;
        Some(format!("{}-{}", modified.as_nanos(), metadata.len()))
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Inventory {
    pub sessions: Vec<Session>,
    pub warning: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_sessions: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<Project>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub roots: Vec<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Model {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasoning_efforts: Vec<ReasoningChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_reasoning_effort: Option<String>,
    #[serde(default)]
    pub fast_mode: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_fast_mode: Option<bool>,
}

impl Model {
    pub fn reasoning_choice(&self, effort: Option<&str>) -> Option<&ReasoningChoice> {
        let effort = effort.or(self.default_reasoning_effort.as_deref())?;
        self.reasoning_efforts
            .iter()
            .find(|choice| choice.id == effort)
    }

    pub fn speed_choice(&self, fast: Option<bool>) -> Option<bool> {
        fast.or(self.default_fast_mode).filter(|_| self.fast_mode)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReasoningChoice {
    pub id: String,
    pub label: String,
}

impl From<&str> for ReasoningChoice {
    fn from(id: &str) -> Self {
        Self {
            id: id.into(),
            label: id.into(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ModelCatalog {
    pub models: Vec<Model>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub permission_modes: Vec<PermissionChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_permission_mode: Option<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub collaboration_modes: Vec<CollaborationMode>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CollaborationMode {
    Default,
    Plan,
}

impl CollaborationMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "执行",
            Self::Plan => "计划模式",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PermissionChoice {
    pub id: String,
    pub label: String,
    pub description: String,
}

impl PermissionChoice {
    fn new(id: &str, label: &str, description: &str) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: description.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Message {
    pub role: Role,
    pub text: String,
    #[serde(skip)]
    pub attachments: Vec<attachments::UserAttachment>,
    #[serde(
        skip_deserializing,
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "attachments::serialize_local_file_labels"
    )]
    pub local_files: Vec<attachments::LocalAttachment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "work::Work::is_empty")]
    pub work: work::Work,
}

impl Message {
    pub fn new(role: Role, text: String) -> Self {
        let mut message = Self {
            role,
            text,
            attachments: Vec::new(),
            local_files: Vec::new(),
            turn_id: None,
            work: work::Work::default(),
        };
        message.restore_local_files();
        message
    }

    fn restore_local_files(&mut self) {
        if self.role == Role::User
            && let Some((prompt, files)) = attachments::split_local_files(&self.text)
        {
            self.text = prompt.to_owned();
            self.local_files = files;
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Transcript {
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub warning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TurnRequest {
    pub provider: Provider,
    #[serde(default)]
    pub session_id: Option<String>,
    pub cwd: PathBuf,
    #[serde(default)]
    pub additional_directories: Vec<PathBuf>,
    #[serde(default)]
    pub context_files: Vec<PathBuf>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub worktree: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(flatten)]
    pub options: ModelOptions,
    pub prompt: String,
    #[serde(default)]
    pub images: Vec<attachments::ImageUpload>,
    #[serde(skip)]
    pub local_files: Vec<attachments::LocalAttachment>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelOptions {
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub fast_mode: Option<bool>,
    pub permission_mode: Option<String>,
    pub approval_policy: Option<String>,
    pub collaboration_mode: Option<CollaborationMode>,
}

pub struct FollowUpInput {
    pub id: String,
    pub prompt: String,
    pub images: Vec<attachments::ImageAttachment>,
    pub local_files: Vec<attachments::LocalAttachment>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Question {
    pub id: String,
    pub header: String,
    pub text: String,
    pub options: Vec<QuestionOption>,
    pub multiple: bool,
    pub allow_custom: bool,
    pub secret: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: String,
}

impl Question {
    pub fn accepts(&self, answers: &[String]) -> bool {
        if answers.is_empty() || (!self.multiple && answers.len() != 1) {
            return false;
        }
        let mut seen = HashSet::new();
        let mut custom = 0;
        answers.iter().all(|answer| {
            if answer.trim().is_empty() || !seen.insert(answer) {
                return false;
            }
            if self.options.iter().any(|option| &option.label == answer) {
                return true;
            }
            custom += 1;
            self.allow_custom && custom == 1
        })
    }
}

pub type QuestionAnswers = BTreeMap<String, Vec<String>>;
pub type QuestionReply = Box<dyn Fn(QuestionAnswers) -> Result<(), String> + Send>;

pub enum Event {
    Session {
        id: String,
        project_id: Option<String>,
        rollout: Option<PathBuf>,
    },
    Model(String),
    PermissionMode(PermissionChoice),
    Title(String),
    Text(String),
    Answer(String),
    Draft(String),
    Step(work::Step),
    Agent(work::Agent),
    Changes(Vec<work::FileChange>),
    Context(work::ContextUsage),
    Plan(Vec<work::PlanStep>),
    OutputImage(PathBuf),
    Activity(String),
    Steered {
        id: String,
        result: Result<(), String>,
    },
    Approve {
        title: String,
        details: String,
        answer: SyncSender<bool>,
    },
    Ask {
        request_id: Value,
        questions: Vec<Question>,
        reply: QuestionReply,
    },
    ResolveQuestion(Value),
}

pub struct Turn<'a> {
    pub cancel: &'a AtomicBool,
    pub images: &'a [attachments::ImageAttachment],
    pub emit: &'a dyn Fn(Event),
    pub next_steering: &'a dyn Fn() -> Option<FollowUpInput>,
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

    /// Preserve native authorization scopes instead of reducing decisions to a bool.
    fn native_decision(
        &self,
        id: Value,
        title: &str,
        details: String,
        choices: Vec<(String, Value)>,
        reply: impl Fn(Value) -> Result<(), String> + Send + 'static,
    ) -> bool {
        let options = choices
            .iter()
            .map(|(label, _)| QuestionOption {
                label: label.clone(),
                description: String::new(),
            })
            .collect();
        self.ask(
            id,
            vec![Question {
                id: "decision".into(),
                header: title.into(),
                text: format!("{title}\n\n{details}"),
                options,
                multiple: false,
                allow_custom: false,
                secret: false,
            }],
            move |answers| {
                let selected = answers
                    .get("decision")
                    .and_then(|v| v.first())
                    .and_then(|label| choices.iter().find(|(name, _)| name == label))
                    .ok_or("Invalid native decision")?;
                reply(selected.1.clone())
            },
        )
    }

    fn ask(
        &self,
        request_id: Value,
        questions: Vec<Question>,
        reply: impl Fn(QuestionAnswers) -> Result<(), String> + Send + 'static,
    ) -> bool {
        let mut ids = HashSet::new();
        if questions.is_empty()
            || questions.iter().any(|question| {
                question.id.is_empty()
                    || question.text.trim().is_empty()
                    || !ids.insert(&question.id)
                    || (question.options.is_empty() && !question.allow_custom)
                    || question
                        .options
                        .iter()
                        .any(|option| option.label.trim().is_empty())
            })
        {
            return false;
        }
        self.send(Event::Ask {
            request_id,
            questions,
            reply: Box::new(reply),
        });
        true
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

pub fn models(provider: Provider) -> Result<ModelCatalog, String> {
    match provider {
        Provider::Codex => codex::models(),
        Provider::Claude => claude::models(),
        Provider::Antigravity => antigravity::models(),
    }
}

pub fn validate_permission_mode(provider: Provider, mode: Option<&str>) -> Result<(), String> {
    if let Some(mode) = mode {
        let supported = match provider {
            Provider::Codex => codex::supports_permission_mode(mode),
            Provider::Claude => claude::supports_permission_mode(mode),
            Provider::Antigravity => {
                !mode.is_empty() && mode.len() <= 128 && !mode.chars().any(char::is_control)
            }
        };
        if !supported {
            return Err("此引擎不支持所选权限模式".into());
        }
    }
    Ok(())
}

pub fn validate_permissions(provider: Provider, options: &ModelOptions) -> Result<(), String> {
    validate_permission_mode(provider, options.permission_mode.as_deref())?;
    if let Some(policy) = options.approval_policy.as_deref()
        && (provider != Provider::Codex || !codex::supports_approval_policy(policy))
    {
        return Err("此引擎不支持所选审批策略".into());
    }
    Ok(())
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

fn reported_model(value: &Value) -> Option<&str> {
    value.as_str().filter(|id| {
        !id.is_empty()
            && id.len() <= 256
            && !id.starts_with('-')
            && !id.chars().any(char::is_control)
            && !id.eq_ignore_ascii_case("default")
            && *id != "<synthetic>"
    })
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

    fn ask(
        &self,
        turn: &Turn<'_>,
        id: &Value,
        questions: Vec<Question>,
        result: impl Fn(QuestionAnswers) -> Value + Send + 'static,
    ) -> Result<(), String> {
        let request_id = id.clone();
        let send = self.process.writer.clone();
        let version = self.version;
        if turn.ask(id.clone(), questions, move |answers| {
            let mut frame = json!({"id":request_id,"result":result(answers)});
            if version {
                frame["jsonrpc"] = json!("2.0");
            }
            Process::send_to(&send, frame)
        }) {
            Ok(())
        } else {
            self.send(json!({"id":id,"error":{"code":-32602,"message":"Invalid questions"}}))
        }
    }

    fn request(
        &mut self,
        method: &str,
        params: Value,
        cancel: &AtomicBool,
        after: Option<Duration>,
        mut observe: impl FnMut(&Self, Value) -> Result<(), String>,
    ) -> Result<Value, String> {
        let id = self.submit(method, params)?;
        let deadline = after.map(|duration| Instant::now() + duration);
        loop {
            let frame = receive(&self.process, cancel, deadline).map_err(|error| {
                if cancel.load(Ordering::Acquire) {
                    error
                } else {
                    format!("{method} · {error}")
                }
            })?;
            if frame["id"] == id && frame.get("method").is_none() {
                return Ok(frame);
            }
            observe(self, frame)?;
        }
    }

    fn submit(&mut self, method: &str, params: Value) -> Result<u32, String> {
        self.next += 1;
        self.send(json!({"id":self.next,"method":method,"params":params}))?;
        Ok(self.next)
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
