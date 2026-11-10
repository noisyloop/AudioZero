//! The arrangement view: tracks, clips, ruler and playhead, and clip editing.
//!
//! - Click a clip to select it, double-click to edit its notes, right-click for more.
//! - Drag a clip to move it (also onto another track), drag its right edge to resize it.
//! - Double-click an empty lane to add a one-bar clip.
//! - Hold Alt while dragging to ignore the grid.

use super::{App, ClipRef, meter, theme};
use audiozero_engine::sample::{PEAK_FRAMES, Sample};
use audiozero_engine::{AudioRegion, Clip, Project, synth::BUILTINS};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, StrokeKind,
    Vec2,
};

pub const HEADER_WIDTH: f32 = 210.0;
pub const RULER_HEIGHT: f32 = 26.0;
pub const TRACK_HEIGHT: f32 = 68.0;
/// How close to a clip's right edge counts as grabbing the edge.
const EDGE_GRAB: f32 = 6.0;

/// A clip drag in progress. Kept across frames because the clip's position (and so its widget)
/// changes while it moves.
pub struct Drag {
    kind: DragKind,
    at: ClipRef,
    /// Beats between the clip start and the pointer when the drag started.
    grab: f64,
}

#[derive(Clone, Copy, PartialEq)]
enum DragKind {
    Move,
    Resize,
}

enum ClipAction {
    Edit(ClipRef),
    Duplicate(ClipRef),
    ToggleMute(ClipRef),
    Delete(ClipRef),
}

enum TrackAction {
    Delete(usize),
    Add,
    SetInstrument(usize, &'static str),
}

/// Converts between beats and screen x for the current scroll and zoom.
#[derive(Clone, Copy)]
struct Timeline {
    left: f32,
    scroll: f32,
    pixels_per_beat: f32,
}

impl Timeline {
    fn x(self, beat: f64) -> f32 {
        self.left + (beat as f32 - self.scroll) * self.pixels_per_beat
    }
    fn beat(self, x: f32) -> f64 {
        f64::from(self.scroll + (x - self.left) / self.pixels_per_beat)
    }
}

impl App {
    pub(super) fn arrangement(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let area = ui.available_rect_before_wrap();
        let background = ui.allocate_rect(area, Sense::click_and_drag());
        let painter = ui.painter_at(area);
        painter.rect_filled(area, 0.0, theme::BG);

        let lanes_left = area.left() + HEADER_WIDTH;
        let ruler = Rect::from_min_max(
            Pos2::new(lanes_left, area.top()),
            Pos2::new(area.right(), area.top() + RULER_HEIGHT),
        );
        let tracks_top = ruler.bottom();
        let lane_width = (area.right() - lanes_left).max(1.0);
        let content_height = (self.session.project().tracks.len() as f32 + 1.0) * TRACK_HEIGHT;
        let visible_height = (area.bottom() - tracks_top).max(1.0);
        let lanes_rect = Rect::from_min_max(Pos2::new(lanes_left, tracks_top), area.right_bottom());

        self.scroll_and_zoom(&ctx, &background, lanes_left, lane_width);
        self.view.scroll_y = self
            .view
            .scroll_y
            .clamp(0.0, (content_height - visible_height).max(0.0));
        let time = Timeline {
            left: lanes_left,
            scroll: self.view.scroll_beats,
            pixels_per_beat: self.view.pixels_per_beat,
        };
        let scroll_y = self.view.scroll_y;
        let row_top = move |index: usize| tracks_top + index as f32 * TRACK_HEIGHT - scroll_y;
        let track_at = move |y: f32| ((y - tracks_top + scroll_y) / TRACK_HEIGHT).floor();

        // Clicking or dragging on the ruler moves the playhead. (`press_origin` is only known while
        // the button is down, so a quick click is handled through `clicked`.)
        let pressed_on_ruler = ctx
            .input(|i| i.pointer.press_origin())
            .is_some_and(|o| ruler.contains(o));
        if let Some(pointer) = background.interact_pointer_pos() {
            let clicked_ruler = background.clicked() && ruler.contains(pointer);
            if clicked_ruler || (pressed_on_ruler && background.is_pointer_button_down_on()) {
                self.seek(time.beat(pointer.x.max(lanes_left)));
            }
        }

        // Empty lanes: a click deselects, a double-click adds a clip.
        if let Some(pointer) = background.interact_pointer_pos()
            && lanes_rect.contains(pointer)
        {
            let track = track_at(pointer.y);
            if background.double_clicked()
                && track >= 0.0
                && (track as usize) < self.session.project().tracks.len()
            {
                self.add_clip_at(&ctx, track as usize, time.beat(pointer.x));
            } else if background.clicked() {
                self.selected_clip = None;
            }
        }

        self.draw_grid(&painter, area, ruler, tracks_top, time, lane_width);

        // Where a dropped audio file would go, highlighted while files are dragged over.
        let tracks = self.session.project().tracks.len();
        self.drop_target = ctx
            .input(|i| i.pointer.hover_pos())
            .filter(|p| lanes_rect.contains(*p))
            .map(|p| {
                let track = track_at(p.y).max(0.0) as usize;
                (track.min(tracks), time.beat(p.x).max(0.0))
            });
        if ctx.input(|i| !i.raw.hovered_files.is_empty())
            && let Some((track, _)) = self.drop_target
        {
            let top = row_top(track);
            let lane = Rect::from_min_max(
                Pos2::new(lanes_left, top),
                Pos2::new(area.right(), top + TRACK_HEIGHT),
            )
            .intersect(lanes_rect);
            painter.rect_filled(lane, 0.0, theme::ACCENT.gamma_multiply(0.15));
            let label = if track == tracks {
                "Drop to add on a new track"
            } else {
                "Drop to add here"
            };
            painter.text(
                lane.left_center() + Vec2::new(12.0, 0.0),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(13.0),
                Color32::WHITE,
            );
        }

        // Track lanes, clips and headers.
        let lane_painter = painter.with_clip_rect(lanes_rect);
        let rows_painter = painter.with_clip_rect(Rect::from_min_max(
            Pos2::new(area.left(), tracks_top),
            area.right_bottom(),
        ));
        let mut clip_rects = Vec::new();
        let mut track_action = None;
        for index in 0..self.session.project().tracks.len() {
            let top = row_top(index);
            if top > area.bottom() || top + TRACK_HEIGHT < tracks_top {
                continue;
            }
            let row = Rect::from_min_size(
                Pos2::new(area.left(), top),
                Vec2::new(area.width(), TRACK_HEIGHT),
            );
            let lane = Rect::from_min_max(Pos2::new(lanes_left, row.top()), row.right_bottom());
            let header = Rect::from_min_max(row.left_top(), Pos2::new(lanes_left, row.bottom()));
            let colour = theme::track(index);
            let lane_fill = if index % 2 == 0 {
                theme::LANE
            } else {
                theme::LANE_ALT
            };
            rows_painter.rect_filled(
                lane.intersect(lanes_rect),
                0.0,
                lane_fill.gamma_multiply(0.9),
            );

            for (clip_index, clip) in self.session.project().tracks[index]
                .clips
                .iter()
                .enumerate()
            {
                let at = ClipRef {
                    track: index,
                    clip: clip_index,
                };
                let x0 = time.x(clip.start);
                let x1 = time.x(clip.end());
                if x1 < lanes_left || x0 > area.right() {
                    continue;
                }
                let body = Rect::from_min_max(
                    Pos2::new(x0 + 1.0, lane.top() + 3.0),
                    Pos2::new(x1 - 1.0, lane.bottom() - 3.0),
                );
                let selected = self.selected_clip == Some(at);
                let wave = clip
                    .audio
                    .as_ref()
                    .map(|a| (a, self.audio.controller.samples().get(&a.file).ok()));
                let bpm = self.session.project().bpm;
                draw_clip(&lane_painter, lanes_rect, body, clip, colour, selected);
                if let Some((region, sample)) = wave {
                    draw_wave(
                        &lane_painter,
                        lanes_rect,
                        body,
                        clip,
                        region,
                        sample.as_deref(),
                        bpm,
                    );
                }
                let visible = body.intersect(lanes_rect);
                if visible.width() > 0.0 && visible.height() > 0.0 {
                    clip_rects.push((at, visible, body));
                }
            }

            // Header: right-click menu (registered first, so the widgets on top keep their clicks),
            // then name, mute/solo, volume and meter.
            rows_painter.rect_filled(header, 0.0, theme::PANEL);
            rows_painter.rect_filled(
                Rect::from_min_size(header.left_top(), Vec2::new(4.0, header.height())),
                0.0,
                colour,
            );
            rows_painter.line_segment(
                [row.left_bottom(), row.right_bottom()],
                Stroke::new(1.0, theme::BG),
            );
            let header_visible = header.intersect(lanes_rect.with_min_x(area.left()));
            if header_visible.height() > 0.0 {
                ui.interact(
                    header_visible,
                    ui.scope_id().with(("track-header", index)),
                    Sense::click(),
                )
                .context_menu(|ui| {
                    ui.label("Instrument");
                    for (id, name) in BUILTINS {
                        if ui.button(name).clicked() {
                            track_action = Some(TrackAction::SetInstrument(index, id));
                        }
                    }
                    ui.separator();
                    if ui.button("Add track").clicked() {
                        track_action = Some(TrackAction::Add);
                    }
                    if ui.button("Delete track").clicked() {
                        track_action = Some(TrackAction::Delete(index));
                    }
                });
            }
            let inner = header
                .shrink2(Vec2::new(10.0, 6.0))
                .translate(Vec2::new(3.0, 0.0));
            if inner.top() < tracks_top {
                continue; // partly scrolled under the ruler; skip the widgets
            }
            self.track_header(ui, index, inner);
        }

        // "+ Track" under the last track.
        let add_top = row_top(self.session.project().tracks.len());
        if add_top + 30.0 <= area.bottom() && add_top >= tracks_top {
            let add = Rect::from_min_size(
                Pos2::new(area.left() + 10.0, add_top + 8.0),
                Vec2::new(110.0, 24.0),
            );
            if ui.put(add, egui::Button::new("+ Add track")).clicked() {
                track_action = Some(TrackAction::Add);
            }
        }
        match track_action {
            Some(TrackAction::Add) => self.add_track(),
            Some(TrackAction::Delete(index)) => self.delete_track(index),
            Some(TrackAction::SetInstrument(index, id)) => self.set_instrument(index, id),
            None => {}
        }

        if self.session.project().tracks.is_empty() {
            painter.text(
                lanes_rect.center(),
                Align2::CENTER_CENTER,
                "Empty project. Add a track to start.",
                FontId::proportional(14.0),
                theme::TEXT_DIM,
            );
        }

        self.clip_interactions(ui, &ctx, clip_rects, time);
        self.continue_drag(&ctx, time, track_at);

        // Playhead on top.
        let x = time.x(self.audio.controller.position());
        if x >= lanes_left {
            painter
                .with_clip_rect(Rect::from_min_max(
                    Pos2::new(lanes_left, area.top()),
                    area.right_bottom(),
                ))
                .line_segment(
                    [Pos2::new(x, area.top()), Pos2::new(x, area.bottom())],
                    Stroke::new(1.5, theme::PLAYHEAD),
                );
        }
        painter.line_segment(
            [
                Pos2::new(lanes_left, area.top()),
                Pos2::new(lanes_left, area.bottom()),
            ],
            Stroke::new(1.0, theme::BG),
        );
    }

    fn scroll_and_zoom(
        &mut self,
        ctx: &egui::Context,
        background: &egui::Response,
        lanes_left: f32,
        lane_width: f32,
    ) {
        // Zoom with Ctrl/Cmd + wheel or pinch, scroll with the wheel (Shift for sideways).
        if background.hovered() {
            let (zoom, scroll) = ctx.input(|i| (i.zoom_delta(), i.smooth_scroll_delta));
            if zoom != 1.0 {
                let anchor = ctx
                    .input(|i| i.pointer.hover_pos())
                    .map_or(lanes_left, |p| p.x.max(lanes_left));
                let beat_at =
                    self.view.scroll_beats + (anchor - lanes_left) / self.view.pixels_per_beat;
                self.view.pixels_per_beat = (self.view.pixels_per_beat * zoom).clamp(4.0, 400.0);
                self.view.scroll_beats =
                    beat_at - (anchor - lanes_left) / self.view.pixels_per_beat;
            }
            self.view.scroll_beats -= scroll.x / self.view.pixels_per_beat;
            self.view.scroll_y -= scroll.y;
            if scroll.x != 0.0 {
                self.view.follow = false;
            }
        }
        let position = self.audio.controller.position() as f32;
        if self.view.follow && self.audio.controller.is_playing() && self.arrangement_drag.is_none()
        {
            let visible_beats = lane_width / self.view.pixels_per_beat;
            if position > self.view.scroll_beats + visible_beats * 0.9
                || position < self.view.scroll_beats
            {
                self.view.scroll_beats = position - visible_beats * 0.1;
            }
        }
        self.view.scroll_beats = self.view.scroll_beats.max(0.0);
    }

    fn draw_grid(
        &self,
        painter: &egui::Painter,
        area: Rect,
        ruler: Rect,
        tracks_top: f32,
        time: Timeline,
        lane_width: f32,
    ) {
        let ppb = time.pixels_per_beat;
        let per_bar = self.session.project().beats_per_bar.max(1) as f32;
        let first_beat = time.scroll.floor() as i64;
        let last_beat = (time.scroll + lane_width / ppb).ceil() as i64;
        let label_every = ((48.0 / (ppb * per_bar)).ceil() as i64).max(1);
        let lane_painter = painter.with_clip_rect(Rect::from_min_max(
            Pos2::new(time.left, area.top()),
            area.right_bottom(),
        ));

        if self.session.project().loop_enabled {
            let x0 = time.x(self.session.project().loop_start);
            let x1 = time.x(self.session.project().loop_end);
            lane_painter.rect_filled(
                Rect::from_min_max(Pos2::new(x0, ruler.top()), Pos2::new(x1, area.bottom())),
                0.0,
                theme::LOOP,
            );
            lane_painter.rect_filled(
                Rect::from_min_max(Pos2::new(x0, ruler.top()), Pos2::new(x1, ruler.top() + 4.0)),
                0.0,
                theme::ACCENT,
            );
        }
        for beat in first_beat.max(0)..=last_beat {
            let x = time.x(beat as f64);
            let is_bar = beat as f32 % per_bar == 0.0;
            if ppb >= 8.0 || is_bar {
                let colour = if is_bar { theme::BAR } else { theme::GRID };
                lane_painter.line_segment(
                    [Pos2::new(x, tracks_top), Pos2::new(x, area.bottom())],
                    Stroke::new(1.0, colour),
                );
            }
            let bar_index = (beat as f32 / per_bar) as i64;
            if is_bar && bar_index % label_every == 0 {
                lane_painter.line_segment(
                    [
                        Pos2::new(x, ruler.bottom() - 8.0),
                        Pos2::new(x, ruler.bottom()),
                    ],
                    Stroke::new(1.0, theme::TEXT_DIM),
                );
                lane_painter.text(
                    Pos2::new(x + 3.0, ruler.center().y),
                    Align2::LEFT_CENTER,
                    bar_index + 1,
                    FontId::proportional(11.0),
                    theme::TEXT_DIM,
                );
            }
        }
        painter.line_segment(
            [ruler.left_bottom(), ruler.right_bottom()],
            Stroke::new(1.0, theme::BAR),
        );
    }

    fn track_header(&mut self, ui: &mut egui::Ui, index: usize, inner: Rect) {
        let mut track = self.session.project().tracks[index].clone();
        let mut changed = false;
        let mut arm_clicked = false;
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(inner)
                .id_salt(("track", index)),
            |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&track.name).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        changed |= ui
                            .toggle_value(&mut track.solo, "S")
                            .on_hover_text("Solo")
                            .changed();
                        changed |= ui
                            .toggle_value(&mut track.mute, "M")
                            .on_hover_text("Mute")
                            .changed();
                        let armed = self.armed == Some(index);
                        let hint = if track.instrument.is_some() {
                            "Arm to record MIDI (and play it from a MIDI keyboard or A–K)"
                        } else {
                            "Arm to record audio from the input"
                        };
                        arm_clicked |= super::record_dot(ui, armed).on_hover_text(hint).clicked();
                    });
                });
                ui.horizontal(|ui| {
                    changed |= ui
                        .add(egui::Slider::new(&mut track.volume, 0.0..=2.0).show_value(false))
                        .on_hover_text(format!("Volume {:.0}%", track.volume * 100.0))
                        .changed();
                    let (l, r) = self.audio.controller.track_peak(index);
                    meter(ui, l, r, Vec2::new(ui.available_width(), 10.0));
                });
            },
        );
        if changed {
            self.set_without_undo(|p| p.tracks[index] = track);
        }
        if arm_clicked {
            self.toggle_arm(index);
        }
    }

    fn add_clip_at(&mut self, ctx: &egui::Context, track: usize, beat: f64) {
        let per_bar = f64::from(self.session.project().beats_per_bar);
        let start = if self.snap_beats() > 0.0 && !ctx.input(|i| i.modifiers.alt) {
            (beat / per_bar).floor() * per_bar
        } else {
            beat.max(0.0)
        };
        let mut new_index = 0;
        self.edit(|p: &mut Project| {
            let clips = &mut p.tracks[track].clips;
            clips.push(Clip {
                name: "Clip".into(),
                start,
                length: per_bar,
                ..Default::default()
            });
            new_index = clips.len() - 1;
        });
        let at = ClipRef {
            track,
            clip: new_index,
        };
        if self.clip(at).is_some() {
            self.selected_clip = Some(at);
            self.status = "Clip added. Double-click it to edit its notes.".into();
        }
    }

    fn clip_interactions(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        clips: Vec<(ClipRef, Rect, Rect)>,
        time: Timeline,
    ) {
        let mut action = None;
        for (at, visible, body) in clips {
            let response = ui.interact(
                visible,
                ui.scope_id().with(("clip", at.track, at.clip)),
                Sense::click_and_drag(),
            );
            let near_edge = response
                .hover_pos()
                .is_some_and(|p| (body.right() - p.x).abs() <= EDGE_GRAB);
            if near_edge {
                ctx.set_cursor_icon(CursorIcon::ResizeHorizontal);
            }
            // Measure from where the button went down: egui reports a drag only after the
            // pointer has moved a few pixels, and starting from there would lose that distance.
            if response.drag_started()
                && let Some(pointer) = ctx
                    .input(|i| i.pointer.press_origin())
                    .or(response.interact_pointer_pos())
                && let Some(clip) = self.clip(at)
            {
                let grab = time.beat(pointer.x) - clip.start;
                let kind = if (body.right() - pointer.x).abs() <= EDGE_GRAB {
                    DragKind::Resize
                } else {
                    DragKind::Move
                };
                self.begin_gesture();
                self.selected_clip = Some(at);
                self.arrangement_drag = Some(Drag { kind, at, grab });
            }
            if response.clicked() {
                self.selected_clip = Some(at);
            }
            if response.double_clicked() {
                action = Some(ClipAction::Edit(at));
            }
            response.context_menu(|ui| {
                self.selected_clip = Some(at);
                let is_audio = self.clip(at).is_some_and(Clip::is_audio);
                if !is_audio && ui.button("Edit notes").clicked() {
                    action = Some(ClipAction::Edit(at));
                }
                if ui.button("Duplicate").clicked() {
                    action = Some(ClipAction::Duplicate(at));
                }
                let muted = self.clip(at).is_some_and(|c| c.muted);
                if ui.button(if muted { "Unmute" } else { "Mute" }).clicked() {
                    action = Some(ClipAction::ToggleMute(at));
                }
                if ui.button("Delete").clicked() {
                    action = Some(ClipAction::Delete(at));
                }
            });
        }
        match action {
            Some(ClipAction::Edit(at)) => self.open_roll(at),
            Some(ClipAction::Duplicate(at)) => self.duplicate_clip(at),
            Some(ClipAction::ToggleMute(at)) => self.toggle_clip_mute(at),
            Some(ClipAction::Delete(at)) => self.delete_clip(at),
            None => {}
        }
    }

    /// Moves or resizes the dragged clip to follow the pointer, and ends the drag on release.
    fn continue_drag(
        &mut self,
        ctx: &egui::Context,
        time: Timeline,
        track_at: impl Fn(f32) -> f32,
    ) {
        let Some(drag) = &self.arrangement_drag else {
            return;
        };
        let (down, pointer) = ctx.input(|i| (i.pointer.primary_down(), i.pointer.latest_pos()));
        let (Some(pointer), true) = (pointer, down) else {
            self.arrangement_drag = None;
            self.end_gesture();
            return;
        };
        let (kind, at, grab) = (drag.kind, drag.at, drag.grab);
        let Some(clip) = self.clip(at).cloned() else {
            self.arrangement_drag = None;
            self.end_gesture();
            return;
        };
        let beat = time.beat(pointer.x);
        match kind {
            DragKind::Move => {
                ctx.set_cursor_icon(CursorIcon::Grabbing);
                let start = self.snap_to_grid(ctx, beat - grab).max(0.0);
                let last_track = self.session.project().tracks.len().saturating_sub(1);
                let target = (track_at(pointer.y).max(0.0) as usize).min(last_track);
                let mut new_at = at;
                self.live(|p| {
                    let mut moved = p.tracks[at.track].clips.remove(at.clip);
                    moved.start = start;
                    let clips = &mut p.tracks[target].clips;
                    if target == at.track {
                        clips.insert(at.clip, moved);
                    } else {
                        clips.push(moved);
                        new_at = ClipRef {
                            track: target,
                            clip: clips.len() - 1,
                        };
                    }
                });
                if self.clip(new_at).is_some() && new_at != at {
                    if let Some(roll) = &mut self.roll
                        && roll.clip == at
                    {
                        roll.clip = new_at;
                    }
                    self.selected_clip = Some(new_at);
                    if let Some(drag) = &mut self.arrangement_drag {
                        drag.at = new_at;
                    }
                }
            }
            DragKind::Resize => {
                ctx.set_cursor_icon(CursorIcon::ResizeHorizontal);
                let minimum = self.snap_beats().max(1.0 / 16.0);
                let end = self.snap_to_grid(ctx, beat).max(clip.start + minimum);
                self.live(|p| p.tracks[at.track].clips[at.clip].length = end - clip.start);
            }
        }
    }
}

/// An audio clip's waveform, or a note that its file can't be played.
fn draw_wave(
    painter: &egui::Painter,
    lanes: Rect,
    body: Rect,
    clip: &Clip,
    region: &AudioRegion,
    sample: Option<&Sample>,
    bpm: f64,
) {
    let area = Rect::from_min_max(
        Pos2::new(body.left(), body.top() + 17.0),
        Pos2::new(body.right(), body.bottom() - 2.0),
    );
    let Some(sample) = sample else {
        painter.with_clip_rect(area.intersect(lanes)).text(
            area.left_center() + Vec2::new(5.0, 0.0),
            Align2::LEFT_CENTER,
            "audio file missing",
            FontId::proportional(11.0),
            Color32::from_rgb(0xef, 0x53, 0x50),
        );
        return;
    };
    let peaks = sample.peaks();
    if peaks.is_empty() || body.width() <= 0.0 {
        return;
    }
    let visible = area.intersect(lanes);
    let mid = area.center().y;
    let half = area.height() / 2.0;
    let beat_width = f64::from(body.width()) / clip.length;
    // Peak index at a given x: beats into the clip, to seconds, to frames, to peaks.
    let peak_at = |x: f32| {
        let beats = f64::from(x - body.left()) / beat_width;
        let frames = (beats * 60.0 / bpm + region.offset) * sample.rate();
        (frames / PEAK_FRAMES as f64).max(0.0)
    };
    let gain = region.gain.max(0.0);
    let mut x = visible.left().floor();
    while x < visible.right() {
        let (i0, i1) = (
            peak_at(x) as usize,
            (peak_at(x + 1.0).ceil() as usize).max(1),
        );
        if i0 >= peaks.len() {
            break;
        }
        let (lo, hi) = peaks[i0..i1.min(peaks.len()).max(i0 + 1)]
            .iter()
            .fold((0.0f32, 0.0f32), |(a, b), &(lo, hi)| (a.min(lo), b.max(hi)));
        let (lo, hi) = ((lo * gain).max(-1.0), (hi * gain).min(1.0));
        painter.line_segment(
            [
                Pos2::new(x + 0.5, mid - hi * half),
                Pos2::new(x + 0.5, mid - lo * half + 1.0),
            ],
            Stroke::new(1.0, Color32::from_white_alpha(190)),
        );
        x += 1.0;
    }
}

fn draw_clip(
    painter: &egui::Painter,
    lanes: Rect,
    body: Rect,
    clip: &Clip,
    colour: Color32,
    selected: bool,
) {
    let fill = if clip.muted {
        colour.gamma_multiply(0.2)
    } else {
        colour.gamma_multiply(0.45)
    };
    painter.rect_filled(body, CornerRadius::same(4), fill);
    let outline = if selected {
        Stroke::new(2.0, theme::SELECTION)
    } else {
        Stroke::new(1.0, colour)
    };
    painter.rect_stroke(body, CornerRadius::same(4), outline, StrokeKind::Inside);
    let title = Rect::from_min_max(body.left_top(), Pos2::new(body.right(), body.top() + 15.0));
    let title_colour = if clip.muted {
        colour.gamma_multiply(0.4)
    } else {
        colour.gamma_multiply(0.8)
    };
    painter.rect_filled(
        title,
        CornerRadius {
            nw: 4,
            ne: 4,
            sw: 0,
            se: 0,
        },
        title_colour,
    );
    let name = if clip.muted {
        format!("{} (muted)", clip.name)
    } else {
        clip.name.clone()
    };
    painter.with_clip_rect(title.intersect(lanes)).text(
        title.left_center() + Vec2::new(5.0, 0.0),
        Align2::LEFT_CENTER,
        name,
        FontId::proportional(11.0),
        Color32::BLACK,
    );
    // A miniature of the notes.
    let (Some(lo), Some(hi)) = (
        clip.notes.iter().map(|n| n.pitch).min(),
        clip.notes.iter().map(|n| n.pitch).max(),
    ) else {
        return;
    };
    let notes_area = Rect::from_min_max(
        Pos2::new(body.left(), title.bottom() + 3.0),
        Pos2::new(body.right(), body.bottom() - 3.0),
    );
    let span = f32::from(hi - lo).max(12.0);
    let beat_width = body.width() / clip.length as f32;
    for note in &clip.notes {
        let nx0 = body.left() + note.start as f32 * beat_width;
        let nx1 = body.left() + (note.start + note.length).min(clip.length) as f32 * beat_width;
        let y = notes_area.bottom() - f32::from(note.pitch - lo) / span * notes_area.height();
        let r = Rect::from_min_max(
            Pos2::new(nx0, y - 1.5),
            Pos2::new(nx1.max(nx0 + 2.0), y + 1.0),
        );
        painter.rect_filled(r.intersect(body), 1.0, Color32::from_white_alpha(200));
    }
}
