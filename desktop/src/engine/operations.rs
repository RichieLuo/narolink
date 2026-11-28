//! Native session controls. This is an allowlist, not an arbitrary RPC proxy.
use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SessionAction {
    Rename {
        name: String,
    },
    Fork,
    Archive,
    Unarchive,
    Context,
    GoalGet,
    GoalSet {
        objective: String,
        token_budget: Option<u64>,
    },
    GoalClear,
    McpStatus,
    Extensions,
    McpReconnect,
    McpAuthorize {
        name: String,
    },
    CheckpointPreview {
        message_id: String,
    },
    CheckpointRewind {
        message_id: String,
    },
}

pub fn session_action(session: &Session, action: &SessionAction) -> Result<Value, String> {
    if let SessionAction::Rename { name } = action
        && (name.trim().is_empty() || name.len() > 512 || name.chars().any(char::is_control))
    {
        return Err("Invalid session name".into());
    }
    if let SessionAction::CheckpointPreview { message_id }
    | SessionAction::CheckpointRewind { message_id } = action
        && uuid::Uuid::parse_str(message_id).is_err()
    {
        return Err("A native user message UUID is required".into());
    }
    match session.provider {
        Provider::Codex => codex::session_action(session, action),
        Provider::Claude => claude::session_action(session, action),
        Provider::Antigravity => Err("Antigravity ACP 1.5.18 does not advertise this session control; no local history was modified".into()),
    }
}

pub fn search(
    provider: Provider,
    query: &str,
    archived: bool,
    cursor: Option<&str>,
) -> Result<Value, String> {
    if query.trim().is_empty() || query.len() > 1024 {
        return Err("Invalid search query".into());
    }
    match provider {
        Provider::Codex => codex::search(query, archived, cursor),
        _ => Err("Native full-text search is not available for this provider".into()),
    }
}

pub fn occurrences(session: &Session, query: &str, cursor: Option<&str>) -> Result<Value, String> {
    if query.trim().is_empty() || query.len() > 1024 {
        return Err("Invalid search query".into());
    }
    match session.provider {
        Provider::Codex => codex::occurrences(session, query, cursor),
        _ => Err("Native search occurrences are not available for this provider".into()),
    }
}
