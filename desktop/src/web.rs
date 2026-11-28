mod auth;
mod remote;
mod reply;
mod stream;

use crate::{
    engine::{
        self, Inventory, Message, Model, ModelCatalog, Provider, QuestionAnswers, Quota, Role,
        Session, Transcript, TurnRequest, usage::UsageReport, work::Work,
    },
    service::{Activity, FollowUpRequest, PermissionTarget, RunSummary, Service, Snapshot},
};
use auth::Auth;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    net::{Ipv4Addr, SocketAddrV4, TcpListener},
    sync::Arc,
    time::Duration,
};

pub struct Options {
    port: u16,
    public_url: Option<String>,
}

impl Options {
    pub fn from_environment() -> Result<Self, String> {
        let port = std::env::var("NAROLINK_PORT")
            .map_or(Ok(4318), |value| value.parse::<u16>())
            .map_err(|_| "NAROLINK_PORT 必须是 1–65535 之间的端口")?;
        let public_url = std::env::var("NAROLINK_PUBLIC_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| {
                let url = url::Url::parse(&value).map_err(|_| "远程地址必须是 HTTPS 网页地址")?;
                if url.scheme() != "https"
                    || !url.has_host()
                    || url.path() != "/"
                    || url.query().is_some()
                    || url.fragment().is_some()
                    || !url.username().is_empty()
                    || url.password().is_some()
                {
                    return Err("NAROLINK_PUBLIC_URL 必须是独立的 HTTPS origin");
                }
                Ok(url.origin().ascii_serialization())
            })
            .transpose()?;
        Ok(Self { port, public_url })
    }
}

#[derive(Clone)]
struct Web {
    service: Service,
    auth: Arc<Auth>,
    remote: remote::Remote,
    previews: Arc<reply::Previews>,
    local: String,
}

pub struct Gateway {
    web: Web,
    runtime: Option<tokio::runtime::Runtime>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}

pub use remote::device_url;

impl Gateway {
    pub fn start(service: Service, options: Options) -> Result<Self, String> {
        let auth = Arc::new(Auth::load()?);
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, options.port))
            .map_err(|error| format!("无法启动 Naro 网页服务。请检查是否已经打开。\n{error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .max_blocking_threads(14)
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        let local = format!("http://{address}");
        let remote =
            remote::Remote::new(local.clone(), options.public_url, auth.relay_device_key());
        let web = Web {
            service,
            auth,
            remote,
            previews: Arc::new(reply::Previews::new()),
            local,
        };
        let listener = {
            let _runtime = runtime.enter();
            tokio::net::TcpListener::from_std(listener).map_err(|error| error.to_string())?
        };
        web.remote.configure_from_environment()?;
        let router = routes(web.clone());
        let (stop, shutdown) = tokio::sync::oneshot::channel();
        runtime.spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = shutdown.await;
                })
                .await;
        });
        Ok(Self {
            web,
            runtime: Some(runtime),
            stop: Some(stop),
        })
    }

    pub fn local_url(&self) -> &str {
        &self.web.local
    }

    pub fn pairing_code(&self) -> String {
        self.web.auth.code()
    }

    pub fn remote(&self) -> &remote::Remote {
        &self.web.remote
    }

    pub fn wait(&self) {
        loop {
            std::thread::park();
        }
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.web.service.shutdown();
        self.web.remote.shutdown();
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_millis(500));
        }
    }
}

fn routes(web: Web) -> Router {
    Router::new()
        .route(
            "/",
            get(|| async {
                asset(
                    "text/html; charset=utf-8",
                    include_bytes!("../../web/public/index.html"),
                )
            }),
        )
        .route(
            "/app.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_bytes!("../../web/public/app.js"),
                )
            }),
        )
        .route(
            "/theme.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_bytes!("../../web/public/theme.js"),
                )
            }),
        )
        .route(
            "/qr-worker.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_bytes!("../../web/public/qr-worker.js"),
                )
            }),
        )
        .route(
            "/vendor/jsqr.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_bytes!("../../web/public/vendor/jsqr.js"),
                )
            }),
        )
        .route(
            "/i18n.json",
            get(|| async {
                asset(
                    "application/json; charset=utf-8",
                    include_bytes!("../../web/public/i18n.json"),
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async {
                asset(
                    "text/css; charset=utf-8",
                    include_bytes!("../../web/public/style.css"),
                )
            }),
        )
        .route(
            "/manifest.webmanifest",
            get(|| async {
                asset(
                    "application/manifest+json",
                    include_bytes!("../../web/public/manifest.webmanifest"),
                )
            }),
        )
        .route(
            "/sw.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_bytes!("../../web/public/sw.js"),
                )
            }),
        )
        .route(
            "/icons/180.png",
            get(|| async {
                asset(
                    "image/png",
                    include_bytes!("../../web/public/icons/180.png"),
                )
            }),
        )
        .route(
            "/icons/192.png",
            get(|| async {
                asset(
                    "image/png",
                    include_bytes!("../../web/public/icons/192.png"),
                )
            }),
        )
        .route(
            "/icons/512.png",
            get(|| async {
                asset(
                    "image/png",
                    include_bytes!("../../web/public/icons/512.png"),
                )
            }),
        )
        .route("/api/access", get(access))
        .route("/api/activity", get(activity))
        .route("/api/pair", post(pair))
        .route("/api/logout", post(logout))
        .route("/api/sessions/{provider}", get(sessions))
        .route("/api/search/{provider}", get(search_sessions))
        .route(
            "/api/sessions/{provider}/{id}/actions",
            post(session_action),
        )
        .route(
            "/api/sessions/{provider}/{id}/occurrences",
            get(search_occurrences),
        )
        .route("/api/sessions/{provider}/{id}", get(transcript))
        .route("/api/previews/{key}", get(preview))
        .route("/api/file-previews/{key}", get(file_preview))
        .route("/api/models/{provider}", get(models))
        .route(
            "/api/permissions/{provider}",
            get(permission_mode).post(save_permission_mode),
        )
        .route("/api/quotas/{provider}", get(quota))
        .route("/api/quotas/codex/reset", post(reset_codex_quota))
        .route("/api/usage", get(token_usage))
        .route("/api/git/status", get(git_status))
        .route("/api/git/diff", get(git_diff))
        .route("/api/git/action", post(git_action))
        .route("/api/git/commit-message", post(git_commit_message))
        .route("/api/runs", get(running).post(start_turn))
        .route("/api/runs/{id}", get(turn))
        .route("/api/runs/{id}/events", get(stream::events))
        .route("/api/runs/{id}/messages", post(follow_up))
        .route(
            "/api/runs/{id}/messages/{message_id}/withdraw",
            post(withdraw_queued),
        )
        .route(
            "/api/runs/{id}/messages/{message_id}/steer",
            post(steer_queued),
        )
        .route("/api/runs/{id}/images/{index}", get(image))
        .route("/api/runs/{id}/stop", post(stop_turn))
        .route("/api/runs/{id}/approval", post(approve))
        .layer(DefaultBodyLimit::max(18 * 1024 * 1024))
        .route(
            "/api/runs/{id}/answer",
            post(answer).layer(DefaultBodyLimit::max(64 * 1024)),
        )
        .layer(middleware::from_fn_with_state(web.clone(), guard))
        .with_state(web)
}

fn asset(content_type: &'static str, bytes: &'static [u8]) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        bytes,
    )
        .into_response()
}

async fn guard(State(web): State<Web>, request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let local = web.local.strip_prefix("http://").unwrap_or_default();

    let remote = web.remote.status().url;
    let expected = if host == local {
        Some(web.local.as_str())
    } else {
        remote.as_deref().filter(|url| {
            url.split_once("://")
                .is_some_and(|(_, authority)| authority == host)
        })
    };
    let Some(origin) = expected else {
        return ApiError(StatusCode::MISDIRECTED_REQUEST, "此地址未配置".into()).into_response();
    };
    let method = request.method();
    if method != Method::GET
        && method != Method::HEAD
        && request
            .headers()
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok())
            != Some(origin)
    {
        return ApiError(StatusCode::FORBIDDEN, "请求来源不匹配".into()).into_response();
    }
    let path = request.uri().path();
    let is_api = path.starts_with("/api/");
    if is_api
        && !matches!(path, "/api/access" | "/api/pair")
        && !web.auth.authorized(request.headers())
    {
        return ApiError(StatusCode::UNAUTHORIZED, "先和电脑配对".into()).into_response();
    }
    let secure = origin.starts_with("https:");
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    if is_api {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    if secure {
        headers.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=86400"),
        );
    }
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(
        "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' blob:; media-src 'self' blob:; connect-src 'self'; \
         manifest-src 'self'; worker-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'",
    ));
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    response
}

async fn access(State(web): State<Web>, headers: axum::http::HeaderMap) -> Json<Value> {
    Json(json!({"paired":web.auth.authorized(&headers)}))
}

#[derive(Deserialize)]
struct Pair {
    code: String,
}

async fn pair(
    State(web): State<Web>,
    headers: axum::http::HeaderMap,
    Json(pair): Json<Pair>,
) -> Result<Response, ApiError> {
    web.auth
        .pair(&pair.code)
        .map_err(|(status, error)| ApiError(status, error.into()))?;
    Ok((
        [(header::SET_COOKIE, web.auth.cookie(is_https(&headers)))],
        Json(json!({"paired":true})),
    )
        .into_response())
}

async fn logout(headers: axum::http::HeaderMap) -> Response {
    (
        [(header::SET_COOKIE, Auth::clear_cookie(is_https(&headers)))],
        StatusCode::NO_CONTENT,
    )
        .into_response()
}

fn is_https(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|origin| origin.starts_with("https://"))
}

#[derive(Default, Deserialize)]
struct Refresh {
    #[serde(default)]
    refresh: bool,
}

async fn sessions(
    State(web): State<Web>,
    Path(provider): Path<Provider>,
    Query(query): Query<Refresh>,
) -> Result<Json<Inventory>, ApiError> {
    blocking(move || web.service.inventory(provider, query.refresh)).await
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default)]
    archived: bool,
    cursor: Option<String>,
}

async fn search_sessions(
    Path(provider): Path<Provider>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Value>, ApiError> {
    blocking(move || engine::search(provider, &query.q, query.archived, query.cursor.as_deref()))
        .await
}

async fn search_occurrences(
    State(web): State<Web>,
    Path((provider, id)): Path<(Provider, String)>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let session = web.service.session(provider, &id)?;
        engine::occurrences(&session, &query.q, query.cursor.as_deref())
    })
    .await
}

async fn session_action(
    State(web): State<Web>,
    Path((provider, id)): Path<(Provider, String)>,
    Json(action): Json<engine::SessionAction>,
) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let session = web.service.session(provider, &id)?;
        web.service.session_action(&session, &action)
    })
    .await
}

async fn quota(Path(provider): Path<Provider>) -> Result<Json<Quota>, ApiError> {
    blocking(move || engine::quota(provider)).await
}

async fn token_usage(State(web): State<Web>) -> Result<Json<UsageReport>, ApiError> {
    blocking(move || Ok(web.service.token_usage())).await
}

#[derive(Deserialize)]
struct GitStatusQuery {
    cwd: String,
}

async fn git_status(
    State(web): State<Web>,
    Query(query): Query<GitStatusQuery>,
) -> Result<Json<crate::git::GitStatus>, ApiError> {
    blocking(move || web.service.git_status(std::path::Path::new(&query.cwd))).await
}

#[derive(Deserialize)]
struct GitDiffQuery {
    cwd: String,
    scope: crate::git::GitDiffScope,
    path: Option<String>,
    commit: Option<String>,
    #[serde(default)]
    full_context: bool,
}

async fn git_diff(
    State(web): State<Web>,
    Query(query): Query<GitDiffQuery>,
) -> Result<Json<crate::git::GitDiff>, ApiError> {
    let request = crate::git::GitDiffRequest {
        scope: query.scope,
        path: query.path,
        commit: query.commit,
        full_context: query.full_context,
    };
    blocking(move || {
        web.service
            .git_diff(std::path::Path::new(&query.cwd), &request)
    })
    .await
}

#[derive(Deserialize)]
struct GitActionRequest {
    cwd: String,
    #[serde(flatten)]
    action: crate::git::GitAction,
}

async fn git_action(
    State(web): State<Web>,
    Json(request): Json<GitActionRequest>,
) -> Result<Json<crate::git::GitActionResult>, ApiError> {
    blocking(move || {
        web.service
            .git_action(std::path::Path::new(&request.cwd), &request.action)
    })
    .await
}

#[derive(Deserialize)]
struct GitCommitMessageRequest {
    cwd: String,
    #[serde(default)]
    amend: bool,
    provider: Option<Provider>,
    model: Option<String>,
    language: Option<String>,
}

async fn git_commit_message(
    State(web): State<Web>,
    Json(request): Json<GitCommitMessageRequest>,
) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let message = web.service.git_generate_commit_message(
            std::path::Path::new(&request.cwd),
            request.amend,
            request.provider,
            request.model.as_deref(),
            request.language.as_deref(),
        )?;
        Ok(json!({"message": message}))
    })
    .await
}

#[derive(Deserialize)]
struct QuotaResetRequest {
    idempotency_key: String,
}

#[derive(Serialize)]
struct QuotaResetResponse {
    outcome: engine::CodexQuotaResetOutcome,
    message: &'static str,
}

async fn reset_codex_quota(
    Json(request): Json<QuotaResetRequest>,
) -> Result<Json<QuotaResetResponse>, ApiError> {
    let idempotency_key = uuid::Uuid::parse_str(&request.idempotency_key)
        .ok()
        .filter(|key| !key.is_nil())
        .ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "无效的重置请求标识。".into()))?;
    blocking(move || {
        let outcome = engine::reset_codex_quota(idempotency_key)?;
        Ok(QuotaResetResponse {
            outcome,
            message: outcome.message(),
        })
    })
    .await
}

#[derive(Deserialize)]
struct ModelQuery {
    #[serde(default)]
    refresh: bool,
    #[serde(default)]
    settings: bool,
}

#[derive(Serialize)]
#[serde(untagged)]
enum ModelListing {
    Models(Vec<Model>),
    Catalog(ModelCatalog),
}

async fn models(
    State(web): State<Web>,
    Path(provider): Path<Provider>,
    Query(query): Query<ModelQuery>,
) -> Result<Json<ModelListing>, ApiError> {
    blocking(move || {
        let catalog = web.service.models(provider, query.refresh)?;
        Ok(if query.settings {
            ModelListing::Catalog(catalog)
        } else {
            ModelListing::Models(catalog.models)
        })
    })
    .await
}

async fn permission_mode(
    State(web): State<Web>,
    Path(provider): Path<Provider>,
    Query(target): Query<PermissionTarget>,
) -> Result<Json<Option<String>>, ApiError> {
    blocking(move || web.service.permission_mode(provider, &target)).await
}

#[derive(Deserialize)]
struct PermissionSelection {
    #[serde(flatten)]
    target: PermissionTarget,
    permission_mode: Option<String>,
}

async fn save_permission_mode(
    State(web): State<Web>,
    Path(provider): Path<Provider>,
    Json(selection): Json<PermissionSelection>,
) -> Result<Json<()>, ApiError> {
    blocking(move || {
        web.service.save_permission_mode(
            provider,
            &selection.target,
            selection.permission_mode.as_deref(),
        )
    })
    .await
}

#[derive(Serialize)]
struct History {
    session: Session,
    transcript: Option<WebTranscript>,
    active: Option<RunSummary>,
}

#[derive(Deserialize)]
struct HistoryQuery {
    #[serde(default)]
    refresh: bool,
    since: Option<String>,
}

#[derive(Serialize)]
struct WebTranscript {
    messages: Vec<WebMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    warning: Option<String>,
    revision: Option<String>,
}

#[derive(Serialize)]
struct WebMessage {
    #[serde(flatten)]
    message: Message,
    #[serde(skip_serializing_if = "Option::is_none")]
    rendered: Option<reply::Rendered>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<reply::AttachmentPreview>,
}

async fn transcript(
    State(web): State<Web>,
    Path((provider, id)): Path<(Provider, String)>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<History>, ApiError> {
    blocking(move || {
        let session = web.service.session(provider, &id)?;
        let active = web
            .service
            .running()
            .into_iter()
            .find(|run| run.provider == provider && run.session_id.as_deref() == Some(&id));
        let transcript = if !query.refresh
            && query.since.is_some()
            && query.since == web.previews.revision(session.revision())
        {
            None
        } else {
            let history = web
                .service
                .transcript(&session, query.refresh)
                .unwrap_or_else(|warning| Transcript {
                    warning: Some(warning),
                    ..Transcript::default()
                });
            let renderer = web.previews.for_workspace(&session.cwd);
            Some(WebTranscript {
                messages: history
                    .messages
                    .into_iter()
                    .map(|message| {
                        let rendered = (message.role == Role::Assistant)
                            .then(|| renderer.render_reply(&message.text, &message.work));
                        let attachments = message
                            .attachments
                            .iter()
                            .map(|attachment| web.previews.user_attachment(attachment))
                            .collect();
                        WebMessage {
                            message,
                            rendered,
                            attachments,
                        }
                    })
                    .collect(),
                model: history.model,
                warning: history.warning,
                revision: web.previews.revision(history.revision),
            })
        };
        Ok(History {
            session,
            transcript,
            active,
        })
    })
    .await
}

async fn start_turn(
    State(web): State<Web>,
    Json(request): Json<TurnRequest>,
) -> Result<Json<RunSummary>, ApiError> {
    blocking(move || web.service.start(request)).await
}

async fn activity(State(web): State<Web>) -> Json<Activity> {
    Json(web.service.activity())
}

async fn running(State(web): State<Web>) -> Json<Vec<RunSummary>> {
    Json(web.service.running())
}

#[derive(Serialize)]
struct LiveSnapshot {
    #[serde(flatten)]
    snapshot: Snapshot,
    replies: Vec<LiveReply>,
}

#[derive(Serialize)]
struct LiveReply {
    after: Option<String>,
    text: String,
    work: Work,
    rendered: reply::Rendered,
}

async fn turn(
    State(web): State<Web>,
    Path(id): Path<String>,
) -> Result<Json<LiveSnapshot>, ApiError> {
    let run = web.service.run(&id).map_err(ApiError::missing)?;
    blocking(move || Ok(live_snapshot(&web, run.cwd(), run.snapshot()))).await
}

fn live_snapshot(web: &Web, cwd: &std::path::Path, snapshot: Snapshot) -> LiveSnapshot {
    let renderer = web.previews.for_workspace(cwd);
    let replies = snapshot
        .replies()
        .into_iter()
        .filter(|reply| !reply.text.is_empty() || !reply.work.is_empty())
        .map(|reply| {
            let rendered = renderer.render_reply(&reply.text, reply.work);
            LiveReply {
                after: reply.after.map(str::to_owned),
                text: reply.text.into_owned(),
                work: reply.work.clone(),
                rendered,
            }
        })
        .collect();
    LiveSnapshot { snapshot, replies }
}

async fn follow_up(
    State(web): State<Web>,
    Path(id): Path<String>,
    Json(request): Json<FollowUpRequest>,
) -> Result<StatusCode, ApiError> {
    let run = web.service.run(&id).map_err(ApiError::missing)?;
    let _ = blocking(move || run.follow_up(request)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn steer_queued(
    State(web): State<Web>,
    Path((id, message_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let run = web.service.run(&id).map_err(ApiError::missing)?;
    run.steer_queued(&message_id)
        .map_err(|error| ApiError(StatusCode::CONFLICT, error))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct WithdrawnMessage {
    prompt: String,
    images: Vec<WithdrawnImage>,
}

#[derive(Serialize)]
struct WithdrawnImage {
    name: String,
    data: String,
}

async fn withdraw_queued(
    State(web): State<Web>,
    Path((id, message_id)): Path<(String, String)>,
) -> Result<Json<WithdrawnMessage>, ApiError> {
    let run = web.service.run(&id).map_err(ApiError::missing)?;
    let draft = run
        .withdraw_queued(&message_id, false)
        .map_err(|error| ApiError(StatusCode::CONFLICT, error))?;
    Ok(Json(WithdrawnMessage {
        prompt: draft.prompt,
        images: draft
            .images
            .iter()
            .map(|image| WithdrawnImage {
                name: image.name().to_owned(),
                data: STANDARD.encode(image.png_bytes()),
            })
            .collect(),
    }))
}

async fn image(
    State(web): State<Web>,
    Path((id, index)): Path<(String, usize)>,
) -> Result<Response, ApiError> {
    let run = web.service.run(&id).map_err(ApiError::missing)?;
    let image = run
        .image(index)
        .ok_or_else(|| ApiError::missing("图片不存在或已过期".into()))?;
    Ok((
        [(header::CONTENT_TYPE, image.media_type())],
        image.png_bytes().to_vec(),
    )
        .into_response())
}
async fn preview(State(web): State<Web>, Path(key): Path<String>) -> Result<Response, ApiError> {
    let image = tokio::task::spawn_blocking(move || web.previews.image(&key))
        .await
        .ok()
        .flatten()
        .ok_or_else(|| ApiError::missing("原图不存在或暂时无法预览".into()))?;
    Ok((
        [(header::CONTENT_TYPE, image.media_type())],
        image.png_bytes().to_vec(),
    )
        .into_response())
}

async fn file_preview(
    State(web): State<Web>,
    Path(key): Path<String>,
) -> Result<Json<reply::FilePreview>, ApiError> {
    tokio::task::spawn_blocking(move || web.previews.file(&key))
        .await
        .map_err(|_| ApiError(StatusCode::INTERNAL_SERVER_ERROR, "本机任务失败".into()))?
        .ok_or_else(|| ApiError::missing("文件预览已失效，请刷新会话。".into()))?
        .map(Json)
        .map_err(|error| ApiError(StatusCode::BAD_REQUEST, error.into()))
}

async fn stop_turn(State(web): State<Web>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    web.service.run(&id).map_err(ApiError::missing)?.stop();
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Decision {
    approval_id: String,
    allow: bool,
}

async fn approve(
    State(web): State<Web>,
    Path(id): Path<String>,
    Json(decision): Json<Decision>,
) -> Result<StatusCode, ApiError> {
    let run = web.service.run(&id).map_err(ApiError::missing)?;
    run.approve(&decision.approval_id, decision.allow)
        .map_err(|error| ApiError(StatusCode::CONFLICT, error))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct AnswerRequest {
    question_id: String,
    answers: QuestionAnswers,
}

async fn answer(
    State(web): State<Web>,
    Path(id): Path<String>,
    Json(request): Json<AnswerRequest>,
) -> Result<StatusCode, ApiError> {
    let run = web.service.run(&id).map_err(ApiError::missing)?;
    run.answer(&request.question_id, request.answers)
        .map_err(|error| ApiError(StatusCode::CONFLICT, error))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn blocking<T: Serialize + Send + 'static>(
    callback: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<Json<T>, ApiError> {
    tokio::task::spawn_blocking(callback)
        .await
        .map_err(|_| ApiError(StatusCode::INTERNAL_SERVER_ERROR, "本机任务失败".into()))?
        .map(Json)
        .map_err(|error| ApiError(StatusCode::BAD_REQUEST, error))
}

struct ApiError(StatusCode, String);

impl ApiError {
    fn missing(error: String) -> Self {
        Self(StatusCode::NOT_FOUND, error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.0,
            [(header::CACHE_CONTROL, "no-store")],
            Json(json!({"error":self.1})),
        )
            .into_response()
    }
}
