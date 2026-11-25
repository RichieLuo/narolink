mod cache;

use super::*;
use std::collections::{HashMap, HashSet};

const CONTROL: Duration = Duration::from_secs(18);

fn connect(cwd: &Path, cancel: &AtomicBool, session: Option<&str>) -> Result<Rpc, String> {
    let mut options = vec![
        "acp",
        "--process-per-session",
        "--include-default-customizations=false",
        "--disable-telemetry",
        "--sandbox",
    ];
    if session.is_some_and(cache::uses_app_storage) {
        options.extend(["--app-data-dir", "antigravity"]);
    }
    let mut rpc = Rpc::start(Provider::Antigravity, cwd, &options, true)?;
    let response = rpc.control("initialize", json!({
        "protocolVersion":1,"clientInfo":{"name":"narolink","version":env!("CARGO_PKG_VERSION")},
        "clientCapabilities":{}
    }), cancel)?;
    if response["protocolVersion"] != 1 {
        return Err("Antigravity CLI 的 ACP 版本不兼容".into());
    }
    Ok(rpc)
}

pub(super) fn inventory() -> Result<Inventory, String> {
    inventory_acp().or_else(|error| {
        let sessions = cache::inventory()?;
        Ok(Inventory {
            sessions,
            warning: Some(format!("Antigravity · {error} 本机索引仍可浏览。")),
        })
    })
}

fn inventory_acp() -> Result<Inventory, String> {
    let cancel = AtomicBool::new(false);
    let mut rpc = connect(&current_workspace(), &cancel, None)?;
    let mut cursor = Value::Null;
    let mut cursors = HashSet::new();
    let cached: HashMap<_, _> = cache::inventory()
        .unwrap_or_default()
        .into_iter()
        .map(|session| (session.id.clone(), session))
        .collect();
    let mut sessions = Vec::new();
    for _ in 0..20 {
        let page = rpc.control("session/list", json!({"cursor":cursor}), &cancel)?;
        sessions.extend(
            page["sessions"]
                .as_array()
                .ok_or("Antigravity 会话列表格式无效")?
                .iter()
                .filter_map(|row| {
                    let id = row["sessionId"].as_str().filter(|id| likely_id(id))?;
                    let stored = cached.get(id);
                    let cwd = row["cwd"]
                        .as_str()
                        .and_then(parse_path)
                        .or_else(|| stored.map(|entry| entry.cwd.clone()))?;
                    Some(Session {
                        provider: Provider::Antigravity,
                        id: id.into(),
                        cwd,
                        title: row["title"]
                            .as_str()
                            .map(|title| short(title, 150))
                            .filter(|title| !title.is_empty())
                            .unwrap_or_else(|| "Antigravity 会话".into()),
                        updated: row["updatedAt"]
                            .as_str()
                            .and_then(timestamp)
                            .unwrap_or_default(),
                        preview: stored.and_then(|entry| entry.preview.clone()),
                    })
                }),
        );
        let Some(next) = page["nextCursor"]
            .as_str()
            .filter(|cursor| cursors.insert(cursor.to_string()))
        else {
            return Ok(Inventory {
                sessions,
                warning: None,
            });
        };
        cursor = json!(next);
    }
    Ok(Inventory {
        sessions,
        warning: Some("已载入近期 Antigravity 会话。".into()),
    })
}

fn parse_path(text: &str) -> Option<PathBuf> {
    let path = if text.starts_with("file:") {
        url::Url::parse(text).ok()?.to_file_path().ok()?
    } else {
        // Windows ACP can return URI-style /C:/folder paths without a file scheme.
        #[cfg(windows)]
        let text = text
            .strip_prefix('/')
            .filter(|rest| {
                rest.as_bytes().get(1) == Some(&b':')
                    && rest.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
            })
            .unwrap_or(text);
        PathBuf::from(text)
    };
    path.is_absolute().then_some(path)
}

fn timestamp(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.timestamp())
}

pub(super) fn transcript(session: &Session) -> Result<Transcript, String> {
    let cancel = AtomicBool::new(false);
    let mut rpc = connect(&session.cwd, &cancel, Some(&session.id))?;
    let mut messages = Vec::<Message>::new();
    let mut budget = 8 * 1024 * 1024;
    let mut truncated = false;
    let reply = rpc.request(
        "session/load",
        json!({
            "sessionId":session.id,"cwd":session.cwd,"mcpServers":[]
        }),
        &cancel,
        Some(CONTROL),
        |rpc, frame| {
            let update = &frame["params"]["update"];
            if frame["method"] == "session/update"
                && frame["params"]["sessionId"] == session.id
                && let Some((role, text)) = chat_chunk(update)
            {
                if text.len() > budget {
                    truncated = true;
                } else {
                    budget -= text.len();
                    if let Some(last) = messages.last_mut().filter(|last| last.role == role) {
                        last.text.push_str(text);
                    } else {
                        messages.push(Message {
                            role,
                            text: text.into(),
                        });
                    }
                }
            }
            rpc.reject(frame)
        },
    )?;
    rpc_result(reply, "session/load")?;
    Ok(Transcript {
        messages,
        warning: truncated.then(|| "历史较长，网页和桌面端显示部分消息。".into()),
    })
}

fn chat_chunk(update: &Value) -> Option<(Role, &str)> {
    let role = match update["sessionUpdate"].as_str()? {
        "user_message_chunk" => Role::User,
        "agent_message_chunk" => Role::Assistant,
        _ => return None,
    };
    let content = &update["content"];
    let text = match content["type"].as_str()? {
        "text" => content["text"].as_str()?,
        "image" => "[图片]",
        _ => return None,
    };
    Some((role, text))
}

fn model_option(config: &Value) -> Option<&Value> {
    config["configOptions"]
        .as_array()?
        .iter()
        .find(|row| row["category"] == "model" || row["id"] == "model")
}

fn model_values(config: &Value) -> impl Iterator<Item = &Value> {
    model_option(config)
        .and_then(|row| row["options"].as_array())
        .into_iter()
        .flatten()
        .flat_map(|row| {
            row["options"]
                .as_array()
                .map_or(std::slice::from_ref(row), Vec::as_slice)
        })
}

pub(super) fn models() -> Result<Vec<Model>, String> {
    let cwd = std::env::temp_dir();
    let cancel = AtomicBool::new(false);
    let mut rpc = connect(&cwd, &cancel, None)?;
    let config = rpc.control("session/new", json!({"cwd":cwd,"mcpServers":[]}), &cancel)?;
    let models = model_values(&config)
        .filter_map(|row| {
            let id = row["value"].as_str()?;
            let label = row["name"].as_str().unwrap_or(id);
            Some(Model {
                id: id.into(),
                label: label.into(),
            })
        })
        .collect::<Vec<_>>();
    if let Some(id) = config["sessionId"].as_str() {
        let _ = rpc.send(json!({"id":900001,"method":"session/close","params":{"sessionId":id}}));
    }
    if models.is_empty() {
        return Err("Antigravity CLI 未报告可切换的模型".into());
    }
    Ok(models)
}

pub(super) fn run(request: &TurnRequest, turn: &Turn<'_>) -> Result<(), String> {
    let mut rpc = connect(&request.cwd, turn.cancel, request.session_id.as_deref())?;
    let mut params = json!({"cwd":request.cwd,"mcpServers":[]});
    let config = if let Some(id) = &request.session_id {
        params["sessionId"] = json!(id);
        let response = rpc.request(
            "session/resume",
            params.clone(),
            turn.cancel,
            Some(CONTROL),
            Rpc::reject,
        )?;
        if response["error"]["code"] == -32601 {
            rpc.control("session/load", params, turn.cancel)?
        } else {
            rpc_result(response, "session/resume")?
        }
    } else {
        rpc.control("session/new", params, turn.cancel)?
    };
    let session_id = config["sessionId"]
        .as_str()
        .or(request.session_id.as_deref())
        .filter(|id| likely_id(id))
        .ok_or("Antigravity CLI 未返回有效的会话 ID")?;
    if request
        .session_id
        .as_deref()
        .is_some_and(|id| id != session_id)
    {
        return Err("Antigravity CLI 返回了不同的会话，发送已取消".into());
    }
    turn.send(Event::Session(session_id.into()));
    if let Some(model) = &request.model {
        let option = model_option(&config).ok_or("Antigravity 的此会话未提供可切换的模型")?;
        if option["currentValue"] != *model {
            let config_id = option["id"]
                .as_str()
                .ok_or("Antigravity 模型配置格式无效")?;
            rpc.control(
                "session/set_config_option",
                json!({
                    "sessionId":session_id,"configId":config_id,"value":model
                }),
                turn.cancel,
            )?;
        }
    }
    let mut prompt = vec![json!({"type":"text","text":request.prompt})];
    prompt.extend(
        turn.images.iter().map(
            |image| json!({"type":"image","mimeType":image.media_type(),"data":image.base64()}),
        ),
    );
    let reply = rpc.request(
        "session/prompt",
        json!({"sessionId":session_id,
        "prompt":prompt}),
        turn.cancel,
        None,
        |rpc, frame| observe(rpc, frame, session_id, turn),
    );
    let reply = match reply {
        Ok(reply) => rpc_result(reply, "session/prompt")?,
        Err(error) => {
            if turn.cancel.load(Ordering::Acquire) {
                let _ =
                    rpc.send(json!({"method":"session/cancel","params":{"sessionId":session_id}}));
            }
            return Err(error);
        }
    };
    match reply["stopReason"].as_str() {
        Some("end_turn") => Ok(()),
        Some("cancelled") => Err("Antigravity 已停止".into()),
        Some(reason) => Err(format!("Antigravity · {}", short(reason, 120))),
        None => Err("Antigravity 未返回结束原因".into()),
    }
}

fn observe(rpc: &Rpc, frame: Value, session: &str, turn: &Turn<'_>) -> Result<(), String> {
    let params = &frame["params"];
    if params["sessionId"] != session {
        return rpc.reject(frame);
    }
    match frame["method"].as_str() {
        Some("session/update") => {
            let update = &params["update"];
            if let Some((Role::Assistant, text)) = chat_chunk(update) {
                turn.send(Event::Text(text.into()));
            } else if update["sessionUpdate"] == "tool_call" {
                let label = update["title"].as_str().unwrap_or("正在执行工具");
                turn.send(Event::Activity(format!(
                    "Antigravity · {}",
                    short(label, 90)
                )));
            }
        }
        Some("session/request_permission") => {
            let call = &params["toolCall"];
            let allow = params["options"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|option| option["kind"] == "allow_once")
                .and_then(|option| option["optionId"].as_str());
            let deny = params["options"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|option| option["kind"] == "reject_once")
                .and_then(|option| option["optionId"].as_str());
            let approved = allow.is_some()
                && turn.approval(
                    call["title"].as_str().unwrap_or("Antigravity 操作"),
                    serde_json::to_string_pretty(call).unwrap_or_default(),
                )?;
            let outcome = if approved { allow } else { deny }.map_or_else(
                || json!({"outcome":"cancelled"}),
                |id| json!({"outcome":"selected","optionId":id}),
            );
            rpc.send(json!({"id":frame["id"],"result":{"outcome":outcome}}))?;
        }
        _ => return rpc.reject(frame),
    }
    Ok(())
}
