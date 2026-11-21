//! Local keyboard layouts to Windows keyboard layout identifiers (KLIDs),
//! reported to the server at connect (spec rdp-keyboard-layout, task 1.1).
//!
//! Windows and xrdp start the session with the reported layout. Where the
//! local layout has no Windows twin (Mac and "no dead keys" variants), the
//! closest Windows layout for the same language is reported; typed text stays
//! exact because characters go as Unicode (requirement 2).

use serde::Serialize;

/// A Windows keyboard layout 1-1 can report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    pub klid: u32,
    /// For the status bar, for example "DE".
    pub short: &'static str,
    pub name: &'static str,
}

/// How a local layout maps to a Windows layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Match {
    /// The same layout exists on Windows.
    Exact,
    /// Windows has no such variant; the base layout of the same language.
    Closest,
}

const fn layout(klid: u32, short: &'static str, name: &'static str) -> Layout {
    Layout { klid, short, name }
}

/// The layouts offered for a fixed choice, and every layout the tables
/// below can produce.
pub const LAYOUTS: &[Layout] = &[
    layout(0x0000_0409, "US", "English (US)"),
    layout(0x0002_0409, "US-INTL", "English (US, International)"),
    layout(0x0001_0409, "DVORAK", "English (US, Dvorak)"),
    layout(0x0000_0809, "GB", "English (UK)"),
    layout(0x0000_0407, "DE", "German"),
    layout(0x0000_0C07, "AT", "German (Austria)"),
    layout(0x0000_0807, "CH", "German (Switzerland)"),
    layout(0x0000_100C, "CH-FR", "French (Switzerland)"),
    layout(0x0000_040C, "FR", "French"),
    layout(0x0000_080C, "BE", "French (Belgium)"),
    layout(0x0000_040A, "ES", "Spanish"),
    layout(0x0000_0410, "IT", "Italian"),
    layout(0x0000_0413, "NL", "Dutch"),
    layout(0x0000_041D, "SE", "Swedish"),
    layout(0x0000_0414, "NO", "Norwegian"),
    layout(0x0000_0406, "DK", "Danish"),
    layout(0x0000_040B, "FI", "Finnish"),
    layout(0x0000_0415, "PL", "Polish (214)"),
    layout(0x0000_0816, "PT", "Portuguese"),
    layout(0x0000_0416, "BR", "Portuguese (Brazil, ABNT)"),
    layout(0x0000_0405, "CZ", "Czech"),
    layout(0x0000_040E, "HU", "Hungarian"),
    layout(0x0000_0419, "RU", "Russian"),
    layout(0x0000_0422, "UA", "Ukrainian"),
    layout(0x0000_041F, "TR", "Turkish Q"),
    layout(0x0000_0411, "JP", "Japanese"),
];

/// XKB layout and variant to KLID; `None` is the layout without a variant.
const XKB: &[(&str, Option<&str>, u32)] = &[
    ("us", None, 0x0000_0409),
    ("us", Some("intl"), 0x0002_0409),
    ("us", Some("dvorak"), 0x0001_0409),
    ("gb", None, 0x0000_0809),
    ("de", None, 0x0000_0407),
    ("at", None, 0x0000_0C07),
    ("ch", None, 0x0000_0807),
    ("ch", Some("de"), 0x0000_0807),
    ("ch", Some("fr"), 0x0000_100C),
    ("fr", None, 0x0000_040C),
    ("be", None, 0x0000_080C),
    ("es", None, 0x0000_040A),
    ("it", None, 0x0000_0410),
    ("nl", None, 0x0000_0413),
    ("se", None, 0x0000_041D),
    ("no", None, 0x0000_0414),
    ("dk", None, 0x0000_0406),
    ("fi", None, 0x0000_040B),
    ("pl", None, 0x0000_0415),
    ("pt", None, 0x0000_0816),
    ("br", None, 0x0000_0416),
    ("cz", None, 0x0000_0405),
    ("hu", None, 0x0000_040E),
    ("ru", None, 0x0000_0419),
    ("ua", None, 0x0000_0422),
    ("tr", None, 0x0000_041F),
    ("jp", None, 0x0000_0411),
];

/// macOS input source IDs (`com.apple.keylayout.` + this) to KLID. Every
/// Mac layout differs from its Windows twin in some keys, so all are closest
/// matches.
#[cfg_attr(not(target_os = "macos"), allow(dead_code, reason = "macOS detection"))]
const MAC: &[(&str, u32)] = &[
    ("US", 0x0000_0409),
    ("ABC", 0x0000_0409),
    ("USInternational-PC", 0x0002_0409),
    ("Dvorak", 0x0001_0409),
    ("British", 0x0000_0809),
    ("British-PC", 0x0000_0809),
    ("German", 0x0000_0407),
    ("Austrian", 0x0000_0C07),
    ("SwissGerman", 0x0000_0807),
    ("SwissFrench", 0x0000_100C),
    ("French", 0x0000_040C),
    ("French-PC", 0x0000_040C),
    ("Belgian", 0x0000_080C),
    ("Spanish", 0x0000_040A),
    ("Spanish-ISO", 0x0000_040A),
    ("Italian", 0x0000_0410),
    ("Italian-Pro", 0x0000_0410),
    ("Dutch", 0x0000_0413),
    ("Swedish", 0x0000_041D),
    ("Swedish-Pro", 0x0000_041D),
    ("Norwegian", 0x0000_0414),
    ("Danish", 0x0000_0406),
    ("Finnish", 0x0000_040B),
    ("Polish", 0x0000_0415),
    ("PolishPro", 0x0000_0415),
    ("Portuguese", 0x0000_0816),
    ("Brazilian", 0x0000_0416),
    ("Czech", 0x0000_0405),
    ("Hungarian", 0x0000_040E),
    ("Russian", 0x0000_0419),
    ("Ukrainian", 0x0000_0422),
    ("Turkish", 0x0000_041F),
];

/// The Windows layout with this identifier, if 1-1 knows it.
pub fn by_klid(klid: u32) -> Option<Layout> {
    LAYOUTS.iter().copied().find(|layout| layout.klid == klid)
}

/// An XKB layout and variant (as in `de(mac)`) to the Windows layout.
pub fn from_xkb(name: &str, variant: Option<&str>) -> Option<(Layout, Match)> {
    let name = name.trim().to_ascii_lowercase();
    let variant = variant
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "basic");
    let find = |variant: Option<&str>| {
        XKB.iter()
            .find(|(n, v, _)| *n == name && *v == variant)
            .and_then(|(_, _, klid)| by_klid(*klid))
    };
    if let Some(exact) = find(variant) {
        return Some((exact, Match::Exact));
    }
    // A variant Windows does not have: the layout's base.
    find(None).map(|base| (base, Match::Closest))
}

/// A macOS input source ID (`com.apple.keylayout.German`) to the Windows
/// layout.
#[cfg_attr(not(target_os = "macos"), allow(dead_code, reason = "macOS detection"))]
pub fn from_mac_source(id: &str) -> Option<(Layout, Match)> {
    let name = id.strip_prefix("com.apple.keylayout.")?;
    MAC.iter()
        .find(|(mac, _)| *mac == name)
        .and_then(|(_, klid)| by_klid(*klid))
        .map(|layout| (layout, Match::Closest))
}

/// The keyboard layout ID in a Windows input locale handle (HKL).
///
/// The handle's high word is the layout, its low word the input language,
/// and the two differ often: English (Germany) typing on the German layout
/// is `0x0407_2000`, where `0x2000` is a placeholder for a language without
/// its own ID. A high word `0xFnnn` is a variant (German IBM, Dvorak) whose
/// KLID the registry names by "Layout Id" `nnn`; `variant` looks it up.
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows detection"))]
pub fn klid_from_hkl(hkl: u32, variant: impl FnOnce(u16) -> Option<u32>) -> Option<u32> {
    let device = (hkl >> 16) as u16;
    match device {
        0 => None,
        _ if device & 0xF000 == 0xF000 => variant(device & 0x0FFF),
        _ => Some(u32::from(device)),
    }
}

/// A Windows layout ID as Windows reports it. A layout 1-1 does not know
/// whose language it does know (a third-party layout such as German (Magic
/// Utilities), `A0010407`, which servers lack) is reported as that
/// language's base layout, a closest match. Anything else goes as it is.
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows detection"))]
pub fn from_windows_klid(klid: u32) -> (Layout, Match) {
    if let Some(layout) = by_klid(klid) {
        return (layout, Match::Exact);
    }
    match by_klid(klid & 0xFFFF) {
        Some(base) => (base, Match::Closest),
        None => (
            Layout {
                klid,
                short: "??",
                name: "Windows layout",
            },
            Match::Exact,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_xkb_layouts_and_variants() {
        assert_eq!(
            from_xkb("de", None),
            Some((by_klid(0x407).unwrap(), Match::Exact))
        );
        assert_eq!(from_xkb("us", Some("intl")).unwrap().0.klid, 0x0002_0409);
        assert_eq!(from_xkb("ch", Some("fr")).unwrap().0.klid, 0x0000_100C);
        assert_eq!(
            from_xkb("DE", Some("")).unwrap(),
            (by_klid(0x407).unwrap(), Match::Exact)
        );
        assert_eq!(from_xkb("de", Some("basic")).unwrap().1, Match::Exact);
    }

    #[test]
    fn variants_windows_lacks_map_to_their_base() {
        // The maintainer's layout.
        assert_eq!(
            from_xkb("de", Some("mac")),
            Some((by_klid(0x407).unwrap(), Match::Closest))
        );
        assert_eq!(
            from_xkb("de", Some("nodeadkeys")).unwrap().1,
            Match::Closest
        );
        assert_eq!(from_xkb("gb", Some("mac")).unwrap().0.short, "GB");
    }

    #[test]
    fn unknown_layouts_report_nothing() {
        assert_eq!(from_xkb("xx", None), None);
        assert_eq!(from_mac_source("com.apple.keylayout.Klingon"), None);
        assert_eq!(from_mac_source("org.example.other"), None);
    }

    #[test]
    fn mac_sources_are_closest_matches() {
        assert_eq!(
            from_mac_source("com.apple.keylayout.German"),
            Some((by_klid(0x407).unwrap(), Match::Closest))
        );
        assert_eq!(
            from_mac_source("com.apple.keylayout.ABC").unwrap().0.short,
            "US"
        );
    }

    #[test]
    fn windows_layouts_are_exact() {
        assert_eq!(
            from_windows_klid(0x407),
            (by_klid(0x407).unwrap(), Match::Exact)
        );
        let unknown = from_windows_klid(0x42F);
        assert_eq!((unknown.0.klid, unknown.1), (0x42F, Match::Exact));
        // German (Magic Utilities), on the maintainer's workstation.
        assert_eq!(
            from_windows_klid(0xA001_0407),
            (by_klid(0x407).unwrap(), Match::Closest)
        );
    }

    #[test]
    fn the_layout_is_the_high_word_of_the_input_locale() {
        let no_variant = |_| panic!("not a variant");
        // German layout, German language.
        assert_eq!(klid_from_hkl(0x0407_0407, no_variant), Some(0x407));
        // English (Germany): a placeholder language with the German layout,
        // as on the maintainer's workstation. The language is not the layout.
        assert_eq!(klid_from_hkl(0x0407_2000, no_variant), Some(0x407));
        // English (US) language typing on the German layout.
        assert_eq!(klid_from_hkl(0x0407_0409, no_variant), Some(0x407));
        // A variant: the registry's "Layout Id" names its KLID.
        let dvorak = |id| (id == 0x002).then_some(0x0001_0409);
        assert_eq!(klid_from_hkl(0xF002_0409, dvorak), Some(0x0001_0409));
        assert_eq!(klid_from_hkl(0xF0FF_0409, dvorak), None);
        assert_eq!(klid_from_hkl(0x0000_0409, no_variant), None);
    }

    #[test]
    fn every_table_entry_names_a_known_layout() {
        for (name, variant, klid) in XKB {
            assert!(by_klid(*klid).is_some(), "{name}({variant:?})");
        }
        for (name, klid) in MAC {
            assert!(by_klid(*klid).is_some(), "{name}");
        }
        let mut klids: Vec<_> = LAYOUTS.iter().map(|l| l.klid).collect();
        klids.sort_unstable();
        klids.dedup();
        assert_eq!(klids.len(), LAYOUTS.len(), "no layout listed twice");
    }
}
