//! Which layout a session reports at connect, and why (spec
//! rdp-keyboard-layout, task 3.1).

use serde::{Deserialize, Serialize};

use super::detect::LocalLayout;
use super::klid::{self, Match};
use crate::store::connections::{KeyboardLayoutChoice, KeyboardLayoutName};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LayoutSource {
    /// The local layout, which Windows has too.
    Detected,
    /// The local layout's closest Windows layout (a Mac variant, say).
    ClosestMatch,
    /// A layout fixed in the host's settings.
    Override,
    /// The host reports nothing: the server's own layout.
    ServerDefault,
    /// Automatic, but the local layout is unknown: nothing reported.
    NotDetected,
}

/// What the session reported, for the status bar and the HUD.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportedLayout {
    /// The Windows layout ID sent to the server; 0 for none.
    pub klid: u32,
    /// For the status bar ("DE"); `None` when nothing was reported.
    pub short: Option<String>,
    /// The Windows layout's name, or why there is none.
    pub name: String,
    pub source: LayoutSource,
    /// The local layout it came from, by its own name.
    pub local: Option<String>,
}

impl ReportedLayout {
    /// The host setting to connect with: the resolved layout, fixed.
    pub fn as_choice(&self) -> KeyboardLayoutChoice {
        if self.klid == 0 {
            KeyboardLayoutChoice::Named(KeyboardLayoutName::ServerDefault)
        } else {
            KeyboardLayoutChoice::Fixed(self.klid)
        }
    }
}

/// Resolves a host's setting; `local` is only consulted for Automatic.
pub fn resolve(choice: KeyboardLayoutChoice, local: Option<LocalLayout>) -> ReportedLayout {
    let none = |source, name: &str, local: Option<String>| ReportedLayout {
        klid: 0,
        short: None,
        name: name.to_string(),
        source,
        local,
    };
    match choice {
        KeyboardLayoutChoice::Named(KeyboardLayoutName::ServerDefault) => {
            none(LayoutSource::ServerDefault, "the server's layout", None)
        }
        KeyboardLayoutChoice::Fixed(klid) => {
            let layout = klid::by_klid(klid);
            ReportedLayout {
                klid,
                short: Some(layout.map_or("??", |l| l.short).to_string()),
                name: layout.map_or_else(|| format!("layout {klid:08X}"), |l| l.name.to_string()),
                source: LayoutSource::Override,
                local: None,
            }
        }
        KeyboardLayoutChoice::Named(KeyboardLayoutName::Automatic) => match local {
            Some(LocalLayout {
                windows: Some(layout),
                matched,
                display_name,
            }) => ReportedLayout {
                klid: layout.klid,
                short: Some(layout.short.to_string()),
                name: layout.name.to_string(),
                source: if matched == Some(Match::Closest) {
                    LayoutSource::ClosestMatch
                } else {
                    LayoutSource::Detected
                },
                local: Some(display_name),
            },
            Some(LocalLayout { display_name, .. }) => none(
                LayoutSource::NotDetected,
                "no Windows layout for the local one",
                Some(display_name),
            ),
            None => none(
                LayoutSource::NotDetected,
                "the local layout is unknown",
                None,
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(layout: &str, variant: Option<&str>) -> LocalLayout {
        LocalLayout::from_xkb(&super::super::detect::XkbGroup {
            layout: layout.into(),
            variant: variant.map(Into::into),
            display_name: Some(format!("{layout} local")),
        })
    }

    #[test]
    fn every_source() {
        let auto = KeyboardLayoutChoice::AUTOMATIC;
        let detected = resolve(auto, Some(local("us", None)));
        assert_eq!(
            (detected.klid, detected.source),
            (0x409, LayoutSource::Detected)
        );
        assert_eq!(detected.as_choice(), KeyboardLayoutChoice::Fixed(0x409));

        let mac = resolve(auto, Some(local("de", Some("mac_nodeadkeys"))));
        assert_eq!((mac.klid, mac.source), (0x407, LayoutSource::ClosestMatch));
        assert_eq!(mac.short.as_deref(), Some("DE"));
        assert_eq!(mac.local.as_deref(), Some("de local"));

        let fixed = resolve(KeyboardLayoutChoice::Fixed(0x809), Some(local("de", None)));
        assert_eq!(
            (fixed.klid, fixed.source, fixed.short.as_deref()),
            (0x809, LayoutSource::Override, Some("GB"))
        );

        let server = resolve(
            KeyboardLayoutChoice::Named(KeyboardLayoutName::ServerDefault),
            Some(local("de", None)),
        );
        assert_eq!(
            (server.klid, server.source),
            (0, LayoutSource::ServerDefault)
        );
        assert_eq!(
            server.as_choice(),
            KeyboardLayoutChoice::Named(KeyboardLayoutName::ServerDefault)
        );

        let unknown = resolve(auto, Some(local("xx", None)));
        assert_eq!(
            (unknown.klid, unknown.source),
            (0, LayoutSource::NotDetected)
        );
        assert_eq!(resolve(auto, None).source, LayoutSource::NotDetected);
    }
}
