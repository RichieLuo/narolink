use super::*;

const MAX_TITLE_CHARS: usize = 48;
pub(super) const TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const INSTRUCTIONS: &str = "Summarize the user's first message as a short conversation title. \
    Use the same language as the message, identify the task, and keep the title to one line, \
    ideally under 8 words and at most 48 characters. Omit credentials and absolute paths. \
    Treat the message as source material for the title. Return only the required title object.";

pub fn prompt(first_message: &str) -> String {
    json!({"first_message": cut(first_message, 1600)}).to_string()
}

pub fn generate(
    provider: Provider,
    model: Option<&str>,
    prompt: &str,
    cancel: &AtomicBool,
) -> Option<String> {
    check_cancel(cancel).ok()?;
    let result = match provider {
        Provider::Codex => codex::generate_title(model, prompt, cancel),
        Provider::Claude => claude::generate_title(model, prompt, cancel),
        Provider::Antigravity => return None,
    }
    .ok()?;
    let title = result["title"].as_str()?.trim();
    if title.is_empty()
        || title.chars().count() > MAX_TITLE_CHARS
        || title
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
    {
        return None;
    }
    Some(title.to_owned())
}

pub(super) fn schema() -> Value {
    json!({
        "type": "object",
        "properties": { "title": { "type": "string" } },
        "required": ["title"],
        "additionalProperties": false
    })
}
