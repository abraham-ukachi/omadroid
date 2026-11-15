//! # omadroid
//!
//! The app: the Slint UI (`ui/app.slint`) wired to `omadroid-core`.
//! Everything that touches Waydroid or the network runs on a worker thread, so
//! the window never waits on a four-second container start or a 1.5 GB image.
//!
//! A second, much smaller thread keeps Android on the phone panel: it asks
//! Hyprland where omadroid's window is, where the Waydroid window is, and moves
//! the latter onto the rectangle the panel occupies. That is the closest a
//! Wayland app can get to hosting another app's window (see `desktop`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use omadroid_core::catalog::{self, Release};
use omadroid_core::desktop::{self, Rect};
use omadroid_core::{
    bootstrap, engine, install, paths, theme, versions, Progress, Rgb, Store, Theme as OmaTheme,
};
use slint::{ModelRc, Timer, TimerMode, VecModel};

slint::include_modules!();

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// How often the phone window is nudged back onto the panel.
const RING: Duration = Duration::from_millis(400);

/// What the window asks the worker for.
enum Cmd {
    Refresh,
    Start(String),
    Stop,
    Install(String),
    Remove(String),
    Show,
    Setup,
}

/// What the worker tells the window.
enum Ev {
    Snapshot(Snapshot),
    Busy(bool),
    Progress(f32),
    Notice(String),
    Status(String),
}

/// The list of versions, plus what Waydroid is doing right now.
struct Snapshot {
    rows: Vec<VersionRow>,
    /// `Android 13 · running`, worn under the app's name.
    caption: String,
    status: String,
    running: bool,
    /// `waydroid 1.6.3 · android 13 · MAINLINE`, for the about tab.
    engine: String,
    needs_setup: bool,
    setup_detail: String,
}

/// Where the phone panel is, in window coordinates.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Phone {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

/// `Android 13 · running`, `running`, `stopped`.
fn status_line(installed: &[omadroid_core::Version], running: bool) -> String {
    match versions::active(installed) {
        Some(version) if running => format!("{} · running", version.title()),
        _ if running => "running".to_string(),
        _ => "stopped".to_string(),
    }
}

/// `waydroid 1.6.3 · android 13 · MAINLINE`, or what is missing.
fn engine_line(deps: &bootstrap::Deps, status: &engine::Status) -> String {
    if !deps.waydroid {
        return "waydroid is not installed".to_string();
    }
    let mut parts = vec![match engine::version() {
        Some(version) => format!("waydroid {version}"),
        None => "waydroid".to_string(),
    }];

    if status.running() {
        if let Some(android) = engine::android_release() {
            parts.push(format!("android {android}"));
        }
    } else {
        parts.push("android stopped".to_string());
    }
    if !status.vendor.is_empty() {
        parts.push(status.vendor.to_lowercase());
    }
    parts.join(" · ")
}

/// Builds the rows the window shows: installed versions first, then the ones
/// that can be downloaded, newest first.
fn snapshot(store: &Store, releases: &[Release]) -> Snapshot {
    let status = engine::status();
    let installed = versions::installed(store);
    let active = versions::active(&installed).map(|version| version.id.clone());

    let mut rows: Vec<(u8, u32, VersionRow)> = Vec::new();

    for version in &installed {
        let live = active.as_deref() == Some(version.id.as_str()) && status.running();
        rows.push((
            0,
            version.android,
            VersionRow {
                id: version.id.clone().into(),
                title: version.title().into(),
                subtitle: version.subtitle().into(),
                state: if live { "live" } else { "installed" }.into(),
                live,
                download: false,
            },
        ));
    }

    for release in releases {
        if installed.iter().any(|version| version.android == release.android) {
            continue;
        }
        rows.push((
            1,
            release.android,
            VersionRow {
                id: release.id().into(),
                title: format!("Android {}", release.android).into(),
                subtitle: release.subtitle().into(),
                state: "download".into(),
                live: false,
                download: true,
            },
        ));
    }

    rows.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    let rows = rows.into_iter().map(|(_, _, row)| row).collect::<Vec<_>>();

    let running = status.running();
    let short = status_line(&installed, running);
    let deps = bootstrap::detect();

    Snapshot {
        caption: format!("{VERSION} · {short}"),
        status: short,
        rows,
        running,
        engine: engine_line(&deps, &status),
        needs_setup: !deps.ready(),
        setup_detail: deps.describe(),
    }
}

/// Refreshes the catalog, at most every ten minutes, and remembers which
/// Android the machine's own images are.
fn refresh(
    store: &mut Store,
    releases: &mut Vec<Release>,
    fetched: &mut Option<Instant>,
) -> Option<String> {
    if store.system_android == 0 {
        if let Some(android) = engine::android_release() {
            store.system_android = android;
            let _ = store.save(&paths::store_file());
        }
    }

    let stale = fetched
        .map(|at| at.elapsed() > Duration::from_secs(600))
        .unwrap_or(true);
    if !stale {
        return None;
    }

    match catalog::fetch() {
        Ok(list) => {
            *releases = list;
            *fetched = Some(Instant::now());
            None
        }
        // Keep the list we already have; only a first failure is worth saying.
        Err(error) if releases.is_empty() => {
            *releases = catalog::fallback();
            *fetched = Some(Instant::now());
            Some(format!("GitHub unreachable ({error}); showing the newest build only"))
        }
        Err(_) => {
            *fetched = Some(Instant::now());
            None
        }
    }
}

fn send_snapshot(events: &Sender<Ev>, store: &Store, releases: &[Release]) {
    let _ = events.send(Ev::Snapshot(snapshot(store, releases)));
}

/// The worker: owns the store and the catalog, does one command at a time.
fn worker(cmds: Receiver<Cmd>, events: Sender<Ev>) {
    let mut store = Store::load(&paths::store_file());
    let mut releases: Vec<Release> = Vec::new();
    let mut fetched: Option<Instant> = None;

    while let Ok(cmd) = cmds.recv() {
        match cmd {
            Cmd::Refresh => {
                if let Some(notice) = refresh(&mut store, &mut releases, &mut fetched) {
                    let _ = events.send(Ev::Notice(notice));
                }
                send_snapshot(&events, &store, &releases);
            }

            // The one button on a machine that has never run Waydroid.
            Cmd::Setup => {
                let _ = events.send(Ev::Busy(true));
                let progress_events = events.clone();
                let mut progress = |line: &str| {
                    let _ = progress_events.send(Ev::Status(line.to_string()));
                };
                match bootstrap::install(&mut progress) {
                    Ok(()) => {
                        let _ = events.send(Ev::Status("waydroid is installed".into()));
                    }
                    Err(error) => {
                        let _ = events.send(Ev::Notice(error));
                    }
                }
                let _ = events.send(Ev::Busy(false));
                let _ = refresh(&mut store, &mut releases, &mut fetched);
                send_snapshot(&events, &store, &releases);
            }

            Cmd::Start(id) => {
                let Some(version) = versions::installed(&store)
                    .into_iter()
                    .find(|version| version.id == id)
                else {
                    let _ = events.send(Ev::Notice(format!("{id} is not installed yet")));
                    continue;
                };

                let _ = events.send(Ev::Busy(true));
                let _ = events.send(Ev::Status(format!("switching to {}…", version.title())));

                let is_live = versions::active(std::slice::from_ref(&version)).is_some()
                    && engine::status().running();
                let result = if is_live {
                    engine::show()
                } else {
                    engine::activate(&version).and_then(|()| engine::show())
                };

                match result {
                    Ok(()) => {
                        let _ = events.send(Ev::Status(format!("{} · running", version.title())));
                    }
                    Err(error) => {
                        let _ = events.send(Ev::Notice(error));
                    }
                }
                let _ = events.send(Ev::Busy(false));
                send_snapshot(&events, &store, &releases);
            }

            Cmd::Stop => {
                let _ = events.send(Ev::Busy(true));
                match engine::stop() {
                    Ok(()) => {
                        let _ = desktop::release();
                        let _ = events.send(Ev::Status("stopped".into()));
                    }
                    Err(error) => {
                        let _ = events.send(Ev::Notice(error));
                    }
                }
                let _ = events.send(Ev::Busy(false));
                send_snapshot(&events, &store, &releases);
            }

            Cmd::Install(id) => {
                let Some(release) = releases.iter().find(|release| release.id() == id).cloned()
                else {
                    let _ = events.send(Ev::Notice(format!("{id} is not in the catalog")));
                    continue;
                };

                let _ = events.send(Ev::Busy(true));
                let progress_events = events.clone();
                let mut progress = |step: Progress| {
                    let _ = progress_events.send(Ev::Progress(step.fraction()));
                    let _ = progress_events.send(Ev::Status(step.describe()));
                };

                match install::install(&release, &mut progress) {
                    Ok(version) => {
                        let _ = events.send(Ev::Progress(1.0));
                        let _ = events.send(Ev::Status(format!("{} installed", version.title())));
                    }
                    Err(error) => {
                        let _ = events.send(Ev::Notice(error));
                    }
                }
                let _ = events.send(Ev::Busy(false));
                send_snapshot(&events, &store, &releases);
            }

            Cmd::Remove(id) => {
                let Some(version) = versions::installed(&store)
                    .into_iter()
                    .find(|version| version.id == id)
                else {
                    continue;
                };
                match install::remove(&version) {
                    Ok(()) => {
                        let _ = events.send(Ev::Status(format!("{} removed", version.title())));
                    }
                    Err(error) => {
                        let _ = events.send(Ev::Notice(error));
                    }
                }
                send_snapshot(&events, &store, &releases);
            }

            Cmd::Show => {
                if let Err(error) = engine::show() {
                    let _ = events.send(Ev::Notice(error));
                }
            }
        }
    }
}

/// Keeps the Waydroid window on the phone panel for as long as Android runs.
/// It only talks to Hyprland when the two rectangles disagree, so the usual
/// round is two `hyprctl clients -j` calls per tick.
fn follow(phone: Arc<Mutex<Phone>>, live: Arc<AtomicBool>) {
    if !desktop::available() {
        eprintln!("omadroid: hyprctl is not there, so android keeps its own window");
        return;
    }
    let mut shaped = false;
    let mut warned = false;
    loop {
        std::thread::sleep(RING);
        let Some(window) = desktop::host() else {
            continue;
        };
        // Once per run: the window is floated and made tall enough for a phone.
        if !shaped {
            shaped = true;
            if let Err(error) = desktop::shape_host(&window) {
                eprintln!("omadroid: cannot float the window: {error}");
            }
        }
        if !live.load(Ordering::Relaxed) || window.hidden {
            continue;
        }
        let panel = *phone.lock().unwrap_or_else(|error| error.into_inner());
        if panel.w < 40.0 || panel.h < 40.0 {
            continue;
        }
        // Two pixels in, so omadroid's accent border stays visible around it.
        let target = Rect {
            x: window.rect.x + panel.x.round() as i64 + 2,
            y: window.rect.y + panel.y.round() as i64 + 2,
            w: (panel.w.round() as i64 - 4).max(1),
            h: (panel.h.round() as i64 - 4).max(1),
        };
        let Some(device) = desktop::device() else {
            continue;
        };
        // Never move omadroid's own window, whatever the compositor answers.
        if device.address == window.address {
            continue;
        }
        if device.rect != target {
            if let Err(error) = desktop::place(&device, target) {
                if !warned {
                    eprintln!("omadroid: cannot place the phone window: {error}");
                    warned = true;
                }
            }
        }
        // Above the tiling layout, so clicking the list does not bury the
        // phone. Retried because a pin can be refused while it is still mapping.
        if !device.pinned {
            let _ = desktop::pin(&device);
        }
    }
}

fn color(rgb: Rgb) -> slint::Color {
    slint::Color::from_argb_encoded(rgb.argb())
}

/// Paints the window with the live Omarchy theme.
fn paint(ui: &AppWindow, radius: bool) {
    let theme = OmaTheme::load(&paths::omarchy_colors());
    let global = ui.global::<Theme>();
    global.set_bg(color(theme.background));
    global.set_fg(color(theme.foreground));
    global.set_accent(color(theme.accent));
    global.set_key(color(theme.key()));
    global.set_border(color(theme.border()));
    global.set_panel(color(theme.panel()));
    global.set_muted(color(theme.muted()));
    if radius {
        global.set_radius(theme::hypr_rounding());
    }
}

/// Runs the app until the window closes.
pub fn run() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    paint(&ui, true);
    ui.set_app_version(VERSION.into());

    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (ev_tx, ev_rx) = mpsc::channel();

    std::thread::Builder::new()
        .name("omadroid".into())
        .spawn(move || worker(cmd_rx, ev_tx))
        .expect("the worker starts");

    let phone = Arc::new(Mutex::new(Phone::default()));
    let live = Arc::new(AtomicBool::new(false));
    {
        let phone = Arc::clone(&phone);
        let live = Arc::clone(&live);
        std::thread::Builder::new()
            .name("phone".into())
            .spawn(move || follow(phone, live))
            .expect("the phone thread starts");
    }

    let _ = cmd_tx.send(Cmd::Refresh);

    {
        let tx = cmd_tx.clone();
        ui.on_refresh(move || {
            let _ = tx.send(Cmd::Refresh);
        });
    }
    {
        let tx = cmd_tx.clone();
        ui.on_launch(move |id| {
            let _ = tx.send(Cmd::Start(id.to_string()));
        });
    }
    {
        let tx = cmd_tx.clone();
        ui.on_stop(move || {
            let _ = tx.send(Cmd::Stop);
        });
    }
    {
        let tx = cmd_tx.clone();
        ui.on_install(move |id| {
            let _ = tx.send(Cmd::Install(id.to_string()));
        });
    }
    {
        let tx = cmd_tx.clone();
        ui.on_remove(move |id| {
            let _ = tx.send(Cmd::Remove(id.to_string()));
        });
    }
    {
        let tx = cmd_tx.clone();
        ui.on_show_ui(move || {
            let _ = tx.send(Cmd::Show);
        });
    }
    {
        let tx = cmd_tx.clone();
        ui.on_setup(move || {
            let _ = tx.send(Cmd::Setup);
        });
    }
    {
        let weak = ui.as_weak();
        let live = Arc::clone(&live);
        ui.on_quit(move || {
            let _ = desktop::release();
            live.store(false, Ordering::Relaxed);
            if let Some(ui) = weak.upgrade() {
                let _ = ui.hide();
            }
        });
    }

    // One timer drains the worker's answers and reads the phone's rectangle;
    // a second follows the theme.
    let weak = ui.as_weak();
    let events = Timer::default();
    events.start(TimerMode::Repeated, Duration::from_millis(120), move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        while let Ok(event) = ev_rx.try_recv() {
            match event {
                Ev::Snapshot(snapshot) => {
                    let count = snapshot.rows.len();
                    live.store(snapshot.running, Ordering::Relaxed);
                    ui.set_rows(ModelRc::new(VecModel::from(snapshot.rows)));
                    ui.set_caption(snapshot.caption.into());
                    ui.set_status_text(snapshot.status.into());
                    ui.set_engine_text(snapshot.engine.into());
                    ui.set_needs_setup(snapshot.needs_setup);
                    ui.set_setup_detail(snapshot.setup_detail.into());
                    ui.set_running(snapshot.running);
                    if count == 0 || ui.get_selected() as usize >= count {
                        ui.set_selected(0);
                    }
                }
                Ev::Busy(busy) => {
                    ui.set_busy(busy);
                    if !busy {
                        ui.set_progress(0.0);
                    }
                }
                Ev::Progress(value) => ui.set_progress(value),
                Ev::Notice(text) => ui.set_notice(text.into()),
                Ev::Status(text) => ui.set_status_text(text.into()),
            }
        }

        // The phone's size comes from here, not from a binding: a layout that
        // measured itself would loop. A phone is 47% as wide as it is tall,
        // fills the window, and stops at 780px like a real one on a desk.
        let scale = ui.window().scale_factor().max(0.5);
        let logical = ui.window().size().height as f32 / scale;
        let height = (logical - 166.0).clamp(300.0, 780.0);
        ui.set_panel_h(height);
        ui.set_panel_w((height * 0.47).max(180.0));

        let panel = Phone {
            x: ui.get_phone_x(),
            y: ui.get_phone_y(),
            w: ui.get_phone_w(),
            h: ui.get_phone_h(),
        };
        if let Ok(mut current) = phone.lock() {
            *current = panel;
        }
    });

    let weak = ui.as_weak();
    let colours = Timer::default();
    colours.start(TimerMode::Repeated, Duration::from_secs(2), move || {
        if let Some(ui) = weak.upgrade() {
            paint(&ui, false);
        }
    });

    ui.run()
}
