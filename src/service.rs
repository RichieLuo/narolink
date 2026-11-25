use crate::engine::attachments::{self, ImageAttachment};
use crate::engine::{
    self, Event, Inventory, Model, Provider, Session, Transcript, Turn, TurnRequest,
};
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
    },
    time::{Duration, Instant, SystemTime},
};

type SessionKey = (Provider, String);
type Timed<T> = (Instant, T);
type Catalog<T> = HashMap<Provider, Timed<Result<T, String>>>;

#[derive(Clone, Default)]
pub struct Service(Arc<Shared>);

#[derive(Default)]
struct Shared {
    inventory: Mutex<Catalog<Inventory>>,
    models: Mutex<Catalog<Vec<Model>>>,
    recent_sessions: Mutex<HashMap<SessionKey, Session>>,
    transcripts: Mutex<HashMap<SessionKey, Timed<Transcript>>>,
    runs: Mutex<HashMap<String, Arc<Run>>>,
}

pub struct Run {
    images: Vec<ImageAttachment>,
    view: Mutex<State>,
    cancel: AtomicBool,
}

struct State {
    info: Snapshot,
    approval: Option<SyncSender<bool>>,
    finished: Option<Instant>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Running,
    Complete,
    Stopped,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    #[serde(flatten)]
    pub run: RunSummary,
    pub status: Status,
    pub prompt: String,
    pub output: String,
    pub images: Vec<String>,
    pub activity: String,
    pub error: Option<String>,
    pub approval: Option<Approval>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RunSummary {
    pub id: String,
    pub provider: Provider,
    pub session_id: Option<String>,
    pub title: String,
}

#[derive(Serialize)]
pub struct Activity {
    pub running: Vec<RunSummary>,
    pub finished: Vec<RunSummary>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Approval {
    pub id: String,
    pub title: String,
    pub details: String,
}

impl Service {
    pub fn inventory(&self, provider: Provider, refresh: bool) -> Result<Inventory, String> {
        let mut list = cached(
            &self.0.inventory,
            provider,
            refresh,
            Duration::from_secs(20),
            engine::inventory,
        )?;
        let recent = self.0.recent_sessions.lock().unwrap();
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

    pub fn models(&self, provider: Provider, refresh: bool) -> Result<Vec<Model>, String> {
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
        {
            let recent = self.0.recent_sessions.lock().unwrap();
            if let Some(session) = recent.get(&key) {
                return Ok(session.clone());
            }
        }
        for refresh in [false, true] {
            if let Some(session) = self
                .inventory(provider, refresh)?
                .sessions
                .into_iter()
                .find(|item| item.id == id)
            {
                return Ok(session);
            }
        }
        Err("找不到这个原始会话，请刷新历史".into())
    }

    pub fn transcript(&self, session: &Session, refresh: bool) -> Result<Transcript, String> {
        let key = (session.provider, session.id.clone());
        let active = self.running().iter().any(|turn| {
            turn.provider == session.provider && turn.session_id.as_deref() == Some(&session.id)
        });
        {
            let cache = self.0.transcripts.lock().unwrap();
            if let Some((at, transcript)) = cache.get(&key)
                && (active || (!refresh && at.elapsed() < Duration::from_secs(10)))
            {
                return Ok(transcript.clone());
            }
        }
        if active {
            return Ok(Transcript::default());
        }
        let transcript = engine::transcript(session)?;
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
        attachments::validate_image_attachments(&images).map_err(|error| error.to_string())?;
        if request.prompt.trim().is_empty() && !images.is_empty() {
            request.prompt = "请查看所附图片。".into();
        }
        let prompt = request.prompt.trim();
        if prompt.is_empty() || prompt.len() > 512 * 1024 {
            return Err("请输入消息，长度上限为 512 KiB".into());
        }
        if request.model.as_ref().is_some_and(|model| {
            model.is_empty()
                || model.len() > 256
                || model.starts_with('-')
                || model.chars().any(char::is_control)
        }) {
            return Err("模型名称无效".into());
        }
        let original = request
            .session_id
            .as_deref()
            .map(|id| self.session(request.provider, id))
            .transpose()?;
        if let Some(session) = &original {
            request.cwd.clone_from(&session.cwd);
        }
        if !request.cwd.is_absolute() || !request.cwd.is_dir() {
            return Err("在电脑上选择一个存在的工作目录".into());
        }
        let summary = RunSummary {
            id: uuid::Uuid::new_v4().to_string(),
            provider: request.provider,
            session_id: request.session_id.clone(),
            title: original
                .as_ref()
                .map(|session| session.title.clone())
                .unwrap_or_else(|| engine::short(prompt, 120)),
        };
        let names = images.iter().map(|image| image.name().to_owned()).collect();
        let run = Arc::new(Run {
            images,
            view: Mutex::new(State {
                info: Snapshot {
                    run: summary.clone(),
                    status: Status::Running,
                    prompt: prompt.to_owned(),
                    output: String::new(),
                    images: names,
                    activity: "正在连接 CLI".into(),
                    error: None,
                    approval: None,
                },
                approval: None,
                finished: None,
            }),
            cancel: AtomicBool::new(false),
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
            .spawn(move || {
                let event = |event| match event {
                    Event::Session(id) => {
                        let mut state = run.view.lock().unwrap();
                        state.info.run.session_id = Some(id);
                        let session = Session {
                            provider: request.provider,
                            id: state.info.run.session_id.clone().unwrap(),
                            cwd: request.cwd.clone(),
                            title: state.info.run.title.clone(),
                            updated: engine::unix_time(SystemTime::now()),
                            preview: Some(engine::short(&request.prompt, 300)),
                        };
                        let key = (request.provider, session.id.clone());
                        service
                            .0
                            .recent_sessions
                            .lock()
                            .unwrap()
                            .insert(key, session);
                    }
                    Event::Text(text) => {
                        let mut state = run.view.lock().unwrap();
                        let remaining =
                            (4 * 1024 * 1024usize).saturating_sub(state.info.output.len());
                        state.info.output.push_str(engine::cut(&text, remaining));
                        state.info.activity = "正在回复".into();
                    }
                    Event::Activity(label) => run.view.lock().unwrap().info.activity = label,
                    Event::Approve {
                        title,
                        details,
                        answer,
                    } => {
                        let mut state = run.view.lock().unwrap();
                        state.info.approval = Some(Approval {
                            id: uuid::Uuid::new_v4().to_string(),
                            title,
                            details,
                        });
                        state.approval = Some(answer);
                    }
                };
                let turn = Turn {
                    cancel: &run.cancel,
                    images: &run.images,
                    emit: &event,
                };
                let result = engine::run(&request, &turn);
                let mut state = run.view.lock().unwrap();
                if let Some(id) = &state.info.run.session_id {
                    let key = (request.provider, id.clone());
                    // Keep the last readable projection until the CLI has published its new history.
                    if let Some((at, _)) = service.0.transcripts.lock().unwrap().get_mut(&key) {
                        *at = Instant::now()
                            .checked_sub(Duration::from_secs(20))
                            .unwrap_or_else(Instant::now);
                    }
                }
                service
                    .0
                    .inventory
                    .lock()
                    .unwrap()
                    .remove(&request.provider);
                state.finish(result, run.cancel.load(Ordering::Acquire));
            });
        if start.is_err() {
            self.0.runs.lock().unwrap().remove(&summary.id);
            return Err("无法启动 CLI 后台任务".into());
        }
        Ok(summary)
    }

    pub fn activity(&self) -> Activity {
        let runs = self.0.runs.lock().unwrap();
        let mut matches = Activity {
            running: Vec::new(),
            finished: Vec::new(),
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
    if !refresh
        && let Some((at, value)) = cache.lock().unwrap().get(&provider)
        && at.elapsed()
            < if value.is_ok() {
                ttl
            } else {
                Duration::from_secs(3)
            }
    {
        return value.clone();
    }
    let value = read(provider);
    cache
        .lock()
        .unwrap()
        .insert(provider, (Instant::now(), value.clone()));
    value
}

impl Run {
    pub fn image(&self, index: usize) -> Option<&ImageAttachment> {
        self.images.get(index)
    }

    pub fn snapshot(&self) -> Snapshot {
        self.view.lock().unwrap().info.clone()
    }

    pub fn approve(&self, approval: &str, allow: bool) -> Result<(), String> {
        let mut state = self.view.lock().unwrap();
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

    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}

impl State {
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
        self.info.approval = None;
        self.approval = None;
        self.finished = Some(Instant::now());
    }
}
