use super::{Desktop, Language, View, style::Palette};
use crate::update::{self, Check, Installation, Release, Restart, Staged};
use anyhow::Result;
use eframe::egui::{self, Context, Ui};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    time::{Duration, Instant},
};

const CHECK_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60);

pub(super) struct DesktopUpdates {
    installation: Option<Installation>,
    next_check: Instant,
    state: State,
    notice: Option<&'static str>,
}

enum State {
    Idle,
    Checking {
        result: Receiver<Result<Check>>,
        manual: bool,
    },
    Current,
    Unpublished,
    Available(Release),
    Downloading {
        release: Release,
        received: Arc<AtomicU64>,
        result: Receiver<Result<Staged>>,
    },
    Ready(Staged),
    Restarting,
}

impl DesktopUpdates {
    pub(super) fn new(failed: bool) -> Self {
        Self {
            installation: Installation::discover(),
            next_check: Instant::now() + Duration::from_secs(3),
            state: State::Idle,
            notice: failed.then_some("更新安装失败，已重新打开 Naro。"),
        }
    }

    pub(super) fn has_update(&self) -> bool {
        matches!(
            self.state,
            State::Available(_) | State::Downloading { .. } | State::Ready(_)
        )
    }

    pub(super) fn poll(&mut self, ctx: &Context) {
        match &self.state {
            State::Checking { result, manual } => {
                if let Some(result) = receive(result) {
                    self.state = match result {
                        Ok(Check::Current) => State::Current,
                        Ok(Check::Unpublished) => State::Unpublished,
                        Ok(Check::Available(release)) => State::Available(release),
                        Err(problem) => {
                            eprintln!("Naro: update check failed: {problem:#}");
                            if *manual {
                                self.notice = Some("无法检查更新，请检查网络后重试。");
                            }
                            State::Idle
                        }
                    };
                }
            }
            State::Downloading {
                release, result, ..
            } => {
                if let Some(result) = receive(result) {
                    self.state = match result {
                        Ok(staged) => State::Ready(staged),
                        Err(problem) => {
                            eprintln!("Naro: update download failed: {problem:#}");
                            self.notice = Some("无法准备更新。请重试或从发布页面下载安装包。");
                            State::Available(release.clone())
                        }
                    };
                }
            }
            _ => {}
        }
        if matches!(self.state, State::Downloading { .. }) {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        if self.can_check() {
            let remaining = self.next_check.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.check(ctx, false);
            } else {
                ctx.request_repaint_after(remaining);
            }
        }
    }

    fn can_check(&self) -> bool {
        self.installation.is_some()
            && matches!(
                self.state,
                State::Idle | State::Current | State::Unpublished | State::Available(_)
            )
    }

    fn check(&mut self, ctx: &Context, manual: bool) {
        self.next_check = Instant::now() + CHECK_INTERVAL;
        if manual {
            self.notice = None;
        }
        self.state = State::Checking {
            result: background(ctx, update::check),
            manual,
        };
    }

    fn download(&mut self, ctx: &Context) {
        let (Some(installation), State::Available(release)) = (&self.installation, &self.state)
        else {
            return;
        };
        let installation = installation.clone();
        let release = release.clone();
        let received = Arc::new(AtomicU64::new(0));
        let progress = received.clone();
        let package = release.clone();
        let result = background(ctx, move || {
            installation.stage(&package, |bytes| progress.store(bytes, Ordering::Relaxed))
        });
        self.notice = None;
        self.state = State::Downloading {
            release,
            received,
            result,
        };
    }

    fn restart(&mut self) -> Result<Restart> {
        let State::Ready(staged) = std::mem::replace(&mut self.state, State::Restarting) else {
            unreachable!("Restart is available only for a staged update");
        };
        self.notice = None;
        staged.start().inspect_err(|_| {
            self.state = State::Idle;
            self.notice = Some("无法启动更新程序，请重试。");
        })
    }

    pub(super) fn show(&mut self, ui: &mut Ui, language: Language) -> bool {
        let palette = Palette::of(ui);
        ui.label(language.format(
            "当前版本 {version}",
            &[("version", update::CURRENT_VERSION)],
        ));
        let status = match &self.state {
            State::Idle if self.installation.is_none() => Some("请使用 Naro 安装包进行在线更新。"),
            State::Checking { .. } => Some("正在检查更新…"),
            State::Current => Some("已是最新版本。"),
            State::Unpublished => Some("暂无适用于此设备的在线更新。"),
            State::Restarting => Some("正在重启更新…"),
            _ => None,
        };
        if let Some(status) = status {
            ui.colored_label(palette.muted, language.text(status));
        }
        let mut download = false;
        let mut restart = false;
        match &self.state {
            State::Available(release) => {
                let version = release.version.to_string();
                ui.label(language.format("发现新版本 {version}", &[("version", &version)]));
                download = ui.button(language.text("下载更新")).clicked();
            }
            State::Downloading {
                release, received, ..
            } => {
                let received = received.load(Ordering::Relaxed);
                let progress = received as f32 / release.size as f32;
                let label = if received == release.size {
                    "正在验证更新…"
                } else {
                    "正在下载更新…"
                };
                ui.label(language.text(label));
                ui.add(egui::ProgressBar::new(progress).show_percentage());
            }
            State::Ready(staged) => {
                let version = staged.version.to_string();
                ui.label(language.format("已下载版本 {version}", &[("version", &version)]));
                restart = ui.button(language.text("重启并更新")).clicked();
            }
            _ => {}
        }
        if let Some(notice) = self.notice {
            ui.colored_label(palette.muted, language.text(notice));
        }
        ui.horizontal_wrapped(|ui| {
            if self.can_check() && ui.button(language.text("检查更新")).clicked() {
                self.check(ui.ctx(), true);
            }
            ui.hyperlink_to(language.text("查看发布页面"), update::RELEASES_URL);
        });
        if download {
            self.download(ui.ctx());
        }
        restart
    }
}

impl Desktop {
    pub(super) fn restart_for_update(&mut self, ctx: &Context) {
        let has_drafts = |view: &View| view.has_draft() || !view.other_drafts.is_empty();
        if has_drafts(&self.view)
            || self
                .local_view
                .as_ref()
                .is_some_and(|saved| has_drafts(&saved.view))
            || self
                .recent_device
                .as_ref()
                .is_some_and(|(_, saved)| has_drafts(&saved.view))
        {
            self.updates.notice = Some("请先处理未发送的草稿，再重启更新。");
            return;
        }
        if !self.can_switch_workspace()
            || self.git.action_pending
            || self.git.generating_message
            || !self.service.reserve_update_restart()
        {
            self.updates.notice = Some("有任务正在运行，请等任务结束后再重启更新。");
            return;
        }
        match self.updates.restart() {
            Ok(restart) => {
                self.restart.set(Some(restart));
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Err(problem) => {
                self.service.release_update_restart();
                eprintln!("Naro: could not start the update: {problem:#}");
            }
        }
    }
}

fn background<T: Send + 'static>(
    ctx: &Context,
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Receiver<Result<T>> {
    let (send, receive) = mpsc::channel();
    let ctx = ctx.clone();
    let _ = std::thread::Builder::new()
        .name("naro-update".into())
        .spawn(move || {
            let _ = send.send(work());
            ctx.request_repaint();
        });
    receive
}

fn receive<T>(result: &Receiver<Result<T>>) -> Option<Result<T>> {
    match result.try_recv() {
        Ok(result) => Some(result),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => Some(Err(anyhow::anyhow!("Update worker stopped"))),
    }
}
