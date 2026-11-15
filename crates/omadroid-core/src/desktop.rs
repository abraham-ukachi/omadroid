//! Putting Android inside the phone panel.
//!
//! Honest limitation, stated here and in the README: Wayland has no way for one
//! application to adopt another's surface. A Slint window cannot host the
//! Waydroid window as a widget.
//!
//! So omadroid does the next best thing, and on a tiling compositor it looks
//! the same: the Waydroid window is made floating and pinned, moved and resized
//! onto the exact rectangle the phone panel occupies. When omadroid moves or the
//! panel changes size, the window is moved with it. The phone can be typed into
//! directly, because it is Android's own window.
//!
//! Since Hyprland 0.55 the compositor's control language is Lua: `hyprctl
//! keyword` is gone and `hyprctl dispatch` takes a dispatcher object, not a
//! string. Every call here is sent as Lua and, for older compositors, retried
//! as the hyprlang string the same dispatcher used to take.

use serde_json::Value;

use crate::engine;

/// The window class Waydroid gives its UI. Matched case-insensitively because
/// Qt sets it from the desktop file name, and that has moved around.
pub const CLASS: &str = "waydroid";

/// The window omadroid asks for: a phone is tall and narrow.
const HOST_W: i64 = 820;
const HOST_H: i64 = 900;

/// A client window, as `hyprctl clients -j` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Client {
    pub address: String,
    pub class: String,
    pub title: String,
    pub rect: Rect,
    /// The workspace the window sits on. The phone is parked on one of its own
    /// while another window has the keyboard, and comes back to this one.
    pub workspace: i64,
    pub floating: bool,
    pub hidden: bool,
    /// Pinned windows float above the tiling layout on every workspace, which
    /// is what keeps the phone visible while omadroid has the keyboard.
    pub pinned: bool,
}

/// A screen rectangle in layout pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
}

/// Is there a Hyprland to talk to? `version` answers without a socket, so this
/// asks a question the compositor itself has to reply to.
pub fn available() -> bool {
    engine::run("hyprctl", &["-j", "activeworkspace"]).is_ok()
}

/// Every mapped window.
pub fn clients() -> Vec<Client> {
    match engine::run("hyprctl", &["clients", "-j"]) {
        Ok(json) => parse(&json),
        Err(_) => Vec::new(),
    }
}

/// Reads `hyprctl clients -j`, keeping what omadroid needs.
pub fn parse(json: &str) -> Vec<Client> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    items.iter().filter_map(client).collect()
}

/// One window out of one entry of `hyprctl clients -j`. `activewindow -j` is the
/// same entry on its own, so both go through here.
fn client(item: &Value) -> Option<Client> {
    let address = item.get("address")?.as_str()?.to_string();
    let field = |key: &str| {
        item.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let number = |key: &str, index: usize| -> i64 {
        item.get(key)
            .and_then(Value::as_array)
            .and_then(|pair| pair.get(index))
            .and_then(Value::as_f64)
            .unwrap_or(0.0) as i64
    };
    Some(Client {
        address,
        class: field("class"),
        title: field("title"),
        rect: Rect {
            x: number("at", 0),
            y: number("at", 1),
            w: number("size", 0),
            h: number("size", 1),
        },
        workspace: item
            .get("workspace")
            .and_then(|workspace| workspace.get("id"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
        floating: item
            .get("floating")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        hidden: item.get("hidden").and_then(Value::as_bool).unwrap_or(false),
        pinned: item.get("pinned").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// The window with the keyboard. `hyprctl` answers `{}` when there is none, and
/// an empty answer is not a window.
pub fn active() -> Option<Client> {
    let json = engine::run("hyprctl", &["-j", "activewindow"]).ok()?;
    let value: Value = serde_json::from_str(&json).ok()?;
    client(&value)
}

/// omadroid's own window: the Slint window is undecorated and has no class, so
/// its title is what names it.
pub fn host() -> Option<Client> {
    clients()
        .into_iter()
        .find(|client| client.title.eq_ignore_ascii_case("omadroid"))
}

/// Waydroid's phone window. Its class is `Waydroid`; the title is not looked
/// at, because a browser tab about Waydroid has it in its title too.
pub fn device() -> Option<Client> {
    clients()
        .into_iter()
        .find(|client| client.class.eq_ignore_ascii_case(CLASS))
}

/// The monitor the phone should open on, in layout pixels.
///
/// `hyprctl monitors` reports `width`/`height` in physical pixels and the scale
/// separately: a 1920x1080 screen at 1.5 is 1280x720 of layout, which is what
/// window positions are counted in. Sizing the window from the physical numbers
/// hangs it off the bottom of the screen.
pub fn screen() -> Option<Rect> {
    let monitor = focused_monitor()?;
    let number = |key: &str| monitor.get(key).and_then(Value::as_f64).unwrap_or(0.0);
    let scale = number("scale").max(0.5);
    Some(Rect {
        x: number("x") as i64,
        y: number("y") as i64,
        w: (number("width") / scale) as i64,
        h: (number("height") / scale) as i64,
    })
}

/// The focused monitor's scale factor: 1.5 here, so a 1920x1080 screen has
/// 1280x720 of layout but 1920x1080 of pixels.
pub fn scale() -> f64 {
    focused_monitor()
        .and_then(|monitor| monitor.get("scale").and_then(Value::as_f64))
        .unwrap_or(1.0)
        .max(0.5)
}

/// The workspace the user is looking at, by id. The phone comes back to this
/// one, and omadroid's own window returns here if it was left parked.
pub fn current_workspace() -> Option<i64> {
    let json = engine::run("hyprctl", &["-j", "activeworkspace"]).ok()?;
    let value: Value = serde_json::from_str(&json).ok()?;
    value.get("id").and_then(Value::as_i64)
}

/// The Lua that gives a window the keyboard. `hl.dsp.focus` is the dispatcher
/// that takes a window; `hl.dsp.window.focus` does not exist on 0.56, and
/// calling it returns an error that looks like a success to anything reading
/// only the exit code.
pub fn focus_call(address: &str) -> String {
    format!("hl.dsp.focus({{ window = hl.get_window(\"address:{address}\") }})")
}

/// Gives a window the keyboard.
pub fn focus(client: &Client) -> Result<(), String> {
    let address = &client.address;
    let lua = focus_call(address);
    if let Ok(out) = engine::run("hyprctl", &["dispatch", &lua]) {
        if !out.trim_start().starts_with("error") {
            return Ok(());
        }
    }
    engine::run("hyprctl", &["dispatch", &format!("focuswindow address:{address}")])
        .map(|_| ())
}

fn focused_monitor() -> Option<Value> {
    let json = engine::run("hyprctl", &["-j", "monitors"]).ok()?;
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&json) else {
        return None;
    };
    items
        .into_iter()
        .find(|monitor| monitor.get("focused").and_then(Value::as_bool).unwrap_or(false))
}

/// `hl.dsp.window.move({ window = hl.get_window("address:0x…"), x = 10 })`.
///
/// The `address:` prefix is not decoration: a bare `0x…` does not resolve, and
/// the dispatcher then acts on the active window instead — which is how the
/// first version of this file kept moving omadroid's own window onto itself.
fn call(action: &str, address: &str, extra: &str) -> String {
    let window = format!("window = hl.get_window(\"address:{address}\")");
    let args = if extra.is_empty() {
        window
    } else {
        format!("{window}, {extra}")
    };
    format!("hl.dsp.window.{action}({{ {args} }})")
}

/// Runs one window dispatcher, as Lua and then as the old string form.
fn send(action: &str, address: &str, extra: &str, legacy: &str) -> Result<(), String> {
    let lua = call(action, address, extra);
    if let Ok(out) = engine::run("hyprctl", &["dispatch", &lua]) {
        // The Lua evaluator answers `ok`, and prints errors on stdout too.
        if !out.trim_start().starts_with("error") {
            return Ok(());
        }
    }
    engine::run("hyprctl", &["dispatch", legacy]).map(|_| ())
}

/// Gives omadroid's own window a phone-shaped, floating, centred place. On a
/// tiling desktop the window's preferred size is otherwise what the layout
/// says, and a phone panel needs a tall window — but only as tall as the screen
/// actually is.
pub fn shape_host(host: &Client) -> Result<(), String> {
    let address = &host.address;
    let screen = screen().unwrap_or(Rect {
        x: 0,
        y: 0,
        w: HOST_W,
        h: HOST_H,
    });
    let (w, h) = (
        HOST_W.min((screen.w - 40).max(520)),
        HOST_H.min((screen.h - 40).max(520)),
    );
    // A window left on the parked workspace by an earlier run is only visible
    // while that workspace is: put it back on the one the user is looking at.
    if host.workspace < 0 {
        if let Some(workspace) = current_workspace() {
            let _ = unpark(host, workspace);
        }
    }
    // omadroid's own window is not pinned: only the phone floats above the
    // tiling layout.
    let _ = send(
        "pin",
        address,
        "action = \"unset\"",
        &format!("unpin address:{address}"),
    );
    send(
        "float",
        address,
        "action = \"set\"",
        &format!("setfloating address:{address}"),
    )?;
    send(
        "resize",
        address,
        &format!("x = {w}, y = {h}"),
        &format!("resizewindowpixel exact {w} {h},address:{address}"),
    )?;

    let x = screen.x + (screen.w - w) / 2;
    let y = screen.y + (screen.h - h) / 2;
    send(
        "move",
        address,
        &format!("x = {x}, y = {y}"),
        &format!("movewindowpixel exact {x} {y},address:{address}"),
    )?;
    Ok(())
}

/// Moves and resizes the phone window onto `rect`, and keeps it above the
/// tiling layout so omadroid's own window can stay focused underneath.
/// Moves and resizes a window onto `rect`, and nothing else: this is what puts
/// omadroid's own window on the whole monitor in fullscreen, and back.
pub fn frame(client: &Client, rect: Rect) -> Result<(), String> {
    let address = &client.address;
    let (w, h) = (rect.w.max(1), rect.h.max(1));
    send(
        "resize",
        address,
        &format!("x = {w}, y = {h}"),
        &format!("resizewindowpixel exact {w} {h},address:{address}"),
    )?;
    send(
        "move",
        address,
        &format!("x = {}, y = {}", rect.x, rect.y),
        &format!("movewindowpixel exact {} {},address:{address}", rect.x, rect.y),
    )
}

/// Where the phone waits while another window has the keyboard. A tiling
/// compositor cannot hide a window, but a workspace nobody is looking at is the
/// same thing from the outside.
pub const PARKED: &str = "special:omadroid";

/// Parks the phone out of sight. Its rectangle is not kept on the way, so the
/// caller places it again when it comes back.
///
/// Unpinned on the way out: a pinned window is shown on every workspace, and a
/// special workspace is no exception, so staying pinned would keep the phone on
/// screen while it is meant to be away.
pub fn park(client: &Client) -> Result<(), String> {
    let address = &client.address;
    let _ = send(
        "pin",
        address,
        "action = \"unset\"",
        &format!("unpin address:{address}"),
    );
    send(
        "move",
        address,
        &format!("workspace = \"{PARKED}\""),
        &format!("movetoworkspacesilent {PARKED},address:{address}"),
    )
}

/// Brings the parked phone back to the workspace omadroid itself is on.
pub fn unpark(client: &Client, workspace: i64) -> Result<(), String> {
    let address = &client.address;
    send(
        "move",
        address,
        &format!("workspace = \"{workspace}\""),
        &format!("movetoworkspacesilent {workspace},address:{address}"),
    )
}

pub fn place(client: &Client, rect: Rect) -> Result<(), String> {
    let address = &client.address;
    let (w, h) = (rect.w.max(1), rect.h.max(1));

    if !client.floating {
        send(
            "float",
            address,
            "action = \"set\"",
            &format!("setfloating address:{address}"),
        )?;
    }
    send(
        "resize",
        address,
        &format!("x = {w}, y = {h}"),
        &format!("resizewindowpixel exact {w} {h},address:{address}"),
    )?;
    send(
        "move",
        address,
        &format!("x = {}, y = {}", rect.x, rect.y),
        &format!("movewindowpixel exact {} {},address:{address}", rect.x, rect.y),
    )?;
    // Pinned: above the tiling layout, still clickable, and omadroid keeps the
    // keyboard. Not worth failing the whole placement over.
    let _ = send(
        "pin",
        address,
        "action = \"set\"",
        &format!("pin address:{address}"),
    );
    Ok(())
}

/// Keeps the phone above the tiling layout. Hyprland can refuse a pin while a
/// window is still mapping, so the caller retries until `pinned` is true.
pub fn pin(client: &Client) -> Result<(), String> {
    let address = &client.address;
    send(
        "pin",
        address,
        "action = \"set\"",
        &format!("pin address:{address}"),
    )
}

/// Lets the phone window go back to the compositor when omadroid quits.
pub fn release() -> Result<(), String> {
    let Some(client) = device() else {
        return Ok(());
    };
    let address = &client.address;
    let _ = send(
        "pin",
        address,
        "action = \"unset\"",
        &format!("unpin address:{address}"),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENTS: &str = r#"[
      {"address":"0x1","class":"","title":"omadroid","at":[647,38],"size":[780,880],
       "floating":false,"hidden":false},
      {"address":"0x2","class":"waydroid","title":"Android 13 - 192.168.240.112",
       "at":[660,520],"size":[320,690],"floating":true,"hidden":false}
    ]"#;

    #[test]
    fn reads_windows_out_of_hyprctl() {
        let clients = parse(CLIENTS);
        assert_eq!(clients.len(), 2);
        assert_eq!(clients[0].rect, Rect { x: 647, y: 38, w: 780, h: 880 });
        assert!(clients[1].floating);
        assert_eq!(clients[1].class, "waydroid");
    }

    #[test]
    fn the_host_is_named_by_its_title() {
        assert_eq!(parse(CLIENTS)[0].title, "omadroid");
    }

    #[test]
    fn the_phone_is_found_by_its_class() {
        assert!(parse(CLIENTS)
            .iter()
            .any(|client| client.class.eq_ignore_ascii_case(CLASS)));
    }

    #[test]
    fn a_browser_tab_about_waydroid_is_not_the_phone() {
        let tab = r#"[{"address":"0x9","class":"zen","title":"Waydroid - Zen Browser",
                       "at":[0,0],"size":[100,100],"floating":false,"hidden":false}]"#;
        assert!(!parse(tab)
            .iter()
            .any(|client| client.class.eq_ignore_ascii_case(CLASS)));
    }

    #[test]
    fn rubbish_is_no_window_at_all() {
        assert!(parse("not json").is_empty());
        assert!(parse("[]").is_empty());
    }

    #[test]
    fn a_window_knows_its_workspace() {
        let clients = parse(CLIENTS);
        assert_eq!(clients[0].workspace, 0);
        let json = r#"[{"address":"0x3","class":"waydroid","title":"Android",
                        "at":[0,0],"size":[1,1],"workspace":{"id":5,"name":"5"}}]"#;
        assert_eq!(parse(json)[0].workspace, 5);
    }

    #[test]
    fn parking_is_a_silent_move_to_its_own_workspace() {
        assert_eq!(
            call("move", "0xabc", &format!("workspace = \"{PARKED}\"")),
            "hl.dsp.window.move({ window = hl.get_window(\"address:0xabc\"), workspace = \"special:omadroid\" })"
        );
    }

    #[test]
    fn focusing_uses_the_dispatcher_that_takes_a_window() {
        assert_eq!(
            focus_call("0xabc"),
            "hl.dsp.focus({ window = hl.get_window(\"address:0xabc\") })"
        );
    }

    #[test]
    fn a_dispatcher_names_the_window_and_its_numbers() {
        let lua = call("move", "0xabc", "x = 20, y = 30");
        assert_eq!(
            lua,
            "hl.dsp.window.move({ window = hl.get_window(\"address:0xabc\"), x = 20, y = 30 })"
        );
        assert_eq!(
            call("float", "0xabc", "action = \"set\""),
            "hl.dsp.window.float({ window = hl.get_window(\"address:0xabc\"), action = \"set\" })"
        );
    }
}
