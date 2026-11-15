//! Operator: a four-operator FM (phase modulation) synth, ported from the C++ app's
//! `OperatorSynth`. The operators are called A to D, and A is always heard.
//!
//! - 8 algorithms (which operator modulates which), feedback on D.
//! - Per operator: waveform, coarse and fine ratio, level, velocity sensitivity and an ADSR.
//! - A low/high/band-pass filter with its own envelope, a pitch envelope and an LFO.
//! - 16 voices and 10 presets.
//!
//! Parameters are stored by name, like an effect's ([`PARAMS`]). Nothing here allocates after
//! construction.

use crate::effects::{Kind, Param};
use crate::synth::Instrument;
use std::f64::consts::{PI, TAU};

const OPERATORS: usize = 4;
const VOICES: usize = 16;
/// How often pitch, LFO and filter coefficients are worked out, in samples.
const CONTROL_RATE: usize = 16;
/// How far a modulator at full level bends its target's phase, in radians.
const MOD_DEPTH: f32 = 4.0;

const WAVES: &[&str] = &["Sine", "Triangle", "Saw", "Square", "Noise"];
const FILTERS: &[&str] = &["Low pass", "High pass", "Band pass"];
const LFO_TARGETS: &[&str] = &["Pitch", "Filter", "Volume", "FM amount"];
/// The algorithms, as shown to people. `>` means "modulates".
pub const ALGORITHMS: &[&str] = &[
    "1: D > C > B > A",
    "2: (C + D) > B > A",
    "3: B > A, D > C > A",
    "4: (B + C + D) > A",
    "5: B > A and D > C",
    "6: D > A, B and C",
    "7: D > C, plus A and B",
    "8: A + B + C + D (no FM)",
];

/// Which operators modulate each operator (a bit per operator, A = bit 0), and which are heard.
/// A modulator always has a higher index than its target, so working from D down to A is
/// enough.
struct Algorithm {
    modulators: [u8; OPERATORS],
    carriers: u8,
}

const fn algorithm(modulators: [u8; OPERATORS], carriers: u8) -> Algorithm {
    Algorithm {
        modulators,
        carriers,
    }
}

const ALGORITHM_TABLE: [Algorithm; 8] = [
    algorithm([0b0010, 0b0100, 0b1000, 0], 0b0001),
    algorithm([0b0010, 0b1100, 0, 0], 0b0001),
    algorithm([0b0110, 0, 0b1000, 0], 0b0001),
    algorithm([0b1110, 0, 0, 0], 0b0001),
    algorithm([0b0010, 0, 0b1000, 0], 0b0101),
    algorithm([0b1000, 0b1000, 0b1000, 0], 0b0111),
    algorithm([0, 0, 0b1000, 0], 0b0111),
    algorithm([0, 0, 0, 0], 0b1111),
];

const fn p(
    name: &'static str,
    label: &'static str,
    min: f32,
    max: f32,
    default: f32,
    unit: &'static str,
    kind: Kind,
) -> Param {
    Param {
        name,
        label,
        min,
        max,
        default,
        unit,
        kind,
    }
}

// Indices into `PARAMS` (and into an `Operator`'s values).
const ALGORITHM: usize = 0;
const VOLUME: usize = 1;
const TRANSPOSE: usize = 2;
const FEEDBACK: usize = 3;
const FILTER_ON: usize = 4;
const FILTER_TYPE: usize = 5;
const CUTOFF: usize = 6;
const RESONANCE: usize = 7;
const FILTER_ENV: usize = 8;
const FILTER_ATTACK: usize = 9;
const FILTER_DECAY: usize = 10;
const FILTER_SUSTAIN: usize = 11;
const FILTER_RELEASE: usize = 12;
const PITCH_AMOUNT: usize = 13;
const PITCH_DECAY: usize = 14;
const LFO_RATE: usize = 15;
const LFO_AMOUNT: usize = 16;
const LFO_TARGET: usize = 17;
/// The first operator parameter; each operator has [`PER_OPERATOR`] of them.
const FIRST_OPERATOR_PARAM: usize = 18;
const PER_OPERATOR: usize = 10;
const ON: usize = 0;
const WAVE: usize = 1;
const COARSE: usize = 2;
const FINE: usize = 3;
const LEVEL: usize = 4;
const ATTACK: usize = 5;
const DECAY: usize = 6;
const SUSTAIN: usize = 7;
const RELEASE: usize = 8;
const VELOCITY: usize = 9;
const PARAM_COUNT: usize = FIRST_OPERATOR_PARAM + OPERATORS * PER_OPERATOR;

/// One operator's parameters. `$level` is its default level: A is heard by default, the others
/// are silent until turned up.
macro_rules! operator_params {
    ($op:literal, $level:literal) => {
        [
            p(
                concat!("op", $op, "On"),
                "On",
                0.0,
                1.0,
                1.0,
                "",
                Kind::Toggle,
            ),
            p(
                concat!("op", $op, "Wave"),
                "Wave",
                0.0,
                4.0,
                0.0,
                "",
                Kind::Choice(WAVES),
            ),
            p(
                concat!("op", $op, "Coarse"),
                "Coarse",
                0.0,
                32.0,
                1.0,
                "",
                Kind::Whole,
            ),
            p(
                concat!("op", $op, "Fine"),
                "Fine",
                0.0,
                1000.0,
                0.0,
                "",
                Kind::Whole,
            ),
            p(
                concat!("op", $op, "Level"),
                "Level",
                0.0,
                1.0,
                $level,
                "",
                Kind::Linear,
            ),
            p(
                concat!("op", $op, "Attack"),
                "Attack",
                0.001,
                10.0,
                0.002,
                "s",
                Kind::Log,
            ),
            p(
                concat!("op", $op, "Decay"),
                "Decay",
                0.001,
                10.0,
                0.6,
                "s",
                Kind::Log,
            ),
            p(
                concat!("op", $op, "Sustain"),
                "Sustain",
                0.0,
                1.0,
                1.0,
                "",
                Kind::Linear,
            ),
            p(
                concat!("op", $op, "Release"),
                "Release",
                0.001,
                10.0,
                0.3,
                "s",
                Kind::Log,
            ),
            p(
                concat!("op", $op, "Vel"),
                "Velocity",
                0.0,
                1.0,
                0.5,
                "",
                Kind::Linear,
            ),
        ]
    };
}

const GLOBAL_PARAMS: [Param; FIRST_OPERATOR_PARAM] = [
    p(
        "algorithm",
        "Algorithm",
        0.0,
        7.0,
        0.0,
        "",
        Kind::Choice(ALGORITHMS),
    ),
    p("volume", "Volume", 0.0, 1.0, 0.7, "", Kind::Linear),
    p(
        "transpose",
        "Transpose",
        -24.0,
        24.0,
        0.0,
        "st",
        Kind::Whole,
    ),
    p("feedback", "Feedback", 0.0, 1.0, 0.0, "", Kind::Linear),
    p("filterOn", "Filter", 0.0, 1.0, 0.0, "", Kind::Toggle),
    p(
        "filterType",
        "Type",
        0.0,
        2.0,
        0.0,
        "",
        Kind::Choice(FILTERS),
    ),
    p(
        "cutoff",
        "Cutoff",
        20.0,
        20_000.0,
        18_000.0,
        "Hz",
        Kind::Log,
    ),
    p("resonance", "Resonance", 0.0, 1.0, 0.1, "", Kind::Linear),
    p("filterEnv", "Envelope", -1.0, 1.0, 0.0, "", Kind::Linear),
    p("filterAttack", "Attack", 0.001, 10.0, 0.002, "s", Kind::Log),
    p("filterDecay", "Decay", 0.001, 10.0, 0.4, "s", Kind::Log),
    p("filterSustain", "Sustain", 0.0, 1.0, 0.0, "", Kind::Linear),
    p("filterRelease", "Release", 0.001, 10.0, 0.3, "s", Kind::Log),
    p(
        "pitchEnvAmount",
        "Pitch envelope",
        -48.0,
        48.0,
        0.0,
        "st",
        Kind::Linear,
    ),
    p(
        "pitchEnvDecay",
        "Pitch decay",
        0.001,
        2.0,
        0.1,
        "s",
        Kind::Log,
    ),
    p("lfoRate", "Rate", 0.05, 20.0, 4.0, "Hz", Kind::Log),
    p("lfoAmount", "Amount", 0.0, 1.0, 0.0, "", Kind::Linear),
    p(
        "lfoDest",
        "Target",
        0.0,
        3.0,
        0.0,
        "",
        Kind::Choice(LFO_TARGETS),
    ),
];

const fn all_params() -> [Param; PARAM_COUNT] {
    let operators = [
        operator_params!("A", 1.0),
        operator_params!("B", 0.0),
        operator_params!("C", 0.0),
        operator_params!("D", 0.0),
    ];
    let mut all = [GLOBAL_PARAMS[0]; PARAM_COUNT];
    let mut i = 0;
    while i < FIRST_OPERATOR_PARAM {
        all[i] = GLOBAL_PARAMS[i];
        i += 1;
    }
    let mut op = 0;
    while op < OPERATORS {
        let mut j = 0;
        while j < PER_OPERATOR {
            all[FIRST_OPERATOR_PARAM + op * PER_OPERATOR + j] = operators[op][j];
            j += 1;
        }
        op += 1;
    }
    all
}

/// Operator's parameters: the global ones, then ten for each of the operators A to D.
pub static PARAMS: [Param; PARAM_COUNT] = all_params();

/// How the parameters are grouped for people: a title, and the range of [`PARAMS`] under it.
pub const SECTIONS: [(&str, std::ops::Range<usize>); 8] = [
    ("Sound", ALGORITHM..FILTER_ON),
    ("Operator A", operator_range(0)),
    ("Operator B", operator_range(1)),
    ("Operator C", operator_range(2)),
    ("Operator D", operator_range(3)),
    ("Filter", FILTER_ON..PITCH_AMOUNT),
    ("Pitch envelope", PITCH_AMOUNT..LFO_RATE),
    ("LFO", LFO_RATE..FIRST_OPERATOR_PARAM),
];

const fn operator_range(op: usize) -> std::ops::Range<usize> {
    let start = FIRST_OPERATOR_PARAM + op * PER_OPERATOR;
    start..start + PER_OPERATOR
}

/// The presets, by name. "Init" is every parameter at its default.
pub const PRESETS: [&str; 10] = [
    "Init",
    "Electric Piano",
    "FM Bass",
    "Glass Bell",
    "Warm Pad",
    "Kick",
    "Pluck",
    "Brass",
    "Wobble Bass",
    "Hi-Hat",
];

/// A preset's parameters by name; anything not listed is at its default. `None` if there's no
/// such preset.
pub fn preset(name: &str) -> Option<&'static [(&'static str, f32)]> {
    Some(match name {
        "Init" => &[],
        "Electric Piano" => &[
            ("algorithm", 4.0),
            ("volume", 0.6),
            ("opALevel", 0.85),
            ("opADecay", 1.8),
            ("opASustain", 0.25),
            ("opARelease", 0.5),
            ("opBLevel", 0.45),
            ("opBDecay", 0.9),
            ("opBSustain", 0.1),
            ("opBVel", 0.9),
            ("opCLevel", 0.5),
            ("opCFine", 4.0),
            ("opCDecay", 2.2),
            ("opCSustain", 0.2),
            ("opCRelease", 0.5),
            ("opDCoarse", 14.0),
            ("opDLevel", 0.18),
            ("opDDecay", 0.25),
            ("opDSustain", 0.0),
        ],
        "FM Bass" => &[
            ("algorithm", 0.0),
            ("feedback", 0.2),
            ("filterOn", 1.0),
            ("cutoff", 1600.0),
            ("resonance", 0.3),
            ("filterEnv", 0.45),
            ("filterDecay", 0.25),
            ("opAAttack", 0.001),
            ("opADecay", 0.4),
            ("opASustain", 0.7),
            ("opARelease", 0.08),
            ("opBLevel", 0.6),
            ("opBDecay", 0.3),
            ("opBSustain", 0.25),
            ("opCCoarse", 2.0),
            ("opCLevel", 0.3),
            ("opCDecay", 0.2),
            ("opCSustain", 0.0),
        ],
        "Glass Bell" => &[
            ("algorithm", 4.0),
            ("volume", 0.55),
            ("opADecay", 3.0),
            ("opASustain", 0.0),
            ("opARelease", 2.0),
            ("opBCoarse", 3.0),
            ("opBFine", 500.0),
            ("opBLevel", 0.5),
            ("opBDecay", 2.0),
            ("opBSustain", 0.0),
            ("opCCoarse", 2.0),
            ("opCLevel", 0.45),
            ("opCDecay", 2.5),
            ("opCSustain", 0.0),
            ("opCRelease", 2.0),
            ("opDCoarse", 7.0),
            ("opDLevel", 0.35),
            ("opDDecay", 1.5),
            ("opDSustain", 0.0),
        ],
        "Warm Pad" => &[
            ("algorithm", 5.0),
            ("volume", 0.5),
            ("filterOn", 1.0),
            ("cutoff", 2400.0),
            ("resonance", 0.2),
            ("lfoDest", 1.0),
            ("lfoRate", 0.3),
            ("lfoAmount", 0.3),
            ("opAWave", 2.0),
            ("opAAttack", 0.6),
            ("opADecay", 1.0),
            ("opASustain", 0.8),
            ("opARelease", 1.5),
            ("opBWave", 2.0),
            ("opBFine", 4.0),
            ("opBLevel", 0.8),
            ("opBAttack", 0.7),
            ("opBSustain", 0.8),
            ("opBRelease", 1.5),
            ("opCCoarse", 2.0),
            ("opCFine", 2.0),
            ("opCLevel", 0.35),
            ("opCAttack", 0.9),
            ("opCSustain", 0.8),
            ("opCRelease", 1.5),
            ("opDLevel", 0.2),
            ("opDAttack", 1.0),
            ("opDSustain", 0.6),
        ],
        "Kick" => &[
            ("algorithm", 0.0),
            ("pitchEnvAmount", 36.0),
            ("pitchEnvDecay", 0.06),
            ("volume", 0.85),
            ("opAAttack", 0.001),
            ("opADecay", 0.45),
            ("opASustain", 0.0),
            ("opARelease", 0.1),
            ("opAVel", 0.3),
            ("opBLevel", 0.25),
            ("opBDecay", 0.04),
            ("opBSustain", 0.0),
        ],
        "Pluck" => &[
            ("algorithm", 1.0),
            ("filterOn", 1.0),
            ("cutoff", 2500.0),
            ("filterEnv", 0.5),
            ("filterDecay", 0.2),
            ("volume", 0.6),
            ("opADecay", 0.35),
            ("opASustain", 0.0),
            ("opARelease", 0.3),
            ("opBCoarse", 2.0),
            ("opBLevel", 0.5),
            ("opBDecay", 0.2),
            ("opBSustain", 0.0),
            ("opCCoarse", 3.0),
            ("opCLevel", 0.2),
            ("opCDecay", 0.15),
            ("opCSustain", 0.0),
        ],
        "Brass" => &[
            ("algorithm", 0.0),
            ("feedback", 0.3),
            ("lfoDest", 0.0),
            ("lfoRate", 5.0),
            ("lfoAmount", 0.08),
            ("volume", 0.55),
            ("opAAttack", 0.04),
            ("opADecay", 0.3),
            ("opASustain", 0.8),
            ("opARelease", 0.2),
            ("opBLevel", 0.55),
            ("opBAttack", 0.08),
            ("opBDecay", 0.3),
            ("opBSustain", 0.6),
            ("opBRelease", 0.2),
        ],
        "Wobble Bass" => &[
            ("algorithm", 0.0),
            ("filterOn", 1.0),
            ("cutoff", 500.0),
            ("resonance", 0.55),
            ("lfoDest", 1.0),
            ("lfoRate", 2.0),
            ("lfoAmount", 0.8),
            ("feedback", 0.4),
            ("opAWave", 2.0),
            ("opASustain", 1.0),
            ("opARelease", 0.1),
            ("opBLevel", 0.7),
            ("opBSustain", 0.8),
        ],
        "Hi-Hat" => &[
            ("algorithm", 7.0),
            ("filterOn", 1.0),
            ("filterType", 1.0),
            ("cutoff", 7000.0),
            ("resonance", 0.3),
            ("volume", 0.5),
            ("opAWave", 4.0),
            ("opALevel", 0.6),
            ("opAAttack", 0.001),
            ("opADecay", 0.07),
            ("opASustain", 0.0),
            ("opARelease", 0.05),
        ],
        _ => return None,
    })
}

#[derive(Clone, Copy, Default, PartialEq)]
enum Stage {
    #[default]
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// A linear attack, then exponential decay and release.
#[derive(Clone, Copy, Default)]
struct Envelope {
    stage: Stage,
    level: f32,
    attack_step: f32,
    decay_coef: f32,
    release_coef: f32,
    sustain: f32,
}

impl Envelope {
    fn set(&mut self, attack: f32, decay: f32, sustain: f32, release: f32, rate: f64) {
        self.attack_step = (1.0 / (f64::from(attack.max(0.0005)) * rate)) as f32;
        self.decay_coef = (-6.9 / (f64::from(decay.max(0.001)) * rate)).exp() as f32;
        self.release_coef = (-6.9 / (f64::from(release.max(0.001)) * rate)).exp() as f32;
        self.sustain = sustain;
    }

    fn note_on(&mut self) {
        self.stage = Stage::Attack;
    }

    fn note_off(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    fn active(&self) -> bool {
        self.stage != Stage::Idle
    }

    fn next(&mut self) -> f32 {
        match self.stage {
            Stage::Idle => return 0.0,
            Stage::Attack => {
                self.level += self.attack_step;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.level = self.sustain + (self.level - self.sustain) * self.decay_coef;
                if (self.level - self.sustain).abs() < 1e-4 {
                    self.level = self.sustain;
                    self.stage = if self.sustain <= 1e-4 {
                        Stage::Idle
                    } else {
                        Stage::Sustain
                    };
                }
            }
            Stage::Sustain => self.level = self.sustain,
            Stage::Release => {
                self.level *= self.release_coef;
                if self.level < 1e-4 {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level
    }
}

#[derive(Clone, Copy, Default)]
struct Voice {
    pitch: u8,
    velocity: f32,
    held: bool,
    age: u32,
    frequency: f64,
    phase: [f64; OPERATORS],
    env: [Envelope; OPERATORS],
    filter_env: Envelope,
    pitch_env: f32,
    feedback: [f32; 2],
    /// The filter's state.
    ic: [f32; 2],
    noise: u32,
}

impl Voice {
    fn active(&self) -> bool {
        self.env.iter().any(Envelope::active)
    }

    fn release(&mut self) {
        self.held = false;
        for env in &mut self.env {
            env.note_off();
        }
        self.filter_env.note_off();
    }
}

/// The parameter values a block is rendered with.
struct Snapshot {
    algorithm: &'static Algorithm,
    on: [bool; OPERATORS],
    wave: [u8; OPERATORS],
    ratio: [f64; OPERATORS],
    level: [f32; OPERATORS],
    velocity: [f32; OPERATORS],
    volume: f32,
    feedback: f32,
    transpose: f32,
    filter_on: bool,
    filter_type: u8,
    cutoff: f32,
    resonance: f32,
    filter_env: f32,
    pitch_amount: f32,
    pitch_coef: f32,
    lfo_amount: f32,
    lfo_target: u8,
    lfo_step: f64,
}

/// The Operator synth.
pub struct Operator {
    values: [f32; PARAM_COUNT],
    sample_rate: f64,
    voices: [Voice; VOICES],
    counter: u32,
    lfo_phase: f64,
}

impl Operator {
    pub fn new() -> Self {
        let mut values = [0.0; PARAM_COUNT];
        for (value, param) in values.iter_mut().zip(&PARAMS) {
            *value = param.default;
        }
        Self {
            values,
            sample_rate: 44_100.0,
            voices: [Voice::default(); VOICES],
            counter: 0,
            lfo_phase: 0.0,
        }
    }

    fn op(&self, op: usize, param: usize) -> f32 {
        self.values[FIRST_OPERATOR_PARAM + op * PER_OPERATOR + param]
    }

    fn snapshot(&self) -> Snapshot {
        let v = &self.values;
        let each = |f: &dyn Fn(usize) -> f32| std::array::from_fn(f);
        let coarse_and_fine = |op: usize| {
            let coarse = f64::from(self.op(op, COARSE).round());
            let coarse = if coarse < 1.0 { 0.5 } else { coarse };
            coarse * (1.0 + f64::from(self.op(op, FINE)) / 1000.0)
        };
        Snapshot {
            algorithm: &ALGORITHM_TABLE[(v[ALGORITHM].round() as usize).min(7)],
            on: std::array::from_fn(|op| self.op(op, ON) > 0.5),
            wave: std::array::from_fn(|op| self.op(op, WAVE).round() as u8),
            ratio: std::array::from_fn(coarse_and_fine),
            level: each(&|op| self.op(op, LEVEL)),
            velocity: each(&|op| self.op(op, VELOCITY)),
            volume: v[VOLUME],
            feedback: v[FEEDBACK],
            transpose: v[TRANSPOSE].round(),
            filter_on: v[FILTER_ON] > 0.5,
            filter_type: v[FILTER_TYPE].round() as u8,
            cutoff: v[CUTOFF],
            resonance: v[RESONANCE],
            filter_env: v[FILTER_ENV],
            pitch_amount: v[PITCH_AMOUNT],
            pitch_coef: (-4.6 / (f64::from(v[PITCH_DECAY].max(0.001)) * self.sample_rate)).exp()
                as f32,
            lfo_amount: v[LFO_AMOUNT],
            lfo_target: v[LFO_TARGET].round() as u8,
            lfo_step: f64::from(v[LFO_RATE]) / self.sample_rate,
        }
    }

    /// The voice a new note takes: one already playing that pitch, else a free one, else the
    /// oldest, preferring ones that have been let go.
    fn pick_voice(&self, pitch: u8) -> usize {
        let voices = &self.voices;
        if let Some(same) = voices.iter().position(|v| v.active() && v.pitch == pitch) {
            return same;
        }
        if let Some(free) = voices.iter().position(|v| !v.active()) {
            return free;
        }
        (0..VOICES)
            .min_by_key(|&i| (voices[i].held, voices[i].age))
            .unwrap_or(0)
    }

    #[cfg(test)]
    fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active()).count()
    }
}

impl Default for Operator {
    fn default() -> Self {
        Self::new()
    }
}

fn next_random(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    *seed
}

impl Instrument for Operator {
    fn prepare(&mut self, sample_rate: f64) {
        self.sample_rate = if sample_rate.is_finite() && sample_rate > 0.0 {
            sample_rate
        } else {
            44_100.0
        };
        self.lfo_phase = 0.0;
        self.all_notes_off();
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if let (Some(slot), Some(param)) = (self.values.get_mut(index), PARAMS.get(index))
            && value.is_finite()
        {
            *slot = value.clamp(param.min, param.max);
        }
    }

    fn note_on(&mut self, pitch: u8, velocity: f32) {
        let pitch = pitch.min(127);
        let index = self.pick_voice(pitch);
        self.counter = self.counter.wrapping_add(1);
        let rate = self.sample_rate;
        let envelopes: [[f32; 4]; OPERATORS] = std::array::from_fn(|op| {
            [ATTACK, DECAY, SUSTAIN, RELEASE].map(|param| self.op(op, param))
        });
        let filter = [FILTER_ATTACK, FILTER_DECAY, FILTER_SUSTAIN, FILTER_RELEASE]
            .map(|param| self.values[param]);

        let voice = &mut self.voices[index];
        let was_silent = !voice.active();
        voice.pitch = pitch;
        voice.velocity = velocity.clamp(0.0, 1.0);
        voice.held = true;
        voice.age = self.counter;
        voice.frequency = 440.0 * 2f64.powf((f64::from(pitch) - 69.0) / 12.0);
        voice.pitch_env = 1.0;
        voice.noise = 0x9e37_79b9 ^ self.counter;
        if was_silent {
            voice.phase = [0.0; OPERATORS];
            voice.feedback = [0.0; 2];
            voice.ic = [0.0; 2];
            for env in &mut voice.env {
                env.level = 0.0;
            }
            voice.filter_env.level = 0.0;
        }
        for (env, [a, d, s, r]) in voice.env.iter_mut().zip(envelopes) {
            env.set(a, d, s, r, rate);
            env.note_on();
        }
        let [a, d, s, r] = filter;
        voice.filter_env.set(a, d, s, r, rate);
        voice.filter_env.note_on();
    }

    fn note_off(&mut self, pitch: u8) {
        for voice in &mut self.voices {
            if voice.held && voice.pitch == pitch {
                voice.release();
            }
        }
    }

    fn all_notes_off(&mut self) {
        self.voices = [Voice::default(); VOICES];
    }

    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len());
        if frames == 0 {
            return;
        }
        let s = self.snapshot();
        let rate = self.sample_rate;
        let algorithm = s.algorithm;
        let carriers = algorithm.carriers.count_ones() as f32;
        let carrier_gain = 1.0 / (1.0 + 0.4 * (carriers - 1.0));
        let output_gain = s.volume * s.volume * 0.7;
        let k = 2.0 - 1.94 * s.resonance;
        let lfo_start = self.lfo_phase;

        for voice in self.voices.iter_mut().filter(|v| v.active()) {
            let mut lfo = lfo_start;
            let mut step = [0.0f64; OPERATORS];
            let velocity: [f32; OPERATORS] =
                std::array::from_fn(|op| 1.0 - s.velocity[op] * (1.0 - voice.velocity));
            let (mut fm_scale, mut amp_scale) = (1.0f32, 1.0f32);
            let (mut a1, mut a2, mut a3) = (0.0f32, 0.0f32, 0.0f32);

            for i in 0..frames {
                if i % CONTROL_RATE == 0 {
                    let lfo_value = (lfo * TAU).sin() as f32 * s.lfo_amount;
                    let mut semitones = s.transpose + voice.pitch_env * s.pitch_amount;
                    if s.lfo_target == 0 {
                        semitones += lfo_value * 2.0;
                    }
                    fm_scale = if s.lfo_target == 3 {
                        1.0 + lfo_value
                    } else {
                        1.0
                    };
                    amp_scale = if s.lfo_target == 2 {
                        1.0 - 0.5 * (s.lfo_amount + lfo_value)
                    } else {
                        1.0
                    };
                    let base = voice.frequency * 2f64.powf(f64::from(semitones) / 12.0) / rate;
                    for (step, ratio) in step.iter_mut().zip(s.ratio) {
                        *step = base * ratio;
                    }
                    if s.filter_on {
                        let mut octaves = s.filter_env * voice.filter_env.level * 6.0;
                        if s.lfo_target == 1 {
                            octaves += lfo_value * 3.0;
                        }
                        let fc = (f64::from(s.cutoff) * 2f64.powf(f64::from(octaves)))
                            .clamp(20.0, rate * 0.45);
                        let g = (PI * fc / rate).tan() as f32;
                        a1 = 1.0 / (1.0 + g * (g + k));
                        a2 = g * a1;
                        a3 = g * a2;
                    }
                }

                let mut out = [0.0f32; OPERATORS];
                for op in (0..OPERATORS).rev() {
                    let envelope = voice.env[op].next();
                    if !s.on[op] {
                        continue;
                    }
                    let mut pm = 0.0f64;
                    for (m, &modulator) in out.iter().enumerate().skip(op + 1) {
                        if (algorithm.modulators[op] >> m) & 1 == 1 {
                            pm += f64::from(modulator * MOD_DEPTH * fm_scale);
                        }
                    }
                    if op == OPERATORS - 1 {
                        pm += f64::from(
                            s.feedback * (voice.feedback[0] + voice.feedback[1]) * 0.5 * 3.0,
                        );
                    }
                    let phase = (voice.phase[op] + pm / TAU).rem_euclid(1.0);
                    let wave = match s.wave[op] {
                        0 => (phase * TAU).sin() as f32,
                        1 => {
                            (if phase < 0.5 {
                                4.0 * phase - 1.0
                            } else {
                                3.0 - 4.0 * phase
                            }) as f32
                        }
                        2 => (2.0 * phase - 1.0) as f32,
                        3 => {
                            if phase < 0.5 {
                                1.0
                            } else {
                                -1.0
                            }
                        }
                        _ => (next_random(&mut voice.noise) >> 8) as f32 / 8_388_608.0 - 1.0,
                    };
                    out[op] = wave * envelope * s.level[op] * velocity[op];
                    voice.phase[op] = (voice.phase[op] + step[op]).rem_euclid(1.0);
                }
                voice.feedback = [out[OPERATORS - 1], voice.feedback[0]];

                let mut sample = 0.0f32;
                for (op, &value) in out.iter().enumerate() {
                    if (algorithm.carriers >> op) & 1 == 1 {
                        sample += value;
                    }
                }
                sample *= carrier_gain;

                voice.filter_env.next();
                if s.filter_on {
                    let v3 = sample - voice.ic[1];
                    let v1 = a1 * voice.ic[0] + a2 * v3;
                    let v2 = voice.ic[1] + a2 * voice.ic[0] + a3 * v3;
                    voice.ic = [2.0 * v1 - voice.ic[0], 2.0 * v2 - voice.ic[1]];
                    sample = match s.filter_type {
                        0 => v2,
                        1 => sample - k * v1 - v2,
                        _ => v1 * k,
                    };
                    if !sample.is_finite() {
                        // A filter pushed too far: start it again rather than pass it on.
                        voice.ic = [0.0; 2];
                        sample = 0.0;
                    }
                }
                voice.pitch_env *= s.pitch_coef;
                lfo += s.lfo_step;

                sample *= output_gain * amp_scale;
                left[i] += sample;
                right[i] += sample;
            }

            // A voice is finished once every operator that's heard is silent.
            let heard = (0..OPERATORS).any(|op| {
                (algorithm.carriers >> op) & 1 == 1 && s.on[op] && voice.env[op].active()
            });
            if !heard {
                *voice = Voice::default();
            }
        }

        self.lfo_phase = (lfo_start + s.lfo_step * frames as f64).rem_euclid(1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0, |m, x| m.max(x.abs()))
    }

    fn index(name: &str) -> usize {
        PARAMS
            .iter()
            .position(|p| p.name == name)
            .unwrap_or_else(|| panic!("no parameter {name}"))
    }

    fn with_preset(name: &str) -> Operator {
        let mut synth = Operator::new();
        synth.prepare(RATE);
        for &(param, value) in preset(name).unwrap() {
            synth.set_param(index(param), value);
        }
        synth
    }

    /// Renders `seconds` in engine-sized blocks.
    fn render(synth: &mut Operator, seconds: f64) -> Vec<f32> {
        let frames = (seconds * RATE) as usize;
        let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
        for (l, r) in left.chunks_mut(512).zip(right.chunks_mut(512)) {
            synth.render(l, r);
        }
        assert_eq!(left, right, "Operator is mono");
        left
    }

    /// How many times the signal crosses zero going up: a rough pitch.
    fn crossings(buf: &[f32]) -> usize {
        buf.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count()
    }

    #[test]
    fn the_parameter_table_is_laid_out_as_the_synth_reads_it() {
        assert_eq!(PARAMS.len(), 58);
        assert_eq!(PARAMS[ALGORITHM].name, "algorithm");
        assert_eq!(PARAMS[LFO_TARGET].name, "lfoDest");
        assert_eq!(PARAMS[FIRST_OPERATOR_PARAM].name, "opAOn");
        assert_eq!(
            PARAMS[FIRST_OPERATOR_PARAM + PER_OPERATOR + WAVE].name,
            "opBWave"
        );
        assert_eq!(PARAMS[PARAM_COUNT - 1].name, "opDVel");
        let mut covered: Vec<usize> = SECTIONS.iter().flat_map(|(_, r)| r.clone()).collect();
        covered.sort_unstable();
        assert_eq!(
            covered,
            (0..PARAM_COUNT).collect::<Vec<_>>(),
            "every one is in a section"
        );
        assert_eq!(PARAMS[index("opALevel")].default, 1.0, "A is heard");
        assert_eq!(PARAMS[index("opBLevel")].default, 0.0);
        let mut names: Vec<_> = PARAMS.iter().map(|p| p.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), PARAMS.len(), "names are unique");
        for p in &PARAMS {
            assert!(
                p.min < p.max && (p.min..=p.max).contains(&p.default),
                "{}",
                p.name
            );
            if let Kind::Choice(labels) = p.kind {
                assert_eq!(labels.len() as f32, p.max + 1.0, "{}", p.name);
            }
        }
    }

    #[test]
    fn the_init_sound_is_a_sine_at_the_notes_pitch() {
        let mut synth = Operator::new();
        synth.prepare(RATE);
        synth.note_on(69, 1.0);
        let out = render(&mut synth, 1.0);
        let level = peak(&out[4800..]);
        assert!((level - 0.343).abs() < 0.01, "volume² × 0.7, got {level}");
        let hz = crossings(&out);
        assert!((439..=441).contains(&hz), "A4 is 440 Hz, got {hz}");
        synth.note_off(69);
        let tail = render(&mut synth, 1.0);
        assert_eq!(peak(&tail[24_000..]), 0.0, "released and finished");
        assert_eq!(synth.active_voices(), 0);
    }

    #[test]
    fn every_preset_makes_a_bounded_sound_and_ends() {
        for name in PRESETS {
            let values = preset(name).unwrap();
            for (param, value) in values {
                let p = &PARAMS[index(param)];
                assert!((p.min..=p.max).contains(value), "{name}: {param} in range");
            }
            let mut synth = with_preset(name);
            for pitch in [36, 48, 60] {
                synth.note_on(pitch, 0.9);
            }
            let out = render(&mut synth, 0.5);
            assert!(out.iter().all(|x| x.is_finite()), "{name} is finite");
            let level = peak(&out);
            assert!(level > 0.02, "{name} makes sound ({level})");
            assert!(level < 1.5, "{name} isn't wildly loud ({level})");
            for pitch in [36, 48, 60] {
                synth.note_off(pitch);
            }
            render(&mut synth, 12.0);
            assert_eq!(synth.active_voices(), 0, "{name} ends after release");
        }
        assert!(preset("Nope").is_none());
    }

    #[test]
    fn modulation_adds_harmonics() {
        let spread = |level: f32| {
            let mut synth = Operator::new();
            synth.prepare(RATE);
            synth.set_param(index("opBLevel"), level);
            synth.note_on(45, 1.0);
            let out = render(&mut synth, 0.5);
            // A pure sine changes slowly; harmonics make neighbouring samples differ more.
            out.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>()
        };
        assert!(
            spread(0.8) > 2.0 * spread(0.0),
            "B modulates A in algorithm 1"
        );
    }

    #[test]
    fn the_filter_and_the_operator_switch_work() {
        let mut bright = with_preset("Wobble Bass");
        bright.set_param(index("filterOn"), 0.0);
        bright.note_on(40, 1.0);
        let open = render(&mut bright, 0.3);
        let mut dark = with_preset("Wobble Bass");
        dark.set_param(index("lfoAmount"), 0.0);
        dark.set_param(index("cutoff"), 80.0);
        dark.note_on(40, 1.0);
        let closed = render(&mut dark, 0.3);
        let rough = |buf: &[f32]| buf.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>();
        assert!(
            rough(&open) > 3.0 * rough(&closed),
            "a low cutoff smooths the saw"
        );

        let mut silent = Operator::new();
        silent.prepare(RATE);
        silent.set_param(index("opAOn"), 0.0);
        silent.note_on(60, 1.0);
        assert_eq!(peak(&render(&mut silent, 0.1)), 0.0, "A is switched off");
    }

    #[test]
    fn the_kick_preset_sweeps_down() {
        let mut synth = with_preset("Kick");
        synth.note_on(36, 1.0);
        let out = render(&mut synth, 0.4);
        let early = crossings(&out[..960]) as f64 / 0.02;
        let late = crossings(&out[9600..14_400]) as f64 / 0.1;
        assert!(
            early > 2.0 * late,
            "starts high ({early} Hz) and drops ({late} Hz)"
        );
    }

    #[test]
    fn velocity_and_transpose_are_heard() {
        let play = |velocity: f32, transpose: f32| {
            let mut synth = Operator::new();
            synth.prepare(RATE);
            synth.set_param(index("transpose"), transpose);
            synth.note_on(69, velocity);
            render(&mut synth, 1.0)
        };
        let (loud, soft) = (play(1.0, 0.0), play(0.2, 0.0));
        assert!(peak(&soft) < 0.7 * peak(&loud), "softer when played softly");
        let up = crossings(&play(1.0, 12.0));
        assert!(
            (878..=882).contains(&up),
            "an octave up is 880 Hz, got {up}"
        );
    }

    #[test]
    fn steals_voices_instead_of_growing() {
        let mut synth = Operator::new();
        synth.prepare(RATE);
        for pitch in 20..60 {
            synth.note_on(pitch, 1.0);
        }
        assert_eq!(synth.active_voices(), VOICES);
        // The same pitch again reuses its voice.
        synth.all_notes_off();
        synth.note_on(60, 1.0);
        synth.note_on(60, 1.0);
        assert_eq!(synth.active_voices(), 1);
        // A released voice is taken before a held one.
        synth.all_notes_off();
        for pitch in 0..VOICES as u8 {
            synth.note_on(pitch, 1.0);
        }
        synth.note_off(5);
        synth.note_on(100, 1.0);
        assert!(synth.voices.iter().all(|v| v.pitch != 5));
        assert!(
            synth.voices.iter().any(|v| v.pitch == 0),
            "held notes are kept"
        );
    }

    #[test]
    fn bad_parameter_values_are_ignored_or_clamped() {
        let mut synth = Operator::new();
        synth.prepare(RATE);
        synth.set_param(index("cutoff"), f32::NAN);
        synth.set_param(index("volume"), 9.0);
        synth.set_param(index("algorithm"), 99.0);
        synth.set_param(9999, 1.0);
        assert_eq!(synth.values[CUTOFF], 18_000.0);
        assert_eq!(synth.values[VOLUME], 1.0);
        synth.set_param(index("filterOn"), 1.0);
        synth.set_param(index("resonance"), 1.0);
        synth.note_on(60, 1.0);
        assert!(render(&mut synth, 0.5).iter().all(|x| x.is_finite()));
    }
}
