//! Colours and visuals. There are three colour schemes; the one in use is global, so drawing
//! code just asks for a colour by what it's for.

use eframe::egui::{self, Color32};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

/// A colour scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scheme {
    /// The name preferences store.
    pub id: &'static str,
    pub name: &'static str,
    dark: bool,
    bg: Color32,
    panel: Color32,
    lane: Color32,
    lane_alt: Color32,
    grid: Color32,
    bar: Color32,
    text_dim: Color32,
    accent: Color32,
    playhead: Color32,
    loop_fill: Color32,
    black_key_row: Color32,
    white_key_row: Color32,
    selection: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub const SCHEMES: [Scheme; 3] = [
    Scheme {
        id: "dark",
        name: "Dark",
        dark: true,
        bg: rgb(0x16171a),
        panel: rgb(0x1e2024),
        lane: rgb(0x1a1c20),
        lane_alt: rgb(0x1d1f23),
        grid: rgb(0x2a2d33),
        bar: rgb(0x3a3e46),
        text_dim: rgb(0x8b909a),
        accent: rgb(0xff8a3d),
        playhead: rgb(0xffd166),
        loop_fill: Color32::from_rgba_premultiplied(0x30, 0x24, 0x10, 0x60),
        black_key_row: rgb(0x17181b),
        white_key_row: rgb(0x1f2125),
        selection: Color32::WHITE,
    },
    Scheme {
        id: "darker",
        name: "Darker",
        dark: true,
        bg: rgb(0x060709),
        panel: rgb(0x0d0e11),
        lane: rgb(0x0a0b0d),
        lane_alt: rgb(0x0c0d10),
        grid: rgb(0x1a1c21),
        bar: rgb(0x2a2d34),
        text_dim: rgb(0x7c828d),
        accent: rgb(0xff8a3d),
        playhead: rgb(0xffd166),
        loop_fill: Color32::from_rgba_premultiplied(0x2a, 0x1e, 0x0c, 0x60),
        black_key_row: rgb(0x08090b),
        white_key_row: rgb(0x101216),
        selection: Color32::WHITE,
    },
    Scheme {
        id: "light",
        name: "Light",
        dark: false,
        bg: rgb(0xe6e8ec),
        panel: rgb(0xf3f4f6),
        lane: rgb(0xdcdfe5),
        lane_alt: rgb(0xe1e4e9),
        grid: rgb(0xc9cdd5),
        bar: rgb(0xaab0bb),
        text_dim: rgb(0x596070),
        accent: rgb(0xd95f0e),
        playhead: rgb(0xb93a04),
        loop_fill: Color32::from_rgba_premultiplied(0x50, 0x32, 0x08, 0x50),
        black_key_row: rgb(0xd3d6dd),
        white_key_row: rgb(0xe7e9ed),
        selection: rgb(0x101114),
    },
];

static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// The colour scheme in use.
pub fn scheme() -> &'static Scheme {
    &SCHEMES[CURRENT.load(Relaxed).min(SCHEMES.len() - 1)]
}

/// Switches to the scheme with this id (the default one if there's no such scheme) and
/// restyles the window.
pub fn set_scheme(ctx: &egui::Context, id: &str) {
    let index = SCHEMES.iter().position(|s| s.id == id).unwrap_or(0);
    CURRENT.store(index, Relaxed);
    apply(ctx);
}

pub fn bg() -> Color32 {
    scheme().bg
}
pub fn panel() -> Color32 {
    scheme().panel
}
pub fn lane() -> Color32 {
    scheme().lane
}
pub fn lane_alt() -> Color32 {
    scheme().lane_alt
}
pub fn grid() -> Color32 {
    scheme().grid
}
pub fn bar() -> Color32 {
    scheme().bar
}
pub fn text_dim() -> Color32 {
    scheme().text_dim
}
pub fn accent() -> Color32 {
    scheme().accent
}
pub fn playhead() -> Color32 {
    scheme().playhead
}
pub fn loop_fill() -> Color32 {
    scheme().loop_fill
}
pub fn black_key_row() -> Color32 {
    scheme().black_key_row
}
pub fn white_key_row() -> Color32 {
    scheme().white_key_row
}
pub fn selection() -> Color32 {
    scheme().selection
}

/// A see-through layer that dims what's under it: black on the dark schemes, grey on the
/// light one. `strength` is 0 to 255.
pub fn shade(strength: u8) -> Color32 {
    if scheme().dark {
        Color32::from_black_alpha(strength)
    } else {
        Color32::from_rgba_unmultiplied(0x40, 0x46, 0x52, strength / 2)
    }
}

const TRACK_COLOURS: [Color32; 8] = [
    rgb(0xff8a3d),
    rgb(0x4fc3f7),
    rgb(0xa5d66b),
    rgb(0xce93d8),
    rgb(0xffd54f),
    rgb(0x4dd0c1),
    rgb(0xf48fb1),
    rgb(0x90a4ff),
];

pub fn track(index: usize) -> Color32 {
    TRACK_COLOURS[index % TRACK_COLOURS.len()]
}

pub fn apply(ctx: &egui::Context) {
    let s = scheme();
    let mut visuals = if s.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.panel_fill = s.panel;
    visuals.window_fill = s.panel;
    visuals.extreme_bg_color = s.bg;
    visuals.selection.bg_fill = s.accent.linear_multiply(0.6);
    visuals.widgets.active.bg_fill = s.accent;
    visuals.hyperlink_color = s.accent;
    ctx.set_visuals(visuals);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemes_have_unique_ids_and_readable_text() {
        let mut ids: Vec<_> = SCHEMES.iter().map(|s| s.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), SCHEMES.len());
        let light = |c: Color32| u32::from(c.r()) + u32::from(c.g()) + u32::from(c.b());
        for s in SCHEMES {
            // Dim text and the selection outline have to stand out from the panels.
            let gap = light(s.text_dim).abs_diff(light(s.panel));
            assert!(gap > 250, "{}: dim text against the panel ({gap})", s.id);
            assert!(light(s.selection).abs_diff(light(s.lane)) > 500, "{}", s.id);
            assert_eq!(s.dark, light(s.bg) < 384, "{}", s.id);
        }
    }
}
