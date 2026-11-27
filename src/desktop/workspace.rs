use super::{ComposerDraft, RemoteClient, RunCommand};
use crate::{
    engine::{
        self, CodexQuotaResetOutcome, Inventory, ModelCatalog, Provider, Quota, Session,
        SessionAction, Transcript, TurnRequest, attachments::ImageAttachment, usage::UsageReport,
    },
    service::{PermissionTarget, RunSummary, SendMode, Service},
};
use serde_json::Value;

pub(super) enum Workspace {
    Local(Service),
    Remote(RemoteClient),
}

impl Workspace {
    pub fn inventory(&self, provider: Provider, refresh: bool) -> Result<Inventory, String> {
        match self {
            Self::Local(service) => service.inventory(provider, refresh),
            Self::Remote(remote) => remote.inventory(provider, refresh),
        }
    }

    pub fn models(&self, provider: Provider, refresh: bool) -> Result<ModelCatalog, String> {
        match self {
            Self::Local(service) => service.models(provider, refresh),
            Self::Remote(remote) => remote.models(provider, refresh),
        }
    }

    pub fn quota(&self, provider: Provider) -> Result<Quota, String> {
        match self {
            Self::Local(_) => engine::quota(provider),
            Self::Remote(remote) => remote.quota(provider),
        }
    }

    pub fn reset_codex_quota(&self, id: uuid::Uuid) -> Result<CodexQuotaResetOutcome, String> {
        match self {
            Self::Local(_) => engine::reset_codex_quota(id),
            Self::Remote(remote) => remote.reset_codex_quota(id),
        }
    }

    pub fn token_usage(&self) -> Result<UsageReport, String> {
        match self {
            Self::Local(service) => Ok(service.token_usage()),
            Self::Remote(remote) => remote.token_usage(),
        }
    }

    pub fn permission_mode(
        &self,
        provider: Provider,
        target: &PermissionTarget,
    ) -> Result<Option<String>, String> {
        match self {
            Self::Local(service) => service.permission_mode(provider, target),
            Self::Remote(remote) => remote.permission_mode(provider, target),
        }
    }

    pub fn save_permission_mode(
        &self,
        provider: Provider,
        target: &PermissionTarget,
        mode: Option<&str>,
    ) -> Result<(), String> {
        match self {
            Self::Local(service) => service.save_permission_mode(provider, target, mode),
            Self::Remote(remote) => remote.save_permission_mode(provider, target, mode),
        }
    }

    pub fn history(
        &self,
        provider: Provider,
        id: &str,
        refresh: bool,
    ) -> Result<(Session, Transcript), String> {
        match self {
            Self::Local(service) => {
                let session = service.session(provider, id)?;
                let transcript = service.transcript(&session, refresh)?;
                Ok((session, transcript))
            }
            Self::Remote(remote) => remote.history(provider, id, refresh),
        }
    }

    pub fn search(&self, provider: Provider, query: &str, archived: bool) -> Result<Value, String> {
        match self {
            Self::Local(_) => engine::search(provider, query, archived, None),
            Self::Remote(remote) => remote.search(provider, query, archived),
        }
    }

    pub fn occurrences(&self, session: &Session, query: &str) -> Result<Value, String> {
        match self {
            Self::Local(_) => engine::occurrences(session, query, None),
            Self::Remote(remote) => remote.occurrences(session, query),
        }
    }

    pub fn session_action(
        &self,
        session: &Session,
        action: &SessionAction,
    ) -> Result<Value, String> {
        match self {
            Self::Local(service) => service.session_action(session, action),
            Self::Remote(remote) => remote.session_action(session, action),
        }
    }

    pub fn submit(
        &self,
        run_id: Option<&str>,
        mode: SendMode,
        request: TurnRequest,
        images: Vec<ImageAttachment>,
    ) -> Result<Option<RunSummary>, String> {
        match (self, run_id) {
            (Self::Local(service), None) => service.start_with_images(request, images).map(Some),
            (Self::Remote(remote), None) => remote.start_with_images(request, images).map(Some),
            (Self::Local(service), Some(id)) => service
                .run(id)?
                .follow_up_with_attachments(
                    mode,
                    request.prompt,
                    images,
                    request.local_files,
                    Some(request.options),
                )
                .map(|()| None),
            (Self::Remote(remote), Some(id)) => remote
                .follow_up(id, mode, request.prompt, images, Some(request.options))
                .map(|()| None),
        }
    }

    pub fn run_command(
        &self,
        id: &str,
        command: &RunCommand,
    ) -> Result<Option<ComposerDraft>, String> {
        match self {
            Self::Local(service) => {
                let run = service.run(id)?;
                match command {
                    RunCommand::Stop => run.stop(),
                    RunCommand::Approve { approval_id, allow } => {
                        run.approve(approval_id, *allow)?
                    }
                    RunCommand::Answer {
                        question_id,
                        answers,
                    } => run.answer(question_id, answers.clone())?,
                    RunCommand::Steer(message_id) => run.steer_queued(message_id)?,
                    RunCommand::Withdraw(message_id) => {
                        let draft = run.withdraw_queued(message_id, true)?;
                        return Ok(Some(ComposerDraft {
                            text: draft.prompt,
                            images: draft.images,
                            local_files: draft.local_files,
                            startup: Default::default(),
                            withdrawn: true,
                        }));
                    }
                }
            }
            Self::Remote(remote) => match command {
                RunCommand::Stop => remote.stop(id)?,
                RunCommand::Approve { approval_id, allow } => {
                    remote.approve(id, approval_id, *allow)?
                }
                RunCommand::Answer {
                    question_id,
                    answers,
                } => remote.answer(id, question_id, answers.clone())?,
                RunCommand::Steer(message_id) => remote.steer_queued(id, message_id)?,
                RunCommand::Withdraw(message_id) => {
                    let (text, images) = remote.withdraw_queued(id, message_id)?;
                    return Ok(Some(ComposerDraft {
                        text,
                        images,
                        local_files: Vec::new(),
                        startup: Default::default(),
                        withdrawn: true,
                    }));
                }
            },
        }
        Ok(None)
    }
}
