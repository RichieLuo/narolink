use super::*;
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
};

fn projects() -> Result<PathBuf, String> {
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
    Ok(Inventory {
        sessions: by_id.into_values().collect(),
        warning: None,
    })
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
    Some(Session {
        provider: Provider::Claude,
        id: session_id(path)?.to_string(),
        title: title
            .or_else(|| preview.as_ref().map(|text| short(text, 150)))
            .unwrap_or_else(|| "Claude Code 会话".into()),
        cwd,
        updated: unix_time(modified),
        preview,
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
        *preview = message(row).map(|message| short(&message.text, 300));
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

fn message(row: &Value) -> Option<Message> {
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
    let text = if let Some(text) = content.as_str() {
        text.to_owned()
    } else {
        content
            .as_array()?
            .iter()
            .filter_map(|part| match part["type"].as_str() {
                Some("text") => part["text"].as_str(),
                Some("image") => Some("[图片]"),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let trimmed = text.trim();
    if trimmed.is_empty()
        || (role == Role::User
            && ["<local-command-stdout>", "<local-command-caveat>"]
                .iter()
                .any(|prefix| trimmed.starts_with(prefix)))
    {
        return None;
    }
    Some(Message { role, text })
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
    let mut rows = Vec::<(String, Option<String>, Option<Message>)>::new();
    let mut bytes = 0;
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
            let message = message(&row);
            bytes += message.as_ref().map_or(0, |message| message.text.len());
            rows.push((
                id.to_owned(),
                row["parentUuid"].as_str().map(str::to_owned),
                message,
            ));
        }
        if line.last() != Some(&b'\n') && line.len() >= 24 * 1024 * 1024 {
            let _ = reader.skip_until(b'\n');
            truncated = true;
        }
        line.clear();
        if bytes > 12 * 1024 * 1024 {
            let remove = rows.len() / 2;
            bytes -= rows
                .drain(..remove.max(1))
                .filter_map(|(_, _, row)| row)
                .map(|row| row.text.len())
                .sum::<usize>();
            truncated = true;
        }
    }
    // Rewinds keep older branches in JSONL. Follow the head's parent chain when present.
    let index: HashMap<_, _> = rows
        .iter()
        .enumerate()
        .map(|(i, (id, _, _))| (id.as_str(), i))
        .collect();
    let mut active = HashSet::new();
    if let Some((_, parent, _)) = rows.last().filter(|(_, parent, _)| parent.is_some()) {
        let mut row = rows.len() - 1;
        let mut next = parent.as_deref();
        while active.insert(row) {
            let Some(&previous) = next.and_then(|id| index.get(id)) else {
                break;
            };
            row = previous;
            next = rows[row].1.as_deref();
        }
    }
    drop(index);
    let messages = rows
        .into_iter()
        .enumerate()
        .filter(|(i, _)| active.is_empty() || active.contains(i))
        .filter_map(|(_, (_, _, message))| message)
        .collect();
    Ok(Transcript {
        messages,
        warning: truncated.then(|| "已略过部分较大的 Claude Code 记录。".into()),
    })
}

fn command(workspace: &Path, args: &[&str]) -> Result<Process, String> {
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
            "--strict-mcp-config",
            "--mcp-config",
            r#"{"mcpServers":{}}"#,
            "--settings",
            r#"{"disableAllHooks":true}"#,
            "--disable-slash-commands",
        ])
        .args(args);
    Process::spawn(&mut command, workspace)
}

fn initialize(process: &Process, cancel: &AtomicBool) -> Result<Value, String> {
    process.send(json!({"type":"control_request","request_id":"narolink-init","request":{"subtype":"initialize","hooks":{}}}))?;
    let deadline = Instant::now() + Duration::from_secs(18);
    loop {
        let frame = receive(process, cancel, Some(deadline))?;
        if frame["type"] == "control_response" && frame["response"]["request_id"] == "narolink-init"
        {
            return if frame["response"]["subtype"] == "success" {
                Ok(frame["response"]["response"].clone())
            } else {
                Err("Claude Code 初始化失败。请检查 CLI 版本和登录状态。".into())
            };
        }
        if frame["type"] == "result" && frame["is_error"] == true {
            return Err("Claude Code 无法初始化。请检查 CLI 登录。".into());
        }
    }
}

pub(super) fn models() -> Result<Vec<Model>, String> {
    let process = command(
        &current_workspace(),
        &["--tools", "", "--no-session-persistence"],
    )?;
    let listing = initialize(&process, &AtomicBool::new(false))?;
    Ok(listing["models"]
        .as_array()
        .ok_or("Claude Code 未返回模型列表")?
        .iter()
        .filter_map(|row| {
            let id = row["value"].as_str()?;
            Some(Model {
                id: id.into(),
                label: row["displayName"].as_str().unwrap_or(id).into(),
            })
        })
        .collect())
}

pub(super) fn run(request: &TurnRequest, turn: &Turn<'_>) -> Result<(), String> {
    let mut args = vec![
        "--permission-mode",
        "manual",
        "--tools",
        "Read,Glob,Grep,Edit,MultiEdit,Write,Bash",
    ];
    if let Some(id) = &request.session_id {
        args.extend(["--resume", id]);
    }
    if let Some(model) = &request.model {
        args.extend(["--model", model]);
    }
    let process = command(&request.cwd, &args)?;
    initialize(&process, turn.cancel)?;
    let mut content = vec![json!({"type":"text","text":request.prompt})];
    content.extend(turn.images.iter().map(|image| {
        json!({"type":"image",
        "source":{"type":"base64","media_type":image.media_type(),"data":image.base64()}})
    }));
    process.send(
        json!({"type":"user","session_id":request.session_id.as_deref().unwrap_or(""),
        "parent_tool_use_id":null,"message":{"role":"user","content":content}}),
    )?;
    let mut session_id = None;
    let mut output = Output::default();
    loop {
        let frame = match receive(&process, turn.cancel, None) {
            Ok(frame) => frame,
            Err(error) => {
                if turn.cancel.load(Ordering::Acquire) {
                    let _ = process.send(json!({"type":"control_request","request_id":"narolink-stop","request":{"subtype":"interrupt"}}));
                }
                return Err(error);
            }
        };
        let frame_type = frame["type"].as_str().unwrap_or_default();
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
            turn.send(Event::Session(id.into()));
        }
        match frame_type {
            "stream_event" => output.stream(&frame["event"], turn),
            "assistant" => output.snapshot(&frame["message"], turn),
            "control_request" => {
                let id = frame["request_id"]
                    .as_str()
                    .ok_or("Claude Code 审批缺少 ID")?;
                let call = &frame["request"];
                let name = call["tool_name"].as_str().unwrap_or_default();
                let safe_tool = matches!(
                    name,
                    "Read" | "Glob" | "Grep" | "Edit" | "MultiEdit" | "Write" | "Bash"
                );
                let allowed = call["subtype"] == "can_use_tool"
                    && safe_tool
                    && turn.approval(
                        name,
                        serde_json::to_string_pretty(&call["input"]).unwrap_or_default(),
                    )?;
                let decision = if allowed {
                    json!({"behavior":"allow","updatedInput":call["input"]})
                } else {
                    json!({"behavior":"deny","message":"User declined this action in NaroLink"})
                };
                process.send(json!({"type":"control_response","response":{"subtype":"success","request_id":id,"response":decision}}))?;
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
                return Ok(());
            }
            _ => {}
        }
    }
}

#[derive(Default)]
struct Output {
    message: String,
    blocks: HashMap<u64, String>,
    emitted: bool,
    needs_break: bool,
}

impl Output {
    fn select(&mut self, id: &str) {
        if !id.is_empty() && self.message != id {
            id.clone_into(&mut self.message);
            self.blocks.clear();
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
        if event["type"] == "content_block_start" && event["content_block"]["type"] == "tool_use" {
            let name = event["content_block"]["name"].as_str().unwrap_or("工具");
            turn.send(Event::Activity(format!(
                "Claude Code · {}",
                short(name, 75)
            )));
        }
    }

    fn snapshot(&mut self, message: &Value, turn: &Turn<'_>) {
        self.select(message["id"].as_str().unwrap_or_default());
        for (index, block) in message["content"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            if block["type"] == "text"
                && let Some(text) = block["text"].as_str()
            {
                self.append(index as u64, text, true, turn);
            }
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
