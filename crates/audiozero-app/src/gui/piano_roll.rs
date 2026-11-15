//! The piano roll: editing one MIDI clip's notes.
//!
//! - Click an empty spot to add a note; Ctrl/Cmd-drag from an empty spot to draw one at the
//!   length you want.
//! - Drag across an empty area to select the notes inside the box (Shift adds to the selection).
//! - Drag notes to move them in time and pitch, drag a note's right edge to change its length.
//! - Click selects (Shift adds to the selection), Ctrl/Cmd + A selects all, Ctrl/Cmd + D copies
//!   the selected notes to straight after them.
//! - Right-click a note, or press Delete, to remove notes; Up/Down transposes (Shift: an octave).
//! - The song's key is marked on the grid; "In key" keeps new and moved notes on the scale, and
//!   Randomize writes a melody in key (or new pitches for the selected notes).
//! - Hold Alt to ignore the grid.

use super::{App, ClipRef, theme};
use audiozero_engine::music::{self, MelodySettings};
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
    /// Keep new and moved notes on the song's scale.
    in_key: bool,
    /// The selection box being dragged, if any.
    band: Option<Rect>,
}

struct NoteDrag {
    kind: DragKind,
    anchor_beat: f64,
    anchor_pitch: f32,
    /// The dragged notes as they were when the drag started.
    originals: Vec<(usize, Note)>,
    /// For a selection box: where it started, and what was selected before it.
    origin: Pos2,
    before: BTreeSet<usize>,
}

#[derive(Clone, Copy, PartialEq)]
enum DragKind {
    Move,
    Resize,
    /// Dragging a box around notes to select them.
    Select,
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
            in_key: false,
            band: None,
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

    pub fn select(&mut self, notes: impl IntoIterator<Item = usize>) {
        self.selected = notes.into_iter().collect();
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
        let mut randomize = false;
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(egui::RichText::new(&clip.name).strong().color(colour));
            ui.separator();
            self.key_picker(ui, "roll");
            if let Some(roll) = &mut self.roll {
                ui.checkbox(&mut roll.in_key, "In key")
                    .on_hover_text("Keep new and moved notes on the scale");
            }
            let selected = self.roll.as_ref().is_some_and(Roll::has_selection);
            randomize = ui
                .button("Randomize")
                .on_hover_text(if selected {
                    "New pitches in key for the selected notes"
                } else {
                    "Write a new melody in key. Press again for another; Undo brings the old notes back"
                })
                .clicked();
            ui.separator();
            let track_name = &self.session.project().tracks[at.track].name;
            ui.label(egui::RichText::new(format!("on {track_name} · {} notes · {} bars", clip.notes.len(), clip.length / f64::from(self.session.project().beats_per_bar))).color(theme::text_dim()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                close = ui.button("Close").clicked();
                ui.label(egui::RichText::new("Hints (hover)").color(theme::text_dim()).size(11.0))
                    .on_hover_text("Click an empty spot to add a note. Drag across an empty area to select notes; Shift adds to the selection.\nCtrl/Cmd-drag draws a note at the length you want. Drag notes to move them, or their right edge to resize.\nCtrl/Cmd + D copies the selected notes to straight after them. Right-click or Delete removes.\nUp/Down transposes (Shift: an octave). Alt ignores the grid.");
            });
        });
        if close {
            self.roll = None;
            return;
        }
        if randomize {
            self.randomize_notes(at, &clip);
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

        // Rows and keyboard. Notes outside the song's scale are dimmed, and its root is tinted.
        let project = self.session.project();
        let (key, scale) = (project.key % 12, music::scale_by_id(&project.scale));
        let chromatic = scale.steps.len() == 12;
        for pitch in lowest..=highest {
            let y = grid.y(pitch);
            let row =
                Rect::from_min_max(Pos2::new(rect.left(), y), Pos2::new(rect.right(), y + ROW));
            grid_painter.rect_filled(
                row,
                0.0,
                if is_black_key(pitch) {
                    theme::black_key_row()
                } else {
                    theme::white_key_row()
                },
            );
            if !chromatic {
                if !music::in_scale(pitch, key, scale) {
                    grid_painter.rect_filled(row, 0.0, theme::shade(70));
                } else if pitch % 12 == key {
                    grid_painter.rect_filled(row, 0.0, colour.gamma_multiply(0.10));
                }
            }
            if pitch % 12 == 0 {
                grid_painter.line_segment(
                    [row.left_bottom(), row.right_bottom()],
                    Stroke::new(1.0, theme::bar()),
                );
            }
            let key =
                Rect::from_min_max(Pos2::new(keys.left(), y), Pos2::new(keys.right(), y + ROW));
            let (fill, text) = if is_black_key(pitch) {
                (Color32::from_rgb(0x22, 0x24, 0x28), theme::text_dim())
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
        let per_bar = f64::from(self.session.project().beats_per_bar);
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
                theme::bar().gamma_multiply(1.6)
            } else if beat.fract() == 0.0 {
                theme::bar()
            } else {
                theme::grid()
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
                theme::shade(140),
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
                Stroke::new(1.5, theme::selection())
            } else {
                Stroke::new(1.0, colour.gamma_multiply(0.5))
            };
            grid_painter.rect_stroke(r, CornerRadius::same(2), outline, StrokeKind::Inside);
        }

        // The selection box.
        if let Some(band) = self.roll.as_ref().and_then(|r| r.band) {
            grid_painter.rect_filled(band, 0.0, theme::selection().gamma_multiply(0.08));
            grid_painter.rect_stroke(
                band,
                0.0,
                Stroke::new(1.0, theme::selection().gamma_multiply(0.7)),
                StrokeKind::Inside,
            );
        }

        // Playhead, if it's inside this clip.
        let position = self.audio.controller.position() - clip.start;
        if (0.0..=clip.length).contains(&position) {
            let x = grid.x(position);
            grid_painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.5, theme::playhead()),
            );
        }
        painter.line_segment(
            [keys.right_top(), keys.right_bottom()],
            Stroke::new(1.0, theme::bg()),
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
        let (shift, alt, command) =
            ctx.input(|i| (i.modifiers.shift, i.modifiers.alt, i.modifiers.command));
        let project = self.session.project();
        let (key, scale) = (project.key % 12, music::scale_by_id(&project.scale));
        let in_key = self.roll.as_ref().is_some_and(|r| r.in_key);
        let fit = move |pitch: u8| {
            if in_key {
                music::snap_to_scale(pitch, key, scale)
            } else {
                pitch
            }
        };
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
        // spot, select the notes inside a box, or with Ctrl/Cmd draw a new note whose length
        // follows the pointer.
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
                None if command => {
                    let note = Note {
                        pitch: fit(grid.pitch(p.y).floor().clamp(0.0, 127.0) as u8),
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
                None => {
                    let roll = self.roll.as_mut().expect("checked above");
                    if !shift {
                        roll.selected.clear();
                    }
                    DragKind::Select
                }
            };
            let notes = &self.session.project().tracks[at.track].clips[at.clip].notes;
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
                origin: p,
                before: roll.selected.clone(),
            });
        }

        // Continue or finish a drag.
        let dragging = self.roll.as_ref().and_then(|r| r.drag.as_ref()).is_some();
        if dragging {
            let (down, pointer) = ctx.input(|i| (i.pointer.primary_down(), i.pointer.latest_pos()));
            match (down, pointer) {
                (true, Some(p))
                    if self
                        .roll
                        .as_ref()
                        .and_then(|r| r.drag.as_ref())
                        .is_some_and(|d| d.kind == DragKind::Select) =>
                {
                    let roll = self.roll.as_mut().expect("checked above");
                    let drag = roll.drag.as_ref().expect("checked above");
                    let band = Rect::from_two_pos(drag.origin, p).intersect(grid.rect);
                    let mut selected = drag.before.clone();
                    selected.extend(
                        clip.notes
                            .iter()
                            .enumerate()
                            .filter(|(_, n)| grid.note_rect(n, clip.length).intersects(band))
                            .map(|(i, _)| i),
                    );
                    roll.selected = selected;
                    roll.band = Some(band);
                }
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
                    // Moving in key: follow the note that was grabbed, a scale note at a time.
                    let d_pitch = if in_key && kind == DragKind::Move {
                        originals.first().map_or(d_pitch, |(_, grabbed)| {
                            let target = (i32::from(grabbed.pitch) + d_pitch).clamp(0, 127) as u8;
                            i32::from(music::snap_to_scale(target, key, scale))
                                - i32::from(grabbed.pitch)
                        })
                    } else {
                        d_pitch
                    };
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
                                DragKind::Select => {}
                            }
                        }
                    });
                }
                _ => {
                    let roll = self.roll.as_mut().expect("checked above");
                    roll.band = None;
                    if let Some(drag) = roll.drag.take()
                        && drag.kind == DragKind::Resize
                        && let Some(&(index, _)) = drag.originals.first()
                        && let Some(note) = self.session.project().tracks[at.track].clips[at.clip]
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
                        pitch: fit(grid.pitch(p.y).floor().clamp(0.0, 127.0) as u8),
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
        let indices = self.existing_notes(at, &selected);
        if !indices.is_empty() {
            let result = self.session.remove_notes(at, &indices);
            self.finish(result);
        }
    }

    /// Ctrl/Cmd + D: copies the selected notes to straight after them and selects the copies,
    /// so pressing it again carries on.
    pub(super) fn duplicate_selected_notes(&mut self) {
        let Some(roll) = &self.roll else { return };
        let at = roll.clip;
        let indices = self.existing_notes(at, &roll.selected);
        if indices.is_empty() {
            return;
        }
        let grid = self.snap_beats();
        let result = self.session.duplicate_notes(at, &indices, grid);
        if let Some(copies) = self.done(result) {
            self.status = if copies.len() < indices.len() {
                "Some copies didn't fit before the next clip.".into()
            } else {
                format!("Duplicated {} notes.", copies.len())
            };
            if let Some(roll) = &mut self.roll
                && !copies.is_empty()
            {
                roll.select(copies);
            }
        }
    }

    /// The Randomize button: new pitches in key for the selected notes, or with nothing
    /// selected a whole new melody in key for the clip.
    fn randomize_notes(&mut self, at: ClipRef, clip: &Clip) {
        let project = self.session.project();
        let (key, scale) = (project.key % 12, music::scale_by_id(&project.scale));
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64);
        let selected = self
            .roll
            .as_ref()
            .map(|r| self.existing_notes(at, &r.selected))
            .unwrap_or_default();
        let mut notes = clip.notes.clone();
        if selected.is_empty() {
            // Stay where the clip's notes already are (a bass line stays low).
            let mut pitches: Vec<u8> = notes.iter().map(|n| n.pitch).collect();
            pitches.sort_unstable();
            let centre = pitches.get(pitches.len() / 2).copied().unwrap_or(60);
            let step = self.snap_beats();
            notes = music::melody(
                seed,
                MelodySettings {
                    key,
                    scale,
                    length: clip.length,
                    step: if (1.0 / 32.0..=1.0).contains(&step) {
                        step
                    } else {
                        0.25
                    },
                    centre,
                },
            );
        } else {
            let mut chosen: Vec<Note> = selected.iter().map(|&i| notes[i]).collect();
            music::repitch(seed, &mut chosen, key, scale);
            for (&i, note) in selected.iter().zip(chosen) {
                notes[i] = note;
            }
        }
        let count = notes.len();
        let result = self.session.set_notes(at, notes);
        if self.done(result.map(|c| ((), c))).is_some() {
            self.status = if selected.is_empty() {
                format!(
                    "A new melody in {} ({count} notes). Randomize again for another.",
                    music::key_name(key, scale.id)
                )
            } else {
                format!("New pitches in {}.", music::key_name(key, scale.id))
            };
            if selected.is_empty()
                && let Some(roll) = &mut self.roll
            {
                roll.clear_selection();
            }
        }
    }

    /// The song's key and scale, as two lists. `salt` keeps the two places it's shown apart.
    pub(super) fn key_picker(&mut self, ui: &mut egui::Ui, salt: &str) {
        let project = self.session.project();
        let (mut key, mut scale) = (project.key % 12, music::scale_by_id(&project.scale));
        let before = (key, scale);
        ui.label("Key");
        egui::ComboBox::from_id_salt((salt, "key"))
            .selected_text(music::KEYS[usize::from(key)])
            .width(44.0)
            .show_ui(ui, |ui| {
                for (i, name) in music::KEYS.iter().enumerate() {
                    ui.selectable_value(&mut key, i as u8, *name);
                }
            });
        egui::ComboBox::from_id_salt((salt, "scale"))
            .selected_text(scale.name)
            .width(118.0)
            .show_ui(ui, |ui| {
                for s in music::SCALES {
                    ui.selectable_value(&mut scale, s, s.name);
                }
            });
        if (key, scale) != before {
            let result = self.session.set_key(key, scale.id);
            self.finish(result);
            self.status = format!("The song is in {}.", music::key_name(key, scale.id));
        }
    }

    pub(super) fn transpose_selected_notes(&mut self, semitones: i32) {
        let Some(roll) = &self.roll else { return };
        let at = roll.clip;
        let indices = self.existing_notes(at, &roll.selected);
        if !indices.is_empty() {
            let result = self.session.transpose_notes(at, Some(&indices), semitones);
            self.finish(result);
        }
    }

    /// The selected notes that still exist.
    fn existing_notes(&self, at: ClipRef, selected: &BTreeSet<usize>) -> Vec<usize> {
        let count = self.clip(at).map_or(0, |c| c.notes.len());
        selected.iter().copied().filter(|&i| i < count).collect()
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
