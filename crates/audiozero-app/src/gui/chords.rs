//! The chord pads and the chord pack in the window: pads that play the key's chords, a library
//! of progressions by genre, random progressions in key, and bass lines under them.
//!
//! Chords are ordinary notes, so what you play, what's generated and what you edit in the piano
//! roll are all the same thing, and they play on any instrument. The chord pads instrument
//! only brings the sounds and the settings (voicing, how many notes a pad adds, a bass note and
//! the register) that the pads and generators share.

use super::{App, theme};
use audiozero_engine::Device;
use audiozero_engine::chords::{
    BassStyle, Chord, Extension, Numeral, Pattern, Voicing, diatonic_chord, voice,
};
use audiozero_engine::drums::KITS;
use audiozero_engine::music;
use audiozero_engine::progressions;
use audiozero_engine::session::{ChordRequest, ChordSource, ClipRef};
use eframe::egui;

/// The chord pack's settings and what's held, kept while the app runs.
pub(super) struct ChordUi {
    /// How many times a progression plays when it's added.
    pub repeats: usize,
    /// For random progressions: how many chords, how many bars, and how spicy (0 to 3).
    pub chords: usize,
    pub bars: usize,
    pub spice: u8,
    /// How chords are played in a new clip; `None` is the genre's usual pattern.
    pub pattern: Option<Pattern>,
    /// Offer progressions that don't match the song's major or minor too.
    pub show_all: bool,
    seed: u64,
    pub bass: BassStyle,
    /// The genre of the last progression added, whose sound and bass the next bass line follows.
    pub genre: Option<String>,
    /// The notes of the last chord played from a pad, which Smooth voicing stays close to.
    pub last: Vec<u8>,
    /// The pad held down with the mouse, and the notes it started.
    held: Option<(usize, Vec<u8>)>,
}

impl Default for ChordUi {
    fn default() -> Self {
        // A different first seed each run, so "Random" isn't the same song every time.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64);
        Self {
            repeats: 2,
            chords: 4,
            bars: 4,
            spice: 1,
            pattern: None,
            show_all: false,
            seed,
            bass: BassStyle::Root,
            genre: None,
            last: Vec::new(),
            held: None,
        }
    }
}

impl ChordUi {
    /// A new seed for each random progression, remembered in the status line.
    fn fresh_seed(&mut self) -> u64 {
        self.seed = self
            .seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.seed >> 20
    }
}

/// The pads' keys on the computer keyboard are the seven white keys from A, and K for the
/// first chord an octave up.
pub(super) const PAD_KEYS: usize = 8;

impl App {
    /// The chord pad `degree` (0 to 6, or 7 for the first chord an octave up) of the chord pads
    /// track `track`, with its notes. Uses the track's pad chords, voicing, bass and register.
    pub(super) fn chord_pad(&mut self, track: usize, degree: usize) -> Option<(Chord, Vec<u8>)> {
        let project = self.session.project();
        let device = project.tracks.get(track)?.instrument.as_ref()?;
        let (key, scale) = (project.key % 12, music::scale_by_id(&project.scale));
        let setting = |name: &str, default: f32| device.param(name).unwrap_or(default);
        let extension = Extension::ALL[setting("extension", 0.0).round().clamp(0.0, 2.0) as usize];
        let voicing = Voicing::ALL[setting("voicing", 1.0).round().clamp(0.0, 2.0) as usize];
        let register = setting("register", 60.0).round().clamp(48.0, 72.0) as u8;
        let bass = setting("bass", 0.0) >= 0.5;
        let chord = diatonic_chord(key, scale, degree % 7, extension);
        let up = if degree >= 7 { 12 } else { 0 };
        let previous = Some(&self.chords.last).filter(|p| !p.is_empty()).cloned();
        let notes: Vec<u8> = voice(chord, voicing, previous.as_deref(), register, bass)
            .into_iter()
            .map(|n| n.saturating_add(up).min(127))
            .collect();
        self.chords.last = notes.clone();
        Some((chord, notes))
    }

    /// Whether `track` plays the chord pads.
    pub(super) fn is_chord_track(&self, track: usize) -> bool {
        self.session
            .project()
            .tracks
            .get(track)
            .and_then(|t| t.instrument.as_ref())
            .and_then(Device::builtin_id)
            == Some("chords")
    }

    /// The progression menu: how many times to play it, then the library by genre and tag. Only
    /// progressions that suit the song's major or minor are listed, unless asked for them all.
    pub(super) fn chords_menu(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        mut chosen: impl FnMut(String),
    ) {
        let project = self.session.project();
        let scale = music::scale_by_id(&project.scale);
        let key_name = music::key_name(project.key, &project.scale);
        let bar = f64::from(project.beats_per_bar);
        ui.menu_button(label, |ui| {
            ui.horizontal(|ui| {
                ui.label("Plays");
                for repeats in [1, 2, 4, 8] {
                    ui.selectable_value(&mut self.chords.repeats, repeats, format!("{repeats}x"));
                }
            });
            ui.checkbox(&mut self.chords.show_all, "Show major and minor")
                .on_hover_text(format!(
                    "The song is in {key_name}. Progressions of the other mood are borrowed from \
                     the parallel key."
                ));
            ui.separator();
            for kit in &KITS {
                ui.menu_button(kit.name, |ui| {
                    for tag in progressions::tags_for(kit.id) {
                        let show_all = self.chords.show_all;
                        let list: Vec<_> = progressions::for_genre(kit.id)
                            .filter(|p| p.tag == tag && (show_all || progressions::suits(p, scale)))
                            .collect();
                        if list.is_empty() {
                            continue;
                        }
                        ui.menu_button(format!("{tag}  ({})", list.len()), |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(380.0)
                                .show(ui, |ui| {
                                    for p in list {
                                        let label = format!(
                                            "{}  ·  {} bars",
                                            p.name,
                                            (p.beats() / bar).ceil()
                                        );
                                        if ui.button(label).clicked() {
                                            chosen(p.id.clone());
                                            ui.close();
                                        }
                                    }
                                });
                        });
                    }
                });
            }
        })
        .response
        .on_hover_text("100 ready-made progressions in each genre, in the song's key");
    }

    /// Adds chords on `track` (or a new chord track) from the library or at random, and selects
    /// the clip.
    pub(super) fn insert_chords(&mut self, track: Option<usize>, source: ChordSource) {
        let genre = match &source {
            ChordSource::Library(id) => progressions::by_id(id).map(|p| p.genre.clone()),
            ChordSource::Random { genre, .. } => genre.clone(),
        };
        let random_seed = match &source {
            ChordSource::Random { seed, .. } => Some(*seed),
            ChordSource::Library(_) => None,
        };
        let request = ChordRequest {
            source,
            repeats: self.chords.repeats.max(1),
            start: None,
            pattern: self.chords.pattern,
            seed: self.chords.fresh_seed(),
        };
        let result = self.session.add_chords(track, request);
        if let Some((at, name)) = self.done(result) {
            self.selected_clip = Some(at);
            self.device_track = Some(at.track);
            self.chords.genre = genre.or(self.chords.genre.take());
            self.status = match random_seed {
                Some(seed) => format!(
                    "Added {} (random, seed {seed}). Press Space to play; click again for another.",
                    name
                ),
                None => format!(
                    "Added {name}. Press Space to play, or double-click the clip to edit it."
                ),
            };
        }
    }

    /// A random progression in the song's key, with the panel's settings.
    pub(super) fn insert_random_chords(&mut self, track: Option<usize>) {
        let seed = self.chords.fresh_seed();
        let source = ChordSource::Random {
            seed,
            chords: self.chords.chords,
            bars: self.chords.bars,
            spice: self.chords.spice,
            genre: self.chords.genre.clone(),
        };
        self.insert_chords(track, source);
    }

    /// Adds a bass line under the chords in a clip, on a new track below.
    pub(super) fn add_bass(&mut self, at: ClipRef, style: BassStyle) {
        let seed = self.chords.fresh_seed();
        // In a genre's own style, use that genre's bass sound.
        let sound = self
            .chords
            .genre
            .as_deref()
            .map(progressions::genre_style)
            .filter(|g| g.bass == style)
            .map(|g| g.bass_sound);
        let result = self.session.add_bass_line(at, style, seed, sound);
        if let Some(bass) = self.done(result) {
            self.selected_clip = Some(bass);
            self.status = format!(
                "Added a {} bass line below. Press Space to play; right-click the chords for another.",
                style.name().to_lowercase()
            );
        }
    }

    /// The chord pads' panel in the device panel: progressions, random chords, bass lines and the
    /// pads.
    pub(super) fn chords_panel(&mut self, ui: &mut egui::Ui, index: usize) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let mut library = None;
            self.chords_menu(ui, "Progressions", |id| library = Some(id));
            if let Some(id) = library {
                self.insert_chords(Some(index), ChordSource::Library(id));
            }
        });
        ui.add_space(2.0);
        egui::CollapsingHeader::new("Random chords in key")
            .id_salt(("chords-random", index))
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Chords");
                    for n in [2, 3, 4, 6, 8] {
                        ui.selectable_value(&mut self.chords.chords, n, n.to_string());
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Bars");
                    for n in [2, 4, 8] {
                        ui.selectable_value(&mut self.chords.bars, n, n.to_string());
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Spice");
                    ui.add(egui::Slider::new(&mut self.chords.spice, 0..=3))
                        .on_hover_text(
                            "0 plain triads, 1 some sevenths, 2 sevenths, ninths and borrowed \
                             chords, 3 also secondary dominants",
                        );
                });
                if ui
                    .button("Random progression")
                    .on_hover_text("A new one in the song's key every click. Undo removes it.")
                    .clicked()
                {
                    self.insert_random_chords(Some(index));
                }
            });
        ui.horizontal(|ui| {
            ui.label("Play as");
            let current = self.chords.pattern.map_or("Genre's usual", Pattern::name);
            egui::ComboBox::from_id_salt(("chords-pattern", index))
                .selected_text(current)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(self.chords.pattern.is_none(), "Genre's usual")
                        .clicked()
                    {
                        self.chords.pattern = None;
                    }
                    for pattern in Pattern::ALL {
                        let on = self.chords.pattern == Some(pattern);
                        if ui.selectable_label(on, pattern.name()).clicked() {
                            self.chords.pattern = Some(pattern);
                        }
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("Bass");
            egui::ComboBox::from_id_salt(("chords-bass", index))
                .selected_text(self.chords.bass.name())
                .show_ui(ui, |ui| {
                    for style in BassStyle::ALL {
                        ui.selectable_value(&mut self.chords.bass, style, style.name());
                    }
                });
            let target = self.selected_clip.filter(|c| c.track == index).or_else(|| {
                let clips = self.session.project().tracks.get(index)?.clips.len();
                clips
                    .checked_sub(1)
                    .map(|clip| ClipRef { track: index, clip })
            });
            if ui
                .add_enabled(target.is_some(), egui::Button::new("Add bass line"))
                .on_hover_text("Writes a bass part under this clip's chords, on a new track below")
                .clicked()
                && let Some(at) = target
            {
                self.add_bass(at, self.chords.bass);
            }
        });
        self.chord_pad_grid(ui, index);
    }

    /// The seven chords of the song's key as pads to play with the mouse (the computer keyboard
    /// plays them too: A S D F G H J, and K for the first chord an octave up).
    fn chord_pad_grid(&mut self, ui: &mut egui::Ui, track: usize) {
        ui.add_space(6.0);
        let extension = self
            .session
            .project()
            .tracks
            .get(track)
            .and_then(|t| t.instrument.as_ref())
            .and_then(|d| d.param("extension"))
            .unwrap_or(0.0)
            .round() as usize;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(music::key_name(
                    self.session.project().key,
                    &self.session.project().scale,
                ))
                .strong(),
            );
            for (i, e) in Extension::ALL.iter().enumerate() {
                if ui.selectable_label(extension == i, e.name()).clicked() {
                    let result = self
                        .session
                        .set_instrument_param(track, "extension", i as f32);
                    self.finish(result);
                }
            }
        });
        let (key, scale) = (
            self.session.project().key % 12,
            music::scale_by_id(&self.session.project().scale),
        );
        let ext = Extension::ALL[extension.min(2)];
        let gap = 4.0;
        let width = ((ui.available_width() - 8.0 - 6.0 * gap) / 7.0).max(34.0);
        let size = egui::vec2(width, 44.0);
        let mut down: Option<usize> = None;
        ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
        ui.horizontal(|ui| {
            for degree in 0..7 {
                let chord = diatonic_chord(key, scale, degree, ext);
                let numeral = Numeral::new((chord.root + 12 - key) % 12, chord.quality).text();
                let lit = self.chords.held.as_ref().is_some_and(|h| h.0 == degree);
                let text = format!("{}\n{numeral}", chord.name());
                let button = egui::Button::new(egui::RichText::new(text).size(11.0))
                    .min_size(size)
                    .fill(if lit {
                        theme::accent()
                    } else {
                        theme::track(degree).gamma_multiply(0.25)
                    });
                let response = ui
                    .add(button)
                    .on_hover_text(format!("{}. Hold the mouse down to play it.", chord.name()));
                if response.is_pointer_button_down_on() {
                    down = Some(degree);
                }
            }
        });
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 4.0);
        let held = self.chords.held.as_ref().map(|h| h.0);
        if down != held {
            if let Some((_, pitches)) = self.chords.held.take() {
                for pitch in pitches {
                    self.play_note(track, pitch, 0.0, false);
                }
            }
            if let Some(degree) = down
                && let Some((_, pitches)) = self.chord_pad(track, degree)
            {
                for &pitch in &pitches {
                    self.play_note(track, pitch, 0.85, true);
                }
                self.chords.held = Some((degree, pitches));
            }
        }
        ui.label(
            egui::RichText::new("Keys A S D F G H J play the pads, K the first one an octave up.")
                .small()
                .color(theme::text_dim()),
        );
        ui.add_space(4.0);
    }
}
