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
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use omadroid_core::catalog::{self, Release};
use omadroid_core::desktop::{self, Rect};
use omadroid_core::{
    bootstrap, engine, install, paths, theme, versions, Progress, Rgb, Store, Theme as OmaTheme,
};
use slint::{ModelRc, Timer, TimerMode, VecModel};

slint::include_modules!();

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// `waydroid --version` costs a Python start, and the answer cannot change
/// while the app runs: asked once, remembered.
static WAYDROID_VERSION: OnceLock<Option<String>> = OnceLock::new();

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
    let mut parts = vec![match WAYDROID_VERSION.get_or_init(engine::version) {
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

/// Within a few pixels, which is what asking Android for a new display size
/// leaves behind.
fn close(a: Rect, b: Rect) -> bool {
    (a.x - b.x).abs() <= 3
        && (a.y - b.y).abs() <= 3
        && (a.w - b.w).abs() <= 3
        && (a.h - b.h).abs() <= 3
}

/// Keeps the Waydroid window on the phone panel for as long as Android runs.
///
/// Three things it is responsible for, all of them the compositor's business in
/// the end: putting the phone on the panel, taking it away when another window
/// has the keyboard, and telling Android to render at the panel's size. It only
/// talks to Hyprland when one of those disagrees with what is on screen.
fn follow(phone: Arc<Mutex<Phone>>, live: Arc<AtomicBool>, fullscreen: Arc<AtomicBool>) {
    if !desktop::available() {
        eprintln!("omadroid: hyprctl is not there, so android keeps its own window");
        return;
    }
    let mut shaped = false;
    let mut warned = false;
    let mut full = false;
    let mut restore: Option<Rect> = None;
    let mut parked = false;

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

        // Fullscreen: omadroid's window takes the monitor and goes back where it
        // was afterwards, because a tiling layout cannot be re-derived from
        // scratch.
        let want_full = fullscreen.load(Ordering::Relaxed);
        if want_full != full {
            let result = if want_full {
                restore = Some(window.rect);
                match desktop::screen() {
                    Some(screen) => desktop::frame(&window, screen),
                    None => Err("there is no monitor to grow into".to_string()),
                }
            } else {
                match restore.take() {
                    Some(rect) => desktop::frame(&window, rect),
                    None => Ok(()),
                }
            };
            match result {
                Ok(()) => {
                    full = want_full;
                    // The toggle keeps the keyboard, so `esc` still reaches
                    // omadroid; a click on the phone is how Android gets it.
                    let _ = desktop::focus(&window);
                }
                Err(error) => {
                    eprintln!("omadroid: cannot make room for fullscreen: {error}");
                    restore = None;
                    fullscreen.store(false, Ordering::Relaxed);
                }
            }
        }

        let Some(device) = desktop::device().filter(|device| device.address != window.address)
        else {
            parked = false;
            continue;
        };
        if !live.load(Ordering::Relaxed) || window.hidden {
            continue;
        }

        // The phone is pinned above the tiling layout, so it would float over
        // whatever the user switched to. Turning to another app parks it.
        let focused = desktop::active();
        let ours = focused
            .as_ref()
            .map(|active| active.address == window.address || active.address == device.address)
            .unwrap_or(true);
        if !ours {
            if !parked {
                parked = true;
                if let Err(error) = desktop::park(&device) {
                    eprintln!("omadroid: cannot hide the phone window: {error}");
                }
            }
            continue;
        }
        if parked {
            parked = false;
            // Back where the user is, not where omadroid was: they may have
            // moved on while the phone was away.
            let workspace = desktop::current_workspace().unwrap_or(window.workspace);
            if let Err(error) = desktop::unpark(&device, workspace) {
                eprintln!("omadroid: cannot bring the phone window back: {error}");
            }
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

        // Android is *not* told to resize its display, and this is why: Waydroid
        // renders its own framebuffer and the compositor scales it, so the panel
        // shows a 1920x1080 desktop squeezed into a phone. `waydroid shell wm
        // size` looks like the fix, and Android does adopt the new size — but
        // Waydroid's window then paints black, at every size tried (357x765,
        // 238x510, 720x1536) and only recovers when the override is reset. A
        // phone that is the right shape and black is worse than one that is the
        // wrong shape and visible, so the display is left alone until Waydroid
        // can be restarted at the new size.

        if !close(device.rect, target) {
            if let Err(error) = desktop::place(&device, target) {
                if !warned {
                    eprintln!("omadroid: cannot place the phone window: {error}");
                    warned = true;
                }
            }
            // Resizing a window can hand it the keyboard. The keyboard belongs
            // to whoever had it, and to the phone only when it was clicked.
            let was_ours = focused
                .as_ref()
                .map(|active| active.address == window.address)
                .unwrap_or(false);
            let kept = desktop::active()
                .map(|active| active.address == window.address)
                .unwrap_or(false);
            if was_ours && !kept {
                let _ = desktop::focus(&window);
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
    let fullscreen = Arc::new(AtomicBool::new(false));
    {
        let phone = Arc::clone(&phone);
        let live = Arc::clone(&live);
        let fullscreen = Arc::clone(&fullscreen);
        std::thread::Builder::new()
            .name("phone".into())
            .spawn(move || follow(phone, live, fullscreen))
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
    let fullscreen_flag = Arc::clone(&fullscreen);
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
        // measured itself would loop. Fullscreen gives the phone the window and
        // leaves the bar, which is the way back out; otherwise a phone is 47%
        // as wide as it is tall, and stops at 780px like a real one on a desk.
        let full = ui.get_wide();
        fullscreen_flag.store(full, Ordering::Relaxed);
        let scale = ui.window().scale_factor().max(0.5);
        let logical = ui.window().size().height as f32 / scale;
        let wide = ui.window().size().width as f32 / scale;
        if full {
            ui.set_panel_h((logical - 74.0).max(240.0));
            ui.set_panel_w((wide - 40.0).max(240.0));
        } else {
            let height = (logical - 166.0).clamp(300.0, 780.0);
            ui.set_panel_h(height);
            ui.set_panel_w((height * 0.47).max(180.0));
        }

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

    // Android can be started from outside omadroid, and a switch can be slow to
    // answer: the state is re-read every few seconds, not only on a click.
    {
        let tx = cmd_tx.clone();
        let state = Timer::default();
        state.start(TimerMode::Repeated, Duration::from_secs(5), move || {
            let _ = tx.send(Cmd::Refresh);
        });
        // The timer has to outlive `run`: it is started before the event loop.
        std::mem::forget(state);
    }

    let weak = ui.as_weak();
    let colours = Timer::default();
    colours.start(TimerMode::Repeated, Duration::from_secs(2), move || {
        if let Some(ui) = weak.upgrade() {
            paint(&ui, false);
        }
    });

    ui.run()
}
