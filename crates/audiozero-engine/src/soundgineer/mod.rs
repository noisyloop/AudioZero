//! Soundgineer: a wavetable synthesizer, ported from the TypeScript original
//! ([noisyloop/soundgineer](https://github.com/noisyloop/soundgineer)).
//!
//! Per voice (16 of them):
//!
//! - three wavetable oscillators with up to 16 unison voices each, morph, detune, blend, stereo
//!   spread, start phase, transpose, fine tune and hard sync, reading band-limited tables;
//! - a sub oscillator and a noise oscillator;
//! - two filters in series or parallel: state-variable (LP/HP/BP/notch, 12 and 24 dB), a
//!   feedback comb and a vowel formant;
//! - a distortion section (soft and hard clip, wavefolder, bit crusher);
//! - six DAHDSR envelopes (the first is the amplitude) and eight LFOs;
//!
//! then a modulation matrix (any of 24 sources to any of 112 targets, 16 slots), four macros and
//! a rack of eight effects. The parameters and the 59 factory presets come from the original's
//! source, through `scripts/soundgineer/generate.py`.
//!
//! Left out of the port, compared to the original: drawing LFO curves (every LFO is a
//! triangle), importing your own wavetables and the noise sample slot, reordering the effects,
//! the sustain pedal and the visualisers.

mod dsp;
pub(crate) mod effects;
#[rustfmt::skip]
mod data;
pub mod userpreset;
pub mod wavetable;

use crate::effects::{Kind, Param};
use crate::synth::Instrument;
use dsp::{EnvParams, Envelope, Lfo, LfoMode, VoiceFilter};
use effects::{Chorus, Compressor, Distortion, Eq3, Flanger, Phaser, Reverb, StereoDelay};
use std::f32::consts::PI;
use wavetable::Table;

pub use data::{
    FIRST_MATRIX_PARAM, FIRST_SHAPE_PARAM, MOD_DEST_LABELS, MOD_SLOTS, MOD_SOURCE_IDS, MOD_SOURCES,
    PARAM_COUNT, PARAMS, PRESET_NAMES, SECTIONS, preset,
};
pub use dsp::{FILTER_TYPES, SHAPE_POINTS, Shape};

/// The value of a modulation slot's "Target" setting that modulates parameter `param`, if
/// that parameter can be modulated.
pub fn mod_dest_for(param: usize) -> Option<usize> {
    data::MOD_DESTS
        .iter()
        .position(|&d| d == param)
        .map(|i| i + 1)
}

// The generated data and the DSP agree on how many points a curve has.
const _: () = assert!(data::SHAPE_POINTS == SHAPE_POINTS);

/// Where LFO `lfo` (0 to 7)'s curve starts in [`PARAMS`]: its point count, then (x, y, bend)
/// for each of its [`SHAPE_POINTS`] points.
pub const fn shape_param(lfo: usize) -> usize {
    FIRST_SHAPE_PARAM + lfo * (1 + 3 * SHAPE_POINTS)
}

/// Builds Soundgineer's wavetables on a background thread, so the first synth made later
/// doesn't have to wait for them. Safe to call any number of times.
pub fn warm_up() {
    let _ = std::thread::Builder::new()
        .name("soundgineer-tables".into())
        .spawn(|| {
            wavetable::tables();
        });
}

/// The preset names, for the generic preset menu.
pub static PRESETS: std::sync::LazyLock<Vec<&'static str>> =
    std::sync::LazyLock::new(|| PRESET_NAMES.iter().map(|(name, _)| *name).collect());

const VOICES: usize = 16;
const MAX_UNISON: usize = 16;
/// Frames rendered between modulation updates.
const BLOCK: usize = 128;
/// Modulation sources: 6 envelopes, 8 LFOs, velocity, key track, random, 4 macros, mod wheel,
/// pitch wheel and pressure.
const SOURCES: usize = 24;
const SRC_ENV: usize = 0;
const SRC_LFO: usize = 6;
const SRC_VELOCITY: usize = 14;
const SRC_KEYTRACK: usize = 15;
const SRC_RANDOM: usize = 16;
const SRC_MACRO: usize = 17;
const SRC_MODWHEEL: usize = 21;
const SRC_PITCHWHEEL: usize = 22;
const SRC_AFTERTOUCH: usize = 23;
/// How long the effects keep running after the last voice ends, in seconds.
const TAIL_SECONDS: f32 = 8.0;

/// How long each tempo-synced LFO/delay division lasts, in beats.
const DIVISION_BEATS: [f32; 13] = [
    4.0,
    2.0,
    4.0 / 3.0,
    1.5,
    1.0,
    2.0 / 3.0,
    0.75,
    0.5,
    1.0 / 3.0,
    0.375,
    0.25,
    1.0 / 6.0,
    0.125,
];

fn find(name: &str) -> usize {
    PARAMS
        .iter()
        .position(|p| p.name == name)
        .unwrap_or_else(|| panic!("Soundgineer has no parameter {name}"))
}

// ---------------------------------------------------------------------------- normalised values

/// 0 to 1 for a raw value, the way the original maps knobs.
fn value_to_norm(p: &Param, v: f32) -> f32 {
    let n = match p.kind {
        Kind::Choice(labels) => {
            if labels.len() > 1 {
                v / (labels.len() - 1) as f32
            } else {
                0.0
            }
        }
        Kind::Log => (v / p.min).max(1e-9).ln() / (p.max / p.min).ln(),
        _ => (v - p.min) / (p.max - p.min),
    };
    n.clamp(0.0, 1.0)
}

/// The raw value for a 0 to 1 position (which is clamped).
fn norm_to_value(p: &Param, n: f32) -> f32 {
    let n = n.clamp(0.0, 1.0);
    match p.kind {
        Kind::Choice(labels) => (n * (labels.len() - 1) as f32).round(),
        Kind::Log => p.min * (p.max / p.min).powf(n),
        Kind::Whole | Kind::Toggle => (p.min + (p.max - p.min) * n).round(),
        Kind::Linear => p.min + (p.max - p.min) * n,
    }
}

// ---------------------------------------------------------------------------- parameter indices

struct OscIdx {
    enabled: usize,
    wavetable: usize,
    morph: usize,
    level: usize,
    pan: usize,
    unison: usize,
    detune: usize,
    blend: usize,
    spread: usize,
    phase: usize,
    phase_rand: usize,
    transpose: usize,
    fine: usize,
    sync: usize,
}

struct FilterIdx {
    enabled: usize,
    kind: usize,
    cutoff: usize,
    resonance: usize,
    drive: usize,
    keytrack: usize,
    mix: usize,
}

struct LfoIdx {
    rate: usize,
    sync: usize,
    division: usize,
    mode: usize,
    phase: usize,
    smooth: usize,
}

/// Where each parameter is in [`PARAMS`], looked up once by name.
struct Idx {
    volume: usize,
    polyphony: usize,
    bend_range: usize,
    mod_wheel: usize,
    pitch_bend: usize,
    pressure: usize,
    osc: [OscIdx; 3],
    sub: [usize; 5],
    noise: [usize; 5],
    filter: [FilterIdx; 2],
    routing: usize,
    dist: [usize; 5],
    env: [[usize; 9]; 6],
    lfo: [LfoIdx; 8],
    macros: [usize; 4],
    chorus: [usize; 4],
    phaser: [usize; 5],
    flanger: [usize; 5],
    delay: [usize; 7],
    reverb: [usize; 5],
    eq: [usize; 5],
    comp: [usize; 6],
    fxdist: [usize; 4],
}

impl Idx {
    fn new() -> Self {
        let group = |prefix: &str, names: &[&str]| -> Vec<usize> {
            names
                .iter()
                .map(|n| find(&format!("{prefix}.{n}")))
                .collect()
        };
        let array = |prefix: &str, names: &[&str]| -> Vec<usize> { group(prefix, names) };
        let osc = |o: usize| {
            let g = |n: &str| find(&format!("osc{o}.{n}"));
            OscIdx {
                enabled: g("enabled"),
                wavetable: g("wavetable"),
                morph: g("morph"),
                level: g("level"),
                pan: g("pan"),
                unison: g("unison"),
                detune: g("detune"),
                blend: g("blend"),
                spread: g("spread"),
                phase: g("phase"),
                phase_rand: g("phase_rand"),
                transpose: g("transpose"),
                fine: g("fine"),
                sync: g("sync"),
            }
        };
        let filter = |f: usize| {
            let g = |n: &str| find(&format!("filter{f}.{n}"));
            FilterIdx {
                enabled: g("enabled"),
                kind: g("type"),
                cutoff: g("cutoff"),
                resonance: g("resonance"),
                drive: g("drive"),
                keytrack: g("keytrack"),
                mix: g("mix"),
            }
        };
        let lfo = |l: usize| {
            let g = |n: &str| find(&format!("lfo{l}.{n}"));
            LfoIdx {
                rate: g("rate"),
                sync: g("sync"),
                division: g("division"),
                mode: g("mode"),
                phase: g("phase"),
                smooth: g("smooth"),
            }
        };
        let env = |e: usize| -> [usize; 9] {
            array(
                &format!("env{e}"),
                &[
                    "delay",
                    "attack",
                    "hold",
                    "decay",
                    "sustain",
                    "release",
                    "atk_curve",
                    "dec_curve",
                    "rel_curve",
                ],
            )
            .try_into()
            .expect("nine envelope parameters")
        };
        macro_rules! fixed {
            ($prefix:literal, [$($name:literal),*]) => {
                array($prefix, &[$($name),*]).try_into().expect("a fixed parameter list")
            };
        }
        Self {
            volume: find("master.volume"),
            polyphony: find("master.polyphony"),
            bend_range: find("master.bend_range"),
            mod_wheel: find("modwheel.value"),
            pitch_bend: find("pitchbend.value"),
            pressure: find("aftertouch.value"),
            osc: [osc(1), osc(2), osc(3)],
            sub: fixed!("sub", ["enabled", "shape", "level", "pan", "octave"]),
            noise: fixed!("noise", ["enabled", "type", "level", "pan", "pitch"]),
            filter: [filter(1), filter(2)],
            routing: find("filter.routing"),
            dist: fixed!("dist", ["type", "drive", "mix", "bits", "downsample"]),
            env: [env(1), env(2), env(3), env(4), env(5), env(6)],
            lfo: [
                lfo(1),
                lfo(2),
                lfo(3),
                lfo(4),
                lfo(5),
                lfo(6),
                lfo(7),
                lfo(8),
            ],
            macros: [
                find("macro1.value"),
                find("macro2.value"),
                find("macro3.value"),
                find("macro4.value"),
            ],
            chorus: fixed!("chorus", ["enabled", "rate", "depth", "mix"]),
            phaser: fixed!("phaser", ["enabled", "rate", "depth", "feedback", "mix"]),
            flanger: fixed!("flanger", ["enabled", "rate", "depth", "feedback", "mix"]),
            delay: fixed!(
                "delay",
                [
                    "enabled", "sync", "division", "time", "feedback", "pingpong", "mix"
                ]
            ),
            reverb: fixed!("reverb", ["enabled", "size", "damp", "width", "mix"]),
            eq: fixed!(
                "eq",
                ["enabled", "low_gain", "mid_gain", "mid_freq", "high_gain"]
            ),
            comp: fixed!(
                "comp",
                [
                    "enabled",
                    "threshold",
                    "ratio",
                    "attack",
                    "release",
                    "makeup"
                ]
            ),
            fxdist: fixed!("fxdist", ["enabled", "drive", "tone", "mix"]),
        }
    }
}

// ---------------------------------------------------------------------------- shared state

/// One route of the mod matrix.
#[derive(Clone, Copy, Default)]
struct Slot {
    /// 1 to 24 (0 is "None": no route).
    source: usize,
    /// The parameter modulated.
    dest: usize,
    depth: f32,
}

/// Everything a voice reads while it renders a block.
struct Shared {
    sr: f32,
    values: Box<[f32; PARAM_COUNT]>,
    idx: Idx,
    tables: &'static [Table; 6],
    slots: [Slot; MOD_SLOTS],
    /// The LFOs' drawn curves.
    shapes: [Shape; 8],
    lfo_freq: [f32; 8],
    lfo_global: [f32; 8],
    lfo_beat: [f32; 8],
    bend_range: f32,
}

impl Shared {
    fn base(&self, i: usize) -> f32 {
        self.values[i]
    }

    /// Whether a switch is on.
    fn on(&self, i: usize) -> bool {
        self.values[i] >= 0.5
    }

    /// A choice's index.
    fn choice(&self, i: usize) -> usize {
        self.values[i].round().max(0.0) as usize
    }

    /// The value with `offsets` (a modulation) applied, in raw units.
    fn with(&self, i: usize, offsets: &[f32; PARAM_COUNT]) -> f32 {
        let p = &PARAMS[i];
        norm_to_value(p, value_to_norm(p, self.values[i]) + offsets[i])
    }

    fn rebuild_slots(&mut self) {
        for (n, slot) in self.slots.iter_mut().enumerate() {
            let at = FIRST_MATRIX_PARAM + n * 3;
            let source = self.values[at].round().max(0.0) as usize;
            let dest = self.values[at + 1].round().max(0.0) as usize;
            let depth = self.values[at + 2];
            *slot = if source > 0
                && dest > 0
                && depth != 0.0
                && source <= SOURCES
                && dest <= data::MOD_DESTS.len()
            {
                Slot {
                    source,
                    dest: data::MOD_DESTS[dest - 1],
                    depth,
                }
            } else {
                Slot::default()
            };
        }
    }

    /// Reads LFO `lfo`'s curve from the settings.
    fn rebuild_shape(&mut self, lfo: usize) {
        let base = shape_param(lfo);
        let mut shape = Shape {
            count: self.values[base].round().clamp(2.0, SHAPE_POINTS as f32) as usize,
            ..Shape::default()
        };
        for i in 0..SHAPE_POINTS {
            let at = base + 1 + i * 3;
            shape.x[i] = self.values[at];
            shape.y[i] = self.values[at + 1];
            shape.bend[i] = self.values[at + 2];
        }
        shape.sort();
        self.shapes[lfo] = shape;
    }

    /// Sums the matrix's routes into `offsets`, from `sources`.
    fn route(&self, sources: &[f32; SOURCES], offsets: &mut [f32; PARAM_COUNT]) {
        offsets.fill(0.0);
        for slot in self.slots.iter().filter(|s| s.source > 0) {
            offsets[slot.dest] += slot.depth * sources[slot.source - 1];
        }
    }
}

// ---------------------------------------------------------------------------- voice

struct Voice {
    note: u8,
    velocity: f32,
    gate: bool,
    age: u32,
    sources: [f32; SOURCES],
    offsets: [f32; PARAM_COUNT],
    phases: [f64; 3 * MAX_UNISON],
    sub_phase: f32,
    prev_amp: f32,
    rng: u32,
    envs: [Envelope; 6],
    lfos: [Lfo; 8],
    filters: [VoiceFilter; 2],
    buf_l: [f32; BLOCK],
    buf_r: [f32; BLOCK],
    tmp_l: [f32; BLOCK],
    tmp_r: [f32; BLOCK],
    dist_hold: [f32; 2],
    dist_count: i32,
    pink: [f32; 3],
}

fn note_to_freq(note: f32) -> f32 {
    440.0 * 2f32.powf((note - 69.0) / 12.0)
}

/// Equal-power left and right gains for a pan from -1 to 1.
fn pan_gains(pan: f32) -> (f32, f32) {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * PI / 4.0;
    (angle.cos(), angle.sin())
}

impl Voice {
    fn new(sr: f32, seed: u32) -> Self {
        Self {
            note: 60,
            velocity: 1.0,
            gate: false,
            age: 0,
            sources: [0.0; SOURCES],
            offsets: [0.0; PARAM_COUNT],
            phases: [0.0; 3 * MAX_UNISON],
            sub_phase: 0.0,
            prev_amp: 0.0,
            rng: seed | 1,
            envs: [Envelope::default(); 6],
            lfos: [Lfo::default(); 8],
            filters: [VoiceFilter::new(sr), VoiceFilter::new(sr)],
            buf_l: [0.0; BLOCK],
            buf_r: [0.0; BLOCK],
            tmp_l: [0.0; BLOCK],
            tmp_r: [0.0; BLOCK],
            dist_hold: [0.0; 2],
            dist_count: 0,
            pink: [0.0; 3],
        }
    }

    fn active(&self) -> bool {
        !self.envs[0].idle()
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / 4_294_967_296.0
    }

    fn note_on(&mut self, sh: &Shared, note: u8, velocity: f32, age: u32) {
        self.note = note;
        self.velocity = velocity;
        self.gate = true;
        self.age = age;
        self.sub_phase = 0.0;
        self.prev_amp = 0.0;
        self.dist_hold = [0.0; 2];
        self.dist_count = 0;
        let random = self.random();
        for o in 0..3 {
            let start = sh.base(sh.idx.osc[o].phase);
            let rand = sh.base(sh.idx.osc[o].phase_rand);
            for u in 0..MAX_UNISON {
                let r = self.random();
                self.phases[o * MAX_UNISON + u] = f64::from((start + rand * r).rem_euclid(1.0));
            }
        }
        self.envs.iter_mut().for_each(Envelope::trigger);
        for (l, lfo) in self.lfos.iter_mut().enumerate() {
            lfo.trigger(sh.base(sh.idx.lfo[l].phase));
        }
        self.filters.iter_mut().for_each(VoiceFilter::reset);
        self.offsets.fill(0.0);
        self.sources = [0.0; SOURCES];
        self.sources[SRC_VELOCITY] = velocity;
        self.sources[SRC_KEYTRACK] = ((f32::from(note) - 60.0) / 36.0).clamp(-1.0, 1.0);
        self.sources[SRC_RANDOM] = random;
    }

    fn note_off(&mut self) {
        self.gate = false;
        self.envs.iter_mut().for_each(Envelope::gate_off);
    }

    fn kill(&mut self) {
        self.envs.iter_mut().for_each(Envelope::kill);
    }

    fn compute_mods(&mut self, sh: &Shared) {
        for m in 0..4 {
            self.sources[SRC_MACRO + m] = sh.base(sh.idx.macros[m]);
        }
        self.sources[SRC_MODWHEEL] = sh.base(sh.idx.mod_wheel);
        self.sources[SRC_PITCHWHEEL] = sh.base(sh.idx.pitch_bend);
        self.sources[SRC_AFTERTOUCH] = sh.base(sh.idx.pressure);
        sh.route(&self.sources, &mut self.offsets);
    }

    fn advance_modulators(&mut self, sh: &Shared, dt: f32) {
        for e in 0..6 {
            let ix = &sh.idx.env[e];
            let v = |i: usize| sh.with(ix[i], &self.offsets);
            let params = EnvParams {
                delay: v(0),
                attack: v(1),
                hold: v(2),
                decay: v(3),
                sustain: v(4),
                release: v(5),
                atk_curve: v(6),
                dec_curve: v(7),
                rel_curve: v(8),
            };
            self.sources[SRC_ENV + e] = self.envs[e].process(&params, dt);
        }
        for l in 0..8 {
            let ix = &sh.idx.lfo[l];
            let mode = match sh.choice(ix.mode) {
                1 => LfoMode::Free,
                2 => LfoMode::Sync,
                _ => LfoMode::Trigger,
            };
            self.sources[SRC_LFO + l] = self.lfos[l].process(
                &sh.shapes[l],
                sh.lfo_freq[l],
                mode,
                sh.lfo_global[l],
                sh.lfo_beat[l],
                sh.base(ix.smooth),
                dt,
            );
        }
    }

    /// Renders one block (up to [`BLOCK`] frames) and adds it to `out_l` and `out_r`. Returns
    /// whether the voice is still sounding.
    fn render(&mut self, sh: &Shared, out_l: &mut [f32], out_r: &mut [f32]) -> bool {
        let n = out_l.len().min(out_r.len()).min(BLOCK);
        let dt = n as f32 / sh.sr;
        self.compute_mods(sh);
        self.advance_modulators(sh, dt);
        if !self.active() {
            return false;
        }
        let idx = &sh.idx;
        self.buf_l[..n].fill(0.0);
        self.buf_r[..n].fill(0.0);

        let bend = sh.base(idx.pitch_bend) * sh.bend_range;
        let base_freq = note_to_freq(f32::from(self.note) + bend);

        // ---- the three wavetable oscillators
        for o in 0..3 {
            let ix = &idx.osc[o];
            if !sh.on(ix.enabled) {
                continue;
            }
            let table = &sh.tables[sh.choice(ix.wavetable).min(5)];
            let morph = (sh.base(ix.morph) + self.offsets[ix.morph]).clamp(0.0, 1.0);
            let level = sh.with(ix.level, &self.offsets);
            if level <= 1e-4 {
                continue;
            }
            let pan = sh.with(ix.pan, &self.offsets);
            let unison = (sh.with(ix.unison, &self.offsets).round() as usize).clamp(1, MAX_UNISON);
            let detune = sh.with(ix.detune, &self.offsets);
            let blend = sh.with(ix.blend, &self.offsets);
            let spread = sh.with(ix.spread, &self.offsets);
            let transpose = sh.with(ix.transpose, &self.offsets);
            let fine = sh.with(ix.fine, &self.offsets);
            let sync = sh.with(ix.sync, &self.offsets);
            let freq = base_freq * 2f32.powf((transpose + fine / 100.0) / 12.0);

            // Gains and pans per unison voice, normalised to constant power.
            let offset_of = |u: usize| {
                if unison == 1 {
                    0.0
                } else {
                    2.0 * u as f32 / (unison - 1) as f32 - 1.0
                }
            };
            let weight_of = |u: usize| (1.0 - blend) * (1.0 - offset_of(u).abs()) + blend;
            let norm = 1.0
                / (0..unison)
                    .map(|u| weight_of(u).powi(2))
                    .sum::<f32>()
                    .max(1e-9)
                    .sqrt();
            for u in 0..unison {
                let off = offset_of(u);
                let inc = freq * 2f32.powf(detune * off / 1200.0) / sh.sr;
                if inc >= 0.5 {
                    continue;
                }
                let w = weight_of(u) * norm * level;
                let (gl, gr) = pan_gains(pan + spread * off * 0.9);
                let (gl, gr) = (w * gl, w * gr);
                let mip = table.mip_for(inc * sync);
                let mut phase = self.phases[o * MAX_UNISON + u] as f32;
                for i in 0..n {
                    phase += inc;
                    if phase >= 1.0 {
                        phase -= 1.0;
                    }
                    // Hard sync: the phase runs at the note's pitch, the table at a multiple.
                    let read = if sync <= 1.001 {
                        phase
                    } else {
                        (phase * sync).fract()
                    };
                    let s = table.read(morph, read, mip);
                    self.buf_l[i] += s * gl;
                    self.buf_r[i] += s * gr;
                }
                self.phases[o * MAX_UNISON + u] = f64::from(phase);
            }
        }

        // ---- sub oscillator
        if sh.on(idx.sub[0]) {
            let level = sh.with(idx.sub[2], &self.offsets);
            if level > 1e-4 {
                let pan = sh.with(idx.sub[3], &self.offsets);
                let shape = sh.choice(idx.sub[1]);
                let octave = sh.with(idx.sub[4], &self.offsets);
                let inc = base_freq * 2f32.powf(octave) / sh.sr;
                let (gl, gr) = pan_gains(pan);
                let (gl, gr) = (level * gl, level * gr);
                let mut phase = self.sub_phase;
                for i in 0..n {
                    phase += inc;
                    if phase >= 1.0 {
                        phase -= 1.0;
                    }
                    let s = match shape {
                        1 => 1.0 - 4.0 * (phase - 0.5).abs(),
                        2 => 2.0 * phase - 1.0,
                        3 => {
                            if phase < 0.5 {
                                1.0
                            } else {
                                -1.0
                            }
                        }
                        _ => (phase * std::f32::consts::TAU).sin(),
                    };
                    self.buf_l[i] += s * gl;
                    self.buf_r[i] += s * gr;
                }
                self.sub_phase = phase;
            }
        }

        // ---- noise
        if sh.on(idx.noise[0]) {
            let level = sh.with(idx.noise[2], &self.offsets);
            if level > 1e-4 {
                let pan = sh.with(idx.noise[3], &self.offsets);
                let pink = sh.choice(idx.noise[1]) == 1;
                let (gl, gr) = pan_gains(pan);
                let (gl, gr) = (level * gl, level * gr);
                for i in 0..n {
                    let white = self.random() * 2.0 - 1.0;
                    let s = if pink {
                        let k = &mut self.pink;
                        k[0] = 0.99765 * k[0] + white * 0.099_046;
                        k[1] = 0.963 * k[1] + white * 0.296_516_4;
                        k[2] = 0.57 * k[2] + white * 1.052_691_3;
                        (k[0] + k[1] + k[2] + white * 0.1848) * 0.2
                    } else {
                        white
                    };
                    self.buf_l[i] += s * gl;
                    self.buf_r[i] += s * gr;
                }
            }
        }

        // ---- filters
        let routing = sh.choice(idx.routing);
        let on = [sh.on(idx.filter[0].enabled), sh.on(idx.filter[1].enabled)];
        let key_offset = f32::from(self.note) - 60.0;
        let settings: [(usize, f32, f32, f32, f32); 2] = std::array::from_fn(|f| {
            let ix = &idx.filter[f];
            let keytrack = sh.with(ix.keytrack, &self.offsets);
            let cutoff =
                sh.with(ix.cutoff, &self.offsets) * 2f32.powf(key_offset / 12.0 * keytrack);
            (
                sh.choice(ix.kind),
                cutoff,
                sh.with(ix.resonance, &self.offsets),
                sh.with(ix.drive, &self.offsets),
                sh.with(ix.mix, &self.offsets),
            )
        });
        let run = |filter: &mut VoiceFilter, l: &mut [f32], r: &mut [f32], f: usize| {
            let (kind, cutoff, res, drive, mix) = settings[f];
            filter.process(l, r, kind, cutoff, res, drive, mix);
        };
        let [f1, f2] = &mut self.filters;
        if routing == 0 {
            if on[0] {
                run(f1, &mut self.buf_l[..n], &mut self.buf_r[..n], 0);
            }
            if on[1] {
                run(f2, &mut self.buf_l[..n], &mut self.buf_r[..n], 1);
            }
        } else if on[0] && on[1] {
            self.tmp_l[..n].copy_from_slice(&self.buf_l[..n]);
            self.tmp_r[..n].copy_from_slice(&self.buf_r[..n]);
            run(f1, &mut self.buf_l[..n], &mut self.buf_r[..n], 0);
            run(f2, &mut self.tmp_l[..n], &mut self.tmp_r[..n], 1);
            for i in 0..n {
                self.buf_l[i] = (self.buf_l[i] + self.tmp_l[i]) * 0.5;
                self.buf_r[i] = (self.buf_r[i] + self.tmp_r[i]) * 0.5;
            }
        } else if on[0] {
            run(f1, &mut self.buf_l[..n], &mut self.buf_r[..n], 0);
        } else if on[1] {
            run(f2, &mut self.buf_l[..n], &mut self.buf_r[..n], 1);
        }

        // ---- distortion section
        let dist = sh.choice(idx.dist[0]);
        if dist > 0 {
            let drive = sh.with(idx.dist[1], &self.offsets);
            let mix = sh.with(idx.dist[2], &self.offsets);
            let gain = 1.0 + drive * 15.0;
            let comp = 1.0 / gain.sqrt();
            if dist == 4 {
                let bits = sh.with(idx.dist[3], &self.offsets);
                let levels = 2f32.powf(bits);
                let down = sh.with(idx.dist[4], &self.offsets).round().max(1.0) as i32;
                for i in 0..n {
                    if self.dist_count <= 0 {
                        self.dist_hold = [
                            (self.buf_l[i] * levels).round() / levels,
                            (self.buf_r[i] * levels).round() / levels,
                        ];
                        self.dist_count = down;
                    }
                    self.dist_count -= 1;
                    self.buf_l[i] = self.buf_l[i] * (1.0 - mix) + self.dist_hold[0] * mix;
                    self.buf_r[i] = self.buf_r[i] * (1.0 - mix) + self.dist_hold[1] * mix;
                }
            } else {
                let shape = |x: f32| match dist {
                    1 => (x * gain).tanh() * comp,
                    2 => (x * gain).clamp(-0.8, 0.8) * comp,
                    _ => (x * gain * 1.5).sin() * comp,
                };
                for i in 0..n {
                    self.buf_l[i] = self.buf_l[i] * (1.0 - mix) + shape(self.buf_l[i]) * mix;
                    self.buf_r[i] = self.buf_r[i] * (1.0 - mix) + shape(self.buf_r[i]) * mix;
                }
            }
        }

        // ---- amplitude envelope (smoothed across the block) and velocity
        let vel_gain = 0.25 + 0.75 * self.velocity;
        let target = self.sources[SRC_ENV] * vel_gain;
        let step = (target - self.prev_amp) / n as f32;
        let mut amp = self.prev_amp;
        for i in 0..n {
            amp += step;
            out_l[i] += self.buf_l[i] * amp;
            out_r[i] += self.buf_r[i] * amp;
        }
        self.prev_amp = target;
        self.active()
    }
}

// ---------------------------------------------------------------------------- the instrument

struct Rack {
    chorus: Chorus,
    phaser: Phaser,
    flanger: Flanger,
    delay: StereoDelay,
    reverb: Reverb,
    eq: Eq3,
    comp: Compressor,
    dist: Distortion,
}

impl Rack {
    fn new(sr: f32) -> Self {
        Self {
            chorus: Chorus::new(sr),
            phaser: Phaser::new(sr),
            flanger: Flanger::new(sr),
            delay: StereoDelay::new(sr),
            reverb: Reverb::new(sr),
            eq: Eq3::new(sr),
            comp: Compressor::new(sr),
            dist: Distortion::new(sr),
        }
    }
}

pub struct Soundgineer {
    sh: Shared,
    voices: Vec<Voice>,
    rack: Rack,
    note_age: u32,
    tempo: f32,
    beat_counter: f64,
    /// Modulation of the effects' parameters, from the newest voice's sources.
    global_offsets: Box<[f32; PARAM_COUNT]>,
    global_sources: [f32; SOURCES],
    mix_l: [f32; BLOCK],
    mix_r: [f32; BLOCK],
    /// Samples since the last voice was sounding, to stop running the effects when nothing
    /// is left to hear.
    idle: u32,
}

impl Soundgineer {
    pub fn new() -> Self {
        Self::with_rate(48_000.0)
    }

    fn with_rate(sr: f32) -> Self {
        let mut values = Box::new([0.0; PARAM_COUNT]);
        for (value, param) in values.iter_mut().zip(PARAMS.iter()) {
            *value = param.default;
        }
        let mut sh = Shared {
            sr,
            values,
            idx: Idx::new(),
            // Built on the first call (not on the audio thread: instruments are made when
            // the engine is) and shared by every instance after that.
            tables: wavetable::tables(),
            slots: [Slot::default(); MOD_SLOTS],
            shapes: [Shape::default(); 8],
            lfo_freq: [0.0; 8],
            lfo_global: [0.0; 8],
            lfo_beat: [0.0; 8],
            bend_range: 2.0,
        };
        sh.rebuild_slots();
        (0..8).for_each(|l| sh.rebuild_shape(l));
        Self {
            sh,
            voices: (0..VOICES)
                .map(|i| Voice::new(sr, 0x9e37_79b9 ^ (i as u32 * 7919)))
                .collect(),
            rack: Rack::new(sr),
            note_age: 0,
            tempo: 120.0,
            beat_counter: 0.0,
            global_offsets: Box::new([0.0; PARAM_COUNT]),
            global_sources: [0.0; SOURCES],
            mix_l: [0.0; BLOCK],
            mix_r: [0.0; BLOCK],
            idle: 0,
        }
    }

    /// How many voices are sounding.
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active()).count()
    }

    fn update_global_lfos(&mut self, dt: f32) {
        self.beat_counter += f64::from(self.tempo / 60.0 * dt);
        let sh = &mut self.sh;
        for l in 0..8 {
            let ix = &sh.idx.lfo[l];
            let synced = sh.values[ix.sync] >= 0.5;
            let division = DIVISION_BEATS[sh.choice(ix.division).min(12)];
            let freq = if synced {
                self.tempo / 60.0 / division
            } else {
                let p = &PARAMS[ix.rate];
                norm_to_value(
                    p,
                    value_to_norm(p, sh.values[ix.rate]) + self.global_offsets[ix.rate],
                )
            };
            sh.lfo_freq[l] = freq;
            sh.lfo_global[l] = (sh.lfo_global[l] + freq * dt).rem_euclid(1.0);
            let beats_per_cycle = if synced {
                division
            } else {
                (60.0 / (freq * self.tempo)).max(1e-4)
            };
            let phase0 = sh.values[ix.phase];
            sh.lfo_beat[l] = (self.beat_counter / f64::from(beats_per_cycle)) as f32 % 1.0;
            sh.lfo_beat[l] = (sh.lfo_beat[l] + phase0).rem_euclid(1.0);
        }
    }

    /// The effects' parameters follow the newest voice's modulation sources.
    fn update_global_mods(&mut self) {
        let newest = self
            .voices
            .iter()
            .filter(|v| v.active())
            .max_by_key(|v| v.age);
        match newest {
            Some(v) => self.global_sources = v.sources,
            None => self.global_sources = [0.0; SOURCES],
        }
        let sh = &self.sh;
        for m in 0..4 {
            self.global_sources[SRC_MACRO + m] = sh.base(sh.idx.macros[m]);
        }
        self.global_sources[SRC_MODWHEEL] = sh.base(sh.idx.mod_wheel);
        self.global_sources[SRC_PITCHWHEEL] = sh.base(sh.idx.pitch_bend);
        self.global_sources[SRC_AFTERTOUCH] = sh.base(sh.idx.pressure);
        sh.route(&self.global_sources, &mut self.global_offsets);
    }

    fn gv(&self, i: usize) -> f32 {
        self.sh.with(i, &self.global_offsets)
    }

    fn run_effects(&mut self, n: usize) {
        let idx = &self.sh.idx;
        let (l, r) = (&mut self.mix_l[..n], &mut self.mix_r[..n]);
        let sh = &self.sh;
        let gv = |i: usize| sh.with(i, &self.global_offsets);
        let on = |i: usize| sh.on(i);
        let rack = &mut self.rack;
        if on(idx.chorus[0]) {
            rack.chorus.process(
                l,
                r,
                gv(idx.chorus[1]),
                gv(idx.chorus[2]),
                gv(idx.chorus[3]),
            );
        }
        if on(idx.phaser[0]) {
            rack.phaser.process(
                l,
                r,
                gv(idx.phaser[1]),
                gv(idx.phaser[2]),
                gv(idx.phaser[3]),
                gv(idx.phaser[4]),
            );
        }
        if on(idx.flanger[0]) {
            rack.flanger.process(
                l,
                r,
                gv(idx.flanger[1]),
                gv(idx.flanger[2]),
                gv(idx.flanger[3]),
                gv(idx.flanger[4]),
            );
        }
        if on(idx.delay[0]) {
            let time = if on(idx.delay[1]) {
                DIVISION_BEATS[sh.choice(idx.delay[2]).min(12)] * (60.0 / self.tempo)
            } else {
                gv(idx.delay[3])
            };
            rack.delay.process(
                l,
                r,
                time,
                gv(idx.delay[4]),
                on(idx.delay[5]),
                gv(idx.delay[6]),
            );
        }
        if on(idx.reverb[0]) {
            rack.reverb.process(
                l,
                r,
                gv(idx.reverb[1]),
                gv(idx.reverb[2]),
                gv(idx.reverb[3]),
                gv(idx.reverb[4]),
            );
        }
        if on(idx.eq[0]) {
            rack.eq.process(
                l,
                r,
                gv(idx.eq[1]),
                gv(idx.eq[2]),
                gv(idx.eq[3]),
                gv(idx.eq[4]),
            );
        }
        if on(idx.comp[0]) {
            rack.comp.process(
                l,
                r,
                gv(idx.comp[1]),
                gv(idx.comp[2]),
                gv(idx.comp[3]),
                gv(idx.comp[4]),
                gv(idx.comp[5]),
            );
        }
        if on(idx.fxdist[0]) {
            rack.dist.process(
                l,
                r,
                gv(idx.fxdist[1]),
                gv(idx.fxdist[2]),
                gv(idx.fxdist[3]),
            );
        }
    }

    fn process_block(&mut self, out_l: &mut [f32], out_r: &mut [f32]) {
        let n = out_l.len();
        let dt = n as f32 / self.sh.sr;
        let any_active = self.voices.iter().any(Voice::active);
        if !any_active {
            self.idle = self.idle.saturating_add(n as u32);
            if self.idle as f32 > TAIL_SECONDS * self.sh.sr {
                return; // nothing playing and nothing left to ring out
            }
        } else {
            self.idle = 0;
        }

        self.update_global_lfos(dt);
        self.sh.bend_range = self.sh.values[self.sh.idx.bend_range].round();
        self.mix_l[..n].fill(0.0);
        self.mix_r[..n].fill(0.0);
        for voice in self.voices.iter_mut().filter(|v| v.active()) {
            voice.render(&self.sh, &mut self.mix_l[..n], &mut self.mix_r[..n]);
        }
        self.update_global_mods();
        self.run_effects(n);

        let volume = self.gv(self.sh.idx.volume);
        for i in 0..n {
            let (l, r) = (self.mix_l[i] * volume, self.mix_r[i] * volume);
            // A safety clamp, and no NaN ever reaches the output.
            out_l[i] += if l.is_finite() {
                l.clamp(-2.0, 2.0)
            } else {
                0.0
            };
            out_r[i] += if r.is_finite() {
                r.clamp(-2.0, 2.0)
            } else {
                0.0
            };
        }
    }
}

impl Default for Soundgineer {
    fn default() -> Self {
        Self::new()
    }
}

impl Instrument for Soundgineer {
    fn prepare(&mut self, sample_rate: f64) {
        let sr = if sample_rate.is_finite() && sample_rate >= 8_000.0 {
            sample_rate as f32
        } else {
            48_000.0
        };
        // Everything that depends on the rate is rebuilt here (the engine is being built, so
        // allocating is fine); the settings stay.
        let values = self.sh.values.clone();
        let tempo = self.tempo;
        *self = Self::with_rate(sr);
        *self.sh.values = *values;
        self.sh.rebuild_slots();
        (0..8).for_each(|l| self.sh.rebuild_shape(l));
        self.tempo = tempo;
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if let Some(param) = PARAMS.get(index)
            && value.is_finite()
        {
            self.sh.values[index] = param.fit(value);
            if index >= FIRST_SHAPE_PARAM {
                let lfo = (index - FIRST_SHAPE_PARAM) / (1 + 3 * SHAPE_POINTS);
                self.sh.rebuild_shape(lfo.min(7));
            } else if index >= FIRST_MATRIX_PARAM {
                self.sh.rebuild_slots();
            }
        }
    }

    fn set_tempo(&mut self, bpm: f64) {
        if bpm.is_finite() && bpm > 0.0 {
            self.tempo = bpm as f32;
        }
    }

    fn note_on(&mut self, pitch: u8, velocity: f32) {
        let pitch = pitch.min(127);
        let polyphony = self.sh.values[self.sh.idx.polyphony].round().max(1.0) as usize;
        let active = self.voices.iter().filter(|v| v.active()).count();
        let mut chosen = None;
        if active < polyphony {
            chosen = self.voices.iter().position(|v| !v.active());
        }
        if chosen.is_none() {
            // Steal the quietest voice that's already letting go, else the oldest.
            chosen = self
                .voices
                .iter()
                .enumerate()
                .filter(|(_, v)| v.active() && v.envs[0].releasing())
                .min_by(|a, b| a.1.envs[0].value().total_cmp(&b.1.envs[0].value()))
                .map(|(i, _)| i)
                .or_else(|| {
                    self.voices
                        .iter()
                        .enumerate()
                        .filter(|(_, v)| v.active())
                        .min_by_key(|(_, v)| v.age)
                        .map(|(i, _)| i)
                });
        }
        if let Some(i) = chosen {
            self.note_age = self.note_age.wrapping_add(1);
            let (sh, age) = (&self.sh, self.note_age);
            self.voices[i].note_on(sh, pitch, velocity.clamp(0.001, 1.0), age);
            self.idle = 0;
        }
    }

    fn note_off(&mut self, pitch: u8) {
        for voice in &mut self.voices {
            if voice.active() && voice.note == pitch && voice.gate {
                voice.note_off();
            }
        }
    }

    fn all_notes_off(&mut self) {
        for voice in &mut self.voices {
            if voice.active() {
                voice.gate = false;
                voice.kill();
            }
        }
    }

    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len());
        for (l, r) in left[..frames]
            .chunks_mut(BLOCK)
            .zip(right[..frames].chunks_mut(BLOCK))
        {
            self.process_block(l, r);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    fn synth() -> Soundgineer {
        let mut s = Soundgineer::new();
        s.prepare(RATE);
        s
    }

    fn set(s: &mut Soundgineer, name: &str, value: f32) {
        s.set_param(find(name), value);
    }

    fn apply(s: &mut Soundgineer, name: &str) {
        for &(param, value) in preset(name).unwrap() {
            set(s, param, value);
        }
    }

    fn render(s: &mut Soundgineer, seconds: f64) -> (Vec<f32>, Vec<f32>) {
        let frames = (seconds * RATE) as usize;
        let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
        for (l, r) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
            s.render(l, r);
        }
        (l, r)
    }

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0, |m, x| m.max(x.abs()))
    }

    fn crossings(buf: &[f32]) -> usize {
        buf.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count()
    }

    #[test]
    fn the_parameter_table_is_consistent() {
        assert_eq!(PARAMS.len(), PARAM_COUNT);
        let mut names: Vec<_> = PARAMS.iter().map(|p| p.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), PARAMS.len(), "unique names");
        for p in PARAMS.iter() {
            assert!(p.min < p.max, "{}", p.name);
            assert!((p.min..=p.max).contains(&p.default), "{} default", p.name);
            if let Kind::Choice(labels) = p.kind {
                assert_eq!(labels.len() as f32, p.max + 1.0, "{}", p.name);
            }
            if p.kind == Kind::Log {
                assert!(p.min > 0.0, "{} can be logarithmic", p.name);
            }
        }
        let mut covered: Vec<usize> = SECTIONS.iter().flat_map(|(_, r)| r.clone()).collect();
        covered.sort_unstable();
        assert_eq!(covered, (0..PARAM_COUNT).collect::<Vec<_>>());
        // The names the engine looks up all exist (this builds every index).
        let _ = Idx::new();
        assert_eq!(MOD_SOURCES.len(), SOURCES + 1);
        assert_eq!(MOD_SOURCE_IDS[SRC_VELOCITY], "velocity");
        assert_eq!(MOD_SOURCE_IDS[SRC_MACRO], "macro1");
        assert_eq!(MOD_SOURCE_IDS[SRC_AFTERTOUCH], "aftertouch");
        assert_eq!(MOD_DEST_LABELS.len(), data::MOD_DESTS.len() + 1);
        for &dest in &data::MOD_DESTS {
            assert!(PARAMS[dest].name.len() > 1 && dest < FIRST_MATRIX_PARAM);
        }
        assert_eq!(PRESETS.len(), 59);
        assert_eq!(PRESETS[0], "Init");
    }

    #[test]
    fn mapping_values_to_knob_positions_round_trips() {
        for p in PARAMS.iter().take(FIRST_MATRIX_PARAM) {
            for v in [p.min, p.default, p.max] {
                let back = norm_to_value(p, value_to_norm(p, v));
                let tolerance = ((p.max - p.min) * 1e-3).max(1e-4);
                assert!(
                    (back - v).abs() <= tolerance,
                    "{}: {v} became {back}",
                    p.name
                );
            }
        }
        let cutoff = &PARAMS[find("filter1.cutoff")];
        let half = norm_to_value(cutoff, 0.5);
        assert!(
            (half - (20.0f32 * 20_000.0).sqrt()).abs() < 1.0,
            "logarithmic: {half}"
        );
    }

    #[test]
    fn every_preset_names_real_parameters_in_range() {
        for (name, _) in PRESET_NAMES.iter() {
            for &(param, value) in preset(name).unwrap() {
                let p = &PARAMS[find(param)];
                assert!(
                    (p.min..=p.max).contains(&value),
                    "{name}: {param} = {value} is outside {}..{}",
                    p.min,
                    p.max
                );
            }
        }
        assert!(preset("Init").unwrap().is_empty());
        assert!(preset("Nope").is_none());
        // A preset with a modulation sets the matrix.
        let bass = preset("Deep Saw Bass").unwrap();
        assert!(bass.iter().any(|&(n, v)| n == "mod1.source" && v > 0.0));
        assert!(bass.iter().any(|&(n, _)| n == "mod1.depth"));
    }

    #[test]
    fn the_init_patch_plays_a_sine_at_the_notes_pitch_and_stops() {
        let mut s = synth();
        s.note_on(69, 1.0);
        let (l, r) = render(&mut s, 1.0);
        assert_eq!(s.active_voices(), 1);
        let hz = crossings(&l);
        assert!((438..=442).contains(&hz), "A4 is 440 Hz, got {hz}");
        let level = peak(&l[9_600..]);
        assert!(
            (0.15..0.9).contains(&level),
            "audible but not loud: {level}"
        );
        assert!(l.iter().chain(&r).all(|x| x.is_finite()));
        s.note_off(69);
        let (l, _) = render(&mut s, 1.0);
        assert!(peak(&l[40_000..]) < 1e-3, "released");
        assert_eq!(s.active_voices(), 0);
    }

    #[test]
    fn every_preset_makes_a_clean_sound() {
        for (name, _) in PRESET_NAMES.iter().skip(1) {
            let mut s = synth();
            apply(&mut s, name);
            // Slow pads would still be fading in; the rest of the patch is what's tested.
            set(&mut s, "env1.delay", 0.0);
            set(&mut s, "env1.attack", 0.002);
            for pitch in [48, 55, 60] {
                s.note_on(pitch, 0.9);
            }
            let (l, r) = render(&mut s, 0.6);
            assert!(
                l.iter().chain(&r).all(|x| x.is_finite()),
                "{name} is finite"
            );
            let level = peak(&l).max(peak(&r));
            assert!(level > 0.01, "{name} makes sound ({level})");
            assert!(level < 2.01, "{name} stays inside the clamp ({level})");
            s.all_notes_off();
            render(&mut s, 0.3);
            assert_eq!(s.active_voices(), 0, "{name} lets go when told to");
        }
    }

    #[test]
    fn every_generated_extra_makes_a_clean_sound() {
        let mut problems: Vec<String> = Vec::new();
        for (category, preset) in userpreset::extras() {
            let name = &preset.name;
            let mut s = synth();
            for (param, value) in &preset.params {
                set(&mut s, param, *value);
            }
            set(&mut s, "env1.delay", 0.0);
            set(&mut s, "env1.attack", 0.002);
            for pitch in [48, 55, 60] {
                s.note_on(pitch, 0.9);
            }
            let (l, r) = render(&mut s, 0.6);
            assert!(
                l.iter().chain(&r).all(|x| x.is_finite()),
                "{category}/{name} is finite"
            );
            let level = peak(&l).max(peak(&r));
            if !(level > 0.01 && level < 2.01) {
                problems.push(format!("{category}/{name}: peak {level}"));
            }
            s.all_notes_off();
            render(&mut s, 0.3);
            assert_eq!(
                s.active_voices(),
                0,
                "{category}/{name} lets go when told to"
            );
        }
        assert!(problems.is_empty(), "silent or too loud: {problems:#?}");
    }

    #[test]
    fn the_mod_matrix_moves_parameters() {
        // Mod wheel to the filter cutoff: turning the wheel changes the sound.
        let bright = |wheel: f32| {
            let mut s = synth();
            set(&mut s, "osc1.wavetable", 0.0);
            set(&mut s, "osc1.morph", 0.7); // a saw-ish shape
            set(&mut s, "filter1.cutoff", 400.0);
            set(&mut s, "filter1.resonance", 0.1);
            set(&mut s, "mod1.source", 22.0); // Mod Wheel
            let dest = MOD_DEST_LABELS
                .iter()
                .position(|l| *l == "Filter 1 Cutoff")
                .unwrap();
            set(&mut s, "mod1.dest", dest as f32);
            set(&mut s, "mod1.depth", 0.9);
            set(&mut s, "modwheel.value", wheel);
            s.note_on(45, 1.0);
            let (l, _) = render(&mut s, 0.4);
            l[4_800..]
                .windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .sum::<f32>()
        };
        assert!(
            bright(1.0) > 2.0 * bright(0.0),
            "the wheel opens the filter"
        );

        // Without a source or target nothing is routed.
        let mut s = synth();
        set(&mut s, "mod1.depth", 0.9);
        assert!(s.sh.slots.iter().all(|slot| slot.source == 0));
        set(&mut s, "mod1.source", 3.0);
        set(&mut s, "mod1.dest", 5.0);
        assert_eq!(s.sh.slots[0].source, 3);
        set(&mut s, "mod1.depth", 0.0);
        assert_eq!(s.sh.slots[0].source, 0, "zero depth is no route");
    }

    #[test]
    fn an_lfos_drawn_curve_shapes_the_modulation() {
        // LFO 1 free-running at 4 Hz, drawn as a square (up for the first half of each cycle),
        // sent to oscillator 1's level: the sound switches on and off.
        let tremolo = |square: bool| {
            let mut s = synth();
            set(&mut s, "osc1.level", 1.0);
            set(&mut s, "lfo1.sync", 0.0);
            set(&mut s, "lfo1.rate", 4.0);
            set(&mut s, "lfo1.mode", 1.0); // free
            let dest = MOD_DEST_LABELS
                .iter()
                .position(|l| *l == "Osc 1 Level")
                .unwrap();
            set(&mut s, "mod1.source", 8.0); // LFO 1
            set(&mut s, "mod1.dest", dest as f32);
            set(&mut s, "mod1.depth", -1.0);
            if square {
                for (i, (x, y)) in [(0.0, 1.0), (0.499, 1.0), (0.5, 0.0), (1.0, 0.0)]
                    .into_iter()
                    .enumerate()
                {
                    set(&mut s, &format!("lfo1.p{i}x"), x);
                    set(&mut s, &format!("lfo1.p{i}y"), y);
                }
                set(&mut s, "lfo1.count", 4.0);
            }
            s.note_on(60, 1.0);
            let (l, _) = render(&mut s, 1.0);
            // The level in 10 ms slices, over the second half (past the envelope's attack).
            let slices: Vec<f32> = l[24_000..].chunks(480).map(peak).collect();
            let (lo, hi) = slices
                .iter()
                .fold((f32::MAX, 0.0f32), |(lo, hi), &x| (lo.min(x), hi.max(x)));
            (lo, hi)
        };
        let (square_lo, square_hi) = tremolo(true);
        assert!(square_hi > 0.2, "on for half the cycle ({square_hi})");
        assert!(square_lo < 0.02, "and silent for the rest ({square_lo})");
        let (_, _) = tremolo(false);

        // Moving a point changes the curve the DSP uses.
        let mut s = synth();
        let before = s.sh.shapes[1].eval(0.5);
        set(&mut s, "lfo2.p1y", 0.25);
        assert_ne!(s.sh.shapes[1].eval(0.5), before);
        assert_eq!(s.sh.shapes[0], Shape::default(), "LFO 1 is untouched");
        // Points typed out of order are sorted, and the count is kept in range.
        set(&mut s, "lfo3.p0x", 0.9);
        set(&mut s, "lfo3.count", 99.0);
        assert_eq!(s.sh.shapes[2].count, SHAPE_POINTS);
        assert!(s.sh.shapes[2].x.windows(2).all(|w| w[0] <= w[1]));
        // The curve survives a change of sample rate.
        set(&mut s, "lfo2.p1y", 0.25);
        s.prepare(44_100.0);
        assert_eq!(s.sh.shapes[1].eval(0.5), 0.25);
        // Where each LFO's curve lives in the parameter table.
        for lfo in 0..8 {
            assert_eq!(
                PARAMS[shape_param(lfo)].name,
                format!("lfo{}.count", lfo + 1)
            );
            assert_eq!(
                PARAMS[shape_param(lfo) + 1 + 3 * 15 + 2].name,
                format!("lfo{}.p15pow", lfo + 1)
            );
        }
        assert_eq!(shape_param(8), PARAM_COUNT);
    }

    #[test]
    fn velocity_pitch_and_unison_do_what_they_say() {
        let play = |setup: &dyn Fn(&mut Soundgineer), velocity: f32| {
            let mut s = synth();
            setup(&mut s);
            s.note_on(60, velocity);
            render(&mut s, 0.5).0
        };
        let (loud, soft) = (play(&|_| {}, 1.0), play(&|_| {}, 0.2));
        assert!(peak(&soft) < 0.6 * peak(&loud));
        let octave = play(&|s| set(s, "osc1.transpose", 12.0), 1.0);
        let plain = play(&|_| {}, 1.0);
        let ratio = crossings(&octave) as f32 / crossings(&plain) as f32;
        assert!((1.95..2.05).contains(&ratio), "an octave up: {ratio}");
        // Unison with detune beats against itself, so the level wobbles.
        let wobble = |buf: &[f32]| {
            let chunks: Vec<f32> = buf[4_800..].chunks(240).map(peak).collect();
            let lo = chunks.iter().cloned().fold(f32::MAX, f32::min);
            let hi = chunks.iter().cloned().fold(0.0, f32::max);
            hi - lo
        };
        let thick = play(
            &|s| {
                set(s, "osc1.unison", 7.0);
                set(s, "osc1.detune", 30.0);
            },
            1.0,
        );
        assert!(wobble(&thick) > 3.0 * wobble(&plain).max(0.01));
    }

    #[test]
    fn voices_are_stolen_and_the_effects_tail_then_stops() {
        let mut s = synth();
        for pitch in 20..60 {
            s.note_on(pitch, 1.0);
        }
        assert_eq!(s.active_voices(), VOICES);
        set(&mut s, "master.polyphony", 4.0);
        s.all_notes_off();
        render(&mut s, 0.1);
        for pitch in 60..70 {
            s.note_on(pitch, 1.0);
        }
        assert!(s.active_voices() <= 4, "the polyphony limit");

        // A reverb keeps ringing after the note, and a long time later nothing is run at all.
        let mut s = synth();
        set(&mut s, "reverb.enabled", 1.0);
        set(&mut s, "reverb.mix", 0.5);
        set(&mut s, "env1.release", 0.05);
        s.note_on(60, 1.0);
        render(&mut s, 0.2);
        s.note_off(60);
        render(&mut s, 0.3);
        let (tail, _) = render(&mut s, 0.5);
        assert!(peak(&tail) > 1e-4, "the reverb tail");
        render(&mut s, 9.0);
        let (silence, _) = render(&mut s, 0.1);
        assert_eq!(peak(&silence), 0.0);
        assert!(s.idle as f32 > TAIL_SECONDS * s.sh.sr);
        s.note_on(60, 1.0);
        assert!(peak(&render(&mut s, 0.1).0) > 0.01, "and it wakes up again");
    }

    #[test]
    fn settings_survive_a_change_of_sample_rate_and_bad_values_are_ignored() {
        let mut s = synth();
        set(&mut s, "master.volume", 0.3);
        set(&mut s, "mod1.source", 2.0);
        set(&mut s, "mod1.dest", 7.0);
        set(&mut s, "mod1.depth", 0.5);
        s.set_tempo(90.0);
        s.prepare(44_100.0);
        assert_eq!(s.sh.values[find("master.volume")], 0.3);
        assert_eq!(s.sh.slots[0].source, 2, "the matrix too");
        assert_eq!(s.tempo, 90.0);
        assert_eq!(s.sh.sr, 44_100.0);
        s.set_param(find("master.volume"), f32::NAN);
        s.set_param(9_999, 1.0);
        s.set_param(find("master.volume"), 99.0);
        assert_eq!(s.sh.values[find("master.volume")], 1.5, "clamped");
        s.set_tempo(f64::NAN);
        s.set_tempo(-5.0);
        assert_eq!(s.tempo, 90.0);
        s.note_on(255, 5.0);
        assert!(render(&mut s, 0.1).0.iter().all(|x| x.is_finite()));
    }

    #[test]
    fn tempo_synced_effects_follow_the_song() {
        let echo = |bpm: f64| {
            let mut s = synth();
            s.set_tempo(bpm);
            set(&mut s, "delay.enabled", 1.0);
            set(&mut s, "delay.sync", 1.0);
            set(&mut s, "delay.division", 4.0); // 1/4
            set(&mut s, "delay.mix", 1.0);
            set(&mut s, "delay.feedback", 0.0);
            set(&mut s, "env1.attack", 0.001);
            set(&mut s, "env1.sustain", 1.0);
            set(&mut s, "env1.release", 0.01);
            s.note_on(72, 1.0);
            render(&mut s, 0.05);
            s.note_off(72);
            let (l, _) = render(&mut s, 1.5);
            // Where the echo starts, once the note itself has stopped.
            l[2_400..].iter().position(|x| x.abs() > 0.01).unwrap_or(0)
        };
        let (slow, fast) = (echo(60.0), echo(120.0));
        assert!(
            slow > fast + 8_000,
            "a quarter note is longer at 60 BPM ({slow} vs {fast})"
        );
    }
}
