mod permissions;
#[cfg(test)]
mod tests;
mod titles;

pub use permissions::PermissionTarget;

use crate::engine::attachments::{self, ImageAttachment};
use crate::engine::usage::{UsageCache, UsageReport};
use crate::engine::work::{StepKind, StepStatus, Work};
use crate::engine::{
    self, Event, FollowUpInput, Inventory, ModelCatalog, ModelOptions, PermissionChoice, Provider,
    Question, QuestionAnswers, QuestionReply, Role, Session, Transcript, Turn, TurnRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque},
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::SyncSender,
    },
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::watch;

type SessionKey = (Provider, String);
type Timed<T> = (Instant, T);
const MAX_FOLLOW_UPS: usize = 16;
const MAX_FOLLOW_UP_TEXT: usize = 1024 * 1024;
const MAX_RUN_IMAGES: usize = 2 * attachments::MAX_TOTAL_IMAGE_BYTES;

#[derive(Default)]
struct Catalog<T> {
    generation: u64,
    entries: HashMap<Provider, Timed<Result<T, String>>>,
}

impl<T> Catalog<T> {
    fn invalidate(&mut self) {
        self.generation += 1;
        self.entries.clear();
    }
}

#[derive(Clone, Default)]
pub struct Service(Arc<Shared>);

#[derive(Default)]
struct Shared {
    permissions: Mutex<Option<rusqlite::Connection>>,
    inventory: Mutex<Catalog<Inventory>>,
    models: Mutex<Catalog<ModelCatalog>>,
    recent_sessions: Mutex<HashMap<SessionKey, Session>>,
    transcripts: Mutex<HashMap<SessionKey, Timed<Transcript>>>,
    runs: Mutex<HashMap<String, Arc<Run>>>,
    token_usage: Mutex<UsageCache>,
    title_revision: AtomicU64,
}

pub struct Run {
    cwd: PathBuf,
    view: Mutex<State>,
    cancel: AtomicBool,
    revision: watch::Sender<RunRevision>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct RunRevision {
    sequence: u64,
    pub structure: u64,
}

struct RunEdit<'a> {
    state: MutexGuard<'a, State>,
    revision: &'a watch::Sender<RunRevision>,
    text_only: bool,
    changed: bool,
}

impl Deref for RunEdit<'_> {
    type Target = State;

    fn deref(&self) -> &State {
        &self.state
    }
}

impl DerefMut for RunEdit<'_> {
    fn deref_mut(&mut self) -> &mut State {
        self.changed = true;
        &mut self.state
    }
}

impl Drop for RunEdit<'_> {
    fn drop(&mut self) {
        if self.changed {
            self.revision.send_modify(|revision| {
                revision.sequence += 1;
                revision.structure += u64::from(!self.text_only);
            });
        }
    }
}

struct State {
    info: Snapshot,
    composer_permission: Option<String>,
    images: HashMap<usize, ImageAttachment>,
    next_image_index: usize,
    sent: usize, // Delivered entries form a prefix in conversation order.
    output_bytes: usize,
    approval: Option<SyncSender<bool>>,
    questions: VecDeque<PendingQuestion>,
    finished: Option<Instant>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Running,
    Complete,
    Stopped,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SendMode {
    #[default]
    Queue,
    Steer,
}

impl SendMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Queue => "加入队列",
            Self::Steer => "引导会话",
        }
    }
}

#[derive(Deserialize, Serialize)]
pub struct FollowUpRequest {
    pub mode: SendMode,
    pub prompt: String,
    pub options: Option<ModelOptions>,
    #[serde(default)]
    pub images: Vec<attachments::ImageUpload>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Delivery {
    Pending,
    Sent,
    Unsent,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FollowUp {
    pub id: String,
    pub mode: SendMode,
    pub delivery: Delivery,
    pub prompt: String,
    pub images: Vec<String>,
    #[serde(
        skip_deserializing,
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "attachments::serialize_local_file_labels"
    )]
    pub local_files: Vec<attachments::LocalAttachment>,
    pub image_start: usize,
    pub output: String,
    #[serde(skip)]
    pub work: Work,
    #[serde(skip)]
    options: Option<ModelOptions>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Snapshot {
    #[serde(flatten)]
    pub run: RunSummary,
    pub status: Status,
    pub prompt: String,
    pub output: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub draft: String,
    #[serde(skip)]
    pub work: Work,
    pub images: Vec<String>,
    #[serde(
        skip_deserializing,
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "attachments::serialize_local_file_labels"
    )]
    pub local_files: Vec<attachments::LocalAttachment>,
    pub activity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_permission_mode: Option<PermissionChoice>,
    pub error: Option<String>,
    pub approval: Option<Approval>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<QuestionRequest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub follow_ups: Vec<FollowUp>,
}

pub struct RunReply<'a> {
    pub after: Option<&'a str>,
    pub text: Cow<'a, str>,
    pub work: &'a Work,
}

impl RunReply<'_> {
    fn append_text(&mut self, extra: &str) {
        if extra.is_empty() {
            return;
        }
        let text = self.text.to_mut();
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(extra);
    }
}

impl Snapshot {
    pub fn replies(&self) -> Vec<RunReply<'_>> {
        let mut current = RunReply {
            after: None,
            text: Cow::Borrowed(&self.output),
            work: &self.work,
        };
        let mut replies = Vec::new();
        for entry in self
            .follow_ups
            .iter()
            .filter(|entry| entry.delivery == Delivery::Sent)
        {
            replies.push(current);
            current = RunReply {
                after: Some(&entry.id),
                text: Cow::Borrowed(&entry.output),
                work: &entry.work,
            };
        }
        current.append_text(&self.draft);
        replies.push(current);
        replies
    }

    pub fn appears_in(&self, history: &Transcript, previous_count: usize) -> bool {
        if self
            .follow_ups
            .iter()
            .any(|entry| entry.delivery != Delivery::Sent)
        {
            return false;
        }
        let messages = history.messages.get(previous_count..).unwrap_or_default();
        let mut users = messages.iter().filter(|entry| entry.role == Role::User);
        let prompts_found = std::iter::once(self.prompt.as_str())
            .chain(self.follow_ups.iter().map(|entry| entry.prompt.as_str()))
            .all(|prompt| users.any(|entry| entry.text.trim() == prompt));
        let replies = self.replies();
        let recorded_agents = messages.iter().flat_map(|entry| &entry.work.agents);
        if !prompts_found
            || replies
                .iter()
                .flat_map(|reply| &reply.work.agents)
                .any(|agent| {
                    !recorded_agents.clone().any(|recorded| {
                        recorded.id == agent.id
                            && (agent.status.is_none()
                                || self.status == Status::Stopped
                                || recorded.status == agent.status)
                    })
                })
        {
            return false;
        }
        if self.run.provider != Provider::Codex {
            return true;
        }
        let mut recorded_replies = HashMap::<&str, String>::new();
        for entry in messages
            .iter()
            .filter(|entry| entry.role == Role::Assistant && !entry.text.is_empty())
        {
            if let Some(turn) = entry.turn_id.as_deref() {
                let text = recorded_replies.entry(turn).or_default();
                if !text.is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str(&entry.text);
            }
        }
        replies.iter().all(|reply| {
            (reply.text.is_empty()
                || recorded_replies
                    .values()
                    .any(|text| text.contains(reply.text.as_ref()))
                || messages.iter().any(|entry| {
                    entry.role == Role::Assistant
                        && entry.turn_id.is_none()
                        && entry.text.contains(reply.text.as_ref())
                }))
                && reply
                    .work
                    .steps
                    .iter()
                    .filter(|step| step.kind == StepKind::Note)
                    .all(|note| {
                        messages
                            .iter()
                            .flat_map(|entry| &entry.work.steps)
                            .any(|step| step.kind == StepKind::Note && step.text == note.text)
                    })
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunSummary {
    pub id: String,
    pub provider: Provider,
    pub session_id: Option<String>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub struct Activity {
    pub running: Vec<RunSummary>,
    pub finished: Vec<RunSummary>,
    pub title_revision: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Approval {
    pub id: String,
    pub title: String,
    pub details: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct QuestionRequest {
    pub id: String,
    pub questions: Vec<Question>,
}

struct PendingQuestion {
    request_id: Value,
    display: QuestionRequest,
    reply: QuestionReply,
}

impl Service {
    pub fn token_usage(&self) -> UsageReport {
        self.0.token_usage.lock().unwrap().report()
    }

    pub fn invalidate_cli_catalogs(&self) {
        self.0.inventory.lock().unwrap().invalidate();
        self.0.models.lock().unwrap().invalidate();
    }

    pub fn inventory(&self, provider: Provider, refresh: bool) -> Result<Inventory, String> {
        let mut list = cached(
            &self.0.inventory,
            provider,
            refresh,
            Duration::from_secs(20),
            engine::inventory,
        )?;
        let saved_titles = titles::for_provider(provider);
        let recent = self.0.recent_sessions.lock().unwrap();
        for session in &mut list.sessions {
            if !session.named
                && let Some(title) = recent
                    .get(&(provider, session.id.clone()))
                    .filter(|recent| recent.named)
                    .map(|recent| &recent.title)
                    .or_else(|| saved_titles.get(&session.id))
            {
                session.title.clone_from(title);
                session.named = true;
            }
        }
        for session in recent
            .values()
            .filter(|session| session.provider == provider)
        {
            if !list
                .sessions
                .iter()
                .any(|existing| existing.id == session.id)
            {
                list.sessions.push(session.clone());
            }
        }
        list.sessions
            .sort_by_key(|session| std::cmp::Reverse(session.updated));
        Ok(list)
    }

    pub fn models(&self, provider: Provider, refresh: bool) -> Result<ModelCatalog, String> {
        cached(
            &self.0.models,
            provider,
            refresh,
            Duration::from_secs(90),
            engine::models,
        )
    }

    pub fn session(&self, provider: Provider, id: &str) -> Result<Session, String> {
        if !engine::likely_id(id) {
            return Err("会话 ID 无效".into());
        }
        let key = (provider, id.to_owned());
        let recent = self.0.recent_sessions.lock().unwrap().get(&key).cloned();
        if let Some(session) = &recent
            && (provider != Provider::Codex || session.rollout.is_some())
        {
            let mut session = session.clone();
            if let Some(title) = self.native_title(provider, id) {
                session.title = title;
                session.named = true;
            }
            return Ok(session);
        }
        for refresh in [false, true] {
            let inventory = match self.inventory(provider, refresh) {
                Ok(inventory) => inventory,
                Err(error) => return recent.ok_or(error),
            };
            if let Some(session) = inventory.sessions.into_iter().find(|item| item.id == id)
                && (session.rollout.is_some() || recent.is_none() || refresh)
            {
                return Ok(session);
            }
        }
        if provider == Provider::Codex && recent.is_none() {
            return engine::codex_session(id);
        }
        recent.ok_or_else(|| "找不到这个原始会话，请刷新历史".into())
    }

    fn native_title(&self, provider: Provider, id: &str) -> Option<String> {
        let inventory = self.0.inventory.lock().unwrap();
        let (_, catalog) = inventory.entries.get(&provider)?;
        catalog
            .as_ref()
            .ok()?
            .sessions
            .iter()
            .find(|session| session.id == id && session.named)
            .map(|session| session.title.clone())
    }

    pub fn transcript(&self, session: &Session, refresh: bool) -> Result<Transcript, String> {
        let key = (session.provider, session.id.clone());
        let revision = session.revision();
        let active = self.running().iter().any(|turn| {
            turn.provider == session.provider && turn.session_id.as_deref() == Some(&session.id)
        });
        {
            let cache = self.0.transcripts.lock().unwrap();
            if let Some((at, transcript)) = cache.get(&key)
                && (active
                    || (!refresh
                        && transcript.revision == revision
                        && (revision.is_some() || at.elapsed() < Duration::from_secs(10))))
            {
                return Ok(transcript.clone());
            }
        }
        if active {
            return Ok(Transcript::default());
        }
        let mut transcript = engine::transcript(session)?;
        transcript.revision = revision;
        let mut cache = self.0.transcripts.lock().unwrap();
        if cache.len() >= 12
            && let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(key, _)| key.clone())
        {
            cache.remove(&oldest);
        }
        cache.insert(key, (Instant::now(), transcript.clone()));
        Ok(transcript)
    }

    pub fn session_action(
        &self,
        session: &Session,
        action: &engine::SessionAction,
    ) -> Result<Value, String> {
        if self.running().iter().any(|run| {
            run.provider == session.provider && run.session_id.as_deref() == Some(&session.id)
        }) {
            return Err("Wait for the running turn before changing the native session".into());
        }
        if session.provider == Provider::Claude
            && let Ok(active) = engine::inventory(Provider::Claude)
            && active
                .native_sessions
                .iter()
                .any(|row| row["sessionId"] == session.id && row["status"] == "busy")
        {
            return Err("Claude Code is executing this session in another client".into());
        }
        let result = engine::session_action(session, action)?;
        self.0.inventory.lock().unwrap().invalidate();
        self.0
            .transcripts
            .lock()
            .unwrap()
            .remove(&(session.provider, session.id.clone()));
        self.0
            .recent_sessions
            .lock()
            .unwrap()
            .remove(&(session.provider, session.id.clone()));
        Ok(result)
    }

    pub fn start(&self, mut request: TurnRequest) -> Result<RunSummary, String> {
        let images = attachments::decode_uploads(std::mem::take(&mut request.images))
            .map_err(|error| error.to_string())?;
        self.start_with_images(request, images)
    }

    pub fn start_with_images(
        &self,
        mut request: TurnRequest,
        images: Vec<ImageAttachment>,
    ) -> Result<RunSummary, String> {
        validate_message(&mut request.prompt, &images, &request.local_files)?;
        if request.additional_directories.len() > 16 || request.context_files.len() > 16 {
            return Err("Too many context paths".into());
        }
        for path in &request.additional_directories {
            if !path.is_absolute() || !path.is_dir() {
                return Err("Additional directory must exist and be absolute".into());
            }
        }
        for path in &request.context_files {
            if !path.is_absolute()
                || !path.is_file()
                || path.metadata().map_or(true, |m| m.len() > 1024 * 1024)
            {
                return Err(
                    "Context file must be an existing absolute file no larger than 1 MiB".into(),
                );
            }
        }
        if request.agent.as_deref().is_some_and(|s| {
            s.is_empty() || s.len() > 128 || s.starts_with('-') || s.chars().any(char::is_control)
        }) {
            return Err("Invalid agent name".into());
        }
        if let Some(name) = request.worktree.as_deref()
            && (request.provider != Provider::Claude
                || request.session_id.is_some()
                || !engine::likely_id(name))
        {
            return Err(
                "A worktree requires a new Claude Code session and a simple worktree name".into(),
            );
        }
        if request.provider != Provider::Antigravity {
            for path in std::mem::take(&mut request.context_files) {
                request.local_files.push(attachments::LocalAttachment {
                    path: path.to_string_lossy().into_owned(),
                    directory: false,
                });
            }
        }
        if request.provider == Provider::Codex && request.agent.is_some() {
            return Err("Codex does not expose a session agent selector in this protocol".into());
        }
        let prompt = request.prompt.trim();
        validate_model_options(request.provider, &request.options)?;
        let original = request
            .session_id
            .as_deref()
            .map(|id| self.session(request.provider, id))
            .transpose()?;
        if request
            .project_id
            .as_deref()
            .is_some_and(|id| request.provider != Provider::Codex || !engine::likely_id(id))
        {
            return Err("项目无效，请重新选择项目".into());
        }
        if request.provider == Provider::Claude
            && let Some(id) = request.session_id.as_deref()
            && let Ok(active) = engine::inventory(Provider::Claude)
            && active
                .native_sessions
                .iter()
                .any(|row| row["sessionId"] == id && row["status"] == "busy")
        {
            return Err("Claude Code is executing this session in another client. Naro can continue a new turn when it is idle; it cannot take over the running turn.".into());
        }
        if let Some(session) = &original {
            request.cwd.clone_from(&session.cwd);
            request.project_id.clone_from(&session.project_id);
        }
        if !request.cwd.is_absolute() || !request.cwd.is_dir() {
            return Err("在电脑上选择一个存在的工作目录".into());
        }
        let summary = RunSummary {
            id: uuid::Uuid::new_v4().to_string(),
            provider: request.provider,
            session_id: request.session_id.clone(),
            model: request
                .options
                .model
                .clone()
                .filter(|id| !id.eq_ignore_ascii_case("default")),
            title: original
                .as_ref()
                .map(|session| session.title.clone())
                .unwrap_or_else(|| engine::short(prompt, 120)),
        };
        let run = Arc::new(Run {
            cwd: request.cwd.clone(),
            view: Mutex::new(State {
                composer_permission: request.options.permission_mode.clone(),
                info: Snapshot {
                    run: summary.clone(),
                    status: Status::Running,
                    prompt: prompt.to_owned(),
                    output: String::new(),
                    draft: String::new(),
                    work: Work::default(),
                    images: image_names(&images),
                    local_files: request.local_files.clone(),
                    activity: "正在连接 CLI".into(),
                    active_permission_mode: None,
                    error: None,
                    approval: None,
                    question: None,
                    follow_ups: Vec::new(),
                },
                images: images.iter().cloned().enumerate().collect(),
                next_image_index: images.len(),
                sent: 0,
                output_bytes: 0,
                approval: None,
                questions: VecDeque::new(),
                finished: None,
            }),
            cancel: AtomicBool::new(false),
            revision: watch::Sender::new(RunRevision::default()),
        });
        {
            let mut runs = self.0.runs.lock().unwrap();
            runs.retain(|_, run| {
                run.view
                    .lock()
                    .unwrap()
                    .finished
                    .is_none_or(|at| at.elapsed() < Duration::from_secs(300))
            });
            let active = runs
                .values()
                .filter_map(|run| {
                    let state = run.view.lock().unwrap();
                    (state.info.status == Status::Running).then(|| state.info.run.clone())
                })
                .collect::<Vec<_>>();
            if summary.session_id.is_some()
                && active.iter().any(|running| {
                    running.provider == summary.provider && running.session_id == summary.session_id
                })
            {
                return Err("此会话正在另一窗口或设备运行".into());
            }
            if active.len() >= 6 || runs.len() >= 100 {
                return Err("运行任务过多，请稍后重试".into());
            }
            runs.insert(summary.id.clone(), run.clone());
        }
        let service = self.clone();
        let start = std::thread::Builder::new()
            .name(format!("narolink-{}", request.provider.label()))
            .spawn(move || service.run_turns(&run, request, images));
        if start.is_err() {
            self.0.runs.lock().unwrap().remove(&summary.id);
            return Err("无法启动 CLI 后台任务".into());
        }
        Ok(summary)
    }

    fn run_turns(
        &self,
        run: &Arc<Run>,
        mut request: TurnRequest,
        mut images: Vec<ImageAttachment>,
    ) {
        loop {
            let event = |event| match event {
                Event::Session {
                    id,
                    project_id,
                    rollout,
                } => {
                    let native_cwd =
                        if request.provider == Provider::Claude && request.worktree.is_some() {
                            engine::inventory(Provider::Claude)
                                .ok()
                                .and_then(|inventory| {
                                    inventory.sessions.into_iter().find(|s| s.id == id)
                                })
                                .map(|s| s.cwd)
                        } else {
                            None
                        };
                    let native_title = self.native_title(request.provider, &id);
                    let key = (request.provider, id.clone());
                    let mut state = run.edit(false);
                    let first = state.info.run.session_id.is_none();
                    if first
                        && let Some(mode) = &state.composer_permission
                        && let Err(problem) =
                            self.save_initial_permission(request.provider, &id, mode)
                    {
                        eprintln!("Naro: {problem}");
                    }
                    state.info.run.session_id = Some(id.clone());
                    let mut recent = self.0.recent_sessions.lock().unwrap();
                    let name = native_title.as_ref().or_else(|| {
                        recent
                            .get(&key)
                            .filter(|session| session.named)
                            .map(|session| &session.title)
                    });
                    if let Some(name) = name {
                        state.info.run.title.clone_from(name);
                    }
                    let session = Session {
                        provider: request.provider,
                        id: id.clone(),
                        cwd: native_cwd.unwrap_or_else(|| request.cwd.clone()),
                        project_id,
                        model: state.info.run.model.clone(),
                        title: state.info.run.title.clone(),
                        named: name.is_some(),
                        updated: engine::unix_time(SystemTime::now()),
                        preview: Some(engine::short(&request.prompt, 300)),
                        rollout,
                    };
                    recent.insert(key, session);
                    let model = state.info.run.model.clone();
                    drop(recent);
                    drop(state);
                    if first && request.provider != Provider::Antigravity {
                        self.generate_title(
                            run.clone(),
                            request.provider,
                            id,
                            model,
                            &request.prompt,
                        );
                    }
                }
                Event::Model(model) => {
                    let mut state = run.edit(false);
                    if state.info.run.model.as_ref() != Some(&model) {
                        state.info.run.model = Some(model.clone());
                        if let Some(id) = &state.info.run.session_id
                            && let Some(session) = self
                                .0
                                .recent_sessions
                                .lock()
                                .unwrap()
                                .get_mut(&(request.provider, id.clone()))
                        {
                            session.model = Some(model);
                        }
                    }
                }
                Event::PermissionMode(mode) => {
                    let mut state = run.edit(false);
                    if state
                        .info
                        .active_permission_mode
                        .as_ref()
                        .is_none_or(|choice| choice.id != mode.id)
                    {
                        state.info.active_permission_mode = Some(mode);
                    }
                }
                Event::Title(title) => {
                    let id = run.view.lock().unwrap().info.run.session_id.clone();
                    if let Some(id) = id {
                        self.publish_title(request.provider, id, title);
                    }
                }
                Event::Text(text) => {
                    let mut state = run.edit(true);
                    state.append_text(&text);
                }
                Event::Answer(text) => {
                    let mut state = run.edit(false);
                    state.info.draft.clear();
                    if !text.is_empty() {
                        if !state.output().is_empty() {
                            state.append_text("\n\n");
                        }
                        state.append_text(&text);
                    }
                }
                Event::Draft(text) => {
                    let mut state = run.edit(true);
                    let limit = (4 * 1024 * 1024usize)
                        .saturating_sub(state.output_bytes + state.info.draft.len());
                    state.info.draft.push_str(engine::cut(&text, limit));
                    if !state.info.draft.is_empty() {
                        state.info.activity = "正在回复".into();
                    }
                }
                Event::Step(mut step) => {
                    let mut state = run.edit(false);
                    step.after = state
                        .info
                        .follow_ups
                        .get(state.sent.wrapping_sub(1))
                        .map(|entry| entry.id.clone());
                    step.text_offset = Some(state.output().chars().count());
                    let work = state.work_for_step(&step.id);
                    work.record(step);
                    state.info.activity = work
                        .steps
                        .iter()
                        .rev()
                        .find(|step| step.status == StepStatus::Running)
                        .map_or("正在回复", |step| step.kind.activity())
                        .into();
                }
                Event::Agent(agent) => {
                    let mut state = run.edit(false);
                    state
                        .work_for(|work| work.agents.iter().any(|entry| entry.id == agent.id))
                        .record_agent(agent);
                }
                Event::Changes(changes) => run.edit(false).work().add_changes(changes),
                Event::Context(context) => run.edit(false).work().context = Some(context),
                Event::Plan(plan) => run.edit(false).work().plan = plan,
                Event::OutputImage(path) => run.edit(false).work().add_image(path),
                Event::Activity(label) => run.edit(false).info.activity = label,
                Event::Steered { id, result } => {
                    let mut state = run.edit(false);
                    if let Some(index) = state
                        .info
                        .follow_ups
                        .iter()
                        .position(|entry| entry.id == id)
                    {
                        match result {
                            Ok(()) => state.mark_sent(index),
                            Err(error) => state.mark_unsent(index, error),
                        }
                    }
                }
                Event::Approve {
                    title,
                    details,
                    answer,
                } => {
                    let mut state = run.edit(false);
                    state.info.approval = Some(Approval {
                        id: uuid::Uuid::new_v4().to_string(),
                        title,
                        details,
                    });
                    state.approval = Some(answer);
                }
                Event::Ask {
                    request_id,
                    questions,
                    reply,
                } => {
                    let mut state = run.edit(false);
                    if !state
                        .questions
                        .iter()
                        .any(|item| item.request_id == request_id)
                    {
                        state.questions.push_back(PendingQuestion {
                            request_id,
                            display: QuestionRequest {
                                id: uuid::Uuid::new_v4().to_string(),
                                questions,
                            },
                            reply,
                        });
                    }
                }
                Event::ResolveQuestion(request_id) => {
                    run.edit(false)
                        .questions
                        .retain(|item| item.request_id != request_id);
                }
            };
            let next_steering = || {
                let state = run.view.lock().unwrap();
                state
                    .pending(SendMode::Steer)
                    .map(|index| state.input(index))
            };
            let turn = Turn {
                cancel: &run.cancel,
                images: &images,
                emit: &event,
                next_steering: &next_steering,
            };
            let result =
                engine::check_cancel(turn.cancel).and_then(|()| engine::run(&request, &turn));
            let mut state = run.edit(false);
            let cancelled = run.cancel.load(Ordering::Acquire);
            state.info.work.finish(cancelled);
            for entry in &mut state.info.follow_ups {
                entry.work.finish(cancelled);
            }
            state.clear_approval();
            state.questions.clear();
            state.close_steering();
            if result.is_ok()
                && !cancelled
                && let Some(index) = state.pending(SendMode::Queue)
                && let Some(id) = state.info.run.session_id.clone()
            {
                let next = state.input(index);
                if let Some(options) = state.info.follow_ups[index].options.take() {
                    request.options = options;
                }
                state.info.run.model = request
                    .options
                    .model
                    .clone()
                    .filter(|id| !id.eq_ignore_ascii_case("default"));
                state.mark_sent(index);
                state.info.activity = "正在发送队列中的消息".into();
                state.info.active_permission_mode = None;
                request.session_id = Some(id);
                request.prompt = next.prompt;
                request.local_files = next.local_files;
                images = next.images;
                continue;
            }
            if let Some(id) = &state.info.run.session_id {
                let key = (request.provider, id.clone());
                // Keep the last readable projection until the CLI has published its new history.
                if let Some((at, _)) = self.0.transcripts.lock().unwrap().get_mut(&key) {
                    *at = Instant::now()
                        .checked_sub(Duration::from_secs(20))
                        .unwrap_or_else(Instant::now);
                }
            }
            self.0
                .inventory
                .lock()
                .unwrap()
                .entries
                .remove(&request.provider);
            state.finish(result, cancelled);
            break;
        }
    }

    fn generate_title(
        &self,
        run: Arc<Run>,
        provider: Provider,
        id: String,
        model: Option<String>,
        prompt: &str,
    ) {
        let prompt = engine::title::prompt(prompt);
        let service = self.clone();
        let _ = std::thread::Builder::new()
            .name("narolink-session-title".into())
            .spawn(move || {
                if let Some(title) =
                    engine::title::generate(provider, model.as_deref(), &prompt, &run.cancel)
                    && !run.cancel.load(Ordering::Acquire)
                {
                    let title = service
                        .native_title(provider, &id)
                        .unwrap_or_else(|| titles::save(provider, &id, &title).unwrap_or(title));
                    service.publish_title(provider, id, title);
                }
            });
    }

    fn publish_title(&self, provider: Provider, id: String, title: String) {
        let key = (provider, id);
        {
            let mut recent = self.0.recent_sessions.lock().unwrap();
            let Some(session) = recent.get_mut(&key) else {
                return;
            };
            if session.named && session.title == title {
                return;
            }
            session.title.clone_from(&title);
            session.named = true;
        }
        let runs = self.0.runs.lock().unwrap();
        for run in runs.values() {
            let mut state = run.edit(false);
            let summary = &state.info.run;
            if summary.provider == provider && summary.session_id.as_ref() == Some(&key.1) {
                state.info.run.title.clone_from(&title);
            }
        }
        self.0.title_revision.fetch_add(1, Ordering::Release);
    }

    pub fn activity(&self) -> Activity {
        let runs = self.0.runs.lock().unwrap();
        let mut matches = Activity {
            running: Vec::new(),
            finished: Vec::new(),
            title_revision: self.0.title_revision.load(Ordering::Acquire),
        };
        let mut finished = Vec::new();
        for run in runs.values() {
            let state = run.view.lock().unwrap();
            if let Some(ended) = state.finished {
                if ended.elapsed() < Duration::from_secs(300) {
                    finished.push((ended, state.info.run.clone()));
                }
            } else {
                matches.running.push(state.info.run.clone());
            }
        }
        finished.sort_by_key(|(time, _)| std::cmp::Reverse(*time));
        matches.finished = finished.into_iter().take(25).map(|(_, run)| run).collect();
        matches
    }
    pub fn running(&self) -> Vec<RunSummary> {
        self.0
            .runs
            .lock()
            .unwrap()
            .values()
            .filter_map(|run| {
                let state = run.view.lock().unwrap();
                (state.info.status == Status::Running).then(|| state.info.run.clone())
            })
            .collect()
    }

    pub fn run(&self, id: &str) -> Result<Arc<Run>, String> {
        self.0
            .runs
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| "任务已经结束或不存在".into())
    }

    pub fn shutdown(&self) {
        for run in self.0.runs.lock().unwrap().values() {
            run.stop();
        }
    }
}

fn cached<T: Clone>(
    cache: &Mutex<Catalog<T>>,
    provider: Provider,
    refresh: bool,
    ttl: Duration,
    read: impl FnOnce(Provider) -> Result<T, String>,
) -> Result<T, String> {
    let generation = {
        let cache = cache.lock().unwrap();
        if !refresh
            && let Some((at, value)) = cache.entries.get(&provider)
            && at.elapsed()
                < if value.is_ok() {
                    ttl
                } else {
                    Duration::from_secs(3)
                }
        {
            return value.clone();
        }
        cache.generation
    };
    let value = read(provider);
    let mut cache = cache.lock().unwrap();
    if generation == cache.generation {
        cache
            .entries
            .insert(provider, (Instant::now(), value.clone()));
    }
    value
}

impl Run {
    fn edit(&self, text_only: bool) -> RunEdit<'_> {
        RunEdit {
            state: self.view.lock().unwrap(),
            revision: &self.revision,
            text_only,
            changed: false,
        }
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn image(&self, index: usize) -> Option<ImageAttachment> {
        self.view.lock().unwrap().images.get(&index).cloned()
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot_from(&self.view.lock().unwrap())
    }

    pub fn subscribe(&self) -> watch::Receiver<RunRevision> {
        self.revision.subscribe()
    }

    pub fn stream_snapshot(&self) -> (RunRevision, Snapshot) {
        let state = self.view.lock().unwrap();
        (*self.revision.borrow(), self.snapshot_from(&state))
    }

    fn snapshot_from(&self, state: &State) -> Snapshot {
        let mut snapshot = state.info.clone();
        if !self.cancel.load(Ordering::Acquire) {
            snapshot.question = state.questions.front().map(|item| item.display.clone());
        }
        snapshot
    }

    pub fn follow_up(&self, request: FollowUpRequest) -> Result<(), String> {
        let images =
            attachments::decode_uploads(request.images).map_err(|error| error.to_string())?;
        self.follow_up_with_attachments(
            request.mode,
            request.prompt,
            images,
            Vec::new(),
            request.options,
        )
    }

    pub fn follow_up_with_attachments(
        &self,
        mode: SendMode,
        mut prompt: String,
        images: Vec<ImageAttachment>,
        local_files: Vec<attachments::LocalAttachment>,
        options: Option<ModelOptions>,
    ) -> Result<(), String> {
        validate_message(&mut prompt, &images, &local_files)?;
        let options = options.filter(|_| mode == SendMode::Queue);
        let mut state = self.edit(false);
        if let Some(options) = &options {
            validate_model_options(state.info.run.provider, options)?;
        }
        let prompt = prompt.trim();
        if state.info.status != Status::Running || self.cancel.load(Ordering::Acquire) {
            return Err("当前回复已结束或正在停止，请再次发送。".into());
        }
        if mode == SendMode::Steer && !state.info.run.provider.supports_steering() {
            return Err("此引擎支持加入队列，请切换发送方式。".into());
        }
        let entries = &state.info.follow_ups;
        let text_bytes: usize = entries
            .iter()
            .map(|entry| attachments::with_local_files(&entry.prompt, &entry.local_files).len())
            .sum();
        let image_bytes: usize = state
            .images
            .values()
            .chain(&images)
            .map(ImageAttachment::byte_len)
            .sum();
        if entries.len() >= MAX_FOLLOW_UPS
            || text_bytes + attachments::with_local_files(prompt, &local_files).len()
                > MAX_FOLLOW_UP_TEXT
            || image_bytes > MAX_RUN_IMAGES
        {
            return Err("本次运行追加的消息或图片已达上限。".into());
        }
        let image_start = state.next_image_index;
        state.info.follow_ups.push(FollowUp {
            id: uuid::Uuid::new_v4().to_string(),
            mode,
            delivery: Delivery::Pending,
            prompt: prompt.to_owned(),
            images: image_names(&images),
            local_files,
            image_start,
            output: String::new(),
            work: Work::default(),
            options,
            error: None,
        });
        state.next_image_index += images.len();
        state.images.extend((image_start..).zip(images));
        Ok(())
    }

    pub fn withdraw_queued(
        &self,
        message_id: &str,
        desktop: bool,
    ) -> Result<FollowUpInput, String> {
        let mut state = self.edit(false);
        if state.info.status != Status::Running || self.cancel.load(Ordering::Acquire) {
            return Err("当前回复已结束或正在停止。".into());
        }
        let index = state.info.follow_ups[state.sent..]
            .iter()
            .position(|entry| {
                entry.id == message_id
                    && entry.mode == SendMode::Queue
                    && entry.delivery == Delivery::Pending
            })
            .map(|offset| state.sent + offset)
            .ok_or("这条消息已离开队列。")?;
        if !desktop && !state.info.follow_ups[index].local_files.is_empty() {
            return Err("请在桌面端撤回含有本地附件的消息。".into());
        }
        let entry = state.info.follow_ups.remove(index);
        let images = (entry.image_start..entry.image_start + entry.images.len())
            .map(|index| state.images.remove(&index).expect("queued image"))
            .collect();
        Ok(FollowUpInput {
            id: entry.id,
            prompt: entry.prompt,
            images,
            local_files: entry.local_files,
        })
    }

    pub fn steer_queued(&self, message_id: &str) -> Result<(), String> {
        let mut state = self.edit(false);
        if !state.info.run.provider.supports_steering() {
            return Err("此引擎支持排队发送。".into());
        }
        let entries = &state.info.follow_ups;
        let index = entries
            .iter()
            .position(|entry| entry.id == message_id)
            .ok_or("队列中找不到这条消息。")?;
        if entries[index].mode == SendMode::Steer {
            return Ok(());
        }
        if entries[index].delivery != Delivery::Pending {
            return Err("这条消息已开始发送或已结束。".into());
        }
        if state.info.status != Status::Running || self.cancel.load(Ordering::Acquire) {
            return Err("当前回复已结束或正在停止。".into());
        }
        let mut entry = state.info.follow_ups.remove(index);
        entry.mode = SendMode::Steer;
        entry.options = None;
        state.info.follow_ups.push(entry);
        Ok(())
    }

    pub fn approve(&self, approval: &str, allow: bool) -> Result<(), String> {
        let mut state = self.edit(false);
        if state
            .info
            .approval
            .as_ref()
            .is_none_or(|item| item.id != approval)
            || self.cancel.load(Ordering::Acquire)
        {
            return Err("这个操作已经处理".into());
        }
        let send = state.approval.take().ok_or("这个操作已经处理")?;
        state.info.approval = None;
        send.try_send(allow)
            .map_err(|_| "CLI 已结束这次请求".into())
    }

    pub fn answer(&self, question_id: &str, answers: QuestionAnswers) -> Result<(), String> {
        let mut state = self.edit(false);
        let pending = state
            .questions
            .front()
            .filter(|item| item.display.id == question_id && !self.cancel.load(Ordering::Acquire))
            .ok_or("这次提问已结束或已在另一端回答。")?;
        if answers.len() != pending.display.questions.len()
            || pending.display.questions.iter().any(|question| {
                answers
                    .get(&question.id)
                    .is_none_or(|values| !question.accepts(values))
            })
        {
            return Err("请为每道题选择或填写有效答案。".into());
        }
        (pending.reply)(answers)?;
        state.questions.pop_front();
        Ok(())
    }

    pub fn stop(&self) {
        let mut state = self.edit(false);
        state.changed = !self.cancel.swap(true, Ordering::AcqRel);
    }
}

impl State {
    fn pending(&self, mode: SendMode) -> Option<usize> {
        self.info.follow_ups[self.sent..]
            .iter()
            .position(|entry| entry.delivery == Delivery::Pending && entry.mode == mode)
            .map(|offset| self.sent + offset)
    }

    fn input(&self, index: usize) -> FollowUpInput {
        let entry = &self.info.follow_ups[index];
        let start = entry.image_start;
        FollowUpInput {
            id: entry.id.clone(),
            prompt: entry.prompt.clone(),
            images: (start..start + entry.images.len())
                .map(|index| self.images[&index].clone())
                .collect(),
            local_files: entry.local_files.clone(),
        }
    }

    fn mark_sent(&mut self, index: usize) {
        let mut entry = self.info.follow_ups.remove(index);
        entry.delivery = Delivery::Sent;
        self.info.follow_ups.insert(self.sent, entry);
        self.sent += 1;
    }

    fn mark_unsent(&mut self, index: usize, error: String) {
        let entry = &mut self.info.follow_ups[index];
        entry.delivery = Delivery::Unsent;
        entry.error = Some(error);
    }

    fn work(&mut self) -> &mut Work {
        match self.sent.checked_sub(1) {
            Some(index) => &mut self.info.follow_ups[index].work,
            None => &mut self.info.work,
        }
    }

    fn work_for_step(&mut self, id: &str) -> &mut Work {
        self.work_for(|work| work.steps.iter().any(|step| step.id == id))
    }

    fn work_for(&mut self, has_item: impl Fn(&Work) -> bool) -> &mut Work {
        // Work may finish after a steering message. Update its original position.
        if has_item(&self.info.work) {
            return &mut self.info.work;
        }
        if let Some(index) = self.info.follow_ups[..self.sent]
            .iter()
            .position(|entry| has_item(&entry.work))
        {
            return &mut self.info.follow_ups[index].work;
        }
        self.work()
    }

    fn output(&mut self) -> &mut String {
        match self.sent.checked_sub(1) {
            Some(index) => &mut self.info.follow_ups[index].output,
            None => &mut self.info.output,
        }
    }

    fn append_text(&mut self, text: &str) {
        let text = engine::cut(
            text,
            (4 * 1024 * 1024usize).saturating_sub(self.output_bytes),
        );
        self.output_bytes += text.len();
        self.output().push_str(text);
        self.info.activity = "正在回复".into();
    }

    fn clear_approval(&mut self) {
        self.info.approval = None;
        self.approval = None;
    }

    fn close_steering(&mut self) {
        while let Some(index) = self.pending(SendMode::Steer) {
            self.mark_unsent(index, "当前轮次已结束，未确认引导。".into());
        }
    }

    fn finish(&mut self, result: Result<(), String>, cancelled: bool) {
        self.info.status = if cancelled {
            Status::Stopped
        } else if result.is_ok() {
            Status::Complete
        } else {
            Status::Failed
        };
        self.info.error = if cancelled { None } else { result.err() };
        self.info.activity.clear();
        while let Some(index) = self.pending(SendMode::Queue) {
            let error = match self.info.status {
                Status::Stopped => "运行已停止。",
                Status::Complete => "CLI 未提供可续聊的会话 ID。",
                _ => "上一条消息未完成。",
            };
            self.mark_unsent(index, error.into());
        }
        self.finished = Some(Instant::now());
    }
}

fn image_names(images: &[ImageAttachment]) -> Vec<String> {
    images.iter().map(|image| image.name().to_owned()).collect()
}

fn validate_model_options(provider: Provider, options: &ModelOptions) -> Result<(), String> {
    if options.collaboration_mode.is_some() && provider != Provider::Codex {
        return Err("此引擎不支持所选的协作模式".into());
    }
    if options.model.as_ref().is_some_and(|model| {
        model.is_empty()
            || model.len() > 256
            || model.starts_with('-')
            || model.chars().any(char::is_control)
    }) {
        return Err("模型名称无效".into());
    }
    if options.reasoning_effort.as_ref().is_some_and(|effort| {
        effort.is_empty()
            || effort.len() > 64
            || effort.starts_with('-')
            || effort.chars().any(char::is_control)
    }) {
        return Err("思考强度无效".into());
    }
    engine::validate_permissions(provider, options)
}

fn validate_message(
    prompt: &mut String,
    images: &[ImageAttachment],
    local_files: &[attachments::LocalAttachment],
) -> Result<(), String> {
    attachments::validate_image_attachments(images).map_err(|error| error.to_string())?;
    if prompt.trim().is_empty() {
        if !local_files.is_empty() {
            *prompt = "请查看所附文件。".into();
        } else if !images.is_empty() {
            *prompt = "请查看所附图片。".into();
        }
    }
    let prompt = prompt.trim();
    if prompt.is_empty() || attachments::with_local_files(prompt, local_files).len() > 512 * 1024 {
        return Err("请输入消息，长度上限为 512 KiB".into());
    }
    Ok(())
}
