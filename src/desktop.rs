use crate::engine::attachments::{
    ImageAttachment, MAX_IMAGE_ATTACHMENTS, validate_image_attachments,
};
mod ui;

use crate::{
    engine::{Inventory, Model, Provider, Role, Session, Transcript, TurnRequest},
    service::{RunSummary, Service, Snapshot, Status},
    web::Gateway,
};
use eframe::egui;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant},
};

pub fn run(service: Service, gateway: Gateway) -> eframe::Result {
    let icon = image::load_from_memory(include_bytes!("../public/icons/192.png"))
        .expect("bundled NaroLink icon")
        .into_rgba8();
    let (width, height) = icon.dimensions();
    let viewport = egui::ViewportBuilder::default()
        .with_title("NaroLink")
        .with_inner_size([1160., 792.])
        .with_min_inner_size([760., 548.])
        .with_icon(egui::IconData {
            rgba: icon.into_raw(),
            width,
            height,
        });
    eframe::run_native(
        "NaroLink",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(move |cc| {
            ui::setup(&cc.egui_ctx);
            let mut app = Desktop::new(service, gateway, cc);
            app.refresh_inventory(&cc.egui_ctx, false);
            app.load_models(&cc.egui_ctx, Provider::Codex, false);
            Ok(Box::new(app))
        }),
    )
}

enum Update {
    Inventory(Provider, Result<Inventory, String>),
    Models(Provider, Result<Vec<Model>, String>),
    Transcript(u64, Result<(Session, Transcript), String>, bool),
    Started(u64, Result<RunSummary, String>),
}

struct Desktop {
    service: Service,
    gateway: Gateway,
    inbox: Receiver<Update>,
    outbox: Sender<Update>,
    inventories: HashMap<Provider, Result<Inventory, String>>,
    models: HashMap<Provider, Result<Vec<Model>, String>>,
    inventory_pending: HashSet<Provider>,
    model_pending: HashSet<Provider>,
    running: Vec<RunSummary>,
    last_runs: Instant,
    recent_run: Option<String>,
    view: View,
    epoch: u64,
    folder: PathBuf,
    filters: Option<Provider>,
    search: String,
    pair_dialog: bool,
    relay_url: String,
    relay_token: String,
    relay_note: Option<String>,
    note: Option<String>,
    markdown: egui_commonmark::CommonMarkCache,
}

struct View {
    provider: Provider,
    session: Option<Session>,
    id: Option<String>,
    run: Option<String>,
    snapshot: Option<Snapshot>,
    transcript: Transcript,
    pending: bool,
    last_finished: Option<String>,
    model: Option<String>,
    draft: String,
    attachments: Vec<ImageAttachment>,
    title: Option<String>,
}

impl View {
    fn new(provider: Provider) -> Self {
        Self {
            provider,
            session: None,
            id: None,
            run: None,
            snapshot: None,
            transcript: Transcript::default(),
            pending: false,
            last_finished: None,
            model: None,
            draft: String::new(),
            attachments: Vec::new(),
            title: None,
        }
    }

    fn busy(&self) -> bool {
        self.pending || self.run.is_some()
    }
}

impl Desktop {
    fn new(service: Service, gateway: Gateway, cc: &eframe::CreationContext<'_>) -> Self {
        let (outbox, inbox) = mpsc::channel();
        let relay_url = cc
            .storage
            .and_then(|storage| storage.get_string("relay_url"))
            .unwrap_or_default();
        gateway.remote().restore(&relay_url);
        Self {
            service,
            gateway,
            inbox,
            outbox,
            inventories: HashMap::new(),
            models: HashMap::new(),
            inventory_pending: HashSet::new(),
            model_pending: HashSet::new(),
            running: Vec::new(),
            last_runs: Instant::now(),
            recent_run: None,
            view: View::new(Provider::Codex),
            epoch: 0,
            folder: cc
                .storage
                .and_then(|storage| storage.get_string("folder"))
                .map(PathBuf::from)
                .unwrap_or_default(),
            filters: None,
            search: String::new(),
            pair_dialog: false,
            relay_url,
            relay_token: String::new(),
            relay_note: None,
            note: None,
            markdown: Default::default(),
        }
    }

    fn background(&self, ctx: &egui::Context, f: impl FnOnce(Service) -> Update + Send + 'static) {
        let outbox = self.outbox.clone();
        let ctx = ctx.clone();
        let service = self.service.clone();
        std::thread::spawn(move || {
            let _ = outbox.send(f(service));
            ctx.request_repaint();
        });
    }

    fn refresh_inventory(&mut self, ctx: &egui::Context, force: bool) {
        for provider in Provider::ALL {
            if self.inventory_pending.insert(provider) {
                self.background(ctx, move |service| {
                    Update::Inventory(provider, service.inventory(provider, force))
                });
            }
        }
    }

    fn load_models(&mut self, ctx: &egui::Context, provider: Provider, force: bool) {
        if (force || !self.models.contains_key(&provider)) && self.model_pending.insert(provider) {
            self.background(ctx, move |service| {
                Update::Models(provider, service.models(provider, force))
            });
        }
    }

    fn new_view(&mut self, ctx: &egui::Context) {
        self.epoch += 1;
        self.view = View::new(self.view.provider);
        self.note = None;
        self.load_models(ctx, self.view.provider, false);
    }

    fn open(&mut self, ctx: &egui::Context, session: Session) {
        self.epoch += 1;
        self.note = None;
        self.view = View::new(session.provider);
        self.view.id = Some(session.id.clone());
        self.view.title = Some(session.title.clone());
        self.view.run = self
            .running
            .iter()
            .find(|turn| {
                turn.provider == session.provider && turn.session_id.as_deref() == Some(&session.id)
            })
            .map(|turn| turn.id.clone());
        self.view.session = Some(session);
        self.view.transcript.warning = Some("正在读取原会话…".into());
        self.load_models(ctx, self.view.provider, false);
        self.read_history(ctx, false);
    }

    fn open_run(&mut self, ctx: &egui::Context, run: RunSummary) {
        self.epoch += 1;
        self.note = None;
        self.view = View::new(run.provider);
        self.view.run = Some(run.id);
        self.view.title = Some(run.title);
        self.view.id = run.session_id;
        self.load_models(ctx, self.view.provider, false);
        self.read_history(ctx, false);
    }

    fn read_history(&self, ctx: &egui::Context, refresh: bool) {
        if let Some(id) = &self.view.id {
            let provider = self.view.provider;
            let id = id.clone();
            let epoch = self.epoch;
            self.background(ctx, move |service| {
                let result = service.session(provider, &id).and_then(|session| {
                    let history = service.transcript(&session, refresh)?;
                    Ok((session, history))
                });
                Update::Transcript(epoch, result, refresh)
            });
        }
    }

    fn add_images(&mut self, paths: Vec<PathBuf>) {
        if self.view.busy() {
            return;
        }
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
    fn submit(&mut self, ctx: &egui::Context) {
        if self.view.busy()
            || (self.view.draft.trim().is_empty() && self.view.attachments.is_empty())
        {
            return;
        }
        let request = TurnRequest {
            provider: self.view.provider,
            session_id: self.view.id.clone(),
            model: self.view.model.clone(),
            cwd: self
                .view
                .session
                .as_ref()
                .map(|session| session.cwd.clone())
                .unwrap_or_else(|| self.folder.clone()),
            prompt: self.view.draft.clone(),
            images: Vec::new(),
        };
        let images = self.view.attachments.clone();
        self.view.pending = true;
        self.note = None;
        let epoch = self.epoch;
        self.background(ctx, move |service| {
            Update::Started(epoch, service.start_with_images(request, images))
        });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.inbox.try_recv() {
            match event {
                Update::Inventory(provider, result) => {
                    self.inventory_pending.remove(&provider);
                    self.inventories.insert(provider, result);
                }
                Update::Models(provider, result) => {
                    self.model_pending.remove(&provider);
                    self.models.insert(provider, result);
                }
                Update::Transcript(epoch, result, refresh) if epoch == self.epoch => match result {
                    Ok((session, history)) => {
                        self.view.session = Some(session);
                        let replace = !refresh
                            || self.view.snapshot.as_ref().is_none_or(|snapshot| {
                                history.messages.len() > self.view.transcript.messages.len()
                                    && history.messages.iter().any(|message| {
                                        message.role == Role::User
                                            && message.text.trim() == snapshot.prompt.trim()
                                    })
                            });
                        if replace {
                            self.view.transcript = history;
                            if refresh {
                                self.view.snapshot = None;
                            }
                        } else {
                            self.view.transcript.warning = history.warning;
                        }
                    }
                    Err(problem) => self.view.transcript.warning = Some(problem),
                },
                Update::Started(epoch, result) if epoch == self.epoch => {
                    self.view.pending = false;
                    match result {
                        Ok(run) => {
                            self.view.title = Some(run.title);
                            self.view.run = Some(run.id);
                            self.view.snapshot = None;
                            self.view.draft.clear();
                            self.view.attachments.clear();
                        }
                        Err(problem) => self.note = Some(problem),
                    }
                }
                _ => {}
            }
        }
        if self.last_runs.elapsed() >= Duration::from_millis(400) {
            let activity = self.service.activity();
            self.running = activity.running;
            self.last_runs = Instant::now();
            let latest = activity.finished.first().map(|run| run.id.clone());
            if latest.is_some() && self.recent_run != latest {
                self.recent_run = latest;
                self.refresh_inventory(ctx, false);
            }
            if !self.view.busy()
                && let Some(id) = &self.view.id
            {
                let matches = |run: &&RunSummary| {
                    run.provider == self.view.provider && run.session_id.as_ref() == Some(id)
                };
                if let Some(run) = self.running.iter().find(matches) {
                    self.view.run = Some(run.id.clone());
                } else if let Some(run) = activity.finished.iter().find(matches)
                    && self.view.last_finished.as_ref() != Some(&run.id)
                {
                    self.view.last_finished = Some(run.id.clone());
                    self.read_history(ctx, true);
                }
            }
        }
        if let Some(id) = &self.view.run {
            match self.service.run(id) {
                Ok(run) => {
                    let snapshot = run.snapshot();
                    if self.view.id.is_none() {
                        self.view.id.clone_from(&snapshot.run.session_id);
                    }
                    let finished = snapshot.status != Status::Running;
                    self.view.snapshot = Some(snapshot);
                    if finished {
                        self.view.last_finished = Some(id.clone());
                        self.view.run = None;
                        self.running = self.service.running();
                        self.read_history(ctx, true);
                        self.refresh_inventory(ctx, false);
                    }
                }
                Err(problem) => {
                    self.view.run = None;
                    self.note = Some(problem);
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
}

impl eframe::App for Desktop {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        self.paint(ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string("relay_url", self.relay_url.clone());
        if let Some(path) = self.folder.to_str() {
            storage.set_string("folder", path.to_owned());
        }
    }
}
