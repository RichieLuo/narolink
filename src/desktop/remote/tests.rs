use super::*;
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::Request,
    response::{IntoResponse, Response as AxumResponse},
};

const ID: &str = "0123456789abcdef0123456789abcdef";
const COOKIE: &str = "narolink_device=scoped-token";
const FILE_KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const LINK_TARGET: &str = "/Users/mac/WorkSpace/%E8%AF%B4%E6%98%8E.md:2";

#[test]
fn pairing_input_keeps_only_the_device_address_and_valid_code() {
    let base = format!("https://example.test/devices/{ID}");
    let (address, code) = pairing_input(&format!("{base}#pair=00123456"), "").unwrap();
    assert_eq!(address.as_str(), format!("{base}/"));
    assert_eq!(code, "00123456");
    assert_eq!(
        endpoint(&address, &["sessions", "codex", "id/behind?query"]).as_str(),
        format!("{base}/api/sessions/codex/id%2Fbehind%3Fquery"),
    );
    assert!(pairing_input(&base, "").is_err());
    assert!(pairing_input(&format!("{base}#pair=abcdef12"), "").is_err());
    assert!(pairing_input(&format!("{base}?name=mac"), "00123456").is_err());
    assert!(pairing_input("https://user@example.test/", "00123456").is_err());
    assert!(pairing_input("http://192.168.1.2/", "00123456").is_err());
}

#[test]
fn remote_wire_preserves_work_and_never_converts_host_files_into_local_paths() {
    let transcript: RemoteTranscript = serde_json::from_value(json!({
        "messages": [{
            "role": "user", "text": "Look at this", "turn_id": "turn-1",
            "local_files": [{"path": "/private/file.txt", "directory": false}],
            "attachments": [
                {"name": "preview.png", "url": "/api/previews/inline-0123"},
                {"name": "external.png", "url": "https://example.test/api/previews/secret"}
            ]
        }, {
            "role": "assistant", "text": "Here is the output",
            "work": {"images": ["/private/output.png"]},
            "rendered": {
                "previews": [{"name": "output.png", "url": "/api/previews/0123"}],
                "file_links": {(LINK_TARGET): FILE_KEY, "other.md": "../../logout"}
            }
        }]
    }))
    .unwrap();
    let transcript = Transcript::from(transcript);
    assert!(transcript.messages[0].local_files.is_empty());
    assert!(matches!(
        &transcript.messages[0].attachments[0].image,
        Some(HistoryImage::Remote(key)) if key == "inline-0123"
    ));
    assert!(transcript.messages[0].attachments[1].image.is_none());
    assert!(transcript.messages[1].work.images.is_empty());
    assert_eq!(transcript.messages[1].work.remote_previews.len(), 1);
    assert_eq!(
        transcript.messages[1].work.remote_file_links,
        BTreeMap::from([(LINK_TARGET.into(), FILE_KEY.into())])
    );

    let live: LiveSnapshot = serde_json::from_value(json!({
        "id": "run", "provider": "codex", "title": "Run",
        "status": "running", "prompt": "Start", "output": "First", "images": [], "activity": "",
        "follow_ups": [{
            "id": "follow", "mode": "queue", "delivery": "sent", "prompt": "Next",
            "images": [], "image_start": 0, "output": "Second"
        }],
        "replies": [{
            "after": null,
            "work": {"steps": [{"id": "thinking", "kind": "thinking", "status": "running"}]},
            "rendered": {"file_links": {"README.md": FILE_KEY}}
        }, {
            "after": "follow", "work": {"plan": [{"step": "Finish", "status": "pending"}]},
            "rendered": {
                "previews": [{"name": "unsafe.png", "url": "/api/previews/../../logout"}],
                "file_links": {(LINK_TARGET): FILE_KEY}
            }
        }]
    }))
    .unwrap();
    let snapshot = Snapshot::from(live);
    assert_eq!(snapshot.work.steps[0].id, "thinking");
    assert_eq!(snapshot.work.remote_file_links["README.md"], FILE_KEY);
    assert_eq!(snapshot.follow_ups[0].work.plan[0].step, "Finish");
    assert!(snapshot.follow_ups[0].work.remote_previews.is_empty());
    assert_eq!(
        snapshot.follow_ups[0].work.remote_file_links[LINK_TARGET],
        FILE_KEY
    );
}

#[test]
fn remote_client_uses_scoped_cookie_and_existing_control_endpoints() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let (shutdown, done) = tokio::sync::oneshot::channel();
    let server = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                axum::serve(listener, Router::new().fallback(device_api))
                    .with_graceful_shutdown(async {
                        let _ = done.await;
                    })
                    .await
                    .unwrap();
            });
    });

    let address = format!("{origin}/devices/{ID}/");
    let client = RemoteClient::connect(&address, "00123456").unwrap();
    assert_eq!(client.name(), "Test Mac");
    assert_eq!(client.address(), address);
    assert!(
        client
            .inventory(Provider::Codex, true)
            .unwrap()
            .sessions
            .is_empty()
    );
    assert!(client.snapshot("missing").unwrap().is_none());
    assert!(client.snapshot("redirect").is_err());
    assert_eq!(client.image("active", 0).unwrap(), b"test png");
    let file = client.file_preview(FILE_KEY).unwrap();
    assert_eq!(file.name, "说明.md");
    assert_eq!(file.text, "# Test Mac\n[Details](docs/details.md)\n");
    assert_eq!(
        file.file_links,
        BTreeMap::from([("docs/details.md".into(), FILE_KEY.into())])
    );
    assert!(client.file_preview("../../logout").is_err());
    client.stop("active").unwrap();

    shutdown.send(()).unwrap();
    server.join().unwrap();
}

async fn device_api(request: Request) -> AxumResponse {
    let scope = format!("/devices/{ID}/");
    let path = request.uri().path().strip_prefix(&scope).unwrap();
    let cookie = request.headers().get(header::COOKIE);
    if path == "api/pair" {
        assert!(cookie.is_none());
        assert_eq!(request.method(), Method::POST);
        assert_origins_match(&request);
        let body = to_bytes(request.into_body(), 4096).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({"code": "00123456"})
        );
        let cookie = format!("{COOKIE}; Path={scope}; Max-Age=2592000; HttpOnly; SameSite=Strict");
        return (
            [(header::SET_COOKIE, cookie)],
            Json(json!({"paired": true})),
        )
            .into_response();
    }
    assert_eq!(cookie.unwrap(), COOKIE);
    match path {
        "api/access" => {
            Json(json!({"paired": true, "online": true, "name": "Test Mac"})).into_response()
        }
        "api/sessions/codex" => {
            assert_eq!(request.uri().query(), Some("refresh=true"));
            Json(json!({"sessions": [], "warning": null})).into_response()
        }
        "api/runs/missing" => StatusCode::NOT_FOUND.into_response(),
        "api/runs/redirect" => (
            StatusCode::TEMPORARY_REDIRECT,
            [(header::LOCATION, format!("{scope}api/runs/missing"))],
        )
            .into_response(),
        "api/runs/active/images/0" => AxumResponse::builder()
            .header(header::CONTENT_TYPE, "image/png")
            .body(Body::from("test png"))
            .unwrap(),
        path if path.strip_prefix("api/file-previews/") == Some(FILE_KEY) => {
            assert_eq!(request.method(), Method::GET);
            Json(json!({
                "name": "说明.md",
                "text": "# Test Mac\n[Details](docs/details.md)\n",
                "file_links": {"docs/details.md": FILE_KEY, "other.md": "../../logout"}
            }))
            .into_response()
        }
        "api/runs/active/stop" => {
            assert_eq!(request.method(), Method::POST);
            assert_origins_match(&request);
            let body = to_bytes(request.into_body(), 4096).await.unwrap();
            assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), json!({}));
            StatusCode::NO_CONTENT.into_response()
        }
        _ => panic!("unexpected API path: {path}"),
    }
}

fn assert_origins_match(request: &Request) {
    let host = request
        .headers()
        .get(header::HOST)
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(
        request.headers().get(header::ORIGIN).unwrap(),
        &format!("http://{host}")
    );
}
