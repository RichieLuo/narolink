use super::{cut, short};
use serde::Serialize;
use std::path::PathBuf;

const MAX_STEPS: usize = 96;
pub const MAX_AGENTS: usize = 64;
const MAX_CHANGED_FILES: usize = 128;
pub const MAX_STEP_TEXT: usize = 4 * 1024;
pub const MAX_DIFF_TEXT: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Note,
    Reasoning,
    Thinking,
    Command,
    Tool,
}

impl StepKind {
    pub fn is_thinking(self) -> bool {
        matches!(self, Self::Reasoning | Self::Thinking)
    }

    pub fn activity(self) -> &'static str {
        match self {
            Self::Note => "进度更新",
            Self::Reasoning | Self::Thinking => "正在思考",
            Self::Command => "正在运行命令",
            Self::Tool => "正在使用工具",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Running,
    Complete,
    Failed,
    Interrupted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Running,
    Started,
    Complete,
    Failed,
    Interrupted,
}

#[derive(Clone, Debug, Serialize)]
pub struct Agent {
    pub id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<AgentStatus>,
}

impl Agent {
    pub fn new(
        id: &str,
        name: Option<&str>,
        description: Option<&str>,
        status: Option<AgentStatus>,
    ) -> Self {
        Self {
            id: id.into(),
            name: short(cut(name.unwrap_or_default(), 512), 80),
            description: short(cut(description.unwrap_or_default(), MAX_STEP_TEXT), 240),
            status,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Step {
    pub id: String,
    pub kind: StepKind,
    pub status: StepStatus,
    pub after: Option<String>,
    /// Character offset in the visible reply where this step first appeared.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_offset: Option<usize>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
}

impl Step {
    pub fn thinking(id: String, status: StepStatus, text: &str) -> Self {
        Self {
            id,
            kind: StepKind::Thinking,
            status,
            after: None,
            text_offset: None,
            text: cut(text, MAX_STEP_TEXT).to_owned(),
            exit_code: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FileChange {
    pub path: String,
    pub added: usize,
    pub removed: usize,
    pub diff: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ContextUsage {
    pub used: Option<u64>,
    pub limit: Option<u64>,
    pub remaining: Option<u64>,
}

impl ContextUsage {
    pub fn new(used: Option<u64>, limit: Option<u64>) -> Self {
        Self {
            used,
            limit,
            remaining: used
                .zip(limit)
                .map(|(used, limit)| limit.saturating_sub(used)),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PlanStep {
    pub step: String,
    pub status: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Work {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<ContextUsage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub plan: Vec<PlanStep>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<Agent>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<FileChange>,
    #[serde(skip)]
    pub images: Vec<PathBuf>,
}

pub enum ReplyPart<'a> {
    Text { offset: usize, text: &'a str },
    Steps(&'a [Step]),
}

impl Work {
    pub fn reply_parts<'a>(&'a self, text: &'a str) -> Vec<ReplyPart<'a>> {
        let mut parts = Vec::new();
        let mut remaining = text;
        let mut offset = 0;
        let mut index = 0;
        while index < self.steps.len() {
            if self.steps[index].kind == StepKind::Note {
                index += 1;
                continue;
            }
            let target = self.steps[index].text_offset.unwrap_or(0).max(offset);
            let byte = remaining
                .char_indices()
                .nth(target - offset)
                .map_or(remaining.len(), |(byte, _)| byte);
            if byte > 0 {
                parts.push(ReplyPart::Text {
                    offset,
                    text: &remaining[..byte],
                });
                offset += remaining[..byte].chars().count();
                remaining = &remaining[byte..];
            }
            let start = index;
            index += 1;
            while index < self.steps.len()
                && self.steps[index].kind != StepKind::Note
                && self.steps[index].text_offset.unwrap_or(0) <= target
            {
                index += 1;
            }
            parts.push(ReplyPart::Steps(&self.steps[start..index]));
        }
        if !remaining.is_empty() {
            parts.push(ReplyPart::Text {
                offset,
                text: remaining,
            });
        }
        parts
    }

    pub fn is_empty(&self) -> bool {
        self.context.is_none()
            && self.plan.is_empty()
            && self.steps.is_empty()
            && self.agents.is_empty()
            && self.changes.is_empty()
            && self.images.is_empty()
    }

    pub fn record_agent(&mut self, agent: Agent) {
        if let Some(previous) = self.agents.iter_mut().find(|entry| entry.id == agent.id) {
            if !agent.name.is_empty() {
                previous.name = agent.name;
            }
            if !agent.description.is_empty() {
                previous.description = agent.description;
            }
            previous.status = agent.status.or(previous.status);
        } else if self.agents.len() < MAX_AGENTS {
            self.agents.push(agent);
        }
    }

    pub fn record(&mut self, mut step: Step) {
        if let Some(previous) = self
            .steps
            .iter_mut()
            .find(|previous| previous.id == step.id)
        {
            step.after = previous.after.take();
            step.text_offset = previous.text_offset;
            if step.text.is_empty() {
                step.text = std::mem::take(&mut previous.text);
            }
            *previous = step;
        } else {
            if self.steps.len() == MAX_STEPS {
                if let Some(index) = self
                    .steps
                    .iter()
                    .position(|step| step.kind != StepKind::Note)
                {
                    self.steps.remove(index);
                } else if step.kind == StepKind::Note {
                    self.steps.remove(0);
                } else {
                    return;
                }
            }
            self.steps.push(step);
        }
    }

    pub fn add_changes(&mut self, changes: Vec<FileChange>) {
        for change in changes {
            if let Some(previous) = self
                .changes
                .iter_mut()
                .find(|file| file.path == change.path)
            {
                previous.added += change.added;
                previous.removed += change.removed;
                let available = MAX_DIFF_TEXT.saturating_sub(previous.diff.len() + 1);
                if available > 0 && !change.diff.is_empty() {
                    previous.diff.push('\n');
                    previous.diff.push_str(cut(&change.diff, available));
                }
                previous.truncated |= change.truncated || change.diff.len() > available;
            } else if self.changes.len() < MAX_CHANGED_FILES {
                self.changes.push(change);
            }
        }
    }

    pub fn add_image(&mut self, path: PathBuf) {
        if self.images.len() < 8 && !self.images.contains(&path) {
            self.images.push(path);
        }
    }

    pub fn finish(&mut self, cancelled: bool) {
        for step in &mut self.steps {
            if step.status == StepStatus::Running {
                step.status = StepStatus::Interrupted;
            }
        }
        self.finish_agents(cancelled);
    }

    pub fn finish_agents(&mut self, cancelled: bool) {
        for agent in &mut self.agents {
            if agent.status == Some(AgentStatus::Running) {
                agent.status = cancelled.then_some(AgentStatus::Interrupted);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_history_keeps_progress_with_its_original_prompt() {
        let mut work = Work::default();
        let mut note = Step {
            id: "note".into(),
            kind: StepKind::Note,
            status: StepStatus::Running,
            after: None,
            text_offset: None,
            text: "Checking".into(),
            exit_code: None,
        };
        work.record(note.clone());
        note.after = Some("steer".into());
        note.status = StepStatus::Complete;
        work.record(note);
        for index in 0..MAX_STEPS + 4 {
            work.record(Step {
                id: index.to_string(),
                kind: StepKind::Command,
                status: StepStatus::Complete,
                after: None,
                text_offset: None,
                text: "Command".into(),
                exit_code: Some(0),
            });
        }
        assert_eq!(work.steps.len(), MAX_STEPS);
        assert_eq!(work.steps[0].kind, StepKind::Note);
        assert_eq!(work.steps[0].status, StepStatus::Complete);
        assert!(work.steps[0].after.is_none());
    }
}

#[cfg(test)]
mod context_tests {
    use super::*;
    #[test]
    fn missing_context_is_not_fabricated_and_remaining_never_underflows() {
        assert_eq!(ContextUsage::new(Some(10), None).remaining, None);
        assert_eq!(ContextUsage::new(None, Some(100)).remaining, None);
        assert_eq!(ContextUsage::new(Some(80), Some(100)).remaining, Some(20));
        assert_eq!(ContextUsage::new(Some(101), Some(100)).remaining, Some(0));
    }
}
