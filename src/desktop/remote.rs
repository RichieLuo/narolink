use crate::{
    engine::{
        CodexQuotaResetOutcome, CollaborationMode, Inventory, Message, Model, ModelCatalog,
        ModelOptions, PermissionChoice, Provider, QuestionAnswers, Quota, Session, SessionAction,
        Transcript, TurnRequest,
        attachments::{
            HistoryImage, ImageAttachment, ImageUpload, MAX_IMAGE_SOURCE_BYTES, UserAttachment,
            decode_uploads,
        },
        usage::UsageReport,
        work::Work,
    },
    service::{Activity, FollowUpRequest, PermissionTarget, RunSummary, SendMode, Snapshot},
    web,
};
use reqwest::{
    Method, StatusCode, Url,
    blocking::{Client, RequestBuilder, Response},
    header::{self, HeaderValue},
    redirect::Policy,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{io::Read, sync::Arc, time::Duration};

const CONNECTION_ERROR: &str = "无法连接电脑。请确认 Naro 和远程网络仍在运行。";
const RESPONSE_ERROR: &str = "电脑端返回的数据无效，请更新目标电脑上的 Naro。";
const MAX_JSON_BYTES: usize = 24 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct RemoteClient(Arc<Connection>);

struct Connection {
    http: Client,
    address: Url,
    origin: String,
    name: String,
    cookie: HeaderValue,
}

impl RemoteClient {
    pub fn connect(address: &str, code: &str) -> Result<Self, String> {
        let (address, code) = pairing_input(address, code)?;
        let origin = address.origin().ascii_serialization();
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(36))
            .redirect(Policy::none())
            .build()
            .map_err(|_| CONNECTION_ERROR)?;

        let response = send(
            http.post(endpoint(&address, &["pair"]))
                .header(header::ORIGIN, origin.as_str())
                .json(&json!({"code": code})),
        )?;
        let cookie = pairing_cookie(&response, &address)?;
        let pair: PairStatus = read_json(response)?;
        if !pair.paired {
            return Err("电脑没有完成配对".into());
        }

        let access: PairStatus = read_json(send(
            http.get(endpoint(&address, &["access"]))
                .header(header::COOKIE, cookie.clone()),
        )?)?;
        if !access.paired {
            return Err("电脑没有完成配对".into());
        }
        if access.online == Some(false) {
            return Err("电脑未连接。请打开 Naro。".into());
        }
        let name = access
            .name
            .into_iter()
            .chain(pair.name)
            .map(|name| {
                name.trim()
                    .chars()
                    .filter(|ch| !ch.is_control())
                    .take(80)
                    .collect::<String>()
            })
            .find(|name| !name.is_empty())
            .unwrap_or_else(|| address.host_str().unwrap_or_default().to_owned());
        Ok(Self(Arc::new(Connection {
            http,
            address,
            origin,
            name,
            cookie,
        })))
    }

    pub fn name(&self) -> &str {
        &self.0.name
    }

    pub fn address(&self) -> &str {
        self.0.address.as_str()
    }

    pub fn inventory(&self, provider: Provider, force: bool) -> Result<Inventory, String> {
        read_json(send(
            self.get(&["sessions", provider_name(provider)])
                .query(&[("refresh", force)]),
        )?)
    }

    pub fn models(&self, provider: Provider, force: bool) -> Result<ModelCatalog, String> {
        let wire: CatalogResponse = read_json(send(
            self.get(&["models", provider_name(provider)])
                .query(&[("settings", true), ("refresh", force)]),
        )?)?;
        Ok(match wire {
            CatalogResponse::Catalog(catalog) => catalog.into(),
            CatalogResponse::Models(models) => ModelCatalog {
                models,
                ..ModelCatalog::default()
            },
        })
    }

    pub fn quota(&self, provider: Provider) -> Result<Quota, String> {
        read_json(send(self.get(&["quotas", provider_name(provider)]))?)
    }

    pub fn token_usage(&self) -> Result<UsageReport, String> {
        read_json(send(self.get(&["usage"]))?)
    }

    pub fn git_status(&self, cwd: &std::path::Path) -> Result<crate::git::GitStatus, String> {
        read_json(send(
            self.get(&["git", "status"])
                .query(&[("cwd", cwd.to_string_lossy().as_ref())]),
        )?)
    }

    pub fn git_diff(
        &self,
        cwd: &std::path::Path,
        request: &crate::git::GitDiffRequest,
    ) -> Result<crate::git::GitDiff, String> {
        let scope = match request.scope {
            crate::git::GitDiffScope::Staged => "staged",
            crate::git::GitDiffScope::Unstaged => "unstaged",
            crate::git::GitDiffScope::Untracked => "untracked",
            crate::git::GitDiffScope::Commit => "commit",
        };
        let mut builder = self
            .get(&["git", "diff"])
            .query(&[("cwd", cwd.to_string_lossy().as_ref()), ("scope", scope)]);
        if let Some(path) = request.path.as_deref() {
            builder = builder.query(&[("path", path)]);
        }
        if let Some(commit) = request.commit.as_deref() {
            builder = builder.query(&[("commit", commit)]);
        }
        if request.full_context {
            builder = builder.query(&[("full_context", "true")]);
        }
        read_json(send(builder)?)
    }

    pub fn git_action(
        &self,
        cwd: &std::path::Path,
        action: &crate::git::GitAction,
    ) -> Result<crate::git::GitActionResult, String> {
        #[derive(Serialize)]
        struct Body<'a> {
            cwd: String,
            #[serde(flatten)]
            action: &'a crate::git::GitAction,
        }
        let body = Body {
            cwd: cwd.to_string_lossy().into_owned(),
            action,
        };
        let mut req = self
            .request(Method::POST, &["git", "action"])
            .header(header::ORIGIN, self.0.origin.as_str())
            .json(&body);
        if matches!(
            action,
            crate::git::GitAction::Fetch
                | crate::git::GitAction::Pull
                | crate::git::GitAction::Push
        ) {
            req = req.timeout(Duration::from_secs(65));
        }
        read_json(send(req)?)
    }

    pub fn git_generate_commit_message(
        &self,
        cwd: &std::path::Path,
        amend: bool,
        provider: Option<Provider>,
        model: Option<&str>,
        language: Option<&str>,
    ) -> Result<String, String> {
        #[derive(Deserialize)]
        struct Reply {
            message: String,
        }
        let body = json!({
            "cwd": cwd.to_string_lossy(),
            "amend": amend,
            "provider": provider,
            "model": model,
            "language": language,
        });
        let req = self
            .request(Method::POST, &["git", "commit-message"])
            .header(header::ORIGIN, self.0.origin.as_str())
            .timeout(Duration::from_secs(45))
            .json(&body);
        let reply: Reply = read_json(send(req)?)?;
        Ok(reply.message)
    }

    pub fn reset_codex_quota(&self, id: uuid::Uuid) -> Result<CodexQuotaResetOutcome, String> {
        let result: QuotaReset = read_json(self.post(
            &["quotas", "codex", "reset"],
            &json!({"idempotency_key": id.to_string()}),
        )?)?;
        Ok(result.outcome)
    }

    pub fn permission_mode(
        &self,
        provider: Provider,
        target: &PermissionTarget,
    ) -> Result<Option<String>, String> {
        read_json(send(
            self.get(&["permissions", provider_name(provider)])
                .query(target),
        )?)
    }

    pub fn save_permission_mode(
        &self,
        provider: Provider,
        target: &PermissionTarget,
        mode: Option<&str>,
    ) -> Result<(), String> {
        self.post(
            &["permissions", provider_name(provider)],
            &json!({"session_id": target.session_id, "run_id": target.run_id, "permission_mode": mode}),
        )?;
        Ok(())
    }

    pub fn history(
        &self,
        provider: Provider,
        id: &str,
        refresh: bool,
    ) -> Result<(Session, Transcript), String> {
        let history: History = read_json(send(
            self.get(&["sessions", provider_name(provider), id])
                .query(&[("refresh", refresh)]),
        )?)?;
        Ok((
            history.session,
            history.transcript.map(Transcript::from).unwrap_or_default(),
        ))
    }

    pub fn session_action(
        &self,
        session: &Session,
        action: &SessionAction,
    ) -> Result<Value, String> {
        read_json(self.post(
            &[
                "sessions",
                provider_name(session.provider),
                &session.id,
                "actions",
            ],
            action,
        )?)
    }

    pub fn search(&self, provider: Provider, query: &str, archived: bool) -> Result<Value, String> {
        read_json(send(
            self.get(&["search", provider_name(provider)])
                .query(&[("q", query)])
                .query(&[("archived", archived)]),
        )?)
    }

    pub fn occurrences(&self, session: &Session, query: &str) -> Result<Value, String> {
        read_json(send(
            self.get(&[
                "sessions",
                provider_name(session.provider),
                &session.id,
                "occurrences",
            ])
            .query(&[("q", query)]),
        )?)
    }

    pub fn activity(&self) -> Result<Activity, String> {
        read_json(send(self.get(&["activity"]))?)
    }

    pub fn snapshot(&self, id: &str) -> Result<Option<Snapshot>, String> {
        let response = self
            .get(&["runs", id])
            .send()
            .map_err(|_| CONNECTION_ERROR)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let result: LiveSnapshot = read_json(check_response(response)?)?;
        Ok(Some(result.into()))
    }

    pub fn start_with_images(
        &self,
        mut request: TurnRequest,
        images: Vec<ImageAttachment>,
    ) -> Result<RunSummary, String> {
        request.images = image_uploads(images);
        read_json(self.post(&["runs"], &request)?)
    }

    pub fn follow_up(
        &self,
        id: &str,
        mode: SendMode,
        prompt: String,
        images: Vec<ImageAttachment>,
        options: Option<ModelOptions>,
    ) -> Result<(), String> {
        self.post(
            &["runs", id, "messages"],
            &FollowUpRequest {
                mode,
                prompt,
                options,
                images: image_uploads(images),
            },
        )?;
        Ok(())
    }

    pub fn stop(&self, id: &str) -> Result<(), String> {
        self.post(&["runs", id, "stop"], &json!({}))?;
        Ok(())
    }

    pub fn approve(&self, id: &str, approval_id: &str, allow: bool) -> Result<(), String> {
        self.post(
            &["runs", id, "approval"],
            &json!({"approval_id": approval_id, "allow": allow}),
        )?;
        Ok(())
    }

    pub fn answer(
        &self,
        id: &str,
        question_id: &str,
        answers: QuestionAnswers,
    ) -> Result<(), String> {
        self.post(
            &["runs", id, "answer"],
            &json!({"question_id": question_id, "answers": answers}),
        )?;
        Ok(())
    }

    pub fn steer_queued(&self, id: &str, message_id: &str) -> Result<(), String> {
        self.post(&["runs", id, "messages", message_id, "steer"], &json!({}))?;
        Ok(())
    }

    pub fn withdraw_queued(
        &self,
        id: &str,
        message_id: &str,
    ) -> Result<(String, Vec<ImageAttachment>), String> {
        let draft: WithdrawnMessage = read_json(self.post(
            &["runs", id, "messages", message_id, "withdraw"],
            &json!({}),
        )?)?;
        let uploads = draft
            .images
            .into_iter()
            .map(|image| ImageUpload {
                name: "image.png".into(),
                data: image.data,
                display_name: Some(image.name),
            })
            .collect();
        let images = decode_uploads(uploads).map_err(|error| error.to_string())?;
        Ok((draft.prompt, images))
    }

    pub fn image(&self, run_id: &str, index: usize) -> Result<Vec<u8>, String> {
        self.image_bytes(&["runs", run_id, "images", &index.to_string()])
    }

    pub fn preview(&self, key: &str) -> Result<Vec<u8>, String> {
        if !valid_preview_key(key) {
            return Err(RESPONSE_ERROR.into());
        }
        self.image_bytes(&["previews", key])
    }

    fn image_bytes(&self, path: &[&str]) -> Result<Vec<u8>, String> {
        let response = send(self.get(path))?;
        if response
            .headers()
            .get(header::CONTENT_TYPE)
            .is_none_or(|value| value != "image/png")
        {
            return Err(RESPONSE_ERROR.into());
        }
        read_bytes(response, MAX_IMAGE_SOURCE_BYTES)
    }

    fn get(&self, path: &[&str]) -> RequestBuilder {
        self.request(Method::GET, path)
    }

    fn post(&self, path: &[&str], body: &impl Serialize) -> Result<Response, String> {
        send(
            self.request(Method::POST, path)
                .header(header::ORIGIN, self.0.origin.as_str())
                .json(body),
        )
    }

    fn request(&self, method: Method, path: &[&str]) -> RequestBuilder {
        self.0
            .http
            .request(method, endpoint(&self.0.address, path))
            .header(header::COOKIE, self.0.cookie.clone())
    }
}

fn pairing_input(address: &str, code: &str) -> Result<(Url, String), String> {
    let address = address.trim();
    let invalid_address = "请粘贴目标电脑上的设备地址";
    let mut url = Url::parse(address).map_err(|_| invalid_address)?;
    if address.chars().any(char::is_control) {
        return Err(invalid_address.into());
    }
    #[cfg(not(test))]
    if url.scheme() != "https" {
        return Err(invalid_address.into());
    }
    let fragment_code = url
        .fragment()
        .map(|fragment| {
            fragment
                .strip_prefix("pair=")
                .filter(|code| is_pairing_code(code))
                .map(str::to_owned)
                .ok_or(invalid_address)
        })
        .transpose()?;
    url.set_fragment(None);
    if url.path().starts_with("/devices/") && !url.path().ends_with('/') {
        url.path_segments_mut()
            .map_err(|_| invalid_address)?
            .push("");
    }
    let url = web::device_url(url.as_str())?;
    let code = if code.trim().is_empty() {
        fragment_code.unwrap_or_default()
    } else {
        code.trim().to_owned()
    };
    if !is_pairing_code(&code) {
        return Err("请输入目标电脑的 8 位配对码".into());
    }
    Ok((url, code))
}

fn is_pairing_code(code: &str) -> bool {
    code.len() == 8 && code.bytes().all(|byte| byte.is_ascii_digit())
}

fn endpoint(address: &Url, path: &[&str]) -> Url {
    let mut url = address.clone();
    url.path_segments_mut()
        .expect("device URLs have path segments")
        .pop_if_empty()
        .push("api")
        .extend(path.iter().copied());
    url
}

fn pairing_cookie(response: &Response, address: &Url) -> Result<HeaderValue, String> {
    let allowed_names: &[&str] = if address.path() == "/" {
        &["narolink", "narolink_remote"]
    } else {
        &["narolink_device"]
    };
    for raw in response.headers().get_all(header::SET_COOKIE) {
        let Ok(raw) = raw.to_str() else { continue };
        let Some((pair, attributes)) = raw.split_once(';') else {
            continue;
        };
        let Some((name, value)) = pair.trim().split_once('=') else {
            continue;
        };
        let mut scope = None;
        let mut secure = false;
        for attribute in attributes.split(';').map(str::trim) {
            if let Some((key, value)) = attribute.split_once('=')
                && key.eq_ignore_ascii_case("path")
            {
                scope = Some(value);
            }
            secure |= attribute.eq_ignore_ascii_case("secure");
        }
        if allowed_names.contains(&name)
            && scope == Some(address.path())
            && (address.scheme() != "https" || secure)
            && !value.is_empty()
            && value.len() <= 256
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
            && let Ok(mut cookie) = HeaderValue::from_str(pair.trim())
        {
            cookie.set_sensitive(true);
            return Ok(cookie);
        }
    }
    Err("电脑没有完成配对".into())
}

fn provider_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Codex => "codex",
        Provider::Claude => "claude",
        Provider::Antigravity => "antigravity",
    }
}

fn send(request: RequestBuilder) -> Result<Response, String> {
    check_response(request.send().map_err(|_| CONNECTION_ERROR)?)
}

fn check_response(response: Response) -> Result<Response, String> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let reason: Option<ErrorResponse> = read_bytes(response, 8192)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    Err(reason
        .map(|reason| reason.error)
        .filter(|reason| !reason.trim().is_empty())
        .unwrap_or_else(|| match status {
            StatusCode::UNAUTHORIZED => "先和电脑配对".into(),
            StatusCode::SERVICE_UNAVAILABLE => "电脑未连接。请打开 Naro。".into(),
            _ => format!("电脑端返回错误（{}）", status.as_u16()),
        }))
}

fn read_json<T: DeserializeOwned>(response: Response) -> Result<T, String> {
    let bytes = read_bytes(response, MAX_JSON_BYTES)?;
    serde_json::from_slice(&bytes).map_err(|_| RESPONSE_ERROR.into())
}

fn read_bytes(response: Response, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    response
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CONNECTION_ERROR)?;
    if bytes.len() > limit {
        return Err("传输内容过大".into());
    }
    Ok(bytes)
}

fn image_uploads(images: Vec<ImageAttachment>) -> Vec<ImageUpload> {
    images
        .into_iter()
        .map(|image| ImageUpload {
            name: "image.png".into(),
            data: image.base64(),
            display_name: Some(image.name().to_owned()),
        })
        .collect()
}

fn valid_preview_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

#[derive(Deserialize)]
struct PairStatus {
    paired: bool,
    online: Option<bool>,
    name: Option<String>,
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum CatalogResponse {
    Catalog(Catalog),
    Models(Vec<Model>),
}

#[derive(Deserialize)]
struct Catalog {
    models: Vec<Model>,
    default_model: Option<String>,
    #[serde(default)]
    permission_modes: Vec<PermissionChoice>,
    default_permission_mode: Option<String>,
    #[serde(default)]
    collaboration_modes: Vec<CollaborationMode>,
}

impl From<Catalog> for ModelCatalog {
    fn from(catalog: Catalog) -> Self {
        Self {
            models: catalog.models,
            default_model: catalog.default_model,
            permission_modes: catalog.permission_modes,
            default_permission_mode: match catalog.default_permission_mode.as_deref() {
                Some("request-approval") => Some("request-approval"),
                Some("manual") => Some("manual"),
                _ => None,
            },
            collaboration_modes: catalog.collaboration_modes,
        }
    }
}

#[derive(Deserialize)]
struct QuotaReset {
    outcome: CodexQuotaResetOutcome,
}

#[derive(Deserialize)]
struct History {
    session: Session,
    transcript: Option<RemoteTranscript>,
}

#[derive(Deserialize)]
struct RemoteTranscript {
    messages: Vec<RemoteMessage>,
    model: Option<String>,
    warning: Option<String>,
    revision: Option<String>,
}

#[derive(Deserialize)]
struct RemoteMessage {
    #[serde(flatten)]
    message: Message,
    #[serde(default)]
    attachments: Vec<AttachmentPreview>,
    rendered: Option<RenderedReply>,
}

#[derive(Deserialize)]
struct AttachmentPreview {
    name: String,
    url: Option<String>,
}

impl From<AttachmentPreview> for UserAttachment {
    fn from(attachment: AttachmentPreview) -> Self {
        Self {
            name: attachment.name,
            image: attachment
                .url
                .as_deref()
                .and_then(|url| url.strip_prefix("/api/previews/"))
                .filter(|key| valid_preview_key(key))
                .map(|key| HistoryImage::Remote(key.to_owned())),
        }
    }
}

#[derive(Deserialize)]
struct RenderedReply {
    #[serde(default)]
    previews: Vec<AttachmentPreview>,
}

impl RenderedReply {
    fn into_previews(self) -> Vec<UserAttachment> {
        self.previews
            .into_iter()
            .map(UserAttachment::from)
            .filter(|attachment| attachment.image.is_some())
            .collect()
    }
}

impl From<RemoteTranscript> for Transcript {
    fn from(history: RemoteTranscript) -> Self {
        let messages = history
            .messages
            .into_iter()
            .map(|entry| {
                let mut message = entry.message;
                message.attachments = entry
                    .attachments
                    .into_iter()
                    .map(UserAttachment::from)
                    .collect();
                message.work.remote_previews = entry
                    .rendered
                    .map(RenderedReply::into_previews)
                    .unwrap_or_default();
                message
            })
            .collect();
        Self {
            messages,
            model: history.model,
            warning: history.warning,
            revision: history.revision,
        }
    }
}

#[derive(Deserialize)]
struct LiveSnapshot {
    #[serde(flatten)]
    snapshot: Snapshot,
    #[serde(default)]
    replies: Vec<LiveReply>,
}

#[derive(Deserialize)]
struct LiveReply {
    after: Option<String>,
    work: Work,
    rendered: Option<RenderedReply>,
}

impl From<LiveSnapshot> for Snapshot {
    fn from(live: LiveSnapshot) -> Self {
        let mut snapshot = live.snapshot;
        for reply in live.replies {
            let mut work = reply.work;
            work.remote_previews = reply
                .rendered
                .map(RenderedReply::into_previews)
                .unwrap_or_default();
            if let Some(after) = reply.after {
                if let Some(entry) = snapshot
                    .follow_ups
                    .iter_mut()
                    .find(|entry| entry.id == after)
                {
                    entry.work = work;
                }
            } else {
                snapshot.work = work;
            }
        }
        snapshot
    }
}

#[derive(Deserialize)]
struct WithdrawnMessage {
    prompt: String,
    images: Vec<WithdrawnImage>,
}

#[derive(Deserialize)]
struct WithdrawnImage {
    name: String,
    data: String,
}

#[cfg(test)]
mod tests;
