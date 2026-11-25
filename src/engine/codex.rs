use super::*;
use std::collections::HashSet;

const HANDSHAKE: Duration = Duration::from_secs(18);

fn connect(cwd: &Path, cancel: &AtomicBool, editing: bool) -> Result<Rpc, String> {
    let mut rpc = Rpc::start(
        Provider::Codex,
        cwd,
        &[
            "app-server",
            "--listen",
            "stdio://",
            "-c",
            "features.hooks=false",
            "-c",
            "features.codex_hooks=false",
            "-c",
            "features.plugins=false",
            "-c",
            "features.apps=false",
            "-c",
            "mcp_servers={}",
            "-c",
            if editing {
                "sandbox_mode=\"workspace-write\""
            } else {
                "sandbox_mode=\"read-only\""
            },
        ],
        false,
    )?;
    rpc.control(
        "initialize",
        json!({
            "clientInfo":{"name":"narolink","title":"NaroLink","version":env!("CARGO_PKG_VERSION")},
            "capabilities":{"experimentalApi":true}
        }),
        cancel,
    )?;
    rpc.send(json!({"method":"initialized"}))?;
    Ok(rpc)
}

pub(super) fn inventory() -> Result<Inventory, String> {
    let cancel = AtomicBool::new(false);
    let mut rpc = connect(&current_workspace(), &cancel, false)?;
    let mut inventory = Inventory::default();
    let mut seen = HashSet::new();
    let mut ids = HashSet::new();
    let mut cursor = Value::Null;
    for _ in 0..20 {
        let page = rpc.control(
            "thread/list",
            json!({
                "limit":100, "cursor":cursor, "archived":false,
                "sourceKinds":["cli","vscode","exec","appServer","unknown"]
            }),
            &cancel,
        )?;
        inventory.sessions.extend(
            page["data"]
                .as_array()
                .ok_or("Codex 历史列表格式无效")?
                .iter()
                .filter_map(session)
                .filter(|session| ids.insert(session.id.clone())),
        );
        let Some(next) = page["nextCursor"]
            .as_str()
            .filter(|cursor| seen.insert(cursor.to_string()))
        else {
            return Ok(inventory);
        };
        cursor = json!(next);
    }
    inventory.warning = Some("已载入最新的 2,000 条 Codex 会话。".into());
    Ok(inventory)
}

fn session(row: &Value) -> Option<Session> {
    let id = row["id"].as_str().filter(|id| likely_id(id))?;
    let cwd = PathBuf::from(row["cwd"].as_str()?);
    if !cwd.is_absolute()
        || row["parentThreadId"].is_string()
        || row["ephemeral"] == true
        || row["canAcceptDirectInput"] == false
    {
        return None;
    }
    let preview = row["preview"].as_str().map(|text| short(text, 300));
    let title = row["name"]
        .as_str()
        .map(|text| short(text, 160))
        .filter(|text| !text.is_empty())
        .or_else(|| {
            preview
                .as_ref()
                .filter(|text| !text.is_empty())
                .map(|text| short(text, 160))
        })
        .unwrap_or_else(|| "Codex 会话".into());
    Some(Session {
        provider: Provider::Codex,
        id: id.into(),
        title,
        cwd,
        preview,
        updated: row["updatedAt"].as_i64().unwrap_or_default(),
    })
}

pub(super) fn transcript(session: &Session) -> Result<Transcript, String> {
    let cancel = AtomicBool::new(false);
    let mut rpc = connect(&current_workspace(), &cancel, false)?;
    let mut messages = Vec::new();
    let mut cursor = Value::Null;
    let mut cursors = HashSet::new();
    let mut budget = 6 * 1024 * 1024;
    for page_number in 0..30 {
        let reply = rpc.request(
            "thread/items/list",
            json!({
                "threadId":session.id,"limit":48,"cursor":cursor,"sortDirection":"desc"
            }),
            &cancel,
            Some(HANDSHAKE),
            Rpc::reject,
        )?;
        if page_number == 0 && reply.get("error").is_some() {
            let legacy = rpc.control(
                "thread/read",
                json!({"threadId":session.id,"includeTurns":true}),
                &cancel,
            )?;
            let items = legacy["thread"]["turns"]
                .as_array()
                .into_iter()
                .flatten()
                .rev()
                .flat_map(|turn| turn["items"].as_array().into_iter().flatten().rev());
            let mut truncated = false;
            for item in items.filter_map(message) {
                if item.text.len() > budget {
                    truncated = true;
                    break;
                }
                budget -= item.text.len();
                messages.push(item);
            }
            messages.reverse();
            return Ok(Transcript {
                messages,
                warning: truncated.then(partial_history),
            });
        }
        let page = rpc_result(reply, "thread/items/list")?;
        for item in page["data"]
            .as_array()
            .ok_or("Codex 消息格式无效")?
            .iter()
            .filter_map(|entry| message(&entry["item"]))
        {
            if item.text.len() > budget {
                messages.reverse();
                return Ok(Transcript {
                    messages,
                    warning: Some(partial_history()),
                });
            }
            budget -= item.text.len();
            messages.push(item);
        }
        let Some(next) = page["nextCursor"]
            .as_str()
            .filter(|cursor| cursors.insert(cursor.to_string()))
        else {
            messages.reverse();
            return Ok(Transcript {
                messages,
                warning: None,
            });
        };
        cursor = json!(next);
    }
    messages.reverse();
    Ok(Transcript {
        messages,
        warning: Some(partial_history()),
    })
}

fn partial_history() -> String {
    "仅显示本会话近期消息。续聊时由 Codex 恢复原有上下文。".into()
}

fn message(item: &Value) -> Option<Message> {
    let (role, text) = match item["type"].as_str()? {
        "userMessage" => (
            Role::User,
            item["content"]
                .as_array()?
                .iter()
                .filter_map(|part| {
                    if part["type"] == "text" {
                        part["text"].as_str()
                    } else if matches!(part["type"].as_str(), Some("image" | "localImage")) {
                        Some("[图片]")
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        "agentMessage" => (Role::Assistant, item["text"].as_str()?.to_owned()),
        _ => return None,
    };
    (!text.trim().is_empty()).then_some(Message { role, text })
}

pub(super) fn models() -> Result<Vec<Model>, String> {
    let cancel = AtomicBool::new(false);
    let mut rpc = connect(&current_workspace(), &cancel, false)?;
    let mut models = Vec::new();
    let mut seen = HashSet::new();
    let mut cursor = Value::Null;
    for _ in 0..12 {
        let page = rpc.control(
            "model/list",
            json!({"limit":100,"cursor":cursor,"includeHidden":false}),
            &cancel,
        )?;
        models.extend(
            page["data"]
                .as_array()
                .ok_or("Codex 模型列表格式无效")?
                .iter()
                .filter_map(|row| {
                    let id = row["model"].as_str().or_else(|| row["id"].as_str())?;
                    let label = row["displayName"].as_str().unwrap_or(id);
                    Some(Model {
                        id: id.into(),
                        label: label.into(),
                    })
                }),
        );
        let Some(next) = page["nextCursor"]
            .as_str()
            .filter(|cursor| seen.insert(cursor.to_string()))
        else {
            break;
        };
        cursor = json!(next);
    }
    Ok(models)
}

pub(super) fn run(request: &TurnRequest, turn: &Turn<'_>) -> Result<(), String> {
    let mut rpc = connect(&request.cwd, turn.cancel, true)?;
    let mut params = json!({
        "cwd":request.cwd,"sandbox":"workspace-write","approvalPolicy":"never",
        "developerInstructions":"Work in the user's selected local workspace. Do not try to bypass the workspace sandbox."
    });
    if let Some(model) = &request.model {
        params["model"] = json!(model);
    }
    let method = if let Some(id) = &request.session_id {
        params["threadId"] = json!(id);
        params["excludeTurns"] = json!(true);
        "thread/resume"
    } else {
        "thread/start"
    };
    let opened = rpc.control(method, params, turn.cancel)?;
    let id = opened["thread"]["id"]
        .as_str()
        .filter(|id| likely_id(id))
        .ok_or("Codex 未返回会话 ID")?;
    if request
        .session_id
        .as_deref()
        .is_some_and(|requested| requested != id)
    {
        return Err("Codex 返回了不同的会话，发送已取消".into());
    }
    turn.send(Event::Session(id.into()));
    let mut input = vec![json!({"type":"text","text":request.prompt,"text_elements":[]})];
    input.extend(
        turn.images
            .iter()
            .map(|image| json!({"type":"image","url":image.data_url()})),
    );
    let mut params = json!({
        "threadId":id,"cwd":request.cwd,"input":input,
        "approvalPolicy":"never","sandboxPolicy":{"type":"workspaceWrite","writableRoots":[],
            "networkAccess":false,"excludeTmpdirEnvVar":true,"excludeSlashTmp":true}
    });
    if let Some(model) = &request.model {
        params["model"] = json!(model);
    }
    let mut output = Output::default();
    let reply = rpc.request(
        "turn/start",
        params,
        turn.cancel,
        Some(HANDSHAKE),
        |rpc, frame| output.observe(rpc, frame, id, turn),
    )?;
    let reply = rpc_result(reply, "turn/start")?;
    if let Some(result) = output.complete.take() {
        return result;
    }
    let turn_id = reply["turn"]["id"].as_str().unwrap_or_default();
    loop {
        match receive(&rpc.process, turn.cancel, None) {
            Ok(frame) => {
                output.observe(&rpc, frame, id, turn)?;
                if let Some(result) = output.complete.take() {
                    return result;
                }
            }
            Err(error) => {
                if turn.cancel.load(Ordering::Acquire) {
                    let _ = rpc.send(json!({"id":900000,"method":"turn/interrupt","params":{"threadId":id,"turnId":turn_id}}));
                }
                return Err(error);
            }
        }
    }
}

#[derive(Default)]
struct Output {
    last_item: String,
    text: String,
    complete: Option<Result<(), String>>,
}

impl Output {
    fn observe(
        &mut self,
        rpc: &Rpc,
        frame: Value,
        id: &str,
        turn: &Turn<'_>,
    ) -> Result<(), String> {
        let params = &frame["params"];
        if params["threadId"]
            .as_str()
            .is_some_and(|thread| thread != id)
        {
            return rpc.reject(frame);
        }
        match frame["method"].as_str().unwrap_or_default() {
            "item/agentMessage/delta" => {
                let delta = params["delta"].as_str().unwrap_or_default();
                self.append(
                    params["itemId"].as_str().unwrap_or_default(),
                    delta,
                    false,
                    turn,
                );
            }
            "item/completed" if params["item"]["type"] == "agentMessage" => {
                let item = &params["item"];
                self.append(
                    item["id"].as_str().unwrap_or_default(),
                    item["text"].as_str().unwrap_or_default(),
                    true,
                    turn,
                );
            }
            "item/started" => {
                let activity = match params["item"]["type"].as_str() {
                    Some("commandExecution") => Some("Codex 正在运行命令"),
                    Some("fileChange") => Some("Codex 正在修改文件"),
                    _ => None,
                };
                if let Some(activity) = activity {
                    turn.send(Event::Activity(activity.into()));
                }
            }
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                rpc.send(json!({"id":frame["id"],"result":{"decision":"decline"}}))?;
            }
            "item/permissions/requestApproval" => {
                rpc.send(json!({"id":frame["id"],"result":{"permissions":{},"scope":"turn"}}))?;
            }
            "turn/completed" => {
                self.complete = Some(match params["turn"]["status"].as_str() {
                    Some("completed") => Ok(()),
                    Some("interrupted") => Err("Codex 会话已中断".into()),
                    _ => Err(format!(
                        "Codex · {}",
                        short(
                            params["turn"]["error"]["message"]
                                .as_str()
                                .unwrap_or("此次回复未完成。请确认 CLI 登录和选定模型。"),
                            240
                        )
                    )),
                });
            }
            _ => rpc.reject(frame)?,
        }
        Ok(())
    }

    fn append(&mut self, id: &str, text: &str, snapshot: bool, turn: &Turn<'_>) {
        if text.is_empty() {
            return;
        }
        if self.last_item != id {
            if !self.text.is_empty() {
                turn.send(Event::Text("\n\n".into()));
            }
            id.clone_into(&mut self.last_item);
            self.text.clear();
        }
        let delta = if snapshot {
            text.strip_prefix(&self.text).unwrap_or_default()
        } else {
            text
        };
        if !delta.is_empty() {
            if self.text.len() < 4 * 1024 * 1024 {
                self.text.push_str(delta);
            }
            turn.send(Event::Text(delta.into()));
        }
    }
}
