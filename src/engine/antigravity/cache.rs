use super::*;

pub(super) fn inventory() -> Result<Vec<Session>, String> {
    let path = current_workspace().join(".gemini/antigravity-cli/conversation_summaries.db");
    if !path.is_file() {
        return Ok(vec![]);
    }
    let db =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| "无法读取 Antigravity 的本机索引")?;
    let mut statement = db
        .prepare(
            "SELECT conversation_id, title, preview, workspace_uris,
         CAST(strftime('%s', last_modified_time) AS INTEGER)
         FROM conversation_summaries
         WHERE parent_conversation_id IS NULL OR parent_conversation_id = ''
         ORDER BY last_modified_time DESC LIMIT 2000",
        )
        .map_err(|_| "Antigravity 本机索引格式已改变，请登录 CLI 后刷新")?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        })
        .map_err(|_| "无法读取 Antigravity 的本机会话")?;
    Ok(rows
        .filter_map(Result::ok)
        .filter_map(|(id, title, preview, paths, updated)| {
            if !likely_id(&id) {
                return None;
            }
            let workspaces: Vec<String> = serde_json::from_str(paths.as_deref()?).ok()?;
            let cwd = workspaces.iter().find_map(|path| parse_path(path))?;
            Some(Session {
                provider: Provider::Antigravity,
                id,
                cwd,
                title: title
                    .filter(|label| !label.trim().is_empty())
                    .map(|label| short(&label, 150))
                    .or_else(|| preview.as_deref().map(|label| short(label, 150)))
                    .unwrap_or_else(|| "Antigravity 会话".into()),
                preview: preview.map(|label| short(&label, 300)),
                updated: updated.unwrap_or_default(),
            })
        })
        .collect())
}

pub(super) fn uses_app_storage(id: &str) -> bool {
    if !likely_id(id) {
        return false;
    }
    let root = current_workspace().join(".gemini");
    let file = |store: &str, extension: &str| {
        root.join(store)
            .join("conversations")
            .join(format!("{id}.{extension}"))
            .is_file()
    };
    // ACP's CLI inventory can include sessions from the Antigravity desktop store.
    // Loading those IDs with the CLI's default data directory returns no messages.
    ["db", "pb"].into_iter().any(|ext| file("antigravity", ext))
        && !["db", "pb"]
            .into_iter()
            .any(|ext| file("antigravity-cli", ext))
}
