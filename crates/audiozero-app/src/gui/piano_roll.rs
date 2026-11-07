//! The piano roll: editing one MIDI clip's notes.
//!
//! - Click an empty spot to add a note; drag from an empty spot to draw one at the length you want.
//! - Drag notes to move them in time and pitch, drag a note's right edge to change its length.
//! - Click selects (Shift adds to the selection), Ctrl/Cmd + A selects all.
//! - Right-click a note, or press Delete, to remove notes; Up/Down transposes (Shift: an octave).
//! - Hold Alt to ignore the grid.

use super::{App, ClipRef, theme};
use audiozero_engine::{Clip, Note};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, StrokeKind,
    Vec2,
};
use std::collections::BTreeSet;

const KEYS_WIDTH: f32 = 56.0;
const ROW: f32 = 12.0;
const EDGE_GRAB: f32 = 5.0;
const NEW_NOTE_VELOCITY: f32 = 0.8;

pub struct Roll {
    pub clip: ClipRef,
    selected: BTreeSet<usize>,
    pixels_per_beat: f32,
    scroll_beats: f32,
    /// The pitch at the top edge of the grid (fractional while scrolling).
    top_pitch: f32,
    fit_pending: bool,
    drag: Option<NoteDrag>,
    /// The length new notes get: the last length drawn or resized.
    new_length: f64,
}

struct NoteDrag {
    kind: DragKind,
    anchor_beat: f64,
    anchor_pitch: f32,
    /// The dragged notes as they were when the drag started.
    originals: Vec<(usize, Note)>,
}

#[derive(Clone, Copy, PartialEq)]
enum DragKind {
    Move,
    Resize,
}

impl Roll {
    pub fn new(clip: ClipRef, contents: Option<&Clip>) -> Self {
        let highest = contents
            .and_then(|c| c.notes.iter().map(|n| n.pitch).max())
            .unwrap_or(72);
        Self {
            clip,
            selected: BTreeSet::new(),
            pixels_per_beat: 60.0,
            scroll_beats: 0.0,
            top_pitch: (f32::from(highest) + 6.0).min(128.0),
            fit_pending: true,
            drag: None,
            new_length: 0.25,
        }
    }

    pub fn has_selection(&self) -> bool {
        !self.selected.is_empty()
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
        self.drag = None;
    }

    pub fn select_all(&mut self, notes: usize) {
        self.selected = (0..notes).collect();
    }
}

/// Converts between clip-relative beats/pitches and the grid's screen positions.
#[derive(Clone, Copy)]
struct Grid {
    rect: Rect,
    scroll: f32,
    pixels_per_beat: f32,
    top_pitch: f32,
}

impl Grid {
    fn x(self, beat: f64) -> f32 {
        self.rect.left() + (beat as f32 - self.scroll) * self.pixels_per_beat
    }
    fn beat(self, x: f32) -> f64 {
        f64::from(self.scroll + (x - self.rect.left()) / self.pixels_per_beat)
    }
    /// Top of the row for `pitch`.
    fn y(self, pitch: u8) -> f32 {
        self.rect.top() + (self.top_pitch - f32::from(pitch) - 1.0) * ROW
    }
    /// The pitch under `y`, as a fraction (for smooth dragging).
    fn pitch(self, y: f32) -> f32 {
        self.top_pitch - (y - self.rect.top()) / ROW
    }
    fn note_rect(self, note: &Note, clip_length: f64) -> Rect {
        let end = (note.start + note.length).min(clip_length);
        let y = self.y(note.pitch);
        Rect::from_min_max(
            Pos2::new(self.x(note.start), y + 1.0),
            Pos2::new(self.x(end).max(self.x(note.start) + 3.0), y + ROW - 1.0),
        )
    }
}

fn is_black_key(pitch: u8) -> bool {
    matches!(pitch % 12, 1 | 3 | 6 | 8 | 10)
}

fn note_name(pitch: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!(
        "{}{}",
        NAMES[usize::from(pitch % 12)],
        i32::from(pitch / 12) - 1
    )
}

impl App {
    pub(super) fn piano_roll(&mut self, ui: &mut egui::Ui) {
        let Some(at) = self.roll.as_ref().map(|r| r.clip) else {
            return;
        };
        let Some(clip) = self.clip(at).cloned() else {
            self.roll = None;
            return;
        };
        let ctx = ui.ctx().clone();
        let colour = theme::track(at.track);

        ui.add_space(4.0);
        let mut close = false;
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(egui::RichText::new(&clip.name).strong().color(colour));
            let track_name = &self.project.tracks[at.track].name;
            ui.label(egui::RichText::new(format!("on {track_name} · {} notes · {} bars", clip.notes.len(), clip.length / f64::from(self.project.beats_per_bar))).color(theme::TEXT_DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                close = ui.button("Close").clicked();
                ui.label(egui::RichText::new("Click to add · drag to move · right-click or Delete to remove · Up/Down transpose · Alt: no grid").color(theme::TEXT_DIM).size(11.0));
            });
        });
        if close {
            self.roll = None;
            return;
        }

        let area = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(area, Sense::click_and_drag());
        let keys = Rect::from_min_max(
            area.left_top(),
            Pos2::new(area.left() + KEYS_WIDTH, area.bottom()),
        );
        let grid_rect =
            Rect::from_min_max(Pos2::new(keys.right(), area.top()), area.right_bottom());
        let Some(roll) = self.roll.as_mut() else {
            return;
        };

        if roll.fit_pending {
            roll.pixels_per_beat =
                (grid_rect.width() / clip.length.max(4.0) as f32).clamp(8.0, 200.0);
            roll.fit_pending = false;
        }
        if response.hovered() {
            let (zoom, scroll) = ctx.input(|i| (i.zoom_delta(), i.smooth_scroll_delta));
            if zoom != 1.0 {
                let anchor = ctx
                    .input(|i| i.pointer.hover_pos())
                    .map_or(grid_rect.left(), |p| p.x.max(grid_rect.left()));
                let beat_at =
                    roll.scroll_beats + (anchor - grid_rect.left()) / roll.pixels_per_beat;
                roll.pixels_per_beat = (roll.pixels_per_beat * zoom).clamp(8.0, 600.0);
                roll.scroll_beats = beat_at - (anchor - grid_rect.left()) / roll.pixels_per_beat;
            }
            roll.scroll_beats -= scroll.x / roll.pixels_per_beat;
            roll.top_pitch += scroll.y / ROW;
        }
        let visible_rows = grid_rect.height() / ROW;
        roll.top_pitch = roll.top_pitch.clamp(visible_rows.min(128.0), 128.0);
        roll.scroll_beats = roll.scroll_beats.clamp(0.0, clip.length as f32);
        let grid = Grid {
            rect: grid_rect,
            scroll: roll.scroll_beats,
            pixels_per_beat: roll.pixels_per_beat,
            top_pitch: roll.top_pitch,
        };

        let painter = ui.painter_at(area);
        self.draw_roll(&painter, keys, grid, &clip, colour);
        self.roll_interactions(&ctx, &response, grid, &clip);
    }

    fn draw_roll(
        &self,
        painter: &egui::Painter,
        keys: Rect,
        grid: Grid,
        clip: &Clip,
        colour: Color32,
    ) {
        let rect = grid.rect;
        let grid_painter = painter.with_clip_rect(rect);
        let keys_painter = painter.with_clip_rect(keys);
        let lowest = grid.pitch(rect.bottom()).floor().max(0.0) as u8;
        let highest = (grid.top_pitch.ceil() as i32 - 1).clamp(0, 127) as u8;

        // Rows and keyboard.
        for pitch in lowest..=highest {
            let y = grid.y(pitch);
            let row =
                Rect::from_min_max(Pos2::new(rect.left(), y), Pos2::new(rect.right(), y + ROW));
            grid_painter.rect_filled(
                row,
                0.0,
                if is_black_key(pitch) {
                    theme::BLACK_KEY_ROW
                } else {
                    theme::WHITE_KEY_ROW
                },
            );
            if pitch % 12 == 0 {
                grid_painter.line_segment(
                    [row.left_bottom(), row.right_bottom()],
                    Stroke::new(1.0, theme::BAR),
                );
            }
            let key =
                Rect::from_min_max(Pos2::new(keys.left(), y), Pos2::new(keys.right(), y + ROW));
            let (fill, text) = if is_black_key(pitch) {
                (Color32::from_rgb(0x22, 0x24, 0x28), theme::TEXT_DIM)
            } else {
                (
                    Color32::from_rgb(0xd8, 0xda, 0xde),
                    Color32::from_rgb(0x30, 0x30, 0x30),
                )
            };
            keys_painter.rect_filled(key.shrink2(Vec2::new(0.0, 0.5)), 1.0, fill);
            if pitch % 12 == 0 {
                keys_painter.text(
                    Pos2::new(key.right() - 4.0, key.center().y),
                    Align2::RIGHT_CENTER,
                    note_name(pitch),
                    FontId::proportional(9.0),
                    text,
                );
            }
        }

        // Time grid: the snap step if it's wide enough, beats and bars stronger.
        let per_bar = f64::from(self.project.beats_per_bar);
        let mut step = self.snap_beats();
        if step <= 0.0 {
            step = 0.25;
        }
        while step * f64::from(grid.pixels_per_beat) < 6.0 {
            step *= 2.0;
        }
        let first = (grid.beat(rect.left()) / step).floor().max(0.0) as i64;
        let last = (grid.beat(rect.right()) / step).ceil() as i64;
        for i in first..=last {
            let beat = i as f64 * step;
            let x = grid.x(beat);
            let colour = if beat % per_bar == 0.0 {
                theme::BAR.gamma_multiply(1.6)
            } else if beat.fract() == 0.0 {
                theme::BAR
            } else {
                theme::GRID
            };
            grid_painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.0, colour),
            );
        }

        // Past the clip's end.
        let end_x = grid.x(clip.length);
        if end_x < rect.right() {
            grid_painter.rect_filled(
                Rect::from_min_max(Pos2::new(end_x, rect.top()), rect.right_bottom()),
                0.0,
                Color32::from_black_alpha(140),
            );
            grid_painter.line_segment(
                [
                    Pos2::new(end_x, rect.top()),
                    Pos2::new(end_x, rect.bottom()),
                ],
                Stroke::new(2.0, colour),
            );
        }

        // Notes.
        let selected = self.roll.as_ref().map(|r| &r.selected);
        for (index, note) in clip.notes.iter().enumerate() {
            let r = grid.note_rect(note, clip.length);
            if !r.intersects(rect) {
                continue;
            }
            let is_selected = selected.is_some_and(|s| s.contains(&index));
            let fill = colour.gamma_multiply(0.45 + 0.55 * note.velocity);
            grid_painter.rect_filled(r, CornerRadius::same(2), fill);
            let outline = if is_selected {
                Stroke::new(1.5, theme::SELECTION)
            } else {
                Stroke::new(1.0, colour.gamma_multiply(0.5))
            };
            grid_painter.rect_stroke(r, CornerRadius::same(2), outline, StrokeKind::Inside);
        }

        // Playhead, if it's inside this clip.
        let position = self.audio.controller.position() - clip.start;
        if (0.0..=clip.length).contains(&position) {
            let x = grid.x(position);
            grid_painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.5, theme::PLAYHEAD),
            );
        }
        painter.line_segment(
            [keys.right_top(), keys.right_bottom()],
            Stroke::new(1.0, theme::BG),
        );
    }

    fn roll_interactions(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        grid: Grid,
        clip: &Clip,
    ) {
        let Some(at) = self.roll.as_ref().map(|r| r.clip) else {
            return;
        };
        let (shift, alt) = ctx.input(|i| (i.modifiers.shift, i.modifiers.alt));
        let step = if alt { 0.0 } else { self.snap_beats() };
        let snap_delta = |d: f64| {
            if step > 0.0 {
                (d / step).round() * step
            } else {
                d
            }
        };
        let snap_down = |b: f64| {
            if step > 0.0 {
                (b / step).floor() * step
            } else {
                b
            }
        };
        let min_length = if step > 0.0 { step } else { 1.0 / 32.0 };

        // The note under a point, and whether the point is on its right edge.
        let note_at = |p: Pos2| {
            clip.notes.iter().enumerate().rev().find_map(|(i, n)| {
                let r = grid.note_rect(n, clip.length);
                r.contains(p)
                    .then(|| (i, (r.right() - p.x).abs() <= EDGE_GRAB))
            })
        };

        if let Some(hover) = response.hover_pos()
            && grid.rect.contains(hover)
            && let Some((_, edge)) = note_at(hover)
        {
            ctx.set_cursor_icon(if edge {
                CursorIcon::ResizeHorizontal
            } else {
                CursorIcon::Grab
            });
        }

        // Start a drag: on a note, move or resize it (and the rest of the selection); on an empty
        // spot, draw a new note whose length follows the pointer.
        // Measure from where the button went down: egui reports a drag only after the pointer
        // has moved a few pixels, and starting from there would lose that distance.
        if response.drag_started()
            && let Some(p) = ctx
                .input(|i| i.pointer.press_origin())
                .or(response.interact_pointer_pos())
            && grid.rect.contains(p)
        {
            self.begin_gesture();
            let kind = match note_at(p) {
                Some((index, edge)) => {
                    let roll = self.roll.as_mut().expect("checked above");
                    if !roll.selected.contains(&index) {
                        if !shift {
                            roll.selected.clear();
                        }
                        roll.selected.insert(index);
                    }
                    if edge {
                        DragKind::Resize
                    } else {
                        DragKind::Move
                    }
                }
                None => {
                    let note = Note {
                        pitch: grid.pitch(p.y).floor().clamp(0.0, 127.0) as u8,
                        start: snap_down(grid.beat(p.x)).clamp(0.0, clip.length - 1.0 / 128.0),
                        length: min_length,
                        velocity: NEW_NOTE_VELOCITY,
                    };
                    let mut index = 0;
                    self.live(|proj| {
                        let notes = &mut proj.tracks[at.track].clips[at.clip].notes;
                        notes.push(note);
                        index = notes.len() - 1;
                    });
                    let roll = self.roll.as_mut().expect("checked above");
                    roll.selected = BTreeSet::from([index]);
                    DragKind::Resize
                }
            };
            let notes = &self.project.tracks[at.track].clips[at.clip].notes;
            let roll = self.roll.as_mut().expect("checked above");
            let originals = roll
                .selected
                .iter()
                .filter_map(|&i| notes.get(i).map(|n| (i, *n)))
                .collect();
            roll.drag = Some(NoteDrag {
                kind,
                anchor_beat: grid.beat(p.x),
                anchor_pitch: grid.pitch(p.y),
                originals,
            });
        }

        // Continue or finish a drag.
        let dragging = self.roll.as_ref().and_then(|r| r.drag.as_ref()).is_some();
        if dragging {
            let (down, pointer) = ctx.input(|i| (i.pointer.primary_down(), i.pointer.latest_pos()));
            match (down, pointer) {
                (true, Some(p)) => {
                    let drag = self
                        .roll
                        .as_ref()
                        .and_then(|r| r.drag.as_ref())
                        .expect("checked above");
                    let d_beats = snap_delta(grid.beat(p.x) - drag.anchor_beat);
                    let d_pitch = (grid.pitch(p.y) - drag.anchor_pitch).round() as i32;
                    let (kind, originals) = (drag.kind, drag.originals.clone());
                    let length = clip.length;
                    ctx.set_cursor_icon(if kind == DragKind::Move {
                        CursorIcon::Grabbing
                    } else {
                        CursorIcon::ResizeHorizontal
                    });
                    self.live(|proj| {
                        let notes = &mut proj.tracks[at.track].clips[at.clip].notes;
                        for (index, original) in &originals {
                            let Some(note) = notes.get_mut(*index) else {
                                continue;
                            };
                            match kind {
                                DragKind::Move => {
                                    note.start =
                                        (original.start + d_beats).clamp(0.0, length - 1.0 / 128.0);
                                    note.pitch =
                                        (i32::from(original.pitch) + d_pitch).clamp(0, 127) as u8;
                                }
                                DragKind::Resize => {
                                    note.length = (original.length + d_beats).max(min_length)
                                }
                            }
                        }
                    });
                }
                _ => {
                    let roll = self.roll.as_mut().expect("checked above");
                    if let Some(drag) = roll.drag.take()
                        && drag.kind == DragKind::Resize
                        && let Some(&(index, _)) = drag.originals.first()
                        && let Some(note) = self.project.tracks[at.track].clips[at.clip]
                            .notes
                            .get(index)
                    {
                        roll.new_length = note.length;
                    }
                    self.end_gesture();
                }
            }
            return;
        }

        // Clicks.
        let Some(p) = response.interact_pointer_pos() else {
            return;
        };
        if !grid.rect.contains(p) {
            return;
        }
        if response.secondary_clicked() {
            if let Some((index, _)) = note_at(p) {
                self.edit(|proj| {
                    proj.tracks[at.track].clips[at.clip].notes.remove(index);
                });
                if let Some(roll) = &mut self.roll {
                    roll.selected.clear();
                }
            }
        } else if response.clicked() {
            match note_at(p) {
                Some((index, _)) => {
                    let roll = self.roll.as_mut().expect("checked above");
                    if shift {
                        if !roll.selected.remove(&index) {
                            roll.selected.insert(index);
                        }
                    } else {
                        roll.selected = BTreeSet::from([index]);
                    }
                }
                None => {
                    let length = self.roll.as_ref().map_or(0.25, |r| r.new_length);
                    let note = Note {
                        pitch: grid.pitch(p.y).floor().clamp(0.0, 127.0) as u8,
                        start: snap_down(grid.beat(p.x)).clamp(0.0, clip.length - 1.0 / 128.0),
                        length,
                        velocity: NEW_NOTE_VELOCITY,
                    };
                    let mut index = 0;
                    self.edit(|proj| {
                        let notes = &mut proj.tracks[at.track].clips[at.clip].notes;
                        notes.push(note);
                        index = notes.len() - 1;
                    });
                    if let Some(roll) = &mut self.roll {
                        roll.selected = BTreeSet::from([index]);
                    }
                }
            }
        }
    }

    pub(super) fn delete_selected_notes(&mut self) {
        let Some(roll) = &mut self.roll else { return };
        let at = roll.clip;
        let selected = std::mem::take(&mut roll.selected);
        if selected.is_empty() {
            return;
        }
        self.edit(|proj| {
            if let Some(clip) = proj
                .tracks
                .get_mut(at.track)
                .and_then(|t| t.clips.get_mut(at.clip))
            {
                for index in selected.iter().rev() {
                    if *index < clip.notes.len() {
                        clip.notes.remove(*index);
                    }
                }
            }
        });
    }

    pub(super) fn transpose_selected_notes(&mut self, semitones: i32) {
        let Some(roll) = &self.roll else { return };
        if roll.selected.is_empty() {
            return;
        }
        let (at, selected) = (roll.clip, roll.selected.clone());
        self.edit(|proj| {
            if let Some(clip) = proj
                .tracks
                .get_mut(at.track)
                .and_then(|t| t.clips.get_mut(at.clip))
            {
                for index in &selected {
                    if let Some(note) = clip.notes.get_mut(*index) {
                        note.pitch = (i32::from(note.pitch) + semitones).clamp(0, 127) as u8;
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_notes_like_most_daws() {
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(69), "A4");
        assert_eq!(note_name(0), "C-1");
        assert!(is_black_key(61) && !is_black_key(64));
    }

    #[test]
    fn grid_converts_both_ways() {
        let grid = Grid {
            rect: Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 240.0)),
            scroll: 2.0,
            pixels_per_beat: 50.0,
            top_pitch: 80.0,
        };
        assert_eq!(grid.x(2.0), 0.0);
        assert!((grid.beat(grid.x(5.5)) - 5.5).abs() < 1e-6);
        let y = grid.y(72) + ROW / 2.0;
        assert_eq!(grid.pitch(y).floor() as u8, 72);
    }
}
