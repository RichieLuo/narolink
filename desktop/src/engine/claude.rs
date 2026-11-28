use super::attachments::{HistoryImage, InlineImage, MAX_ENCODED_IMAGE_BYTES, UserAttachment};
use super::work::{Agent, AgentStatus, MAX_STEP_TEXT, Step, StepKind, StepStatus, Work};
use super::*;

#[cfg(test)]
mod tests;
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
};

pub(super) fn projects() -> Result<PathBuf, String> {
    let config = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(".claude")))
        .ok_or("未找到 Claude Code 的本机目录")?;
    Ok(config.join("projects"))
}

fn files() -> Result<Vec<PathBuf>, String> {
    let entries = match projects()?.read_dir() {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(_) => return Err("无法读取 Claude Code 的本机会话目录".into()),
    };
    Ok(entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.path().read_dir().ok())
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "jsonl") && session_id(path).is_some()
        })
        .collect())
}

fn session_id(path: &Path) -> Option<&str> {
    path.file_stem()?
        .to_str()
        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
}

pub(super) fn inventory() -> Result<Inventory, String> {
    let mut by_id = HashMap::<String, Session>::new();
    for entry in files()?.into_iter().filter_map(|path| info(&path)) {
        let stored = by_id
            .entry(entry.id.clone())
            .or_insert_with(|| entry.clone());
        if entry.updated > stored.updated {
            *stored = entry;
        }
    }
    let native = active_sessions();
    Ok(Inventory {
        sessions: by_id.into_values().collect(),
        native_sessions: native
            .as_ref()
            .ok()
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        warning: native.err(),
        ..Inventory::default()
    })
}

pub(super) fn active_sessions() -> Result<Value, String> {
    let mut command = process::cli_command(Provider::Claude)?;
    command.args(["agents", "--json"]);
    Process::json_output(&mut command, &current_workspace(), Duration::from_secs(5))
}

fn records(bytes: &[u8]) -> impl Iterator<Item = Value> + '_ {
    bytes
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
}

fn info(path: &Path) -> Option<Session> {
    let mut file = File::open(path).ok()?;
    let modified = file.metadata().ok()?.modified().ok()?;
    let size = file.metadata().ok()?.len();
    let mut head = Vec::new();
    (&mut file).take(256 * 1024).read_to_end(&mut head).ok()?;
    let mut cwd = None;
    let mut preview = None;
    let mut title = None;
    for row in records(&head) {
        take_info(&row, &mut cwd, &mut preview, &mut title);
    }
    if size > head.len() as u64 {
        file.seek(SeekFrom::Start(size.saturating_sub(96 * 1024)))
            .ok()?;
        let mut tail = Vec::new();
        file.take(96 * 1024).read_to_end(&mut tail).ok()?;
        for row in records(&tail) {
            take_info(&row, &mut cwd, &mut preview, &mut title);
        }
    }
    let cwd = PathBuf::from(cwd?);
    if !cwd.is_absolute() || (title.is_none() && preview.is_none()) {
        return None;
    }
    let named = title.is_some();
    Some(Session {
        provider: Provider::Claude,
        id: session_id(path)?.to_string(),
        title: title
            .or_else(|| preview.as_ref().map(|text| short(text, 150)))
            .unwrap_or_else(|| "Claude Code 会话".into()),
        named,
        cwd,
        project_id: None,
        model: None,
        updated: unix_time(modified),
        preview,
        rollout: None,
    })
}

fn take_info(
    row: &Value,
    cwd: &mut Option<String>,
    preview: &mut Option<String>,
    title: &mut Option<String>,
) {
    if row["isSidechain"] == true {
        return;
    }
    if cwd.is_none() {
        *cwd = row["cwd"]
            .as_str()
            .filter(|path| Path::new(path).is_absolute())
            .map(str::to_owned);
    }
    if preview.is_none() && row["type"] == "user" {
        *preview = message(row, false).map(|message| short(&message.text, 300));
    }
    if let Some(label) = row["customTitle"]
        .as_str()
        .or_else(|| row["aiTitle"].as_str())
    {
        let label = short(label, 150);
        if !label.is_empty() {
            *title = Some(label);
        }
    }
}

fn message(row: &Value, include_images: bool) -> Option<Message> {
    if row["isSidechain"] == true
        || row["isMeta"] == true
        || row["isVisibleInTranscriptOnly"] == true
    {
        return None;
    }
    let role = match row["type"].as_str()? {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        _ => return None,
    };
    let content = &row["message"]["content"];
    let mut attachments = Vec::new();
    let mut remaining = MAX_ENCODED_IMAGE_BYTES;
    let text = if let Some(text) = content.as_str() {
        text.to_owned()
    } else {
        content
            .as_array()?
            .iter()
            .filter_map(|part| match part["type"].as_str() {
                Some("text") => part["text"].as_str(),
                Some("image") if include_images && role == Role::User => {
                    if attachments.len() < 16 {
                        let source = &part["source"];
                        let image = (|| {
                            if source["type"] != "base64" {
                                return None;
                            }
                            InlineImage::from_base64(
                                source["media_type"].as_str()?,
                                source["data"].as_str()?,
                                &mut remaining,
                            )
                            .map(HistoryImage::Inline)
                        })();
                        attachments.push(UserAttachment {
                            name: format!("图片 {}", attachments.len() + 1),
                            image,
                        });
                    }
                    None
                }
                Some("image") => Some("[图片]"),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let trimmed = text.trim();
    if (trimmed.is_empty() && attachments.is_empty())
        || (role == Role::User
            && ["<local-command-stdout>", "<local-command-caveat>"]
                .iter()
                .any(|prefix| trimmed.starts_with(prefix)))
    {
        return None;
    }
    let mut message = Message::new(role, text);
    message.turn_id = row["uuid"].as_str().map(str::to_owned);
    message.attachments = attachments;
    Some(message)
}

struct HistoryRow {
    id: String,
    parent: Option<String>,
    message: Option<Message>,
    results: Vec<ToolResult>,
    notice: Option<AgentNotice>,
    model: Option<String>,
}

fn history_message(row: &Value) -> Option<Message> {
    if row["type"] == "user" && row["origin"]["kind"] == "task-notification" {
        return None;
    }
    let mut entry = message(row, true);
    if row["type"] != "assistant"
        || row["isSidechain"] == true
        || row["isMeta"] == true
        || row["isVisibleInTranscriptOnly"] == true
    {
        return entry;
    }
    let mut text_offset = 0;
    for (index, block) in row["message"]["content"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if block["type"] == "tool_use"
            && let Some(agent) = agent_call(block)
        {
            entry
                .get_or_insert_with(|| Message::new(Role::Assistant, String::new()))
                .work
                .record_agent(agent);
            continue;
        }
        let step = match block["type"].as_str() {
            Some("text") => {
                if let Some(text) = block["text"].as_str() {
                    if text_offset > 0 {
                        text_offset += 1;
                    }
                    text_offset += text.chars().count();
                }
                None
            }
            Some("tool_use") => tool_step(block, StepStatus::Running),
            Some("thinking") => thinking_text(block)
                .filter(|text| !text.trim().is_empty())
                .map(|text| {
                    thinking_step(
                        row["uuid"]
                            .as_str()
                            .or_else(|| row["message"]["id"].as_str())
                            .unwrap_or_default(),
                        index as u64,
                        StepStatus::Complete,
                        text,
                    )
                }),
            _ => None,
        };
        if let Some(mut step) = step {
            step.text_offset = Some(text_offset);
            entry
                .get_or_insert_with(|| Message::new(Role::Assistant, String::new()))
                .work
                .record(step);
        }
    }
    entry
}

struct ToolResult {
    id: String,
    failed: bool,
    task_id: Option<String>,
    background: bool,
}

impl ToolResult {
    fn agent_status(&self) -> Option<AgentStatus> {
        if self.failed {
            Some(AgentStatus::Failed)
        } else {
            (!self.background).then_some(AgentStatus::Complete)
        }
    }
}

fn tool_results(row: &Value) -> impl Iterator<Item = ToolResult> + '_ {
    let details = row.get("toolUseResult").unwrap_or(&row["tool_use_result"]);
    row["message"]["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| row["type"] == "user" && block["type"] == "tool_result")
        .filter_map(move |block| {
            Some(ToolResult {
                id: block["tool_use_id"].as_str()?.to_owned(),
                failed: block["is_error"] == true,
                task_id: details["agentId"]
                    .as_str()
                    .filter(|id| likely_id(id))
                    .map(str::to_owned),
                background: details["status"] == "async_launched" || details["isAsync"] == true,
            })
        })
}

struct AgentNotice {
    task_id: String,
    tool_id: Option<String>,
    status: AgentStatus,
}

fn agent_status(status: &str) -> Option<AgentStatus> {
    match status {
        "completed" => Some(AgentStatus::Complete),
        "failed" => Some(AgentStatus::Failed),
        "stopped" | "killed" => Some(AgentStatus::Interrupted),
        _ => None,
    }
}

fn history_agent_notice(row: &Value) -> Option<AgentNotice> {
    let text = match row["type"].as_str()? {
        "attachment"
            if row["attachment"]["type"] == "queued_command"
                && row["attachment"]["origin"]["kind"] == "task-notification" =>
        {
            &row["attachment"]["prompt"]
        }
        "user" if row["origin"]["kind"] == "task-notification" => &row["message"]["content"],
        _ => return None,
    };
    let text = text.as_str().or_else(|| {
        text.as_array()?
            .iter()
            .filter_map(|block| block["text"].as_str())
            .find(|text| text.trim_start().starts_with("<task-notification>"))
    })?;
    let body = text.trim_start().strip_prefix("<task-notification>")?;
    let fields = body
        .split_once("<summary>")
        .map_or(body, |(fields, _)| fields);
    let field = |name: &str| {
        fields
            .split_once(&format!("<{name}>"))?
            .1
            .split_once(&format!("</{name}>"))
            .map(|(value, _)| value.trim())
    };
    Some(AgentNotice {
        task_id: field("task-id").filter(|id| likely_id(id))?.into(),
        tool_id: field("tool-use-id")
            .filter(|id| likely_id(id))
            .map(str::to_owned),
        status: agent_status(field("status")?)?,
    })
}

fn merge_history(
    messages: &mut Vec<Message>,
    row: HistoryRow,
    agent_ids: &mut HashMap<String, String>,
) {
    for result in row.results {
        if let Some(agent) = messages
            .iter_mut()
            .rev()
            .flat_map(|entry| &mut entry.work.agents)
            .find(|agent| agent.id == result.id)
        {
            if result.failed || agent.status == Some(AgentStatus::Running) {
                agent.status = result.agent_status().or(agent.status);
            }
            if let Some(task_id) = result.task_id {
                agent_ids.insert(task_id, agent.id.clone());
            }
            continue;
        }
        if let Some(step) = messages
            .iter_mut()
            .rev()
            .take_while(|entry| entry.role == Role::Assistant)
            .flat_map(|entry| &mut entry.work.steps)
            .find(|step| step.id == result.id)
        {
            step.status = if result.failed {
                StepStatus::Failed
            } else {
                StepStatus::Complete
            };
        }
    }
    if let Some(notice) = row.notice
        && let Some(id) = notice
            .tool_id
            .as_ref()
            .or_else(|| agent_ids.get(&notice.task_id))
        && let Some(agent) = messages
            .iter_mut()
            .rev()
            .flat_map(|entry| &mut entry.work.agents)
            .find(|agent| agent.id == *id)
    {
        agent.status = Some(notice.status);
    }
    let Some(entry) = row.message else { return };
    if entry.role == Role::Assistant
        && let Some(previous) = messages
            .last_mut()
            .filter(|entry| entry.role == Role::Assistant)
    {
        let mut text_offset = previous.text.chars().count();
        if !entry.text.is_empty() {
            if !previous.text.is_empty() {
                previous.text.push_str("\n\n");
                text_offset += 2;
            }
            previous.text.push_str(&entry.text);
        }
        for mut step in entry.work.steps {
            step.text_offset = Some(text_offset + step.text_offset.unwrap_or(0));
            previous.work.record(step);
        }
        for agent in entry.work.agents {
            previous.work.record_agent(agent);
        }
    } else {
        messages.push(entry);
    }
}

pub(super) fn transcript(session: &Session) -> Result<Transcript, String> {
    let file = files()?
        .into_iter()
        .filter(|path| session_id(path) == Some(&session.id))
        .filter_map(|path| info(&path).map(|entry| (entry, path)))
        .filter(|(entry, _)| entry.cwd == session.cwd)
        .max_by_key(|(entry, _)| entry.updated)
        .ok_or("Claude Code 的原始会话文件已经移动或清理")?;
    let file = fs::File::open(file.1).map_err(|_| "无法打开 Claude Code 会话文件")?;
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut rows = Vec::<HistoryRow>::new();
    let mut bytes = 0;
    let mut image_bytes = 0;
    let mut oldest_image_row = 0usize;
    let mut truncated = false;
    while (&mut reader)
        .take(24 * 1024 * 1024)
        .read_until(b'\n', &mut line)
        .map_err(|_| "无法读取 Claude Code 会话")?
        != 0
    {
        if let Ok(row) = serde_json::from_slice::<Value>(&line)
            && row["isSidechain"] != true
            && let Some(id) = row["uuid"].as_str()
        {
            let message = history_message(&row);
            bytes += message.as_ref().map_or(0, history_bytes);
            image_bytes += message.as_ref().map_or(0, history_image_bytes);
            let model = if row["type"] == "assistant" && row["isMeta"] != true {
                reported_model(&row["message"]["model"]).map(str::to_owned)
            } else {
                None
            };
            rows.push(HistoryRow {
                id: id.to_owned(),
                parent: row["parentUuid"].as_str().map(str::to_owned),
                message,
                results: tool_results(&row).collect(),
                notice: history_agent_notice(&row),
                model,
            });
        }
        if line.last() != Some(&b'\n') && line.len() >= 24 * 1024 * 1024 {
            let _ = reader.skip_until(b'\n');
            truncated = true;
        }
        line.clear();
        if bytes > 12 * 1024 * 1024 {
            let remove = (rows.len() / 2).max(1);
            for message in rows.drain(..remove).filter_map(|row| row.message) {
                bytes -= history_bytes(&message);
                image_bytes -= history_image_bytes(&message);
            }
            oldest_image_row = oldest_image_row.saturating_sub(remove);
            truncated = true;
        }
        while image_bytes > MAX_ENCODED_IMAGE_BYTES {
            if let Some(message) = &mut rows[oldest_image_row].message {
                image_bytes -= history_image_bytes(message);
                for attachment in &mut message.attachments {
                    attachment.image = None;
                }
            }
            oldest_image_row += 1;
        }
    }
    // Rewinds keep older branches in JSONL. Follow the head's parent chain when present.
    let index: HashMap<_, _> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| (row.id.as_str(), i))
        .collect();
    let mut active = HashSet::new();
    if let Some(parent) = rows.last().and_then(|row| row.parent.as_deref()) {
        let mut row = rows.len() - 1;
        let mut next = Some(parent);
        while active.insert(row) {
            let Some(&previous) = next.and_then(|id| index.get(id)) else {
                break;
            };
            row = previous;
            next = rows[row].parent.as_deref();
        }
    }
    drop(index);
    let mut messages = Vec::new();
    let mut agent_ids = HashMap::new();
    let mut model = None;
    for (index, row) in rows.into_iter().enumerate() {
        if active.is_empty() || active.contains(&index) {
            if row.model.is_some() {
                model = row.model.clone();
            }
            merge_history(&mut messages, row, &mut agent_ids);
        }
    }
    for entry in &mut messages {
        entry.work.finish(false);
    }
    // The control channel reports effective context after restoring the native session.
    if let Ok(data) = session_action(session, &SessionAction::Context)
        && let Some(entry) = messages
            .iter_mut()
            .rev()
            .find(|entry| entry.role == Role::Assistant)
    {
        entry.work.context = Some(work::ContextUsage::new(
            data["totalTokens"].as_u64(),
            data["maxTokens"].as_u64(),
        ));
    }
    Ok(Transcript {
        messages,
        model,
        warning: truncated.then(|| "已略过部分较大的 Claude Code 记录。".into()),
        revision: None,
    })
}

fn command(workspace: &Path, args: &[&str], fast_mode: Option<bool>) -> Result<Process, String> {
    let mut settings = json!({});
    if let Some(fast) = fast_mode {
        settings["fastMode"] = json!(fast);
    }
    let settings = settings.to_string();
    let mut command = process::cli_command(Provider::Claude)?;
    command
        .args([
            "--print",
            "--verbose",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--include-partial-messages",
            "--permission-prompt-tool",
            "stdio",
            "--settings",
            &settings,
        ])
        .args(args);
    command.env("CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING", "1");
    Process::spawn(&mut command, workspace)
}

fn initialize(process: &Process, cancel: &AtomicBool) -> Result<Value, String> {
    control(
        process,
        cancel,
        Instant::now() + Duration::from_secs(18),
        "narolink-init",
        json!({"subtype":"initialize","hooks":{}}),
    )
}

fn control(
    process: &Process,
    cancel: &AtomicBool,
    deadline: Instant,
    id: &str,
    request: Value,
) -> Result<Value, String> {
    process.send(json!({"type":"control_request","request_id":id,"request":request}))?;
    loop {
        let frame = receive(process, cancel, Some(deadline))?;
        if frame["type"] == "control_response" && frame["response"]["request_id"] == id {
            return if frame["response"]["subtype"] == "success" {
                Ok(frame["response"]["response"].clone())
            } else {
                Err("Claude Code 未接受请求。请检查 CLI 版本和登录状态。".into())
            };
        }
        if frame["type"] == "result" && frame["is_error"] == true {
            return Err("Claude Code 无法初始化。请检查 CLI 登录。".into());
        }
    }
}

pub(super) fn quota() -> Result<Quota, String> {
    let process = command(
        &std::env::temp_dir(),
        &["--safe-mode", "--tools", "", "--no-session-persistence"],
        None,
    )?;
    let cancel = AtomicBool::new(false);
    let deadline = Instant::now() + Duration::from_secs(22);
    control(
        &process,
        &cancel,
        deadline,
        "narolink-init",
        json!({"subtype":"initialize","hooks":{}}),
    )?;
    let response = control(
        &process,
        &cancel,
        deadline,
        "narolink-usage",
        json!({"subtype":"get_usage","skip_behaviors":true}),
    )?;
    if response["rate_limits_available"] == false {
        return Err("当前 Claude Code 登录方式未提供套餐额度。".into());
    }
    let rates = &response["rate_limits"];
    let mut windows: Vec<_> = [
        ("five_hour", "5 小时"),
        ("seven_day", "7 天"),
        ("seven_day_oauth_apps", "已连接应用 · 7 天"),
    ]
    .into_iter()
    .filter_map(|(key, label)| claude_quota_window(label.to_owned(), &rates[key]))
    .collect();
    let model_windows = rates["model_scoped"].as_array();
    for model in model_windows.into_iter().flatten() {
        if let Some(name) = model["display_name"]
            .as_str()
            .filter(|name| !name.trim().is_empty())
        {
            windows.extend(claude_quota_window(
                format!("{} · 7 天", short(name, 80)),
                model,
            ));
        }
    }
    for (key, name) in [("seven_day_opus", "Opus"), ("seven_day_sonnet", "Sonnet")] {
        let already_listed = model_windows.into_iter().flatten().any(|model| {
            model["display_name"]
                .as_str()
                .is_some_and(|label| label.eq_ignore_ascii_case(name))
                && model["utilization"].is_number()
        });
        if !already_listed {
            windows.extend(claude_quota_window(format!("{name} · 7 天"), &rates[key]));
        }
    }
    if windows.is_empty() {
        return Err("Claude Code 暂未返回套餐额度。".into());
    }
    Ok(Quota {
        limits: vec![QuotaLimit {
            name: "套餐额度".into(),
            windows,
        }],
        ..Quota::default()
    })
}

fn claude_quota_window(label: String, data: &Value) -> Option<QuotaWindow> {
    let used = data["utilization"].as_f64()?.clamp(0., 100.);
    Some(QuotaWindow {
        label,
        remaining_percent: Some((100. - used).round() as u8),
        remaining_amount: None,
        resets_at: quota_reset(&data["resets_at"]),
    })
}

const DEFAULT_PERMISSION_MODE: &str = "manual";
const PERMISSION_MODES: [(&str, &str, &str); 6] = [
    (
        DEFAULT_PERMISSION_MODE,
        "手动审批",
        "Claude Code 按需询问是否允许使用工具。",
    ),
    (
        "acceptEdits",
        "自动接受编辑",
        "常规文件编辑自动获准，其他工具按需询问。",
    ),
    ("plan", "计划模式", "先查看代码并制定计划。"),
    ("dontAsk", "从不询问", "自动拒绝需要批准的工具请求。"),
    ("auto", "自动审批", "由 Claude Code 自动判断是否允许操作。"),
    (
        "bypassPermissions",
        "跳过权限检查",
        "以运行 Naro 的账户权限使用已启用的工具。",
    ),
];

pub(super) fn supports_permission_mode(mode: &str) -> bool {
    PERMISSION_MODES.iter().any(|(id, _, _)| *id == mode)
}

pub(super) fn models() -> Result<ModelCatalog, String> {
    let process = command(
        &current_workspace(),
        &["--tools", "", "--no-session-persistence"],
        None,
    )?;
    let cancel = AtomicBool::new(false);
    let listing = initialize(&process, &cancel)?;
    let rows = listing["models"]
        .as_array()
        .ok_or("Claude Code 未返回模型列表")?;
    let default = rows.iter().find(|row| row["value"] == "default");
    let default_model = default
        .and_then(|row| reported_model(&row["resolvedModel"]))
        .map(str::to_owned);
    let mut seen = HashSet::new();
    let mut models: Vec<_> = rows
        .iter()
        .filter(|row| row["value"] != "default")
        .chain(default)
        .filter_map(|row| {
            let id =
                reported_model(&row["resolvedModel"]).or_else(|| reported_model(&row["value"]))?;
            if !seen.insert(id) {
                return None;
            }
            let label = if row["value"] == id {
                row["displayName"].as_str().unwrap_or(id)
            } else {
                id
            };
            Some(Model {
                id: id.into(),
                label: label.into(),
                reasoning_efforts: row["supportedEffortLevels"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(ReasoningChoice::from)
                    .collect(),
                default_reasoning_effort: None,
                fast_mode: row["supportsFastMode"] == true,
                default_fast_mode: None,
            })
        })
        .collect();
    let deadline = Instant::now() + Duration::from_secs(3);
    let request_settings = |id: &str| {
        control(
            &process,
            &cancel,
            deadline,
            id,
            json!({"subtype":"get_settings"}),
        )
    };
    let settings = request_settings("narolink-settings").ok();
    let fast = match listing["fast_mode_state"].as_str() {
        Some("off" | "unavailable") => Some(false),
        Some("on" | "cooldown") => Some(true),
        _ => None,
    };
    for (index, model) in models.iter_mut().enumerate() {
        let current = Some(model.id.as_str()) == default_model.as_deref();
        if model.fast_mode
            && (current
                || settings.as_ref().is_some_and(|settings| {
                    fast == Some(false) && settings["effective"]["fastMode"] != true
                }))
        {
            model.default_fast_mode = fast;
        }
        if model.reasoning_efforts.is_empty() {
            continue;
        }
        let Some(settings) = &settings else { continue };
        let applied = if current {
            settings["applied"].clone()
        } else {
            if Instant::now() >= deadline {
                continue;
            }
            let switched = control(
                &process,
                &cancel,
                deadline,
                &format!("narolink-model-{index}"),
                json!({"subtype":"set_model","model":model.id}),
            );
            if switched.is_err() {
                continue;
            }
            let Ok(settings) = request_settings(&format!("narolink-settings-{index}")) else {
                continue;
            };
            settings["applied"].clone()
        };
        if applied["model"] == model.id
            && let Some(effort) = model.reasoning_choice(applied["effort"].as_str())
        {
            model.default_reasoning_effort = Some(effort.id.clone());
        }
    }
    Ok(ModelCatalog {
        models,
        default_model,
        permission_modes: PERMISSION_MODES
            .into_iter()
            .map(|(id, label, description)| PermissionChoice::new(id, label, description))
            .collect(),
        default_permission_mode: Some(DEFAULT_PERMISSION_MODE),
        ..Default::default()
    })
}

pub(super) fn generate_title(
    model: Option<&str>,
    prompt: &str,
    cancel: &AtomicBool,
) -> Result<Value, String> {
    generate_structured(
        model,
        title::INSTRUCTIONS,
        title::schema(),
        prompt,
        title::TIMEOUT,
        cancel,
    )
}

pub(super) fn generate_structured(
    model: Option<&str>,
    instructions: &str,
    schema: Value,
    prompt: &str,
    timeout: Duration,
    cancel: &AtomicBool,
) -> Result<Value, String> {
    let schema = schema.to_string();
    let mut args = vec![
        "--safe-mode",
        "--no-session-persistence",
        "--tools",
        "",
        "--system-prompt",
        instructions,
        "--json-schema",
        &schema,
    ];
    if let Some(model) = model {
        args.extend(["--model", model]);
    }
    let process = command(&current_workspace(), &args, None)?;
    initialize(&process, cancel)?;
    process.send(
        json!({"type":"user","session_id":"","parent_tool_use_id":null,
        "message":{"role":"user","content":[{"type":"text","text":prompt}]}}),
    )?;
    let deadline = Instant::now() + timeout;
    loop {
        let mut frame = receive(&process, cancel, Some(deadline))?;
        if frame["type"] == "result" {
            if frame["subtype"] != "success" || frame["is_error"] == true {
                return Err("Claude Code 生成失败".into());
            }
            return Ok(frame["structured_output"].take());
        }
    }
}

const STEERING_TIMEOUT: Duration = Duration::from_secs(20);

struct SteeringRequest {
    message_id: String,
    deadline: Option<Instant>,
}

fn user_content(prompt: &str, images: &[attachments::ImageAttachment]) -> Vec<Value> {
    let mut content = vec![json!({"type":"text","text":prompt})];
    content.extend(images.iter().map(|image| {
        json!({"type":"image",
        "source":{"type":"base64","media_type":image.media_type(),"data":image.base64()}})
    }));
    content
}

pub(super) fn run(request: &TurnRequest, turn: &Turn<'_>) -> Result<(), String> {
    let mut args = vec![
        "--replay-user-messages",
        "--permission-mode",
        request
            .options
            .permission_mode
            .as_deref()
            .unwrap_or(DEFAULT_PERMISSION_MODE),
        "--tools",
        "default",
    ];
    if let Some(id) = &request.session_id {
        args.extend(["--resume", id]);
    }
    if let Some(agent) = &request.agent {
        args.extend(["--agent", agent]);
    }
    if let Some(worktree) = &request.worktree {
        args.extend(["--worktree", worktree]);
    }
    let directories: Vec<_> = request
        .additional_directories
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    for directory in &directories {
        args.extend(["--add-dir", directory]);
    }
    if let Some(model) = &request.options.model {
        args.extend(["--model", model]);
    }
    if let Some(effort) = &request.options.reasoning_effort {
        args.extend(["--effort", effort]);
    }
    let process = command(&request.cwd, &args, request.options.fast_mode)?;
    initialize(&process, turn.cancel)?;
    let _ = process.send(json!({"type":"control_request","request_id":"naro-context",
        "request":{"subtype":"get_context_usage"}}));
    process.send(json!({
        "type": "user",
        "session_id": request.session_id.as_deref().unwrap_or(""),
        "parent_tool_use_id": null,
        "message": {"role": "user", "content": user_content(&attachments::with_local_files(&request.prompt, &request.local_files), turn.images)}
    }))?;
    let mut session_id = None;
    let mut output = Output::default();
    let mut steering: Option<SteeringRequest> = None;
    loop {
        if let Err(error) = check_cancel(turn.cancel) {
            let _ = process.send(json!({"type":"control_request","request_id":"narolink-stop","request":{"subtype":"interrupt"}}));
            return Err(error);
        }
        if steering.as_ref().is_some_and(|request| {
            request
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
        }) {
            let request = steering.take().unwrap();
            turn.send(Event::Steered {
                id: request.message_id,
                result: Err("等待 Claude Code 确认引导超时。".into()),
            });
            return Ok(());
        }
        if steering.is_none()
            && let Some(active_session) = session_id.as_deref()
            && let Some(message) = (turn.next_steering)()
        {
            match process.send(json!({
                "type": "user",
                "uuid": message.id,
                "session_id": active_session,
                "parent_tool_use_id": null,
                "message": {"role": "user", "content": user_content(&attachments::with_local_files(&message.prompt, &message.local_files), &message.images)}
            })) {
                Ok(()) => {
                    steering = Some(SteeringRequest {
                        message_id: message.id,
                        deadline: None,
                    });
                }
                Err(error) => turn.send(Event::Steered {
                    id: message.id,
                    result: Err(error),
                }),
            }
        }
        let frame = match process.lines.recv_timeout(Duration::from_millis(75)) {
            Ok(Ok(frame)) => frame,
            Ok(Err(error)) => {
                if turn.cancel.load(Ordering::Acquire) {
                    let _ = process.send(json!({"type":"control_request","request_id":"narolink-stop","request":{"subtype":"interrupt"}}));
                }
                return Err(error);
            }
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                if let Some(request) = steering.take().filter(|request| request.deadline.is_some())
                {
                    turn.send(Event::Steered {
                        id: request.message_id,
                        result: Err("当前轮次已结束，Claude Code 未确认引导。".into()),
                    });
                    return Ok(());
                }
                return Err("CLI 已断开".into());
            }
        };
        let frame_type = frame["type"].as_str().unwrap_or_default();
        let model = match frame_type {
            "system" if frame["subtype"] == "init" => reported_model(&frame["model"]),
            "assistant" if frame["parent_tool_use_id"].is_null() => {
                reported_model(&frame["message"]["model"])
            }
            _ => None,
        };
        if let Some(model) = model {
            turn.send(Event::Model(model.to_owned()));
        }
        if matches!(frame_type, "system" | "result")
            && session_id.is_none()
            && let Some(id) = frame["session_id"].as_str().filter(|id| likely_id(id))
        {
            if request
                .session_id
                .as_deref()
                .is_some_and(|requested| requested != id)
            {
                return Err("Claude Code 返回了不同的原会话".into());
            }
            session_id = Some(id.to_owned());
            turn.send(Event::Session {
                id: id.into(),
                project_id: None,
                rollout: None,
            });
        }
        match frame_type {
            "control_response" if frame["response"]["request_id"] == "naro-context" => {
                let data = &frame["response"]["response"];
                turn.send(Event::Context(work::ContextUsage::new(
                    data["totalTokens"].as_u64(),
                    data["maxTokens"].as_u64(),
                )));
            }
            "system" => {
                let subtype = frame["subtype"].as_str();
                if matches!(subtype, Some("init" | "status"))
                    && let Some(mode) = frame["permissionMode"].as_str()
                {
                    let mode = if mode == "default" {
                        DEFAULT_PERMISSION_MODE
                    } else {
                        mode
                    };
                    if let Some(&(id, label, description)) =
                        PERMISSION_MODES.iter().find(|(id, _, _)| *id == mode)
                    {
                        turn.send(Event::PermissionMode(PermissionChoice::new(
                            id,
                            label,
                            description,
                        )));
                    }
                }
                if subtype == Some("session_title_changed")
                    && let Some(title) = frame["title"].as_str()
                {
                    turn.send(Event::Title(title.into()));
                }
                output.observe_task(&frame, turn);
            }
            "stream_event" if frame["parent_tool_use_id"].is_null() => {
                output.stream(&frame["event"], turn)
            }
            "assistant" if frame["parent_tool_use_id"].is_null() => {
                output.snapshot(&frame["message"], turn)
            }
            "user" if frame["parent_tool_use_id"].is_null() => {
                for result in tool_results(&frame) {
                    output.tool_result(result, turn);
                }
                if steering.as_ref().is_some_and(|request| {
                    frame["uuid"].as_str() == Some(request.message_id.as_str())
                }) {
                    let request = steering.take().unwrap();
                    output.reset_for_steer();
                    turn.send(Event::Steered {
                        id: request.message_id,
                        result: Ok(()),
                    });
                }
            }
            "control_request" => {
                let id = frame["request_id"]
                    .as_str()
                    .ok_or("Claude Code 审批缺少 ID")?;
                let call = &frame["request"];
                let name = call["tool_name"].as_str().unwrap_or_default();
                if call["subtype"] == "can_use_tool" && name == "AskUserQuestion" {
                    ask(&process, turn, id, &call["input"])?;
                    continue;
                }
                if call["subtype"] == "can_use_tool" && name == "ExitPlanMode" {
                    let allowed = turn.approval(
                        "批准计划并实施",
                        serde_json::to_string_pretty(&call["input"]).unwrap_or_default(),
                    )?;
                    process.send(control_response(
                        id,
                        if allowed {
                            json!({"behavior":"allow","updatedInput":call["input"]})
                        } else {
                            json!({"behavior":"deny","message":"User declined plan implementation"})
                        },
                    ))?;
                    // Claude Code restores the permission mode that preceded planning.
                    continue;
                }
                // The CLI applies project rules and its permission mode before asking
                // the host. Do not silently deny native, plugin or MCP tools here.
                let allowed = if call["subtype"] == "can_use_tool" && !name.is_empty() {
                    match turn.approval(
                        name,
                        serde_json::to_string_pretty(&call["input"]).unwrap_or_default(),
                    ) {
                        Ok(allowed) => allowed,
                        Err(error) => {
                            if turn.cancel.load(Ordering::Acquire) {
                                let _ = process.send(json!({"type":"control_request","request_id":"narolink-stop","request":{"subtype":"interrupt"}}));
                            }
                            return Err(error);
                        }
                    }
                } else {
                    false
                };
                let decision = if allowed {
                    json!({"behavior":"allow","updatedInput":call["input"]})
                } else {
                    json!({"behavior":"deny","message":"User declined this action in Naro"})
                };
                process.send(control_response(id, decision))?;
            }
            "control_cancel_request" => {
                turn.send(Event::ResolveQuestion(frame["request_id"].clone()));
            }
            "result" if frame["is_error"] == true || frame["subtype"] != "success" => {
                return Err(format!(
                    "Claude Code · {}",
                    short(
                        frame["result"]
                            .as_str()
                            .unwrap_or("回复失败。请确认 CLI 登录、所选模型和项目路径。"),
                        240
                    )
                ));
            }
            "result" => {
                if !output.emitted
                    && let Some(text) = frame["result"].as_str().filter(|text| !text.is_empty())
                {
                    turn.send(Event::Text(text.into()));
                }
                if let Some(request) = steering.as_mut() {
                    request.deadline = Some(Instant::now() + STEERING_TIMEOUT);
                    output.reset_for_steer();
                    continue;
                }
                // At a turn boundary there are no active tool calls to lose while
                // querying the SDK control channel.
                if let Ok(data) = control(
                    &process,
                    turn.cancel,
                    Instant::now() + Duration::from_secs(4),
                    "naro-context-final",
                    json!({"subtype":"get_context_usage"}),
                ) {
                    turn.send(Event::Context(work::ContextUsage::new(
                        data["totalTokens"].as_u64(),
                        data["maxTokens"].as_u64(),
                    )));
                }
                return Ok(());
            }
            _ => {}
        }
    }
}

fn ask(process: &Process, turn: &Turn<'_>, id: &str, input: &Value) -> Result<(), String> {
    let questions = input["questions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|row| {
            let text = row["question"].as_str()?;
            Some(Question {
                id: text.into(),
                header: row["header"].as_str().unwrap_or_default().into(),
                text: text.into(),
                options: serde_json::from_value(row["options"].clone()).ok()?,
                multiple: row["multiSelect"] == true,
                allow_custom: true,
                secret: false,
            })
        })
        .collect::<Option<_>>()
        .unwrap_or_default();
    let send = process.writer.clone();
    let response_id = id.to_owned();
    let input = input.clone();
    if turn.ask(json!(id), questions, move |answers| {
        let mut updated = input.clone();
        let answers: BTreeMap<_, _> = answers
            .into_iter()
            .map(|(id, values)| (id, values.join(", ")))
            .collect();
        updated["answers"] = json!(answers);
        Process::send_to(
            &send,
            control_response(
                &response_id,
                json!({"behavior":"allow","updatedInput":updated}),
            ),
        )
    }) {
        Ok(())
    } else {
        process.send(control_response(
            id,
            json!({"behavior":"deny","message":"Invalid questions"}),
        ))
    }
}

fn control_response(id: &str, response: Value) -> Value {
    json!({"type":"control_response","response":{"subtype":"success","request_id":id,"response":response}})
}

fn history_bytes(message: &Message) -> usize {
    message.text.len()
        + message
            .work
            .steps
            .iter()
            .map(|step| step.text.len())
            .sum::<usize>()
        + message
            .work
            .agents
            .iter()
            .map(|agent| agent.id.len() + agent.name.len() + agent.description.len())
            .sum::<usize>()
}

fn history_image_bytes(message: &Message) -> usize {
    message
        .attachments
        .iter()
        .filter_map(|attachment| match &attachment.image {
            Some(HistoryImage::Inline(image)) => Some(image.encoded_len()),
            _ => None,
        })
        .sum()
}

fn thinking_text(block: &Value) -> Option<&str> {
    block["thinking"]
        .as_str()
        .filter(|_| block["type"] == "thinking")
}

fn thinking_step(message: &str, index: u64, status: StepStatus, text: &str) -> Step {
    Step::thinking(format!("thinking:{message}:{index}"), status, text)
}

fn agent_call(block: &Value) -> Option<Agent> {
    let name = block["name"].as_str()?;
    if !matches!(name, "Agent" | "Task") {
        return None;
    }
    let id = block["id"].as_str().filter(|id| likely_id(id))?;
    let input = &block["input"];
    Some(Agent::new(
        id,
        input["subagent_type"].as_str(),
        input["description"].as_str(),
        Some(AgentStatus::Running),
    ))
}

fn tool_step(block: &Value, status: StepStatus) -> Option<Step> {
    let name = block["name"].as_str()?;
    let input = &block["input"];
    let detail = match name {
        "Bash" => input["command"].as_str(),
        "Read" | "Edit" | "MultiEdit" | "Write" => input["file_path"].as_str(),
        "Glob" | "Grep" => input["pattern"].as_str(),
        _ => input["description"].as_str(),
    }
    .unwrap_or_default();
    let kind = if name == "Bash" {
        StepKind::Command
    } else {
        StepKind::Tool
    };
    let text = if kind == StepKind::Command && !detail.is_empty() {
        cut(detail, MAX_STEP_TEXT).to_owned()
    } else if detail.is_empty() {
        short(name, 100)
    } else {
        format!(
            "{} · {}",
            short(name, 100),
            short(cut(detail, MAX_STEP_TEXT), 1000)
        )
    };
    Some(Step {
        id: block["id"].as_str()?.to_owned(),
        kind,
        status,
        after: None,
        text_offset: None,
        text,
        exit_code: None,
    })
}

struct ToolBlock {
    block: Value,
    input: String,
}

#[derive(Default)]
struct Output {
    message: String,
    blocks: HashMap<u64, String>,
    tool_blocks: HashMap<u64, ToolBlock>,
    thinking: HashMap<u64, Option<String>>,
    tools: Work,
    agent_ids: HashMap<String, String>,
    background_agents: HashSet<String>,
    emitted: bool,
    needs_break: bool,
    tasks: BTreeMap<String, (String, String)>,
}

impl Output {
    fn reset_for_steer(&mut self) {
        self.emitted = false;
        self.needs_break = false;
    }

    fn select(&mut self, id: &str) {
        if !id.is_empty() && self.message != id {
            id.clone_into(&mut self.message);
            self.blocks.clear();
            self.tool_blocks.clear();
            self.thinking.clear();
            self.needs_break = self.emitted;
        }
    }

    fn stream(&mut self, event: &Value, turn: &Turn<'_>) {
        if event["type"] == "message_start" {
            self.select(event["message"]["id"].as_str().unwrap_or_default());
        }
        let block = event["index"].as_u64().unwrap_or(0);
        if event["delta"]["type"] == "text_delta"
            && let Some(text) = event["delta"]["text"].as_str()
        {
            self.append(block, text, false, turn);
        }
        if event["type"] == "content_block_start" {
            let content = &event["content_block"];
            match content["type"].as_str() {
                Some("tool_use") => {
                    self.record_tool(content, turn);
                    self.tool_blocks.insert(
                        block,
                        ToolBlock {
                            block: content.clone(),
                            input: String::new(),
                        },
                    );
                }
                Some("thinking" | "redacted_thinking") => {
                    let text = (content["type"] == "thinking").then(|| {
                        cut(thinking_text(content).unwrap_or_default(), MAX_STEP_TEXT).to_owned()
                    });
                    turn.send(Event::Step(thinking_step(
                        &self.message,
                        block,
                        StepStatus::Running,
                        text.as_deref().unwrap_or_default(),
                    )));
                    self.thinking.insert(block, text);
                }
                _ => {}
            }
        }
        if event["delta"]["type"] == "input_json_delta"
            && let Some(partial) = event["delta"]["partial_json"].as_str()
            && let Some(tool) = self.tool_blocks.get_mut(&block)
        {
            let available = (4 * 1024 * 1024usize).saturating_sub(tool.input.len());
            tool.input.push_str(cut(partial, available));
            if let Ok(input) = serde_json::from_str::<Value>(&tool.input) {
                tool.block["input"] = input;
                let content = tool.block.clone();
                self.record_tool(&content, turn);
            }
        }
        if event["delta"]["type"] == "thinking_delta"
            && let Some(delta) = event["delta"]["thinking"].as_str()
            && let Some(Some(text)) = self.thinking.get_mut(&block)
        {
            text.push_str(cut(delta, MAX_STEP_TEXT.saturating_sub(text.len())));
            turn.send(Event::Step(thinking_step(
                &self.message,
                block,
                StepStatus::Running,
                text,
            )));
        }
        if event["type"] == "content_block_stop" {
            self.tool_blocks.remove(&block);
            if let Some(text) = self.thinking.get(&block) {
                turn.send(Event::Step(thinking_step(
                    &self.message,
                    block,
                    StepStatus::Complete,
                    text.as_deref().unwrap_or_default(),
                )));
            }
        }
    }

    fn snapshot(&mut self, message: &Value, turn: &Turn<'_>) {
        self.select(message["id"].as_str().unwrap_or_default());
        let mut reconciled = HashSet::new();
        let mut reconciled_thinking = HashSet::new();
        let text_blocks = message["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|block| block["type"] == "text")
            .count();
        for (index, block) in message["content"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            match block["type"].as_str() {
                Some("text") => {
                    if let Some(text) = block["text"].as_str() {
                        // Claude can emit one assistant snapshot per content block, so
                        // its array index need not match the stream's block index.
                        let indexed = (text_blocks > 1).then_some(index as u64).filter(|index| {
                            self.blocks
                                .get(index)
                                .is_some_and(|existing| text.starts_with(existing.as_str()))
                        });
                        let streamed = indexed.or_else(|| {
                            self.blocks
                                .iter()
                                .filter(|(index, existing)| {
                                    !reconciled.contains(*index)
                                        && !existing.is_empty()
                                        && (text.starts_with(existing.as_str())
                                            || text.trim_end() == existing.trim_end())
                                })
                                .max_by_key(|(_, existing)| existing.len())
                                .map(|(index, _)| *index)
                        });
                        let index = streamed.unwrap_or(index as u64);
                        reconciled.insert(index);
                        self.append(index, text, true, turn);
                    }
                }
                Some("tool_use") => self.record_tool(block, turn),
                Some("thinking") => {
                    if let Some(text) = thinking_text(block).filter(|text| !text.trim().is_empty())
                    {
                        self.snapshot_thinking(index as u64, text, &mut reconciled_thinking, turn);
                    }
                }
                _ => {}
            }
        }
    }

    fn snapshot_thinking(
        &mut self,
        index: u64,
        text: &str,
        reconciled: &mut HashSet<u64>,
        turn: &Turn<'_>,
    ) {
        let text = cut(text, MAX_STEP_TEXT);
        // A per-block assistant snapshot can have a different index than the stream.
        let streamed = self
            .thinking
            .iter()
            .filter_map(|(&index, saved)| Some((index, saved.as_deref()?)))
            .filter(|(index, saved)| {
                !reconciled.contains(index)
                    && !saved.is_empty()
                    && (text.starts_with(*saved) || text.trim_end() == saved.trim_end())
            })
            .max_by_key(|(block, saved)| (saved.len(), *block == index, std::cmp::Reverse(*block)));
        let index = streamed.map_or_else(
            || {
                (index..)
                    .find(|index| {
                        !reconciled.contains(index)
                            && self
                                .thinking
                                .get(index)
                                .is_none_or(|saved| saved.as_deref() == Some(""))
                    })
                    .unwrap_or(index)
            },
            |(index, _)| index,
        );
        reconciled.insert(index);
        self.thinking.insert(index, Some(text.to_owned()));
        turn.send(Event::Step(thinking_step(
            &self.message,
            index,
            StepStatus::Complete,
            text,
        )));
    }

    fn record_tool(&mut self, block: &Value, turn: &Turn<'_>) {
        let input = &block["input"];
        if block["name"] == "TodoWrite" {
            let plan = input["todos"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|task| {
                    Some(work::PlanStep {
                        step: task["content"].as_str()?.into(),
                        status: match task["status"].as_str()? {
                            "in_progress" => "inProgress",
                            "completed" => "completed",
                            _ => "pending",
                        }
                        .into(),
                    })
                })
                .collect();
            turn.send(Event::Plan(plan));
        } else if matches!(block["name"].as_str(), Some("TaskCreate" | "TaskUpdate")) {
            let id = input["taskId"]
                .as_str()
                .or(block["id"].as_str())
                .unwrap_or_default()
                .to_owned();
            let task = self.tasks.entry(id).or_default();
            if let Some(subject) = input["subject"].as_str() {
                task.0 = subject.into();
            }
            if let Some(status) = input["status"].as_str() {
                task.1 = status.into();
            }
            turn.send(Event::Plan(
                self.tasks
                    .values()
                    .map(|(subject, status)| work::PlanStep {
                        step: subject.clone(),
                        status: match status.as_str() {
                            "in_progress" => "inProgress",
                            "completed" => "completed",
                            _ => "pending",
                        }
                        .into(),
                    })
                    .collect(),
            ));
        }
        if let Some(mut agent) = agent_call(block) {
            // The same call can arrive again as a full assistant snapshot.
            if self.tools.agents.iter().any(|entry| entry.id == agent.id) {
                agent.status = None;
            }
            self.record_agent(agent, turn);
            return;
        }
        let Some(mut step) = tool_step(block, StepStatus::Running) else {
            return;
        };
        if let Some(previous) = self
            .tools
            .steps
            .iter()
            .find(|previous| previous.id == step.id)
        {
            step.status = previous.status;
        }
        self.tools.record(step.clone());
        turn.send(Event::Step(step));
    }

    fn record_agent(&mut self, agent: Agent, turn: &Turn<'_>) {
        self.tools.record_agent(agent.clone());
        turn.send(Event::Agent(agent));
    }

    fn observe_task(&mut self, frame: &Value, turn: &Turn<'_>) {
        let Some(task_id) = frame["task_id"].as_str().filter(|id| likely_id(id)) else {
            return;
        };
        let id = frame["tool_use_id"]
            .as_str()
            .filter(|id| likely_id(id))
            .or_else(|| self.agent_ids.get(task_id).map(String::as_str));
        let Some(id) = id.map(str::to_owned) else {
            return;
        };
        match frame["subtype"].as_str() {
            Some("task_started") if frame["task_type"] == "local_agent" => {
                if frame["is_backgrounded"] == true {
                    self.background_agents.insert(id.clone());
                }
                self.agent_ids.insert(task_id.into(), id.clone());
                self.record_agent(
                    Agent::new(
                        &id,
                        None,
                        frame["description"].as_str(),
                        Some(AgentStatus::Running),
                    ),
                    turn,
                );
            }
            Some("task_notification") if self.tools.agents.iter().any(|agent| agent.id == id) => {
                if let Some(status) = frame["status"].as_str().and_then(agent_status) {
                    self.record_agent(Agent::new(&id, None, None, Some(status)), turn);
                }
            }
            _ => {}
        }
    }

    fn tool_result(&mut self, mut result: ToolResult, turn: &Turn<'_>) {
        if let Some(agent) = self.tools.agents.iter().find(|agent| agent.id == result.id) {
            result.background |= self.background_agents.contains(&result.id);
            if result.failed || agent.status == Some(AgentStatus::Running) {
                let update = Agent::new(&result.id, None, None, result.agent_status());
                if update.status.is_some() {
                    self.record_agent(update, turn);
                }
            }
            if let Some(task_id) = result.task_id {
                self.agent_ids.insert(task_id, result.id);
            }
        } else {
            self.finish_tool(&result.id, result.failed, turn);
        }
    }

    fn finish_tool(&mut self, id: &str, failed: bool, turn: &Turn<'_>) {
        if let Some(step) = self.tools.steps.iter_mut().find(|step| step.id == id) {
            step.status = if failed {
                StepStatus::Failed
            } else {
                StepStatus::Complete
            };
            turn.send(Event::Step(step.clone()));
        }
    }

    fn append(&mut self, index: u64, value: &str, snapshot: bool, turn: &Turn<'_>) {
        let existing = self.blocks.entry(index).or_default();
        let tail = if snapshot {
            value.strip_prefix(existing.as_str()).unwrap_or_default()
        } else {
            value
        };
        if !tail.is_empty() {
            if self.needs_break {
                turn.send(Event::Text("\n\n".into()));
                self.needs_break = false;
            }
            self.emitted = true;
            turn.send(Event::Text(tail.to_owned()));
            if existing.len() < 4 * 1024 * 1024 {
                existing.push_str(tail);
            }
        }
    }
}

pub(super) fn session_action(session: &Session, action: &SessionAction) -> Result<Value, String> {
    let mut args = vec!["--resume", session.id.as_str()];
    if matches!(action, SessionAction::Fork) {
        args.push("--fork-session");
    }
    let process = command(&session.cwd, &args, None)?;
    let cancel = AtomicBool::new(false);
    initialize(&process, &cancel)?;
    let request = match action {
        SessionAction::Rename { name } => json!({"subtype":"rename_session","title":name}),
        SessionAction::Context => json!({"subtype":"get_context_usage"}),
        SessionAction::McpStatus => json!({"subtype":"mcp_status"}),
        SessionAction::CheckpointPreview { message_id } => {
            json!({"subtype":"rewind_files","user_message_id":message_id,"dry_run":true})
        }
        SessionAction::CheckpointRewind { message_id } => {
            json!({"subtype":"rewind_files","user_message_id":message_id,"dry_run":false})
        }
        SessionAction::Fork => {
            // SDK initialization loads the fork without sending a model prompt.
            // Query the new session through a native title change event.
            process.send(json!({"type":"control_request","request_id":"naro-fork-name",
                "request":{"subtype":"rename_session","title":format!("{} (fork)", session.title)}}))?;
            let deadline = Instant::now() + Duration::from_secs(18);
            loop {
                let frame = receive(&process, &cancel, Some(deadline))?;
                if let Some(id) = frame["session_id"]
                    .as_str()
                    .filter(|id| likely_id(id) && *id != session.id)
                {
                    return Ok(json!({"session_id":id,"native":frame}));
                }
                if frame["type"] == "control_response" && frame["response"]["subtype"] == "error" {
                    return Err("Claude Code failed to create the native fork".into());
                }
            }
        }
        _ => return Err("This Claude Code session control is unavailable".into()),
    };
    let result = control(
        &process,
        &cancel,
        Instant::now() + Duration::from_secs(18),
        "naro-session-control",
        request,
    )?;
    if matches!(
        action,
        SessionAction::CheckpointPreview { .. } | SessionAction::CheckpointRewind { .. }
    ) && result["canRewind"] == false
    {
        return Err(result["error"]
            .as_str()
            .unwrap_or("File rewind is unavailable")
            .into());
    }
    Ok(result)
}
