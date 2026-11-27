use crate::engine::attachments::{
    ImageAttachment, LocalAttachment, MAX_IMAGE_ATTACHMENTS, validate_image_attachments,
};
mod files;
mod git;
mod i18n;
mod input;
#[cfg(target_os = "macos")]
mod macos;
mod markdown;
mod remote;
mod style;
mod thumbnails;
mod ui;
mod workspace;

use crate::{
    engine::{
        self, CodexQuotaResetOutcome, CollaborationMode, Inventory, ModelCatalog, ModelOptions,
        Project, Provider, QuestionAnswers, Quota, Session, Transcript, TurnRequest,
        usage::{UsagePeriod, UsageReport},
    },
    service::{Activity, PermissionTarget, RunSummary, SendMode, Service, Snapshot, Status},
    settings::STATE_APP_ID,
    web::Gateway,
};
use eframe::egui;
use i18n::Language;
use remote::RemoteClient;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    rc::Rc,
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant},
};
use workspace::Workspace;

pub fn run(service: Service, gateway: Gateway) -> eframe::Result {
    let logo = image::load_from_memory(include_bytes!("../public/icons/192.png"))
        .expect("bundled Naro logo")
        .into_rgba8();
    let brand = egui::ColorImage::from_rgba_unmultiplied(
        [logo.width() as usize, logo.height() as usize],
        logo.as_raw(),
    );
    // eframe sets the Dock icon at runtime, including when Naro runs outside an app bundle.
    #[cfg(target_os = "macos")]
    let icon = image::load_from_memory(include_bytes!("../assets/narolink-macos.png"))
        .expect("bundled macOS app icon")
        .into_rgba8();
    #[cfg(not(target_os = "macos"))]
    let icon = logo;
    let (width, height) = icon.dimensions();
    let viewport = egui::ViewportBuilder::default()
        .with_title("Naro")
        .with_app_id(STATE_APP_ID)
        .with_inner_size([1160., 792.])
        .with_min_inner_size([760., 548.])
        .with_icon(egui::IconData {
            rgba: icon.into_raw(),
            width,
            height,
        });
    #[cfg(target_os = "macos")]
    let viewport = viewport
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false);
    let event_loop =
        winit::event_loop::EventLoop::<eframe::UserEvent>::with_user_event().build()?;
    let clipboard = Rc::new(input::ComposerClipboard::default());
    let app_clipboard = clipboard.clone();
    let app = eframe::create_native(
        "Naro",
        eframe::NativeOptions {
            viewport,
            persistence_path: std::env::var_os("NAROLINK_STATE_DIR")
                .map(|dir| PathBuf::from(dir).join("app.ron")),
            ..Default::default()
        },
        Box::new(move |cc| {
            #[cfg(target_os = "macos")]
            macos::install_ime_workaround(cc);
            style::setup(&cc.egui_ctx);
            let brand = cc
                .egui_ctx
                .load_texture("narolink", brand, Default::default());
            let mut app = Desktop::new(service, gateway, cc, brand, app_clipboard);
            app.refresh_inventory(&cc.egui_ctx, false);
            app.load_models(&cc.egui_ctx, Provider::Codex, false);
            Ok(Box::new(app))
        }),
        &event_loop,
    );
    event_loop.run_app(&mut input::NativeApp::new(app, clipboard.clone()))?;
    if !clipboard.initialized() {
        return Err(eframe::Error::AppCreation("无法打开 Naro 窗口".into()));
    }
    Ok(())
}

enum Update {
    Inventory(u64, Provider, Result<Inventory, String>),
    Models(u64, Provider, Result<ModelCatalog, String>),
    Quota(Provider, Result<Quota, String>),
    TokenUsage(Result<UsageReport, String>),
    CodexQuotaReset(Result<CodexQuotaResetOutcome, String>),
    Transcript(u64, Result<(Session, Transcript), String>, bool),
    Submitted(u64, String, Result<Option<RunSummary>, String>),
    NativeAction(NativeRequest, Result<serde_json::Value, String>),
    NativeSearch(u64, Result<serde_json::Value, String>),
    Connected(Result<RemoteClient, String>),
    Activity(Result<Activity, String>),
    Snapshot(u64, String, Result<Option<Snapshot>, String>),
    Permission(u64, Result<Option<String>, String>),
    RunCommand(String, RunCommand, Result<Option<ComposerDraft>, String>),
    GitStatus(u64, Result<crate::git::GitStatus, String>),
    GitDiff(u64, Result<crate::git::GitDiff, String>),
    GitAction(
        u64,
        crate::git::GitAction,
        Result<crate::git::GitActionResult, String>,
    ),
    GitCommitMessage(u64, Result<String, String>),
}

struct Desktop {
    service: Service,
    remote: Option<RemoteClient>,
    local_view: Option<SavedView>,
    recent_device: Option<(RemoteClient, SavedView)>,
    workspace_epoch: u64,
    gateway: Gateway,
    inbox: Receiver<(u64, Update)>,
    outbox: Sender<(u64, Update)>,
    inventories: HashMap<Provider, Result<Inventory, String>>,
    models: HashMap<Provider, ModelCatalog>,
    model_errors: HashMap<Provider, String>,
    inventory_pending: HashSet<Provider>,
    model_pending: HashSet<Provider>,
    catalog_epoch: u64,
    running: Vec<RunSummary>,
    last_runs: Instant,
    activity_pending: bool,
    last_snapshot: Instant,
    snapshot_pending: bool,
    snapshot_serial: u64,
    remote_error: Option<String>,
    snapshot_error: Option<String>,
    run_command_pending: bool,
    permission_pending: bool,
    permission_serial: u64,
    recent_run: Option<String>,
    title_revision: u64,
    view: View,
    model_preferences: HashMap<Provider, ModelSelection>,
    clipboard: Rc<input::ComposerClipboard>,
    language: Language,
    appearance: style::Appearance,
    send_mode: SendMode,
    epoch: u64,
    folder: PathBuf,
    filters: Option<Provider>,
    search: String,
    native_search: NativeSearch,
    action_dialog: Option<ActionDialog>,
    action_serial: u64,
    action_pending: HashMap<(Provider, String), u64>,
    composer_focus: Option<egui::Id>,
    restore_composer_focus: bool,
    pair_dialog: bool,
    device_tab: ui::DeviceTab,
    pairing_qr: Option<ui::PairingQr>,
    quota_dialog: bool,
    showing_account_quota: bool,
    quota_provider: Provider,
    quota_pending: HashSet<Provider>,
    quotas: HashMap<Provider, Result<Quota, String>>,
    token_usage: Option<UsageReport>,
    token_usage_error: Option<String>,
    token_usage_pending: bool,
    token_period: UsagePeriod,
    codex_quota_reset: CodexQuotaReset,
    target_device_url: String,
    target_device_code: String,
    target_device_pending: bool,
    target_device_error: Option<String>,
    relay_url: String,
    relay_token: String,
    relay_note: Option<String>,
    settings_page: Option<SettingsPage>,
    note: Option<String>,
    markdown: egui_commonmark::CommonMarkCache,
    files: files::FileViewer,
    git: git::GitPanel,
    #[cfg(not(target_os = "macos"))]
    attachment_picker: Option<(egui::Id, egui_file_dialog::FileDialog)>,
    thumbnails: thumbnails::Thumbnails,
    brand: egui::TextureHandle,
}

struct SavedView {
    view: View,
    folder: PathBuf,
    model_preferences: HashMap<Provider, ModelSelection>,
}

enum RunCommand {
    Stop,
    Approve {
        approval_id: String,
        allow: bool,
    },
    Answer {
        question_id: String,
        answers: QuestionAnswers,
    },
    Steer(String),
    Withdraw(String),
}

#[derive(Default)]
struct CodexQuotaReset {
    request: Option<uuid::Uuid>,
    pending: bool,
    error: Option<String>,
    message: Option<&'static str>,
}

#[derive(Default)]
struct SettingsPage {
    proxy_address: String,
    proxy_result: Option<Result<(), String>>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct ModelSelection {
    model: Option<String>,
    reasoning_effort: Option<String>,
    fast_mode: Option<bool>,
}

impl ModelSelection {
    fn retain_supported(&mut self, catalog: &ModelCatalog) {
        let Some(id) = &self.model else { return };
        let Some(model) = catalog.models.iter().find(|model| &model.id == id) else {
            *self = Self::default();
            return;
        };
        if self
            .reasoning_effort
            .as_deref()
            .is_some_and(|effort| model.reasoning_choice(Some(effort)).is_none())
        {
            self.reasoning_effort = None;
        }
        if !model.fast_mode {
            self.fast_mode = None;
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct StartupDraft {
    additional_directories: String,
    context_files: String,
    agent: String,
    worktree: String,
}

#[derive(Default)]
struct NativeSearch {
    serial: u64,
    query: String,
    archived: bool,
    pending: bool,
    hits: Vec<serde_json::Value>,
    error: Option<String>,
}

#[derive(Clone)]
struct NativeRequest {
    serial: u64,
    dialog: u64,
    epoch: u64,
    session: Session,
    action: engine::SessionAction,
}

impl NativeRequest {
    fn is_current(&self, view: &View, epoch: u64) -> bool {
        epoch == self.epoch && view.provider == self.session.provider
            && view.id.as_deref() == Some(self.session.id.as_str())
    }
}

struct ActionDialog {
    id: u64,
    epoch: u64,
    session: Session,
    body: ActionBody,
    pending: bool,
    error: Option<String>,
    status: Option<&'static str>,
}

enum ActionBody {
    Rename { name: String },
    Goal { objective: String, budget: String, loaded: bool },
    Review { branch_mode: bool, branch: String },
    Rewind { selected: Option<String>, preview: Option<(String, serde_json::Value)> },
    Context,
    Extensions { result: Option<serde_json::Value>, full: bool },
    Confirm { label: &'static str, action: engine::SessionAction },
}

impl ActionBody {
    fn title(&self) -> &'static str {
        match self {
            Self::Rename { .. } => "重命名",
            Self::Goal { .. } => "目标",
            Self::Review { .. } => "Review",
            Self::Rewind { .. } => "文件回退",
            Self::Context => "上下文用量",
            Self::Extensions { .. } => "MCP／扩展",
            Self::Confirm { label, .. } => label,
        }
    }
}

struct View {
    provider: Provider,
    project_id: Option<String>,
    session: Option<Session>,
    id: Option<String>,
    run: Option<String>,
    snapshot: Option<Snapshot>,
    transcript: Transcript,
    context_report: Option<engine::work::ContextUsage>,
    pending: bool,
    stopping: bool,
    last_finished: Option<String>,
    selection: ModelSelection,
    run_model: Option<String>,
    permission_mode: Option<String>,
    collaboration_mode: Option<CollaborationMode>,
    question_form: Option<ui::QuestionForm>,
    draft: String,
    startup: StartupDraft,
    editing_withdrawn: bool,
    other_drafts: VecDeque<ComposerDraft>,
    ime_composing: bool,
    attachments: Vec<ImageAttachment>,
    local_files: Vec<LocalAttachment>,
    title: Option<String>,
}

struct ComposerDraft {
    text: String,
    images: Vec<ImageAttachment>,
    local_files: Vec<LocalAttachment>,
    startup: StartupDraft,
    withdrawn: bool,
}

impl View {
    fn new(provider: Provider) -> Self {
        Self {
            provider,
            project_id: None,
            session: None,
            id: None,
            run: None,
            snapshot: None,
            transcript: Transcript::default(),
            context_report: None,
            pending: false,
            stopping: false,
            last_finished: None,
            selection: ModelSelection::default(),
            run_model: None,
            permission_mode: None,
            collaboration_mode: None,
            question_form: None,
            draft: String::new(),
            startup: StartupDraft::default(),
            editing_withdrawn: false,
            other_drafts: VecDeque::new(),
            ime_composing: false,
            attachments: Vec::new(),
            local_files: Vec::new(),
            title: None,
        }
    }

    fn latest_context(&self) -> Option<&engine::work::ContextUsage> {
        self.context_report.as_ref().or_else(|| {
            self.snapshot.as_ref().and_then(|snapshot| {
                snapshot.follow_ups.iter().rev()
                    .filter(|entry| entry.delivery == crate::service::Delivery::Sent)
                    .find_map(|entry| entry.work.context.as_ref())
                    .or(snapshot.work.context.as_ref())
            })
        }).or_else(|| self.transcript.messages.iter().rev()
            .find_map(|message| message.work.context.as_ref()))
    }

    fn is_new(&self) -> bool {
        self.id.is_none() && self.run.is_none() && self.snapshot.is_none()
    }

    fn selected_model<'a>(&'a self, catalog: Option<&'a ModelCatalog>) -> Option<&'a str> {
        self.selection
            .model
            .as_deref()
            .or(self.run_model.as_deref())
            .or(self.transcript.model.as_deref())
            .or_else(|| self.session.as_ref()?.model.as_deref())
            .or_else(|| {
                if self.id.is_none() && self.run.is_none() {
                    catalog?.default_model.as_deref()
                } else {
                    None
                }
            })
    }

    fn permission_target(&self) -> PermissionTarget {
        PermissionTarget {
            session_id: self.id.clone(),
            run_id: self.run.clone(),
        }
    }

    fn selected_permission<'a>(&'a self, catalog: Option<&ModelCatalog>) -> Option<&'a str> {
        self.permission_mode.as_deref().filter(|id| {
            catalog.is_none_or(|catalog| catalog.permission_modes.iter().any(|mode| mode.id == *id))
        })
    }

    fn busy(&self) -> bool {
        self.pending || self.run.is_some()
    }

    fn has_draft(&self) -> bool {
        !self.draft.trim().is_empty()
            || !self.attachments.is_empty()
            || !self.local_files.is_empty()
    }

    fn clear_startup_settings(&mut self) {
        self.startup = StartupDraft::default();
        for draft in &mut self.other_drafts {
            draft.startup = StartupDraft::default();
        }
    }

    fn show_draft(&mut self, draft: ComposerDraft) {
        let has_draft = self.has_draft();
        let previous = ComposerDraft {
            text: std::mem::replace(&mut self.draft, draft.text),
            images: std::mem::replace(&mut self.attachments, draft.images),
            local_files: std::mem::replace(&mut self.local_files, draft.local_files),
            startup: std::mem::replace(&mut self.startup, draft.startup),
            withdrawn: std::mem::replace(&mut self.editing_withdrawn, draft.withdrawn),
        };
        if has_draft {
            self.other_drafts.push_back(previous);
        }
    }
}

impl Desktop {
    fn new(
        service: Service,
        gateway: Gateway,
        cc: &eframe::CreationContext<'_>,
        brand: egui::TextureHandle,
        clipboard: Rc<input::ComposerClipboard>,
    ) -> Self {
        let (outbox, inbox) = mpsc::channel();
        let relay_url = cc
            .storage
            .and_then(|storage| storage.get_string("relay_url"))
            .unwrap_or_default();
        gateway.remote().restore(&relay_url);
        clipboard.set_context(cc.egui_ctx.clone());
        let mut app = Self {
            service,
            remote: None,
            local_view: None,
            recent_device: None,
            workspace_epoch: 0,
            gateway,
            inbox,
            outbox,
            inventories: HashMap::new(),
            models: HashMap::new(),
            model_errors: HashMap::new(),
            inventory_pending: HashSet::new(),
            model_pending: HashSet::new(),
            catalog_epoch: 0,
            running: Vec::new(),
            last_runs: Instant::now(),
            activity_pending: false,
            last_snapshot: Instant::now(),
            snapshot_pending: false,
            snapshot_serial: 0,
            remote_error: None,
            snapshot_error: None,
            run_command_pending: false,
            permission_pending: false,
            permission_serial: 0,
            recent_run: None,
            title_revision: 0,
            view: View::new(Provider::Codex),
            model_preferences: cc
                .storage
                .and_then(|storage| eframe::get_value(storage, "model_preferences"))
                .unwrap_or_default(),
            clipboard,
            language: cc
                .storage
                .and_then(|storage| storage.get_string("language"))
                .and_then(|code| Language::from_code(&code))
                .unwrap_or_default(),
            appearance: cc
                .storage
                .and_then(|storage| storage.get_string("appearance"))
                .and_then(|code| style::Appearance::from_code(&code))
                .unwrap_or_default(),
            send_mode: cc
                .storage
                .and_then(|storage| eframe::get_value(storage, "send_mode"))
                .unwrap_or_default(),
            epoch: 0,
            folder: cc
                .storage
                .and_then(|storage| storage.get_string("folder"))
                .map(PathBuf::from)
                .unwrap_or_default(),
            filters: None,
            search: String::new(),
            native_search: NativeSearch::default(),
            action_dialog: None,
            action_serial: 0,
            action_pending: HashMap::new(),
            composer_focus: None,
            restore_composer_focus: false,
            pair_dialog: false,
            device_tab: ui::DeviceTab::default(),
            pairing_qr: None,
            quota_dialog: false,
            showing_account_quota: false,
            quota_provider: Provider::Codex,
            quota_pending: HashSet::new(),
            quotas: HashMap::new(),
            token_usage: None,
            token_usage_error: None,
            token_usage_pending: false,
            token_period: UsagePeriod::default(),
            codex_quota_reset: CodexQuotaReset::default(),
            target_device_url: String::new(),
            target_device_code: String::new(),
            target_device_pending: false,
            target_device_error: None,
            relay_url,
            relay_token: String::new(),
            relay_note: None,
            settings_page: None,
            note: None,
            markdown: Default::default(),
            files: Default::default(),
            git: Default::default(),
            #[cfg(not(target_os = "macos"))]
            attachment_picker: None,
            thumbnails: Default::default(),
            brand,
        };
        app.appearance.apply(&cc.egui_ctx);
        app.restore_model_preference();
        app.restore_permission_mode(&cc.egui_ctx);
        app
    }

    fn workspace(&self) -> Workspace {
        match &self.remote {
            Some(remote) => Workspace::Remote(remote.clone()),
            None => Workspace::Local(self.service.clone()),
        }
    }

    fn background(
        &self,
        ctx: &egui::Context,
        f: impl FnOnce(Workspace) -> Update + Send + 'static,
    ) {
        let outbox = self.outbox.clone();
        let ctx = ctx.clone();
        let workspace = self.workspace();
        let epoch = self.workspace_epoch;
        std::thread::spawn(move || {
            let _ = outbox.send((epoch, f(workspace)));
            ctx.request_repaint();
        });
    }

    fn connect_device(&mut self, ctx: &egui::Context) {
        if !self.can_switch_workspace() {
            return;
        }
        self.target_device_pending = true;
        self.target_device_error = None;
        let address = self.target_device_url.clone();
        let code = std::mem::take(&mut self.target_device_code);
        self.background(ctx, move |_| {
            Update::Connected(RemoteClient::connect(&address, &code))
        });
    }

    fn return_to_local(&mut self, ctx: &egui::Context) {
        if !self.can_switch_workspace() {
            return;
        }
        if let Some(saved) = self.local_view.take()
            && let Some(remote) = self.remote.take()
        {
            self.recent_device = Some((remote, self.replace_view(saved)));
            self.reload_workspace(ctx);
        }
    }

    fn can_switch_workspace(&self) -> bool {
        !self.target_device_pending
            && !self.view.pending
            && !self.run_command_pending
            && self.action_pending.is_empty()
            && !self.codex_quota_reset.pending
    }

    fn can_navigate_in_workspace(&self) -> bool {
        self.remote.is_none() || (!self.view.pending && !self.run_command_pending)
    }

    fn reopen_device(&mut self, ctx: &egui::Context) {
        if self.can_switch_workspace()
            && let Some((remote, saved)) = self.recent_device.take()
        {
            self.open_device(ctx, remote, saved);
        }
    }

    fn open_device(&mut self, ctx: &egui::Context, remote: RemoteClient, saved: SavedView) {
        self.local_view = Some(self.replace_view(saved));
        self.remote = Some(remote);
        self.reload_workspace(ctx);
    }

    fn replace_view(&mut self, saved: SavedView) -> SavedView {
        SavedView {
            view: std::mem::replace(&mut self.view, saved.view),
            folder: std::mem::replace(&mut self.folder, saved.folder),
            model_preferences: std::mem::replace(
                &mut self.model_preferences,
                saved.model_preferences,
            ),
        }
    }

    fn reload_workspace(&mut self, ctx: &egui::Context) {
        self.workspace_epoch += 1;
        self.catalog_epoch += 1;
        self.begin_navigation();
        self.inventories.clear();
        self.models.clear();
        self.model_errors.clear();
        self.inventory_pending.clear();
        self.model_pending.clear();
        self.running.clear();
        self.recent_run = None;
        self.title_revision = 0;
        self.activity_pending = false;
        self.last_runs = Instant::now() - Duration::from_secs(2);
        self.remote_error = None;
        self.run_command_pending = false;
        self.filters = None;
        self.search.clear();
        self.native_search = NativeSearch::default();
        self.action_dialog = None;
        self.action_pending.clear();
        self.quota_dialog = false;
        self.quotas.clear();
        self.quota_pending.clear();
        self.token_usage = None;
        self.token_usage_error = None;
        self.token_usage_pending = false;
        self.codex_quota_reset = CodexQuotaReset::default();
        self.pair_dialog = false;
        self.pairing_qr = None;
        self.files = files::FileViewer::default();
        self.git.reset_for_workspace();
        self.thumbnails = thumbnails::Thumbnails::default();
        self.markdown = egui_commonmark::CommonMarkCache::default();
        #[cfg(not(target_os = "macos"))]
        {
            self.attachment_picker = None;
        }
        self.note = None;
        self.restore_permission_mode(ctx);
        self.refresh_inventory(ctx, false);
        self.load_models(ctx, self.view.provider, false);
        if self.view.run.is_none() {
            self.read_history(ctx, self.view.snapshot.is_some());
        }
    }

    fn refresh_inventory(&mut self, ctx: &egui::Context, force: bool) {
        let epoch = self.catalog_epoch;
        for provider in Provider::ALL {
            if self.inventory_pending.insert(provider) {
                self.background(ctx, move |service| {
                    Update::Inventory(epoch, provider, service.inventory(provider, force))
                });
            }
        }
    }

    fn load_models(&mut self, ctx: &egui::Context, provider: Provider, force: bool) {
        if !self.model_pending.insert(provider) {
            return;
        }
        let catalog = self.models.get(&provider);
        let missing_model = provider == Provider::Codex
            && catalog.is_some_and(|catalog| {
                self.view
                    .selected_model(Some(catalog))
                    .is_some_and(|id| !catalog.models.iter().any(|model| model.id == id))
            });
        let epoch = self.catalog_epoch;
        self.background(ctx, move |service| {
            Update::Models(
                epoch,
                provider,
                service.models(provider, force || missing_model),
            )
        });
    }

    fn load_quota(&mut self, ctx: &egui::Context, provider: Provider) {
        if provider == Provider::Codex && self.codex_quota_reset.pending {
            return;
        }
        if self.quota_pending.insert(provider) {
            self.background(ctx, move |workspace| {
                Update::Quota(provider, workspace.quota(provider))
            });
        }
    }

    fn load_token_usage(&mut self, ctx: &egui::Context) {
        if self.token_usage_pending {
            return;
        }
        self.token_usage_pending = true;
        self.token_usage_error = None;
        self.background(ctx, |service| Update::TokenUsage(service.token_usage()));
    }

    fn submit_quota_reset(&mut self, ctx: &egui::Context) {
        let reset = &mut self.codex_quota_reset;
        if let Some(id) = reset.request
            && !reset.pending
        {
            reset.pending = true;
            reset.error = None;
            self.background(ctx, move |workspace| {
                Update::CodexQuotaReset(workspace.reset_codex_quota(id))
            });
        }
    }

    fn open_settings(&mut self) {
        let mut page = SettingsPage::default();
        match crate::settings::cli_proxy() {
            Ok(Some(proxy)) => page.proxy_address = proxy.to_string(),
            Ok(None) => {}
            Err(error) => page.proxy_result = Some(Err(error)),
        }
        self.settings_page = Some(page);
    }

    fn save_proxy_settings(&mut self, ctx: &egui::Context) {
        let Some(page) = &mut self.settings_page else {
            return;
        };
        let result = crate::settings::save_cli_proxy(&page.proxy_address);
        let saved = result.is_ok();
        page.proxy_result = Some(result);
        if saved {
            self.service.invalidate_cli_catalogs();
        }
        if saved && self.remote.is_none() {
            self.catalog_epoch += 1;
            self.inventory_pending.clear();
            self.model_pending.clear();
            self.models.clear();
            self.model_errors.clear();
            self.refresh_inventory(ctx, true);
            self.load_models(ctx, self.view.provider, true);
        }
    }

    fn codex_projects(&self) -> &[Project] {
        self.inventories
            .get(&Provider::Codex)
            .and_then(|result| result.as_ref().ok())
            .map_or(&[], |inventory| inventory.projects.as_slice())
    }

    fn codex_project(&self, id: &str) -> Option<&Project> {
        self.codex_projects()
            .iter()
            .find(|project| project.id == id)
    }

    fn restore_model_preference(&mut self) {
        let provider = self.view.provider;
        self.view.selection = self
            .model_preferences
            .get(&provider)
            .cloned()
            .unwrap_or_default();
        if let Some(catalog) = self.models.get(&provider) {
            self.view.selection.retain_supported(catalog);
        }
    }

    fn restore_permission_mode(&mut self, ctx: &egui::Context) {
        self.permission_serial += 1;
        let serial = self.permission_serial;
        let provider = self.view.provider;
        let target = self.view.permission_target();
        self.view.permission_mode = None;
        if self.remote.is_some() {
            self.permission_pending = true;
            self.background(ctx, move |workspace| {
                Update::Permission(serial, workspace.permission_mode(provider, &target))
            });
        } else {
            self.finish_permission(self.service.permission_mode(provider, &target));
        }
    }

    fn save_permission_mode(&mut self, ctx: &egui::Context, mode: Option<String>) {
        self.permission_serial += 1;
        let serial = self.permission_serial;
        let provider = self.view.provider;
        let target = self.view.permission_target();
        if self.remote.is_some() {
            self.permission_pending = true;
            self.background(ctx, move |workspace| {
                let result = workspace.save_permission_mode(provider, &target, mode.as_deref());
                Update::Permission(serial, result.map(|()| mode))
            });
        } else {
            let result = self
                .service
                .save_permission_mode(provider, &target, mode.as_deref());
            self.finish_permission(result.map(|()| mode));
        }
    }

    fn finish_permission(&mut self, result: Result<Option<String>, String>) {
        self.permission_pending = false;
        match result {
            Ok(mode) => self.view.permission_mode = mode,
            Err(problem) => {
                self.note = Some(problem);
            }
        }
    }

    fn reset_snapshot_poll(&mut self) {
        self.snapshot_serial += 1;
        self.snapshot_pending = false;
        self.last_snapshot = Instant::now() - Duration::from_secs(1);
    }

    fn begin_navigation(&mut self) {
        self.epoch += 1;
        self.snapshot_error = None;
        self.reset_snapshot_poll();
    }

    fn current_git_cwd(&self) -> Option<PathBuf> {
        let path = self
            .view
            .session
            .as_ref()
            .map(|session| &session.cwd)
            .filter(|path| !path.as_os_str().is_empty())
            .or_else(|| (!self.folder.as_os_str().is_empty()).then_some(&self.folder))?;
        Some(path.clone())
    }

    fn sync_git_cwd(&mut self, ctx: &egui::Context) {
        let cwd = self.current_git_cwd();
        if self.git.set_cwd(cwd) && self.git.is_open() {
            self.refresh_git(ctx);
        }
    }

    fn toggle_git_panel(&mut self, ctx: &egui::Context) {
        if self.git.is_open() {
            self.git.close();
            return;
        }
        self.files.close();
        self.git.open = true;
        let cwd = self.current_git_cwd();
        self.git.set_cwd(cwd);
        self.refresh_git(ctx);
    }

    fn refresh_git(&mut self, ctx: &egui::Context) {
        let Some(cwd) = self.current_git_cwd() else {
            self.git.loading = false;
            return;
        };
        self.git.cwd = Some(cwd.clone());
        self.git.serial += 1;
        let serial = self.git.serial;
        self.git.loading = true;
        self.git.error = None;
        self.background(ctx, move |workspace| {
            Update::GitStatus(serial, workspace.git_status(&cwd))
        });
    }

    fn load_git_diff(&mut self, ctx: &egui::Context, request: crate::git::GitDiffRequest) {
        let Some(cwd) = self.git.cwd.clone().or_else(|| self.current_git_cwd()) else {
            return;
        };
        self.git.diff_serial += 1;
        let serial = self.git.diff_serial;
        self.git.diff_loading = true;
        self.git.diff_error = None;
        self.background(ctx, move |workspace| {
            Update::GitDiff(serial, workspace.git_diff(&cwd, &request))
        });
    }

    fn run_git_action(&mut self, ctx: &egui::Context, action: crate::git::GitAction) {
        if self.git.action_pending {
            return;
        }
        let Some(cwd) = self.git.cwd.clone().or_else(|| self.current_git_cwd()) else {
            return;
        };
        self.git.serial += 1;
        let serial = self.git.serial;
        self.git.action_pending = true;
        self.git.error = None;
        self.git.notice = None;
        let action_for_update = action.clone();
        self.background(ctx, move |workspace| {
            let result = workspace.git_action(&cwd, &action);
            Update::GitAction(serial, action_for_update, result)
        });
    }

    fn generate_git_commit_message(&mut self, ctx: &egui::Context, amend: bool) {
        if self.git.generating_message || self.git.action_pending {
            return;
        }
        let Some(cwd) = self.git.cwd.clone().or_else(|| self.current_git_cwd()) else {
            return;
        };
        self.git.generate_serial += 1;
        let serial = self.git.generate_serial;
        self.git.generating_message = true;
        self.git.error = None;
        self.git.notice = None;
        let provider = Some(self.view.provider);
        let model = self
            .view
            .selected_model(self.models.get(&self.view.provider))
            .map(str::to_owned);
        let language = Some(self.language.code().to_owned());
        self.background(ctx, move |workspace| {
            let result = workspace.git_generate_commit_message(
                &cwd,
                amend,
                provider,
                model.as_deref(),
                language.as_deref(),
            );
            Update::GitCommitMessage(serial, result)
        });
    }

    fn handle_git_command(&mut self, ctx: &egui::Context, command: git::GitCommand) {
        match command {
            git::GitCommand::Refresh => self.refresh_git(ctx),
            git::GitCommand::LoadDiff(request) => self.load_git_diff(ctx, request),
            git::GitCommand::RunAction(action) => self.run_git_action(ctx, action),
            git::GitCommand::GenerateCommitMessage { amend } => {
                self.generate_git_commit_message(ctx, amend);
            }
            git::GitCommand::Close => self.git.close(),
        }
    }

    fn new_view(
        &mut self,
        ctx: &egui::Context,
        folder: Option<PathBuf>,
        project_id: Option<String>,
    ) {
        if !self.can_navigate_in_workspace() {
            return;
        }
        let provider = if project_id.is_some() {
            Provider::Codex
        } else if folder.is_some() {
            self.filters.unwrap_or(self.view.provider)
        } else {
            self.view.provider
        };
        if let Some(folder) = folder {
            self.folder = folder;
        }
        self.begin_navigation();
        self.view = View::new(provider);
        self.restore_model_preference();
        self.view.project_id = project_id;
        self.note = None;
        self.restore_permission_mode(ctx);
        self.load_models(ctx, self.view.provider, false);
    }

    fn open(&mut self, ctx: &egui::Context, session: Session) {
        if !self.can_navigate_in_workspace() {
            return;
        }
        self.begin_navigation();
        self.note = None;
        self.view = View::new(session.provider);
        self.view.id = Some(session.id.clone());
        self.view.title = Some(session.title.clone());
        if let Some(run) = self.running.iter().find(|turn| {
            turn.provider == session.provider && turn.session_id.as_deref() == Some(&session.id)
        }) {
            self.view.run = Some(run.id.clone());
            self.view.run_model.clone_from(&run.model);
        }
        self.view.session = Some(session);
        self.restore_permission_mode(ctx);
        self.view.transcript.warning = Some("正在读取原会话…".into());
        self.load_models(ctx, self.view.provider, false);
        self.read_history(ctx, false);
    }

    fn open_run(&mut self, ctx: &egui::Context, run: RunSummary) {
        if !self.can_navigate_in_workspace() {
            return;
        }
        self.begin_navigation();
        self.note = None;
        self.view = View::new(run.provider);
        self.view.run = Some(run.id);
        self.view.run_model = run.model;
        self.view.title = Some(run.title);
        self.view.id = run.session_id;
        self.restore_permission_mode(ctx);
        self.load_models(ctx, self.view.provider, false);
        self.read_history(ctx, false);
    }

    fn read_history(&self, ctx: &egui::Context, refresh: bool) {
        if let Some(id) = &self.view.id {
            let provider = self.view.provider;
            let id = id.clone();
            let epoch = self.epoch;
            self.background(ctx, move |workspace| {
                Update::Transcript(epoch, workspace.history(provider, &id, refresh), refresh)
            });
        }
    }

    fn add_files(&mut self, paths: Vec<PathBuf>) {
        if self.view.pending {
            return;
        }
        let (images, references) = paths.into_iter().partition::<Vec<_>, _>(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    matches!(
                        ext.to_ascii_lowercase().as_str(),
                        "png" | "jpg" | "jpeg" | "gif" | "webp"
                    )
                })
                && path.is_file()
        });
        if self.remote.is_some() && !references.is_empty() {
            self.note = Some("远程会话支持从本机发送图片。".into());
        }
        for path in references.into_iter().filter(|_| self.remote.is_none()) {
            let directory = path.is_dir();
            let Ok(path) = path.into_os_string().into_string() else {
                self.note = Some("附件路径包含无法显示的字符。".into());
                continue;
            };
            if !self.view.local_files.iter().any(|file| file.path == path) {
                self.view
                    .local_files
                    .push(LocalAttachment { path, directory });
            }
        }
        if !images.is_empty() {
            self.add_images(images);
        }
    }

    fn add_images(&mut self, paths: Vec<PathBuf>) {
        let mut images = self.view.attachments.clone();
        for path in paths {
            if images.len() >= MAX_IMAGE_ATTACHMENTS {
                self.note = Some("最多可以附加四张图片".into());
                break;
            }
            match ImageAttachment::from_file(path) {
                Ok(image) => images.push(image),
                Err(error) => {
                    self.note = Some(error.to_string());
                    break;
                }
            }
        }
        match validate_image_attachments(&images) {
            Ok(()) => self.view.attachments = images,
            Err(error) => self.note = Some(error.to_string()),
        }
    }

    fn add_clipboard_image(&mut self, image: Result<ImageAttachment, String>) {
        let result = image.and_then(|image| {
            if self.view.pending {
                return Err("消息正在发送，请稍后再粘贴图片。".into());
            }
            if self.view.attachments.len() >= MAX_IMAGE_ATTACHMENTS {
                return Err("最多可以附加四张图片".into());
            }
            self.view.attachments.push(image);
            if let Err(error) = validate_image_attachments(&self.view.attachments) {
                self.view.attachments.pop();
                return Err(error.to_string());
            }
            Ok(())
        });
        self.note = result.err();
    }

    fn submit(&mut self, ctx: &egui::Context) {
        if self.view.pending
            || self.view.stopping
            || self.permission_pending
            || self.run_command_pending
            || self.native_pending()
            || !self.view.has_draft()
        {
            return;
        }
        let draft = std::mem::take(&mut self.view.draft);
        let catalog = self.models.get(&self.view.provider);
        let request = TurnRequest {
            provider: self.view.provider,
            session_id: self.view.id.clone(),
            project_id: self.view.project_id.clone(),
            additional_directories: self
                .view.startup.additional_directories
                .lines()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .collect(),
            context_files: self
                .view.startup.context_files
                .lines()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .collect(),
            agent: (self.view.provider != Provider::Codex
                && !self.view.startup.agent.trim().is_empty())
            .then(|| self.view.startup.agent.trim().into()),
            worktree: (self.view.id.is_none()
                && self.view.provider == Provider::Claude
                && !self.view.startup.worktree.trim().is_empty())
            .then(|| self.view.startup.worktree.trim().into()),
            options: ModelOptions {
                model: self.view.selected_model(catalog).map(str::to_owned),
                reasoning_effort: self.view.selection.reasoning_effort.clone(),
                fast_mode: self.view.selection.fast_mode,
                permission_mode: self
                    .view
                    .selected_permission(catalog)
                    .or_else(|| {
                        if self.view.provider == Provider::Codex {
                            catalog?.default_permission_mode
                        } else {
                            None
                        }
                    })
                    .map(str::to_owned),
                approval_policy: None,
                collaboration_mode: self.view.collaboration_mode,
            },
            cwd: self
                .view
                .session
                .as_ref()
                .map(|session| session.cwd.clone())
                .unwrap_or_else(|| self.folder.clone()),
            prompt: draft.clone(),
            images: Vec::new(),
            local_files: self.view.local_files.clone(),
        };
        let images = self.view.attachments.clone();
        self.view.pending = true;
        self.note = None;
        let epoch = self.epoch;
        let run = self.view.run.clone();
        let mode = if request.provider.supports_steering() && !self.view.editing_withdrawn {
            self.send_mode
        } else {
            SendMode::Queue
        };
        self.background(ctx, move |workspace| {
            Update::Submitted(
                epoch,
                draft,
                workspace.submit(run.as_deref(), mode, request, images),
            )
        });
    }

    fn run_command(&mut self, ctx: &egui::Context, id: String, command: RunCommand) {
        if self.run_command_pending {
            return;
        }
        if matches!(command, RunCommand::Stop) {
            self.view.stopping = true;
        }
        if self.remote.is_some() {
            self.run_command_pending = true;
            self.reset_snapshot_poll();
            self.background(ctx, move |workspace| {
                let result = workspace.run_command(&id, &command);
                Update::RunCommand(id, command, result)
            });
        } else {
            let result = self.workspace().run_command(&id, &command);
            self.finish_run_command(ctx, &id, command, result);
        }
    }

    fn finish_run_command(
        &mut self,
        ctx: &egui::Context,
        id: &str,
        command: RunCommand,
        result: Result<Option<ComposerDraft>, String>,
    ) {
        self.run_command_pending = false;
        let same_run =
            self.view.run.as_deref() == Some(id) || self.view.last_finished.as_deref() == Some(id);
        if !same_run {
            return;
        }
        self.last_snapshot = Instant::now() - Duration::from_secs(1);
        match result {
            Ok(draft) => {
                if let Some(draft) = draft {
                    self.view.show_draft(draft);
                    if let Some(editor) = self.composer_focus {
                        ctx.memory_mut(|memory| memory.request_focus(editor));
                    }
                }
                if let RunCommand::Answer { question_id, .. } = command
                    && let Some(snapshot) = &mut self.view.snapshot
                    && snapshot
                        .question
                        .as_ref()
                        .is_some_and(|request| request.id == question_id)
                {
                    self.view.question_form = None;
                    snapshot.question = None;
                }
            }
            Err(problem) => match command {
                RunCommand::Stop => {
                    self.view.stopping = false;
                    self.note = Some(problem);
                }
                RunCommand::Answer { question_id, .. } => {
                    if let Some(form) = &mut self.view.question_form
                        && form.id == question_id
                    {
                        form.error = Some(problem);
                    }
                }
                _ => self.note = Some(problem),
            },
        }
    }

    fn native_pending(&self) -> bool {
        self.view.id.as_ref().is_some_and(|id| {
            self.action_pending.contains_key(&(self.view.provider, id.clone()))
        })
    }

    fn open_action(&mut self, body: ActionBody) {
        let Some(session) = self.view.session.clone() else { return };
        self.action_serial += 1;
        self.action_dialog = Some(ActionDialog {
            id: self.action_serial,
            epoch: self.epoch,
            session,
            body,
            pending: false,
            error: None,
            status: None,
        });
    }

    fn request_native_action(&mut self, ctx: &egui::Context, action: engine::SessionAction) {
        if self.view.busy() || self.native_pending() { return; }
        let Some(dialog) = &mut self.action_dialog else { return };
        if dialog.pending || dialog.epoch != self.epoch { return; }
        self.action_serial += 1;
        let request = NativeRequest {
            serial: self.action_serial,
            dialog: dialog.id,
            epoch: dialog.epoch,
            session: dialog.session.clone(),
            action,
        };
        dialog.pending = true;
        dialog.error = None;
        dialog.status = None;
        self.action_pending.insert((request.session.provider, request.session.id.clone()), request.serial);
        self.background(ctx, move |service| {
            let result = service.session_action(&request.session, &request.action);
            Update::NativeAction(request, result)
        });
    }

    fn finish_native_action(
        &mut self,
        ctx: &egui::Context,
        request: NativeRequest,
        result: Result<serde_json::Value, String>,
    ) {
        use engine::SessionAction as A;
        let key = (request.session.provider, request.session.id.clone());
        if self.action_pending.get(&key) != Some(&request.serial) { return; }
        self.action_pending.remove(&key);
        let current = request.is_current(&self.view, self.epoch);
        let visible = current && self.action_dialog.as_ref().is_some_and(|dialog| {
            dialog.id == request.dialog && dialog.epoch == request.epoch
        });
        let mutation = matches!(request.action, A::Rename { .. } | A::Fork | A::Archive
            | A::Unarchive | A::GoalSet { .. } | A::GoalClear | A::CheckpointRewind { .. });
        if mutation && result.is_ok() {
            self.refresh_inventory(ctx, true);
        }
        if let Ok(value) = &result {
            match &request.action {
                A::Rename { name } => {
                    if let Some(Ok(inventory)) = self.inventories.get_mut(&request.session.provider)
                        && let Some(session) = inventory.sessions.iter_mut().find(|row| row.id == request.session.id)
                    {
                        session.title = name.clone();
                    }
                    if current {
                        self.view.title = Some(name.clone());
                        if let Some(session) = &mut self.view.session { session.title = name.clone(); }
                    }
                }
                A::Fork if current => {
                    if let Some(id) = value["session_id"].as_str() {
                        let mut session = request.session.clone();
                        session.id = id.into();
                        session.title = format!("{} ({})", session.title, self.language.text("创建分支"));
                        self.action_dialog = None;
                        self.open(ctx, session);
                        self.restore_composer_focus = true;
                        return;
                    }
                }
                A::Archive if current => {
                    self.action_dialog = None;
                    self.new_view(ctx, None, None);
                    self.restore_composer_focus = true;
                    return;
                }
                A::CheckpointRewind { .. } if current => self.read_history(ctx, true),
                _ => {}
            }
        }
        if !visible { return; }
        let dialog = self.action_dialog.as_mut().unwrap();
        dialog.pending = false;
        match result {
            Err(error) => dialog.error = Some(error),
            Ok(value) => {
                dialog.status = Some("操作完成");
                match (&mut dialog.body, request.action) {
                    (ActionBody::Goal { objective, budget, loaded }, A::GoalGet) => {
                        *objective = value["goal"]["objective"].as_str().unwrap_or_default().into();
                        *budget = value["goal"]["tokenBudget"].as_u64().map(|n| n.to_string()).unwrap_or_default();
                        *loaded = true;
                        dialog.status = None;
                    }
                    (ActionBody::Goal { objective, budget, .. }, A::GoalClear) => {
                        objective.clear();
                        budget.clear();
                    }
                    (ActionBody::Context, A::Context) => {
                        self.view.context_report = Some(engine::work::ContextUsage::new(value["totalTokens"].as_u64(), value["maxTokens"].as_u64()));
                        dialog.status = None;
                    }
                    (ActionBody::Rewind { selected, preview }, A::CheckpointPreview { message_id })
                        if selected.as_deref() == Some(&message_id) => {
                        *preview = (value["canRewind"] == true).then_some((message_id, value));
                        dialog.status = preview.is_none().then_some("引擎未确认可回退，未执行文件更改。");
                    }
                    (ActionBody::Rewind { preview, .. }, A::CheckpointRewind { .. }) => *preview = None,
                    (ActionBody::Extensions { result, .. }, A::Extensions | A::McpStatus) => {
                        *result = Some(value);
                        dialog.status = None;
                    }
                    (ActionBody::Rename { .. }, A::Rename { .. }) => {
                        self.action_dialog = None;
                        self.restore_composer_focus = true;
                    }
                    (ActionBody::Confirm { .. }, A::Fork) => {
                        dialog.error = Some("引擎未返回新会话，未切换页面。".into());
                    }
                    _ => {}
                }
            }
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((workspace, event)) = self.inbox.try_recv() {
            if workspace != self.workspace_epoch {
                continue;
            }
            match event {
                Update::Connected(result) => {
                    self.target_device_pending = false;
                    match result {
                        Ok(remote) => {
                            self.target_device_url = remote.address().to_string();
                            let saved = SavedView {
                                view: View::new(Provider::Codex),
                                folder: PathBuf::new(),
                                model_preferences: HashMap::new(),
                            };
                            if self.can_switch_workspace() {
                                self.recent_device = None;
                                self.open_device(ctx, remote, saved);
                            } else {
                                self.recent_device = Some((remote, saved));
                            }
                        }
                        Err(error) => self.target_device_error = Some(error),
                    }
                }
                Update::Activity(result) => {
                    self.activity_pending = false;
                    match result {
                        Ok(activity) => {
                            if self.remote_error.take().is_some() {
                                self.refresh_inventory(ctx, false);
                                self.load_models(ctx, self.view.provider, false);
                                if self.view.run.is_none() {
                                    self.read_history(ctx, false);
                                }
                            }
                            self.apply_activity(ctx, activity);
                        }
                        Err(error) => self.remote_error = Some(error),
                    }
                }
                Update::Snapshot(serial, id, result) if serial == self.snapshot_serial => {
                    self.snapshot_pending = false;
                    if self.view.run.as_deref() == Some(&id) {
                        match result {
                            Ok(Some(snapshot)) => {
                                self.snapshot_error = None;
                                self.apply_snapshot(ctx, &id, snapshot);
                            }
                            Ok(None) => {
                                self.snapshot_error = None;
                                self.view.last_finished = Some(id);
                                self.view.run = None;
                                self.view.run_model = None;
                                self.view.snapshot = None;
                                self.view.stopping = false;
                                self.read_history(ctx, true);
                            }
                            Err(error) => self.snapshot_error = Some(error),
                        }
                    }
                }
                Update::Permission(serial, result) if serial == self.permission_serial => {
                    self.finish_permission(result);
                }
                Update::RunCommand(id, command, result) => {
                    self.finish_run_command(ctx, &id, command, result);
                }
                Update::GitStatus(serial, result) => {
                    self.git.apply_status(serial, result);
                }
                Update::GitDiff(serial, result) => {
                    self.git.apply_diff(serial, result);
                }
                Update::GitAction(serial, action, result) => {
                    if let Some(request) = self.git.apply_action(serial, &action, result) {
                        self.load_git_diff(ctx, request);
                    }
                }
                Update::GitCommitMessage(serial, result) => {
                    self.git.apply_generated_message(serial, result);
                }
                Update::Inventory(epoch, provider, result) if epoch == self.catalog_epoch => {
                    self.inventory_pending.remove(&provider);
                    if self.view.provider == provider
                        && !self.view.busy()
                        && let (Some(id), Ok(inventory)) = (&self.view.id, &result)
                        && let Some(session) = inventory.sessions.iter().find(|row| &row.id == id)
                    {
                        self.view.title = Some(session.title.clone());
                    }
                    self.inventories.insert(provider, result);
                }
                Update::Models(epoch, provider, result) if epoch == self.catalog_epoch => {
                    self.model_pending.remove(&provider);
                    match result {
                        Ok(catalog) => {
                            self.model_errors.remove(&provider);
                            if self.view.provider == provider && self.view.is_new() {
                                self.view.selection.retain_supported(&catalog);
                            }
                            self.models.insert(provider, catalog);
                        }
                        Err(problem) => {
                            self.model_errors.insert(provider, problem);
                        }
                    }
                }
                Update::Quota(provider, result) => {
                    self.quota_pending.remove(&provider);
                    self.quotas.insert(provider, result);
                }
                Update::NativeSearch(serial, result) if serial == self.native_search.serial => {
                    self.native_search.pending = false;
                    match result {
                        Ok(value) => self.native_search.hits =
                            value["data"].as_array().cloned().unwrap_or_default(),
                        Err(error) => self.native_search.error = Some(error),
                    }
                }
                Update::NativeAction(request, result) => self.finish_native_action(ctx, request, result),
                Update::TokenUsage(report) => {
                    self.token_usage_pending = false;
                    match report {
                        Ok(report) => self.token_usage = Some(report),
                        Err(error) => self.token_usage_error = Some(error),
                    }
                }
                Update::CodexQuotaReset(result) => {
                    let reset = &mut self.codex_quota_reset;
                    reset.pending = false;
                    match result {
                        Ok(outcome) => {
                            reset.request = None;
                            reset.message = Some(outcome.message());
                            self.load_quota(ctx, Provider::Codex);
                        }
                        Err(error) => reset.error = Some(error),
                    }
                }
                Update::Transcript(epoch, result, refresh) if epoch == self.epoch => match result {
                    Ok((session, history)) => {
                        if !self.view.busy() && (session.named || self.view.title.is_none()) {
                            self.view.title = Some(session.title.clone());
                        }
                        self.view.session = Some(session);
                        let replace = !refresh
                            || self.view.snapshot.as_ref().is_none_or(|snapshot| {
                                snapshot.appears_in(&history, self.view.transcript.messages.len())
                            });
                        if replace {
                            self.view.transcript = history;
                            self.view.context_report = None;
                            if refresh && !self.view.busy() {
                                self.view.snapshot = None;
                                self.view.run_model = None;
                            }
                        } else {
                            self.view.transcript.warning = history.warning;
                        }
                    }
                    Err(problem) => self.view.transcript.warning = Some(problem),
                },
                Update::Submitted(epoch, draft, result) if epoch == self.epoch => {
                    self.view.pending = false;
                    match result {
                        Ok(run) => {
                            if let Some(run) = run {
                                self.view.title = Some(run.title);
                                self.view.run = Some(run.id);
                                self.view.run_model = run.model;
                                self.view.snapshot = None;
                                self.reset_snapshot_poll();
                            }
                            self.view.attachments.clear();
                            self.view.local_files.clear();
                            self.view.editing_withdrawn = false;
                            if !self.view.has_draft()
                                && let Some(draft) = self.view.other_drafts.pop_front()
                            {
                                self.view.show_draft(draft);
                            }
                        }
                        Err(problem) => {
                            if !draft.is_empty() {
                                if !self.view.draft.is_empty() {
                                    self.view.draft.insert_str(0, "\n\n");
                                }
                                self.view.draft.insert_str(0, &draft);
                            }
                            self.note = Some(problem);
                        }
                    }
                }
                _ => {}
            }
        }
        let remote = self.remote.is_some();
        let activity_interval = if remote { 1_500 } else { 400 };
        if !self.activity_pending
            && self.last_runs.elapsed() >= Duration::from_millis(activity_interval)
        {
            self.last_runs = Instant::now();
            if let Some(remote) = self.remote.clone() {
                self.activity_pending = true;
                self.background(ctx, move |_| Update::Activity(remote.activity()));
            } else {
                self.apply_activity(ctx, self.service.activity());
            }
        }
        if let Some(id) = self.view.run.clone() {
            if let Some(remote) = self.remote.clone() {
                if !self.snapshot_pending
                    && !self.run_command_pending
                    && self.last_snapshot.elapsed() >= Duration::from_millis(600)
                {
                    self.last_snapshot = Instant::now();
                    self.snapshot_pending = true;
                    let serial = self.snapshot_serial;
                    self.background(ctx, move |_| {
                        let result = remote.snapshot(&id);
                        Update::Snapshot(serial, id, result)
                    });
                }
            } else {
                match self.service.run(&id) {
                    Ok(run) => {
                        self.apply_snapshot(ctx, &id, run.snapshot());
                    }
                    Err(problem) => {
                        self.view.run = None;
                        self.view.run_model = None;
                        self.view.stopping = false;
                        self.note = Some(problem);
                    }
                }
            }
        }
        let interval = if self.view.busy() || self.pair_dialog {
            120
        } else {
            400
        };
        ctx.request_repaint_after(Duration::from_millis(interval));
    }

    fn apply_activity(&mut self, ctx: &egui::Context, activity: Activity) {
        self.running = activity.running;
        let latest = activity.finished.first().map(|run| run.id.clone());
        if self.inventory_pending.is_empty()
            && ((latest.is_some() && self.recent_run != latest)
                || self.title_revision != activity.title_revision)
        {
            self.recent_run = latest;
            self.title_revision = activity.title_revision;
            self.refresh_inventory(ctx, false);
        }
        if !self.view.busy()
            && let Some(id) = &self.view.id
        {
            let matches = |run: &&RunSummary| {
                run.provider == self.view.provider && run.session_id.as_ref() == Some(id)
            };
            if let Some(run) = self.running.iter().find(matches)
                && self.view.last_finished.as_deref() != Some(&run.id)
            {
                self.view.run = Some(run.id.clone());
                self.view.run_model.clone_from(&run.model);
            } else if let Some(run) = activity.finished.iter().find(matches)
                && self.view.last_finished.as_ref() != Some(&run.id)
            {
                self.view.last_finished = Some(run.id.clone());
                self.read_history(ctx, true);
            }
        }
    }

    fn apply_snapshot(&mut self, ctx: &egui::Context, id: &str, snapshot: Snapshot) {
        self.view.context_report = None;
        if self.view.id.is_none() {
            self.view.id.clone_from(&snapshot.run.session_id);
        }
        if snapshot.run.model.is_some() {
            self.view.run_model.clone_from(&snapshot.run.model);
        }
        self.view.title = Some(snapshot.run.title.clone());
        let finished = snapshot.status != Status::Running;
        self.view.snapshot = Some(snapshot);
        if finished {
            self.view.last_finished = Some(id.to_owned());
            self.view.run = None;
            self.view.stopping = false;
            if self.remote.is_some() {
                self.running.retain(|run| run.id != id);
                self.last_runs = Instant::now() - Duration::from_secs(2);
            } else {
                self.running = self.service.running();
            }
            self.read_history(ctx, true);
            self.refresh_inventory(ctx, false);
            if self.git.is_open() {
                self.refresh_git(ctx);
            }
        }
    }
}

impl eframe::App for Desktop {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        if ctx.input(|input| input.events.contains(&egui::Event::WindowFocused(true))) {
            self.load_models(ctx, self.view.provider, false);
        }
        self.clipboard.editor.set(None);
        self.language.apply(ctx);
        self.paint(ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string("relay_url", self.relay_url.clone());
        storage.set_string("language", self.language.code().to_owned());
        storage.set_string("appearance", self.appearance.code().to_owned());
        eframe::set_value(storage, "send_mode", &self.send_mode);
        let (preferences, folder) = match &self.local_view {
            Some(local) => (&local.model_preferences, &local.folder),
            None => (&self.model_preferences, &self.folder),
        };
        eframe::set_value(storage, "model_preferences", preferences);
        if let Some(path) = folder.to_str() {
            storage.set_string("folder", path.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_results_require_the_same_provider_session_and_navigation_epoch() {
        let mut view = View::new(Provider::Codex);
        view.id = Some("origin".into());
        let request = NativeRequest {
            serial: 4,
            dialog: 3,
            epoch: 8,
            session: Session {
                provider: Provider::Codex, id: "origin".into(), title: "Original".into(),
                named: true, cwd: PathBuf::from("/workspace"), project_id: None, model: None,
                updated: 0, preview: None, rollout: None,
            },
            action: engine::SessionAction::Fork,
        };
        assert!(request.is_current(&view, 8));
        assert!(!request.is_current(&view, 9), "returning to origin is a different view");
        view.id = Some("other".into());
        assert!(!request.is_current(&view, 8));
        view.id = Some("origin".into());
        view.provider = Provider::Claude;
        assert!(!request.is_current(&view, 8));
    }

    #[test]
    fn startup_settings_follow_the_draft_and_new_views_start_clean() {
        let mut view = View::new(Provider::Claude);
        view.draft = "original".into();
        view.startup.agent = "original-agent".into();
        view.startup.additional_directories = "/first".into();
        let replacement = StartupDraft { agent: "replacement-agent".into(), ..Default::default() };
        view.show_draft(ComposerDraft {
            text: "replacement".into(), images: Vec::new(), local_files: Vec::new(),
            startup: replacement.clone(), withdrawn: false,
        });
        assert_eq!(view.startup, replacement);
        let previous = view.other_drafts.pop_front().unwrap();
        assert_eq!(previous.startup.agent, "original-agent");
        assert_eq!(previous.startup.additional_directories, "/first");
        view.show_draft(previous);
        assert_eq!(view.startup.agent, "original-agent");
        view.clear_startup_settings();
        assert_eq!(view.startup, StartupDraft::default());
        assert!(view.other_drafts.iter().all(|draft| draft.startup == StartupDraft::default()));
        assert_eq!(View::new(Provider::Claude).startup, StartupDraft::default());
        assert_eq!(View::new(Provider::Codex).startup, StartupDraft::default());
    }

    #[test]
    fn session_context_uses_the_newest_report_without_inventing_numbers() {
        let mut view = View::new(Provider::Codex);
        assert!(view.latest_context().is_none());
        let mut old = engine::Message::new(engine::Role::Assistant, "old".into());
        old.work.context = Some(engine::work::ContextUsage::new(Some(10), Some(100)));
        let mut recent = engine::Message::new(engine::Role::Assistant, "recent".into());
        recent.work.context = Some(engine::work::ContextUsage::new(Some(30), Some(100)));
        view.transcript.messages = vec![old, recent, engine::Message::new(engine::Role::User, "next".into())];
        assert_eq!(view.latest_context().unwrap().used, Some(30));
        view.context_report = Some(engine::work::ContextUsage::default());
        assert_eq!(view.latest_context().unwrap().used, None, "new unknown report must not reuse older numbers");
    }

    #[test]
    fn remembered_model_selection_keeps_supported_options() {
        let mut selection = ModelSelection {
            model: Some("chosen".into()),
            reasoning_effort: Some("high".into()),
            fast_mode: Some(false),
        };
        let mut catalog = ModelCatalog {
            models: vec![engine::Model {
                id: "chosen".into(),
                label: "Chosen".into(),
                reasoning_efforts: vec!["high".into()],
                default_reasoning_effort: None,
                fast_mode: true,
                default_fast_mode: Some(true),
            }],
            ..Default::default()
        };

        selection.retain_supported(&catalog);
        assert_eq!(selection.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(selection.fast_mode, Some(false));

        catalog.models[0].reasoning_efforts.clear();
        selection.retain_supported(&catalog);
        assert_eq!(selection.model.as_deref(), Some("chosen"));
        assert_eq!(selection.reasoning_effort, None);
        assert_eq!(selection.fast_mode, Some(false));

        catalog.models.clear();
        selection.retain_supported(&catalog);
        assert_eq!(selection.model, None);
        assert_eq!(selection.fast_mode, None);
    }
}
