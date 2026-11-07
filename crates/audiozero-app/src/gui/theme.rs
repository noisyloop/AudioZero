//! Colours and visuals.

use eframe::egui::{self, Color32};

pub const BG: Color32 = Color32::from_rgb(0x16, 0x17, 0x1a);
pub const PANEL: Color32 = Color32::from_rgb(0x1e, 0x20, 0x24);
pub const LANE: Color32 = Color32::from_rgb(0x1a, 0x1c, 0x20);
pub const LANE_ALT: Color32 = Color32::from_rgb(0x1d, 0x1f, 0x23);
pub const GRID: Color32 = Color32::from_rgb(0x2a, 0x2d, 0x33);
pub const BAR: Color32 = Color32::from_rgb(0x3a, 0x3e, 0x46);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8b, 0x90, 0x9a);
pub const ACCENT: Color32 = Color32::from_rgb(0xff, 0x8a, 0x3d);
pub const PLAYHEAD: Color32 = Color32::from_rgb(0xff, 0xd1, 0x66);
pub const LOOP: Color32 = Color32::from_rgba_premultiplied(0x30, 0x24, 0x10, 0x60);
pub const BLACK_KEY_ROW: Color32 = Color32::from_rgb(0x17, 0x18, 0x1b);
pub const WHITE_KEY_ROW: Color32 = Color32::from_rgb(0x1f, 0x21, 0x25);
pub const SELECTION: Color32 = Color32::WHITE;

const TRACK_COLOURS: [Color32; 8] = [
    Color32::from_rgb(0xff, 0x8a, 0x3d),
    Color32::from_rgb(0x4f, 0xc3, 0xf7),
    Color32::from_rgb(0xa5, 0xd6, 0x6b),
    Color32::from_rgb(0xce, 0x93, 0xd8),
    Color32::from_rgb(0xff, 0xd5, 0x4f),
    Color32::from_rgb(0x4d, 0xd0, 0xc1),
    Color32::from_rgb(0xf4, 0x8f, 0xb1),
    Color32::from_rgb(0x90, 0xa4, 0xff),
];

pub fn track(index: usize) -> Color32 {
    TRACK_COLOURS[index % TRACK_COLOURS.len()]
}

pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = BG;
    visuals.selection.bg_fill = ACCENT.linear_multiply(0.6);
    visuals.widgets.active.bg_fill = ACCENT;
    visuals.hyperlink_color = ACCENT;
    ctx.set_visuals(visuals);
}
