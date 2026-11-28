mod cache;

use super::attachments::{HistoryImage, InlineImage, MAX_ENCODED_IMAGE_BYTES, UserAttachment};
use super::work::{Agent, AgentStatus, MAX_STEP_TEXT, Step, StepStatus};
use super::*;
use std::collections::{HashMap, HashSet};

const CONTROL: Duration = Duration::from_secs(18);

fn connect(cwd: &Path, cancel: &AtomicBool, session: Option<&str>) -> Result<Rpc, String> {
    connect_with_agent(cwd, cancel, session, None)
}

fn connect_with_agent(
    cwd: &Path,
    cancel: &AtomicBool,
    session: Option<&str>,
    agent: Option<&str>,
) -> Result<Rpc, String> {
    let mut options = vec![
        "acp",
        "--process-per-session",
        "--disable-telemetry",
        "--sandbox",
    ];
    if let Some(agent) = agent {
        options.extend(["--agent", agent]);
    }
    if session.is_some_and(cache::uses_app_storage) {
        options.extend(["--app-data-dir", "antigravity"]);
    }
    let mut rpc = Rpc::start(Provider::Antigravity, cwd, &options, true)?;
    let response = rpc.control("initialize", json!({
        "protocolVersion":1,"clientInfo":{"name":"narolink","version":env!("CARGO_PKG_VERSION")},
        "clientCapabilities":{"elicitation":{"form":{}}}
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
            ..Inventory::default()
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
                    let title = row["title"]
                        .as_str()
                        .map(|title| short(title, 150))
                        .filter(|title| !title.is_empty())
                        .or_else(|| {
                            stored
                                .filter(|entry| entry.named)
                                .map(|entry| entry.title.clone())
                        });
                    Some(Session {
                        provider: Provider::Antigravity,
                        id: id.into(),
                        cwd,
                        project_id: None,
                        model: None,
                        rollout: None,
                        named: title.is_some(),
                        title: title.unwrap_or_else(|| "Antigravity 会话".into()),
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
            for session in cached.values() {
                if !sessions.iter().any(|existing| existing.id == session.id) {
                    sessions.push(session.clone());
                }
            }
            return Ok(Inventory {
                sessions,
                ..Inventory::default()
            });
        };
        cursor = json!(next);
    }
    Ok(Inventory {
        sessions,
        warning: Some("已载入近期 Antigravity 会话。".into()),
        ..Inventory::default()
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
        let path = PathBuf::from(text);
        if text.contains('%') && !path.is_dir() {
            // ACP can return a percent-encoded path without its file scheme.
            percent_encoding::percent_decode_str(text)
                .decode_utf8()
                .ok()
                .map(|decoded| PathBuf::from(decoded.as_ref()))
                .filter(|decoded| decoded.is_absolute() && decoded.is_dir())
                .unwrap_or(path)
        } else {
            path
        }
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
    let mut agent_calls = HashSet::new();
    let mut thoughts = ThoughtStream::new(session.id.clone());
    let mut budget = 8 * 1024 * 1024;
    let mut image_budget = MAX_ENCODED_IMAGE_BYTES;
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
            if frame["method"] != "session/update" || frame["params"]["sessionId"] != session.id {
                return rpc.reject(frame);
            }
            if let Some(text) = thought_chunk(update) {
                let visible = cut(text, budget);
                budget -= visible.len();
                truncated |= visible.len() != text.len();
                thoughts.append(visible);
            } else if thought_boundary(update)
                && let Some(step) = thoughts.finish()
            {
                record_thought(&mut messages, step);
            }
            let changes = thoughts.changes(update);
            if !changes.is_empty() {
                if messages
                    .last()
                    .is_none_or(|entry| entry.role != Role::Assistant)
                {
                    messages.push(Message::new(Role::Assistant, String::new()));
                }
                messages.last_mut().unwrap().work.add_changes(changes);
            }
            if update["sessionUpdate"] == "usage_update" {
                if messages
                    .last()
                    .is_none_or(|entry| entry.role != Role::Assistant)
                {
                    messages.push(Message::new(Role::Assistant, String::new()));
                }
                messages.last_mut().unwrap().work.context = Some(work::ContextUsage::new(
                    update["used"].as_u64(),
                    update["size"].as_u64(),
                ));
            }
            if let Some(agent) = subagent_tool(update, &mut agent_calls) {
                let existing = messages
                    .iter()
                    .rposition(|entry| entry.work.agents.iter().any(|item| item.id == agent.id));
                let index = existing.unwrap_or_else(|| {
                    if messages
                        .last()
                        .is_none_or(|entry| entry.role != Role::Assistant)
                    {
                        messages.push(Message::new(Role::Assistant, String::new()));
                    }
                    messages.len() - 1
                });
                messages[index].work.record_agent(agent);
            } else if let Some((role, text)) = chat_chunk(update) {
                let content = &update["content"];
                if role == Role::User && content["type"] == "image" {
                    if messages.last().is_none_or(|last| last.role != role) {
                        messages.push(Message::new(role, String::new()));
                    }
                    let attachments = &mut messages.last_mut().unwrap().attachments;
                    if attachments.len() < 16 {
                        let image = content["mimeType"]
                            .as_str()
                            .zip(content["data"].as_str())
                            .and_then(|(mime_type, data)| {
                                InlineImage::from_base64(mime_type, data, &mut image_budget)
                            })
                            .map(HistoryImage::Inline);
                        attachments.push(UserAttachment {
                            name: format!("图片 {}", attachments.len() + 1),
                            image,
                        });
                    }
                } else if text.len() > budget {
                    truncated = true;
                } else {
                    budget -= text.len();
                    if let Some(last) = messages.last_mut().filter(|last| last.role == role) {
                        last.text.push_str(text);
                    } else {
                        messages.push(Message::new(role, text.into()));
                    }
                }
            }
            rpc.reject(frame)
        },
    )?;
    let config = rpc_result(reply, "session/load")?;
    if let Some(step) = thoughts.finish() {
        record_thought(&mut messages, step);
    }
    for message in &mut messages {
        message.work.finish_agents(false);
        if message.local_files.is_empty() {
            message.restore_local_files();
        }
    }
    Ok(Transcript {
        messages,
        model: current_model(&config).map(str::to_owned),
        warning: truncated.then(|| "历史较长，网页和桌面端显示部分消息。".into()),
        revision: None,
    })
}

fn subagent_tool(update: &Value, calls: &mut HashSet<String>) -> Option<Agent> {
    let id = update["toolCallId"]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 128)?;
    match update["sessionUpdate"].as_str()? {
        "tool_call" if update["name"] == "invoke_subagent" => {
            calls.insert(id.into());
        }
        "tool_call_update" if calls.contains(id) => {}
        _ => return None,
    }
    let input = update.get("rawInput").unwrap_or(&update["args"]);
    let status = match update["status"].as_str() {
        Some("pending" | "in_progress") => Some(AgentStatus::Running),
        // ACP confirms the launch call, while the child can continue independently.
        Some("completed") => Some(AgentStatus::Started),
        Some("failed") => Some(AgentStatus::Failed),
        _ => None,
    };
    Some(Agent::new(
        id,
        input["TypeName"]
            .as_str()
            .filter(|name| !name.is_empty())
            .or_else(|| input["Role"].as_str()),
        input["Prompt"].as_str(),
        status,
    ))
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

fn thought_chunk(update: &Value) -> Option<&str> {
    let content = &update["content"];
    content["text"]
        .as_str()
        .filter(|_| update["sessionUpdate"] == "agent_thought_chunk" && content["type"] == "text")
}

fn thought_boundary(update: &Value) -> bool {
    matches!(
        update["sessionUpdate"].as_str(),
        Some("user_message_chunk" | "agent_message_chunk" | "tool_call" | "tool_call_update")
    )
}

struct ThoughtStream {
    scope: String,
    sequence: usize,
    active: Option<Step>,
    edits: HashSet<String>,
}

impl ThoughtStream {
    fn new(scope: String) -> Self {
        Self {
            scope,
            sequence: 0,
            active: None,
            edits: HashSet::new(),
        }
    }

    fn changes(&mut self, update: &Value) -> Vec<work::FileChange> {
        if !matches!(
            update["sessionUpdate"].as_str(),
            Some("tool_call" | "tool_call_update")
        ) {
            return vec![];
        }
        // Pending updates may be replayed with the same payload on completion.
        if update["status"] == "failed" {
            return vec![];
        }
        update["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|content| {
                if content["type"] != "diff" {
                    return None;
                }
                let path = content["path"].as_str()?;
                let old = content["oldText"].as_str().unwrap_or_default();
                let new = content["newText"].as_str()?;
                let key = json!([update["toolCallId"], path, old, new]).to_string();
                if !self.edits.insert(key) {
                    return None;
                }
                let mut diff = format!(
                    "--- {path}\n+++ {path}\n@@ -1,{} +1,{} @@\n",
                    old.lines().count(),
                    new.lines().count()
                );
                for line in old.lines() {
                    diff.push('-');
                    diff.push_str(line);
                    diff.push('\n');
                }
                for line in new.lines() {
                    diff.push('+');
                    diff.push_str(line);
                    diff.push('\n');
                }
                Some(work::FileChange {
                    path: cut(path, 1024).into(),
                    added: new.lines().count(),
                    removed: old.lines().count(),
                    truncated: diff.len() > work::MAX_DIFF_TEXT,
                    diff: cut(&diff, work::MAX_DIFF_TEXT).into(),
                })
            })
            .take(128)
            .collect()
    }

    fn append(&mut self, text: &str) -> &Step {
        let step = self.active.get_or_insert_with(|| {
            self.sequence += 1;
            Step::thinking(
                format!("thinking:{}:{}", self.scope, self.sequence),
                StepStatus::Running,
                "",
            )
        });
        step.text
            .push_str(cut(text, MAX_STEP_TEXT.saturating_sub(step.text.len())));
        step
    }

    fn finish(&mut self) -> Option<Step> {
        let mut step = self.active.take()?;
        step.status = StepStatus::Complete;
        Some(step)
    }
}

fn record_thought(messages: &mut Vec<Message>, mut step: Step) {
    if step.text.trim().is_empty() {
        return;
    }
    if messages
        .last()
        .is_none_or(|entry| entry.role != Role::Assistant)
    {
        messages.push(Message::new(Role::Assistant, String::new()));
    }
    let entry = messages.last_mut().unwrap();
    step.text_offset = Some(entry.text.chars().count());
    entry.work.record(step);
}

fn config_option<'a>(config: &'a Value, category: &str) -> Option<&'a Value> {
    config["configOptions"]
        .as_array()?
        .iter()
        .find(|row| row["category"] == category || row["id"] == category)
}

fn current_model(config: &Value) -> Option<&str> {
    config_option(config, "model")
        .and_then(|option| reported_model(&option["currentValue"]))
        .or_else(|| reported_model(&config["models"]["currentModelId"]))
}

fn option_values(option: Option<&Value>) -> impl Iterator<Item = &Value> {
    option
        .and_then(|row| row["options"].as_array())
        .into_iter()
        .flatten()
        .flat_map(|row| {
            row["options"]
                .as_array()
                .map_or(std::slice::from_ref(row), Vec::as_slice)
        })
}

fn speed_value(option: Option<&Value>, fast: bool) -> Option<&str> {
    option_values(option)
        .filter_map(|option| option["value"].as_str())
        .find(|value| {
            if fast {
                *value == "fast"
            } else {
                matches!(*value, "standard" | "normal" | "default")
            }
        })
}

fn permission_modes(config: &Value) -> Vec<PermissionChoice> {
    let modes = config_option(config, "mode");
    if modes.is_some() {
        return option_values(modes)
            .filter_map(|row| {
                let id = row["value"].as_str()?;
                Some(PermissionChoice::new(
                    id,
                    row["name"].as_str().unwrap_or(id),
                    row["description"].as_str().unwrap_or_default(),
                ))
            })
            .collect();
    }
    config["modes"]["availableModes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let id = row["id"].as_str()?;
            Some(PermissionChoice::new(
                id,
                row["name"].as_str().unwrap_or(id),
                row["description"].as_str().unwrap_or_default(),
            ))
        })
        .collect()
}

pub(super) fn quota() -> Result<Quota, String> {
    let cwd = std::env::temp_dir();
    let mut version = process::cli_command(Provider::Antigravity)?;
    version.arg("--version");
    let version = Process::first_line(&mut version, &cwd, Duration::from_secs(4))?;
    let mut components = version.trim_start_matches('v').split(['.', '-', '+']);
    let number = |component: Option<&str>| component.and_then(|part| part.parse::<u32>().ok());
    let installed = (
        number(components.next()),
        number(components.next()),
        number(components.next()),
    );
    // Older versions can pass a slash command through as a model prompt.
    if !matches!(installed, (Some(major), Some(minor), Some(patch)) if (major, minor, patch) >= (1, 1, 11))
    {
        return Err("查看额度需要 Antigravity CLI 1.1.11 或更新版本。".into());
    }
    let mut command = process::cli_command(Provider::Antigravity)?;
    command.args(["--print", "/usage", "--output-format", "stream-json"]);
    let process = Process::spawn(&mut command, &cwd)?;
    let cancel = AtomicBool::new(false);
    let deadline = Instant::now() + Duration::from_secs(19);
    loop {
        let frame = receive(&process, &cancel, Some(deadline))
            .map_err(|_| "Antigravity 未返回额度，请检查目标电脑的登录和 CLI 网络设置。")?;
        if let Some(groups) = frame["command"]["data"]["groups"].as_array() {
            let limits = groups.iter().filter_map(antigravity_quota_limit).collect();
            return Ok(Quota {
                limits,
                ..Quota::default()
            });
        }
        if frame["event"] == "result" {
            return Err("Antigravity 未返回额度，请检查目标电脑的登录和 CLI 网络设置。".into());
        }
    }
}

fn antigravity_quota_limit(group: &Value) -> Option<QuotaLimit> {
    let name = short(group["name"].as_str()?, 100);
    let windows: Vec<_> = group["buckets"]
        .as_array()?
        .iter()
        .filter(|bucket| bucket["disabled"] != true)
        .filter_map(|bucket| {
            let remaining_percent = bucket["remaining_fraction"]
                .as_f64()
                .map(|fraction| (fraction.clamp(0., 1.) * 100.).round() as u8);
            let remaining_amount = bucket["remaining_amount"]
                .as_f64()
                .filter(|amount| *amount >= 0.);
            if remaining_percent.is_none() && remaining_amount.is_none() {
                return None;
            }
            let label = match bucket["window"].as_str() {
                Some("5h") => "5 小时".to_owned(),
                Some("weekly") => "7 天".to_owned(),
                _ => short(bucket["name"].as_str().unwrap_or("用量窗口"), 100),
            };
            Some(QuotaWindow {
                label,
                remaining_percent,
                remaining_amount: remaining_amount.filter(|_| remaining_percent.is_none()),
                resets_at: quota_reset(&bucket["reset_time"]),
            })
        })
        .collect();
    (!windows.is_empty()).then_some(QuotaLimit { name, windows })
}

pub(super) fn models() -> Result<ModelCatalog, String> {
    let cwd = std::env::temp_dir();
    let cancel = AtomicBool::new(false);
    let mut rpc = connect(&cwd, &cancel, None)?;
    let config = rpc.control("session/new", json!({"cwd":cwd,"mcpServers":[]}), &cancel)?;
    let thought = config_option(&config, "thought_level");
    let reasoning_efforts: Vec<_> = option_values(thought)
        .filter_map(|row| {
            let id = row["value"].as_str()?;
            Some(ReasoningChoice {
                id: id.into(),
                label: row["name"].as_str().unwrap_or(id).into(),
            })
        })
        .collect();
    let speed = config_option(&config, "speed");
    let fast_mode = speed_value(speed, true).is_some() && speed_value(speed, false).is_some();
    let default_model = current_model(&config);
    let default_reasoning_effort = thought
        .and_then(|option| option["currentValue"].as_str())
        .filter(|value| reasoning_efforts.iter().any(|choice| choice.id == *value));
    let default_fast_mode = speed
        .and_then(|option| option["currentValue"].as_str())
        .and_then(|value| {
            [false, true]
                .into_iter()
                .find(|&fast| speed_value(speed, fast) == Some(value))
        });
    let models = option_values(config_option(&config, "model"))
        .filter_map(|row| {
            let id = reported_model(&row["value"])?;
            let label = row["name"].as_str().unwrap_or(id);
            let current = Some(id) == default_model;
            Some(Model {
                id: id.into(),
                label: label.into(),
                reasoning_efforts: reasoning_efforts.clone(),
                default_reasoning_effort: default_reasoning_effort
                    .filter(|_| current)
                    .map(str::to_owned),
                fast_mode,
                default_fast_mode: default_fast_mode.filter(|_| current && fast_mode),
            })
        })
        .collect::<Vec<_>>();
    if let Some(id) = config["sessionId"].as_str() {
        let _ = rpc.send(json!({"id":900001,"method":"session/close","params":{"sessionId":id}}));
    }
    if models.is_empty() {
        return Err("Antigravity CLI 未报告可切换的模型".into());
    }
    Ok(ModelCatalog {
        models,
        default_model: default_model.map(str::to_owned),
        permission_modes: permission_modes(&config),
        ..Default::default()
    })
}

pub(super) fn run(request: &TurnRequest, turn: &Turn<'_>) -> Result<(), String> {
    let mut rpc = connect_with_agent(
        &request.cwd,
        turn.cancel,
        request.session_id.as_deref(),
        request.agent.as_deref(),
    )?;
    let mut params = json!({"cwd":request.cwd,"mcpServers":[],"additionalDirectories":request.additional_directories});
    let mut config = if let Some(id) = &request.session_id {
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
    turn.send(Event::Session {
        id: session_id.into(),
        project_id: None,
        rollout: None,
    });
    let session_id = session_id.to_owned();
    for (category, value) in [
        ("model", request.options.model.as_deref()),
        ("thought_level", request.options.reasoning_effort.as_deref()),
        (
            "speed",
            request
                .options
                .fast_mode
                .map(|fast| if fast { "fast" } else { "standard" }),
        ),
    ] {
        let Some(value) = value else { continue };
        let option =
            config_option(&config, category).ok_or("Antigravity 的此会话未提供所选设置")?;
        let value = if category == "speed" {
            speed_value(Some(option), value == "fast").ok_or("Antigravity 不支持所选速度")?
        } else {
            value
        };
        if option["currentValue"] != value {
            let config_id = option["id"].as_str().ok_or("Antigravity 配置格式无效")?;
            let updated = rpc.control(
                "session/set_config_option",
                json!({"sessionId":session_id,"configId":config_id,"value":value}),
                turn.cancel,
            )?;
            if updated["configOptions"].is_array() {
                config["configOptions"] = updated["configOptions"].clone();
            }
        }
    }
    if let Some(model) = current_model(&config) {
        turn.send(Event::Model(model.to_owned()));
    }
    if let Some(mode) = request.options.permission_mode.as_deref() {
        if !permission_modes(&config)
            .iter()
            .any(|option| option.id == mode)
        {
            return Err("Antigravity 的此会话不支持所选执行模式".into());
        }
        if let Some(option) = config_option(&config, "mode") {
            if option["currentValue"] != mode {
                let config_id = option["id"]
                    .as_str()
                    .ok_or("Antigravity 执行模式配置格式无效")?;
                rpc.control(
                    "session/set_config_option",
                    json!({"sessionId":session_id,"configId":config_id,"value":mode}),
                    turn.cancel,
                )?;
            }
        } else if config["modes"]["currentModeId"] != mode {
            rpc.control(
                "session/set_mode",
                json!({"sessionId":session_id,"modeId":mode}),
                turn.cancel,
            )?;
        }
    }
    let mut blocks = prompt_blocks(&request.prompt, turn.images);
    for path in request.context_files.iter().chain(
        request
            .local_files
            .iter()
            .filter(|f| !f.directory)
            .map(|f| Path::new(&f.path).to_path_buf())
            .collect::<Vec<_>>()
            .iter(),
    ) {
        let text =
            std::fs::read_to_string(path).map_err(|_| "Cannot read embedded context as UTF-8")?;
        if text.len() > 1024 * 1024 {
            return Err("Embedded context exceeds 1 MiB".into());
        }
        let uri = url::Url::from_file_path(path).map_err(|_| "Invalid embedded context path")?;
        blocks.push(json!({"type":"resource","resource":{"uri":uri.as_str(),"mimeType":"text/plain","text":text}}));
    }
    let mut active_prompt = rpc.submit(
        "session/prompt",
        json!({"sessionId":session_id,"prompt":blocks}),
    )?;
    let mut questions = HashMap::new();
    let mut agent_calls = HashSet::new();
    let mut thoughts = ThoughtStream::new(uuid::Uuid::new_v4().to_string());
    let mut steering: Option<SteeringRequest> = None;
    loop {
        if let Err(error) = check_cancel(turn.cancel) {
            let _ = rpc.send(json!({"method":"session/cancel","params":{"sessionId":session_id}}));
            return Err(error);
        }
        if steering.is_none()
            && let Some(message) = (turn.next_steering)()
        {
            match rpc.submit(
                "session/prompt",
                json!({
                    "sessionId": session_id,
                    "prompt": prompt_blocks(&attachments::with_local_files(&message.prompt, &message.local_files), &message.images)
                }),
            ) {
                Ok(rpc_id) => {
                    steering = Some(SteeringRequest {
                        rpc_id,
                        message_id: message.id,
                    });
                }
                Err(error) => turn.send(Event::Steered {
                    id: message.id,
                    result: Err(error),
                }),
            }
        }
        match rpc.process.lines.recv_timeout(Duration::from_millis(75)) {
            Ok(Ok(frame)) if frame.get("method").is_none() && frame["id"] == active_prompt => {
                let result = prompt_result(frame);
                if result.is_ok()
                    && let Some(step) = thoughts.finish()
                {
                    turn.send(Event::Step(step));
                }
                if let Some(request) = steering.take() {
                    match result {
                        Ok(()) => {
                            active_prompt = request.rpc_id;
                            turn.send(Event::Steered {
                                id: request.message_id,
                                result: Ok(()),
                            });
                        }
                        Err(error) => return Err(error),
                    }
                } else {
                    return result;
                }
            }
            Ok(Ok(frame))
                if frame.get("method").is_none()
                    && steering
                        .as_ref()
                        .is_some_and(|request| frame["id"] == request.rpc_id) =>
            {
                let request = steering.take().unwrap();
                let result = prompt_result(frame);
                turn.send(Event::Steered {
                    id: request.message_id,
                    result,
                });
            }
            Ok(Ok(frame)) => {
                if let Err(error) = observe(
                    &rpc,
                    frame,
                    &session_id,
                    turn,
                    &mut questions,
                    &mut agent_calls,
                    &mut thoughts,
                ) {
                    if turn.cancel.load(Ordering::Acquire) {
                        let _ = rpc.send(
                            json!({"method":"session/cancel","params":{"sessionId":session_id}}),
                        );
                    }
                    return Err(error);
                }
            }
            Ok(Err(error)) => {
                if turn.cancel.load(Ordering::Acquire) {
                    let _ = rpc
                        .send(json!({"method":"session/cancel","params":{"sessionId":session_id}}));
                }
                return Err(error);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err("session/prompt · CLI 已断开".into());
            }
        }
    }
}

struct SteeringRequest {
    rpc_id: u32,
    message_id: String,
}

fn prompt_blocks(prompt: &str, images: &[attachments::ImageAttachment]) -> Vec<Value> {
    let mut blocks = vec![json!({"type":"text","text":prompt})];
    blocks.extend(
        images.iter().map(
            |image| json!({"type":"image","mimeType":image.media_type(),"data":image.base64()}),
        ),
    );
    blocks
}

fn prompt_result(frame: Value) -> Result<(), String> {
    let reply = rpc_result(frame, "session/prompt")?;
    match reply["stopReason"].as_str() {
        Some("end_turn") => Ok(()),
        Some("cancelled") => Err("Antigravity 已停止".into()),
        Some(reason) => Err(format!("Antigravity · {}", short(reason, 120))),
        None => Err("Antigravity 未返回结束原因".into()),
    }
}

fn observe(
    rpc: &Rpc,
    frame: Value,
    session: &str,
    turn: &Turn<'_>,
    questions: &mut HashMap<String, Value>,
    agent_calls: &mut HashSet<String>,
    thoughts: &mut ThoughtStream,
) -> Result<(), String> {
    let params = &frame["params"];
    if params["sessionId"] != session {
        return rpc.reject(frame);
    }
    match frame["method"].as_str() {
        Some("session/update") => {
            let update = &params["update"];
            if update["sessionUpdate"] == "usage_update" {
                turn.send(Event::Context(work::ContextUsage::new(
                    update["used"].as_u64(),
                    update["size"].as_u64(),
                )));
            }
            if let Some(text) = thought_chunk(update) {
                turn.send(Event::Step(thoughts.append(text).clone()));
            } else if thought_boundary(update)
                && let Some(step) = thoughts.finish()
            {
                turn.send(Event::Step(step));
            }
            let changes = thoughts.changes(update);
            if !changes.is_empty() {
                turn.send(Event::Changes(changes));
            }
            if matches!(
                update["sessionUpdate"].as_str(),
                Some("tool_call" | "tool_call_update")
            ) && let Some(id) = update["toolCallId"].as_str()
            {
                turn.send(Event::Step(Step {
                    id: id.into(),
                    kind: work::StepKind::Tool,
                    status: match update["status"].as_str() {
                        Some("completed") => StepStatus::Complete,
                        Some("failed") => StepStatus::Failed,
                        _ => StepStatus::Running,
                    },
                    after: None,
                    text_offset: None,
                    text: short(update["title"].as_str().unwrap_or_default(), 200),
                    exit_code: None,
                }));
            }
            if let Some(agent) = subagent_tool(update, agent_calls) {
                turn.send(Event::Agent(agent));
            }
            if update["sessionUpdate"] == "tool_call_update"
                && matches!(update["status"].as_str(), Some("completed" | "failed"))
                && let Some(id) = update["toolCallId"]
                    .as_str()
                    .and_then(|id| questions.remove(id))
            {
                turn.send(Event::ResolveQuestion(id));
            }
            if update["sessionUpdate"] == "session_info_update" {
                if let Some(title) = update["title"]
                    .as_str()
                    .map(|title| short(title, 150))
                    .filter(|title| !title.is_empty())
                {
                    turn.send(Event::Title(title));
                }
            } else if let Some((Role::Assistant, text)) = chat_chunk(update) {
                turn.send(Event::Text(text.into()));
            } else if update["sessionUpdate"] == "tool_call" {
                let label = update["title"].as_str().unwrap_or("正在执行工具");
                turn.send(Event::Activity(format!(
                    "Antigravity · {}",
                    short(label, 90)
                )));
            }
        }
        Some("elicitation/create") if params["mode"] == "form" => {
            if let Some(id) = frame.get("id") {
                if let Some(tool) = params["toolCallId"].as_str() {
                    questions.insert(tool.to_owned(), id.clone());
                }
                ask(rpc, turn, id, &params["requestedSchema"])?;
            }
        }
        Some("session/request_permission") => {
            let call = &params["toolCall"];
            let choices = params["options"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|option| {
                    let id = option["optionId"].as_str()?;
                    let name = option["name"].as_str().unwrap_or(id);
                    let kind = option["kind"].as_str().unwrap_or_default();
                    Some((format!("{name} · {kind} [{id}]"), json!(id)))
                })
                .collect();
            let writer = rpc.process.writer.clone();
            let response_id = frame["id"].clone();
            if !turn.native_decision(
                frame["id"].clone(),
                call["title"].as_str().unwrap_or("Antigravity 操作"),
                serde_json::to_string_pretty(call).unwrap_or_default(),
                choices,
                move |id| {
                    Process::send_to(
                        &writer,
                        json!({"jsonrpc":"2.0","id":response_id,
                    "result":{"outcome":{"outcome":"selected","optionId":id}}}),
                    )
                },
            ) {
                rpc.send(json!({"id":frame["id"],"result":{"outcome":{"outcome":"cancelled"}}}))?;
            }
        }
        _ => return rpc.reject(frame),
    }
    Ok(())
}

struct FormField {
    id: String,
    choices: HashMap<String, String>,
    multiple: bool,
    custom: Option<String>,
}

fn ask(rpc: &Rpc, turn: &Turn<'_>, id: &Value, schema: &Value) -> Result<(), String> {
    let Some(properties) = schema["properties"].as_object() else {
        return rpc.send(json!({"id":id,"result":{"action":"decline"}}));
    };
    let parsed: Option<Vec<_>> = properties
        .iter()
        .filter(|(key, _)| {
            !key.strip_suffix("_other")
                .is_some_and(|base| properties.contains_key(base))
        })
        .map(|(key, value)| {
            let multiple = match value["type"].as_str()? {
                "string" => false,
                "array" => true,
                _ => return None,
            };
            let variant = if multiple {
                &value["items"]["anyOf"]
            } else {
                &value["oneOf"]
            };
            let mut choices = HashMap::new();
            let options = variant
                .as_array()
                .into_iter()
                .flatten()
                .map(|entry| {
                    let value = entry["const"].as_str()?;
                    let label = entry["title"].as_str().unwrap_or(value);
                    choices.insert(label.to_owned(), value.to_owned());
                    Some(QuestionOption {
                        label: label.to_owned(),
                        description: entry["description"].as_str().unwrap_or_default().into(),
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            if multiple && options.is_empty() {
                return None;
            }
            let other = format!("{key}_other");
            let custom = if properties
                .get(&other)
                .is_some_and(|entry| entry["type"] == "string")
            {
                Some(other)
            } else if options.is_empty() {
                Some(key.clone())
            } else {
                None
            };
            let question = Question {
                id: key.clone(),
                header: String::new(),
                text: value["title"].as_str().unwrap_or(key).into(),
                multiple,
                allow_custom: custom.is_some(),
                secret: value["format"] == "password",
                options,
            };
            Some((
                question,
                FormField {
                    id: key.clone(),
                    choices,
                    multiple,
                    custom,
                },
            ))
        })
        .collect();
    let Some(parsed) = parsed else {
        return rpc.send(json!({"id":id,"result":{"action":"decline"}}));
    };
    let (questions, fields): (Vec<_>, Vec<_>) = parsed.into_iter().unzip();
    rpc.ask(turn, id, questions, move |answers| {
        let mut content = serde_json::Map::new();
        for field in &fields {
            let mut selected = Vec::new();
            for answer in &answers[&field.id] {
                if let Some(value) = field.choices.get(answer) {
                    selected.push(json!(value));
                } else if let Some(key) = &field.custom {
                    content.insert(key.clone(), json!(answer));
                }
            }
            if !selected.is_empty() {
                let value = if field.multiple {
                    Value::Array(selected)
                } else {
                    selected.remove(0)
                };
                content.insert(field.id.clone(), value);
            }
        }
        json!({"action":"accept","content":content})
    })
}

#[cfg(test)]
mod native_diff_tests {
    use super::*;
    #[test]
    fn acp_edits_deduplicate_replayed_status_updates_but_keep_distinct_edits() {
        let mut stream = ThoughtStream::new("test".into());
        let mut update = json!({"sessionUpdate":"tool_call","toolCallId":"edit-1","status":"in_progress",
            "content":[{"type":"diff","path":"/project/file.txt","oldText":"old\n","newText":"new\n"}]});
        let changes = stream.changes(&update);
        assert_eq!(changes.len(), 1);
        assert_eq!((changes[0].added, changes[0].removed), (1, 1));
        assert!(changes[0].diff.contains("-old\n+new\n"));
        update["sessionUpdate"] = json!("tool_call_update");
        update["status"] = json!("completed");
        assert!(stream.changes(&update).is_empty());
        update["toolCallId"] = json!("edit-2");
        assert_eq!(stream.changes(&update).len(), 1);
        update["toolCallId"] = json!("failed-edit");
        update["status"] = json!("failed");
        assert!(stream.changes(&update).is_empty());
    }
}
