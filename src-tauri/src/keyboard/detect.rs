//! Detects the local keyboard layout (spec rdp-keyboard-layout, task 2).
//!
//! Linux on Wayland: the compositor sends every client its compiled keymap
//! (`wl_keyboard.keymap`) and the active layout group (`wl_keyboard.modifiers`).
//! 1-1 listens on GDK's connection, so the result follows layout switches.
//! The keymap's `xkb_symbols` line names the layouts, for example
//! `pc_de(mac_nodeadkeys)_us_2_inet(evdev)_altwin(swap_ralt_rwin)` on
//! Hyprland, and `name[1]="German (Macintosh, no dead keys)"` lines give their
//! display names.

use std::sync::Mutex;

use serde::Serialize;

use super::klid::{self, Layout, Match};

/// The layout the user types with right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalLayout {
    /// As the platform names it, for example "German (Macintosh, no dead keys)".
    pub display_name: String,
    /// The Windows layout it maps to, if any.
    pub windows: Option<Layout>,
    pub matched: Option<Match>,
}

/// One layout group of an XKB keymap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XkbGroup {
    pub layout: String,
    pub variant: Option<String>,
    pub display_name: Option<String>,
}

/// XKB symbol files that carry options, not layouts.
const OPTION_FILES: &[&str] = &[
    "pc",
    "inet",
    "altwin",
    "compose",
    "level3",
    "level5",
    "shift",
    "ctrl",
    "capslock",
    "caps",
    "terminate",
    "group",
    "keypad",
    "kpdl",
    "srvr_ctrl",
    "nbsp",
    "lv3",
    "lv5",
    "eurosign",
    "rupeesign",
    "grp",
    "grp_led",
    "numpad",
    "japan",
    "korean",
    "mod_led",
    "apple",
    "typo",
    "parens",
    "keypad",
    "esperanto_upper_case",
    "fkeys",
    "lv5",
    "mousekeys",
    "solaris",
    "srvrkeys",
    "macintosh_vndr",
    "lenovo_vndr",
    "sun_vndr",
];

/// Parses a compiled keymap's layout groups, in group order.
pub fn parse_keymap(text: &str) -> Vec<XkbGroup> {
    let Some(symbols) = text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("xkb_symbols \"")?
            .split('"')
            .next()
    }) else {
        return Vec::new();
    };
    let mut groups: Vec<(usize, XkbGroup)> = Vec::new();
    let bytes = symbols.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        // name
        let start = at;
        while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'-') {
            at += 1;
        }
        let name = &symbols[start..at];
        // (variant)
        let mut variant = None;
        if at < bytes.len() && bytes[at] == b'(' {
            let close = symbols[at..].find(')').map_or(bytes.len(), |i| at + i);
            variant = Some(symbols[at + 1..close].to_string());
            at = (close + 1).min(bytes.len());
        }
        // _N group number, then the _ separator
        let mut group = None;
        if at < bytes.len() && bytes[at] == b'_' {
            let digits = symbols[at + 1..]
                .bytes()
                .take_while(u8::is_ascii_digit)
                .count();
            let after = at + 1 + digits;
            if digits > 0 && (after == bytes.len() || bytes[after] == b'_') {
                group = symbols[at + 1..after].parse::<usize>().ok();
                at = after;
            }
        }
        if at < bytes.len() && bytes[at] == b'_' {
            at += 1;
        }
        if name.is_empty() {
            // Not a token we understand; skip one byte to make progress.
            at = (start + 1).max(at);
            continue;
        }
        if OPTION_FILES.contains(&name) {
            continue;
        }
        let index = group.unwrap_or(if groups.is_empty() {
            1
        } else {
            groups.len() + 1
        });
        groups.push((
            index,
            XkbGroup {
                layout: name.to_string(),
                variant: variant.filter(|v| !v.is_empty()),
                display_name: None,
            },
        ));
    }
    // name[N]="…" or name[GroupN]="…"
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("name[") else {
            continue;
        };
        let Some((index, value)) = rest.split_once("]=") else {
            continue;
        };
        let index = index.trim_start_matches("Group").parse::<usize>().ok();
        let value = value.trim().trim_end_matches(';').trim_matches('"');
        if let Some((_, group)) = groups.iter_mut().find(|(i, _)| Some(*i) == index) {
            group.display_name = Some(value.to_string());
        }
    }
    groups.sort_by_key(|(index, _)| *index);
    groups.into_iter().map(|(_, group)| group).collect()
}

impl LocalLayout {
    pub fn from_xkb(group: &XkbGroup) -> Self {
        let resolved = klid::from_xkb(&group.layout, group.variant.as_deref());
        Self {
            display_name: group
                .display_name
                .clone()
                .unwrap_or_else(|| match &group.variant {
                    Some(variant) => format!("{} ({variant})", group.layout),
                    None => group.layout.clone(),
                }),
            windows: resolved.map(|(layout, _)| layout),
            matched: resolved.map(|(_, matched)| matched),
        }
    }
}

/// What the platform reported last (Wayland updates it on every switch).
static CURRENT: Mutex<Option<LocalLayout>> = Mutex::new(None);

/// The local layout right now, if it could be detected.
pub fn current() -> Option<LocalLayout> {
    // Tests never depend on the machine's layout.
    if cfg!(test) {
        return None;
    }
    #[cfg(target_os = "linux")]
    if let Some(layout) = CURRENT.lock().unwrap().clone() {
        return Some(layout);
    }
    platform::detect_now()
}

#[tauri::command]
pub fn detect_keyboard_layout() -> Option<LocalLayout> {
    current()
}

#[cfg(target_os = "linux")]
pub use platform::watch_wayland;

#[cfg(target_os = "linux")]
mod platform {
    use std::cell::RefCell;
    use std::io::Read;

    use gtk::glib;
    use gtk::glib::translate::ToGlibPtr;
    use gtk::prelude::*;
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::{
        wl_keyboard::{self, WlKeyboard},
        wl_registry::WlRegistry,
        wl_seat::WlSeat,
    };
    use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};

    use super::{CURRENT, LocalLayout, XkbGroup, parse_keymap};

    #[derive(Default)]
    struct State {
        groups: Vec<XkbGroup>,
        active: usize,
    }

    impl State {
        fn publish(&self) {
            let layout = self
                .groups
                .get(self.active)
                .or(self.groups.first())
                .map(LocalLayout::from_xkb);
            let mut current = CURRENT.lock().unwrap();
            if *current != layout {
                if let Some(layout) = &layout {
                    log::info!(
                        "Keyboard layout: {} ({})",
                        layout.display_name,
                        layout.windows.map_or("no Windows layout", |w| w.name)
                    );
                }
                *current = layout;
            }
        }
    }

    impl Dispatch<WlRegistry, GlobalListContents> for State {
        fn event(
            _: &mut Self,
            _: &WlRegistry,
            _: <WlRegistry as Proxy>::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<WlSeat, ()> for State {
        fn event(
            _: &mut Self,
            _: &WlSeat,
            _: <WlSeat as Proxy>::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<WlKeyboard, ()> for State {
        fn event(
            state: &mut Self,
            _: &WlKeyboard,
            event: wl_keyboard::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                wl_keyboard::Event::Keymap { fd, size, .. } => {
                    let mut text = vec![0u8; size as usize];
                    if std::fs::File::from(fd).read_exact(&mut text).is_ok() {
                        state.groups = parse_keymap(&String::from_utf8_lossy(&text));
                        state.publish();
                    }
                }
                wl_keyboard::Event::Modifiers { group, .. } if state.active != group as usize => {
                    state.active = group as usize;
                    state.publish();
                }
                _ => {}
            }
        }
    }

    struct Watch {
        queue: EventQueue<State>,
        state: State,
        _keyboard: WlKeyboard,
    }

    thread_local! {
        static WATCH: RefCell<Option<Watch>> = const { RefCell::new(None) };
    }

    /// Listens to the compositor's keymap on GDK's Wayland connection. Runs
    /// on GTK's main thread; does nothing on X11.
    pub fn watch_wayland() {
        let Some(display) = gtk::gdk::Display::default() else {
            return;
        };
        if !display.type_().name().contains("Wayland") {
            return;
        }
        // SAFETY: GDK owns the wl_display for the app's lifetime; this only
        // adds our own event queue to it.
        let conn = unsafe {
            let ptr: *mut gtk::gdk::ffi::GdkDisplay = display.to_glib_none().0;
            let wl_display = gdk_wayland_sys::gdk_wayland_display_get_wl_display(ptr as *mut _);
            Connection::from_backend(wayland_backend::client::Backend::from_foreign_display(
                wl_display as *mut _,
            ))
        };
        let Ok((globals, mut queue)) = registry_queue_init::<State>(&conn) else {
            return;
        };
        let qh = queue.handle();
        let Ok(seat) = globals.bind::<WlSeat, _, _>(&qh, 1..=7, ()) else {
            log::info!("Keyboard layout: no wl_seat, not detected");
            return;
        };
        let keyboard = seat.get_keyboard(&qh, ());
        let mut state = State::default();
        let _ = queue.roundtrip(&mut state);
        WATCH.with(|slot| {
            *slot.borrow_mut() = Some(Watch {
                queue,
                state,
                _keyboard: keyboard,
            })
        });
        glib::timeout_add_local(std::time::Duration::from_millis(500), || {
            WATCH.with(|slot| {
                if let Some(watch) = slot.borrow_mut().as_mut() {
                    let _ = watch.queue.dispatch_pending(&mut watch.state);
                }
            });
            glib::ControlFlow::Continue
        });
    }

    /// X11: `setxkbmap -query`, first layout (task 2.2).
    pub fn detect_now() -> Option<LocalLayout> {
        let output = std::process::Command::new("setxkbmap")
            .arg("-query")
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        let field = |name: &str| {
            text.lines()
                .find_map(|line| {
                    line.strip_prefix(name)?
                        .trim_start_matches(':')
                        .trim()
                        .split(',')
                        .next()
                        .map(str::to_string)
                })
                .filter(|value| !value.is_empty())
        };
        let group = XkbGroup {
            layout: field("layout")?,
            variant: field("variant"),
            display_name: None,
        };
        Some(LocalLayout::from_xkb(&group))
    }
}

#[cfg(not(target_os = "linux"))]
mod platform {
    use super::LocalLayout;

    /// Windows and macOS detection: task 2.2.
    pub fn detect_now() -> Option<LocalLayout> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// From the maintainer's Hyprland (21 November 2026), shortened.
    const HYPRLAND: &str = r#"xkb_keymap {
xkb_keycodes "evdev_aliases(qwertz)" {
};
xkb_symbols "pc_de(mac_nodeadkeys)_us_2_inet(evdev)_altwin(swap_ralt_rwin)_compose(caps)_level3(lalt_switch)_shift(both_capslock_cancel)" {
	name[1]="German (Macintosh, no dead keys)";
	name[2]="English (US)";
};
};"#;

    #[test]
    fn hyprland_keymap_with_two_layouts_and_options() {
        let groups = parse_keymap(HYPRLAND);
        assert_eq!(
            groups,
            vec![
                XkbGroup {
                    layout: "de".into(),
                    variant: Some("mac_nodeadkeys".into()),
                    display_name: Some("German (Macintosh, no dead keys)".into()),
                },
                XkbGroup {
                    layout: "us".into(),
                    variant: None,
                    display_name: Some("English (US)".into()),
                },
            ]
        );
        let local = LocalLayout::from_xkb(&groups[0]);
        assert_eq!(local.windows.unwrap().short, "DE");
        assert_eq!(local.matched, Some(Match::Closest));
        assert_eq!(
            LocalLayout::from_xkb(&groups[1]).matched,
            Some(Match::Exact)
        );
    }

    #[test]
    fn a_single_layout_and_the_older_name_form() {
        let text = "xkb_symbols \"pc+gb+inet(evdev)\" {\n\tname[Group1]=\"English (UK)\";\n};";
        let groups = parse_keymap(text.replace('+', "_").as_str());
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].layout, "gb");
        assert_eq!(groups[0].display_name.as_deref(), Some("English (UK)"));
    }

    #[test]
    fn a_variant_with_its_own_parentheses_group_and_three_layouts() {
        let text = "xkb_symbols \"pc_us(intl)_ch(fr)_2_ru_3_inet(evdev)\" {};";
        let names: Vec<_> = parse_keymap(text)
            .into_iter()
            .map(|g| (g.layout, g.variant))
            .collect();
        assert_eq!(
            names,
            vec![
                ("us".to_string(), Some("intl".to_string())),
                ("ch".to_string(), Some("fr".to_string())),
                ("ru".to_string(), None),
            ]
        );
    }

    #[test]
    fn no_symbols_line_means_not_detected() {
        assert!(parse_keymap("xkb_keymap { };").is_empty());
        assert!(
            LocalLayout::from_xkb(&XkbGroup {
                layout: "xx".into(),
                variant: None,
                display_name: None
            })
            .windows
            .is_none()
        );
    }
}
