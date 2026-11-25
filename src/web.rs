mod auth;
mod remote;

use crate::{
    engine::{Inventory, Model, Provider, Session, Transcript, TurnRequest},
    service::{Activity, RunSummary, Service, Snapshot},
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
    local: String,
}

pub struct Gateway {
    web: Web,
    runtime: Option<tokio::runtime::Runtime>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Gateway {
    pub fn start(service: Service, options: Options) -> Result<Self, String> {
        let auth = Arc::new(Auth::load()?);
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, options.port))
            .map_err(|error| {
                format!("无法启动 NaroLink 网页服务。请检查是否已经打开。\n{error}")
            })?;
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
        let web = Web {
            service,
            auth,
            remote: remote::Remote::new(local.clone(), options.public_url),
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
                    include_bytes!("../public/index.html"),
                )
            }),
        )
        .route(
            "/app.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_bytes!("../public/app.js"),
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async {
                asset(
                    "text/css; charset=utf-8",
                    include_bytes!("../public/style.css"),
                )
            }),
        )
        .route(
            "/manifest.webmanifest",
            get(|| async {
                asset(
                    "application/manifest+json",
                    include_bytes!("../public/manifest.webmanifest"),
                )
            }),
        )
        .route(
            "/sw.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_bytes!("../public/sw.js"),
                )
            }),
        )
        .route(
            "/icons/180.png",
            get(|| async { asset("image/png", include_bytes!("../public/icons/180.png")) }),
        )
        .route(
            "/icons/192.png",
            get(|| async { asset("image/png", include_bytes!("../public/icons/192.png")) }),
        )
        .route(
            "/icons/512.png",
            get(|| async { asset("image/png", include_bytes!("../public/icons/512.png")) }),
        )
        .route("/api/access", get(access))
        .route("/api/activity", get(activity))
        .route("/api/pair", post(pair))
        .route("/api/logout", post(logout))
        .route("/api/sessions/{provider}", get(sessions))
        .route("/api/sessions/{provider}/{id}", get(transcript))
        .route("/api/models/{provider}", get(models))
        .route("/api/runs", get(running).post(start_turn))
        .route("/api/runs/{id}", get(turn))
        .route("/api/runs/{id}/images/{index}", get(image))
        .route("/api/runs/{id}/stop", post(stop_turn))
        .route("/api/runs/{id}/approval", post(approve))
        .layer(DefaultBodyLimit::max(18 * 1024 * 1024))
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
        "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' blob:; connect-src 'self'; \
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

async fn models(
    State(web): State<Web>,
    Path(provider): Path<Provider>,
    Query(query): Query<Refresh>,
) -> Result<Json<Vec<Model>>, ApiError> {
    blocking(move || web.service.models(provider, query.refresh)).await
}

#[derive(Serialize)]
struct History {
    session: Session,
    transcript: Transcript,
    active: Option<RunSummary>,
}

async fn transcript(
    State(web): State<Web>,
    Path((provider, id)): Path<(Provider, String)>,
    Query(query): Query<Refresh>,
) -> Result<Json<History>, ApiError> {
    blocking(move || {
        let session = web.service.session(provider, &id)?;
        let transcript = web
            .service
            .transcript(&session, query.refresh)
            .unwrap_or_else(|warning| Transcript {
                messages: vec![],
                warning: Some(warning),
            });
        let active = web
            .service
            .running()
            .into_iter()
            .find(|run| run.provider == provider && run.session_id.as_deref() == Some(&id));
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

async fn turn(State(web): State<Web>, Path(id): Path<String>) -> Result<Json<Snapshot>, ApiError> {
    Ok(Json(
        web.service.run(&id).map_err(ApiError::missing)?.snapshot(),
    ))
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
