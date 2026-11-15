//! The glitch effect: stutters, reversed snippets, tape stops, gating, bit-crush bursts and
//! pitched repeats, dropped into the sound on the beat grid, the way IDM and glitch-hop
//! producers do it by hand.
//!
//! The song is cut into *units* of a chosen length (a note value, times a few steps). At the
//! start of each unit the effect decides, from the song position and a seed, whether to glitch
//! and how. The same position and seed always make the same decision, so a render is identical
//! to what you heard, playback can start anywhere, and the result doesn't depend on the block
//! size. Everything the effect needs is allocated in `prepare`.

use crate::effects::{Effect, Kind, Param};

const fn p(
    name: &'static str,
    label: &'static str,
    min: f32,
    max: f32,
    default: f32,
    kind: Kind,
) -> Param {
    Param {
        name,
        label,
        min,
        max,
        default,
        unit: "",
        kind,
    }
}

const RATES: &[&str] = &["1/4", "1/8", "1/16", "1/32"];
const SLICES: &[&str] = &["1/16", "1/32", "1/64", "1/128"];

/// The glitch effect's settings.
pub static PARAMS: [Param; 12] = [
    p("amount", "Amount", 0.0, 1.0, 0.5, Kind::Linear),
    p("rate", "Rate", 0.0, 3.0, 2.0, Kind::Choice(RATES)),
    p("length", "Length", 1.0, 4.0, 1.0, Kind::Whole),
    p("slice", "Slice", 0.0, 3.0, 1.0, Kind::Choice(SLICES)),
    p("stutter", "Stutter", 0.0, 1.0, 1.0, Kind::Toggle),
    p("reverse", "Reverse", 0.0, 1.0, 1.0, Kind::Toggle),
    p("tape", "Tape stop", 0.0, 1.0, 1.0, Kind::Toggle),
    p("gate", "Gate", 0.0, 1.0, 1.0, Kind::Toggle),
    p("crush", "Crush", 0.0, 1.0, 1.0, Kind::Toggle),
    p("pitch", "Pitch jump", 0.0, 1.0, 1.0, Kind::Toggle),
    p("seed", "Seed", 0.0, 99.0, 0.0, Kind::Whole),
    p("mix", "Mix", 0.0, 1.0, 1.0, Kind::Linear),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Style {
    Stutter,
    Reverse,
    Tape,
    Gate,
    Crush,
    Pitch,
}

const STYLES: [Style; 6] = [
    Style::Stutter,
    Style::Reverse,
    Style::Tape,
    Style::Gate,
    Style::Crush,
    Style::Pitch,
];

/// A well-mixed 64-bit number from two inputs (SplitMix64), so each unit's random choices
/// come from its position, not from a running random number generator.
fn hash(a: u64, b: u64) -> u64 {
    let mut z = a
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(b.wrapping_mul(0xbf58_476d_1ce4_e5b9))
        .wrapping_add(0x94d0_49bb_1331_11eb);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// 0 to 1 from the bits of `h`.
fn fraction(h: u64) -> f32 {
    (h >> 40) as f32 / (1u64 << 24) as f32
}

pub struct Glitch {
    values: [f32; 12],
    sr: f32,
    bpm: f64,
    /// The playhead in beats at the start of the next sample.
    beat: f64,
    /// The last two seconds of input, per side.
    history: [Vec<f32>; 2],
    /// How many samples have been written to `history` since the start.
    written: u64,
    unit_index: i64,
    /// The style chosen for the current unit, if it's glitched.
    style: Option<Style>,
    /// `written` when the unit started.
    start: u64,
    unit_samples: f64,
    // The current unit's settings.
    slice_start: f64,
    slice_len: f64,
    min_slice: f64,
    increment: f64,
    phase: f64,
    shrink: bool,
    tape_pos: f64,
    gate_gain: f32,
    hold: [f32; 2],
    count: f32,
    bits: f32,
    divide: f32,
    /// 0 (dry) to 1 (glitched): eased, so entering and leaving a glitch never clicks.
    fade: f32,
}

impl Default for Glitch {
    fn default() -> Self {
        let mut values = [0.0; 12];
        for (v, param) in values.iter_mut().zip(&PARAMS) {
            *v = param.default;
        }
        Self {
            values,
            sr: 48_000.0,
            bpm: 120.0,
            beat: 0.0,
            history: [Vec::new(), Vec::new()],
            written: 0,
            unit_index: i64::MIN,
            style: None,
            start: 0,
            unit_samples: 1.0,
            slice_start: 0.0,
            slice_len: 1.0,
            min_slice: 1.0,
            increment: 1.0,
            phase: 0.0,
            shrink: false,
            tape_pos: 0.0,
            gate_gain: 1.0,
            hold: [0.0; 2],
            count: 0.0,
            bits: 8.0,
            divide: 1.0,
            fade: 0.0,
        }
    }
}

impl Glitch {
    fn on(&self, index: usize) -> bool {
        self.values[index] >= 0.5
    }

    fn history_len(&self) -> usize {
        self.history[0].len()
    }

    /// How long a unit lasts in beats: the rate, times how many steps long.
    fn unit_beats(&self) -> f64 {
        let step = [1.0, 0.5, 0.25, 0.125][self.values[1].round().clamp(0.0, 3.0) as usize];
        step * f64::from(self.values[2].round().clamp(1.0, 4.0))
    }

    /// The stutter slice length in samples.
    fn slice_samples(&self) -> f64 {
        let beats = [0.25, 0.125, 0.0625, 0.03125][self.values[3].round().clamp(0.0, 3.0) as usize];
        (beats * 60.0 / self.bpm * f64::from(self.sr)).max(32.0)
    }

    /// Reads the history `pos` samples from the start of time, between samples. Positions are
    /// kept inside what's been written and still remembered.
    fn read(&self, pos: f64) -> [f32; 2] {
        let len = self.history_len();
        let newest = self.written.saturating_sub(1) as f64;
        let oldest = (self.written.saturating_sub(len as u64 - 2)) as f64;
        let pos = pos.clamp(oldest.min(newest), newest);
        let i0 = pos.floor();
        let frac = (pos - i0) as f32;
        let (a, b) = (i0 as u64 as usize % len, (i0 as u64 + 1) as usize % len);
        let mut out = [0.0; 2];
        for (ch, o) in out.iter_mut().enumerate() {
            *o = self.history[ch][a] + (self.history[ch][b] - self.history[ch][a]) * frac;
        }
        out
    }

    /// Decides what the unit that starts now does.
    fn start_unit(&mut self, unit: i64) {
        self.unit_index = unit;
        self.start = self.written;
        let unit_beats = self.unit_beats();
        let max_unit = (self.history_len() as f64 - 128.0).max(1.0);
        self.unit_samples =
            (unit_beats * 60.0 / self.bpm * f64::from(self.sr)).clamp(1.0, max_unit);
        let seed = self.values[10].round() as u64;
        let h = hash(seed, unit as u64);
        self.fade = self.fade.min(1.0);
        // Does this unit glitch?
        let amount = self.values[0];
        let mut pool = [Style::Stutter; 6];
        let mut count = 0;
        for (i, style) in STYLES.iter().enumerate() {
            if self.on(4 + i) {
                pool[count] = *style;
                count += 1;
            }
        }
        if count == 0 || fraction(h) >= amount {
            self.style = None;
            return;
        }
        let style = pool[(h >> 8) as usize % count];
        self.style = Some(style);
        let r1 = fraction(hash(h, 1));
        let r2 = fraction(hash(h, 2));
        let slice = self.slice_samples();
        // What the style needs: a slice of the recent past, a loop, a speed.
        self.min_slice = (slice / 4.0).max(48.0);
        self.slice_len = slice * [1.0, 2.0, 0.5][(r1 * 3.0) as usize % 3];
        self.slice_len = self.slice_len.clamp(self.min_slice, max_unit);
        self.slice_start = self.written as f64 - self.slice_len;
        self.phase = 0.0;
        self.increment = 1.0;
        self.shrink = r2 > 0.55 && style == Style::Stutter;
        self.tape_pos = self.written as f64 - 1.0;
        self.gate_gain = 1.0;
        self.count = 0.0;
        self.bits = 2.0 + (r1 * 4.0).floor();
        self.divide = 3.0 + (r2 * 13.0).floor();
        if style == Style::Pitch {
            self.increment = [2.0, 0.5, 1.5, 0.75, 4.0 / 3.0, 1.0 / 3.0][(r2 * 6.0) as usize % 6];
        }
        // Not enough history yet (the very start): let this unit through unchanged.
        if (self.written as f64) < self.slice_len + 2.0 {
            self.style = None;
        }
    }
}

impl Effect for Glitch {
    fn prepare(&mut self, sample_rate: f64) {
        self.sr = sample_rate as f32;
        let len = (sample_rate * 2.0) as usize;
        self.history = [vec![0.0; len], vec![0.0; len]];
        self.written = 0;
        self.unit_index = i64::MIN;
        self.style = None;
        self.fade = 0.0;
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if let Some(slot) = self.values.get_mut(index) {
            *slot = value;
        }
    }

    fn sync(&mut self, bpm: f64, beat: f64, playing: bool) {
        if bpm.is_finite() && bpm > 0.0 {
            self.bpm = bpm;
        }
        // While the song plays, glitches fall on its beats; stopped, the clock runs freely.
        if playing && beat.is_finite() {
            self.beat = beat;
        }
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let len = self.history_len();
        if len == 0 {
            return;
        }
        let beats_per_sample = self.bpm / 60.0 / f64::from(self.sr);
        let unit_beats = self.unit_beats();
        let mix = self.values[11];
        let ease = 1.0 - (-1.0 / (0.002 * self.sr)).exp();
        let edge = f64::from(0.002 * self.sr);
        let gate_ease = 1.0 - (-1.0 / (0.001 * self.sr)).exp();
        let frames = left.len().min(right.len());
        for i in 0..frames {
            let b = self.beat + i as f64 * beats_per_sample;
            let unit_now = (b / unit_beats).floor() as i64;
            let dry = [left[i], right[i]];
            // The input goes into the history first, so the newest sample can be read.
            let at = (self.written % len as u64) as usize;
            self.history[0][at] = dry[0];
            self.history[1][at] = dry[1];
            self.written += 1;
            if unit_now != self.unit_index {
                self.start_unit(unit_now);
                // The unit begins at this sample, which is already in the history.
                self.start = self.written - 1;
                self.slice_start = self.start as f64 - self.slice_len;
                self.tape_pos = self.start as f64;
            }
            let in_unit = (b / unit_beats - unit_now as f64) * self.unit_samples;

            let mut glitched = dry;
            if let Some(style) = self.style {
                match style {
                    Style::Stutter | Style::Pitch => {
                        self.phase += self.increment;
                        if self.phase >= self.slice_len {
                            self.phase %= self.slice_len;
                            if self.shrink {
                                // Each repeat is shorter than the last: the stutter speeds up.
                                self.slice_len = (self.slice_len * 0.78).max(self.min_slice);
                            }
                        }
                        glitched = self.read(self.slice_start + self.phase);
                    }
                    Style::Reverse => {
                        glitched = self.read(self.start as f64 - in_unit);
                    }
                    Style::Tape => {
                        let u = (in_unit / self.unit_samples).clamp(0.0, 1.0);
                        let speed = (1.0 - u).powf(1.6);
                        self.tape_pos += speed;
                        glitched = self.read(self.tape_pos);
                    }
                    Style::Gate => {
                        let on = ((in_unit / self.slice_len) as i64) % 2 == 0;
                        self.gate_gain += (if on { 1.0 } else { 0.0 } - self.gate_gain) * gate_ease;
                        glitched = [dry[0] * self.gate_gain, dry[1] * self.gate_gain];
                    }
                    Style::Crush => {
                        self.count -= 1.0;
                        if self.count <= 0.0 {
                            let levels = 2f32.powf(self.bits - 1.0);
                            self.hold = [
                                (dry[0] * levels).round() / levels,
                                (dry[1] * levels).round() / levels,
                            ];
                            self.count += self.divide;
                        }
                        glitched = self.hold;
                    }
                }
            }
            // Away from the unit's edges a glitch is fully in; at the edges it eases out and in.
            let inside =
                self.style.is_some() && in_unit > edge && in_unit < self.unit_samples - edge;
            self.fade += ((if inside { 1.0 } else { 0.0 }) - self.fade) * ease;
            let wet = self.fade * mix;
            left[i] = dry[0] * (1.0 - wet) + glitched[0] * wet;
            right[i] = dry[1] * (1.0 - wet) + glitched[1] * wet;
        }
        self.beat += frames as f64 * beats_per_sample;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;
    /// 120 BPM: a beat is half a second, a sixteenth 6 000 samples.
    const SIXTEENTH: usize = 6_000;

    fn glitch(settings: &[(&str, f32)]) -> Glitch {
        let mut g = Glitch::default();
        g.prepare(f64::from(SR));
        g.sync(120.0, 0.0, true);
        // Everything off, then what the test asks for.
        for name in ["stutter", "reverse", "tape", "gate", "crush", "pitch"] {
            g.set_param(index(name), 0.0);
        }
        g.set_param(index("amount"), 1.0);
        for &(name, value) in settings {
            g.set_param(index(name), value);
        }
        g
    }

    fn index(name: &str) -> usize {
        PARAMS.iter().position(|p| p.name == name).unwrap()
    }

    /// Noise-like but deterministic input, so repeats and reversals can be recognised.
    fn signal(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let x = i as f32;
                (x * 0.031).sin() * 0.4 + (x * 0.173).sin() * 0.3 + (x * 0.0071).cos() * 0.2
            })
            .collect()
    }

    fn run(g: &mut Glitch, input: &[f32], block: usize) -> Vec<f32> {
        let (mut l, mut r) = (input.to_vec(), input.to_vec());
        let mut at = 0;
        while at < input.len() {
            let end = (at + block).min(input.len());
            g.sync(120.0, at as f64 / f64::from(SR) * 2.0, true);
            g.process(&mut l[at..end], &mut r[at..end]);
            at = end;
        }
        assert!(l.iter().chain(&r).all(|x| x.is_finite() && x.abs() < 2.0));
        l
    }

    #[test]
    fn no_amount_or_no_mix_leaves_the_sound_alone() {
        let input = signal(96_000);
        assert_eq!(
            run(
                &mut glitch(&[("stutter", 1.0), ("amount", 0.0)]),
                &input,
                512
            ),
            input
        );
        assert_eq!(
            run(&mut glitch(&[("stutter", 1.0), ("mix", 0.0)]), &input, 512),
            input
        );
        // With nothing switched on there's nothing to do either.
        assert_eq!(run(&mut glitch(&[]), &input, 512), input);
    }

    #[test]
    fn the_result_is_the_same_whatever_the_block_size_and_follows_the_seed() {
        let input = signal(192_000);
        let all = [
            ("stutter", 1.0),
            ("reverse", 1.0),
            ("tape", 1.0),
            ("gate", 1.0),
            ("crush", 1.0),
            ("pitch", 1.0),
            ("amount", 0.7),
        ];
        let reference = run(&mut glitch(&all), &input, 512);
        for block in [64, 777, 4096] {
            let other = run(&mut glitch(&all), &input, block);
            let worst = reference
                .iter()
                .zip(&other)
                .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
            assert!(worst < 1e-4, "block size {block}: differs by {worst}");
        }
        assert_ne!(reference, input, "something happened");
        let mut seeded = all.to_vec();
        seeded.push(("seed", 7.0));
        assert_ne!(
            run(&mut glitch(&seeded), &input, 512),
            reference,
            "another seed, another pattern"
        );
        // The same seed twice is the same pattern.
        assert_eq!(
            run(&mut glitch(&seeded), &input, 512),
            run(&mut glitch(&seeded), &input, 512)
        );
    }

    #[test]
    fn a_stutter_repeats_a_slice() {
        // Only stutters, every sixteenth, 1/32 slices (3 000 samples).
        let input = signal(96_000);
        let mut g = glitch(&[("stutter", 1.0), ("rate", 2.0), ("slice", 1.0)]);
        let out = run(&mut g, &input, 512);
        // Look inside a glitched sixteenth, away from its edges and the first (history-less) ones.
        let mut found = false;
        for unit in 3..14 {
            // The fade-in takes about 10 ms to settle, so start well inside the unit.
            let from = unit * SIXTEENTH + 1_500;
            let to = (unit + 1) * SIXTEENTH - 300;
            let region = &out[from..to];
            // A repeated slice has a period that's a simple fraction of 3 000 samples.
            for period in [1_500usize, 3_000, 6_000] {
                if period + 200 < region.len()
                    && region[..region.len() - period]
                        .iter()
                        .zip(&region[period..])
                        .all(|(a, b)| (a - b).abs() < 1e-3)
                {
                    found = true;
                }
            }
        }
        assert!(found, "a stutter loops its slice");
        assert_ne!(out, input);
    }

    #[test]
    fn a_reversed_unit_plays_the_recent_past_backwards() {
        let input = signal(96_000);
        let mut g = glitch(&[("reverse", 1.0), ("rate", 1.0)]); // every eighth: 12 000 samples
        let out = run(&mut g, &input, 512);
        let unit = 12_000usize;
        let mut checked = 0;
        for u in 2..7 {
            let start = u * unit;
            // Away from the edges: out[start + k] is the input k samples before the start.
            for k in [3_000usize, 5_000, 8_000] {
                let (got, expected) = (out[start + k], input[start - k]);
                if (got - expected).abs() < 2e-3 {
                    checked += 1;
                }
            }
        }
        assert!(
            checked >= 12,
            "most probes matched the reversed past ({checked}/15)"
        );
    }

    #[test]
    fn a_tape_stop_drops_in_pitch_and_a_gate_makes_silences() {
        let tone: Vec<f32> = (0..96_000)
            .map(|i| (i as f32 / SR * 440.0 * std::f32::consts::TAU).sin() * 0.6)
            .collect();
        let crossings = |b: &[f32]| b.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        let mut g = glitch(&[("tape", 1.0), ("rate", 0.0)]); // a quarter note: 24 000 samples
        let out = run(&mut g, &tone, 512);
        let unit = 24_000;
        let (early, late) = (
            &out[unit + 2_000..unit + 8_000],
            &out[unit + 14_000..unit + 20_000],
        );
        assert!(
            crossings(late) * 2 < crossings(early),
            "{} then {}",
            crossings(early),
            crossings(late)
        );

        let mut gated = glitch(&[("gate", 1.0), ("rate", 1.0), ("slice", 1.0)]);
        let out = run(&mut gated, &tone, 512);
        let silent = out[12_000..60_000]
            .windows(300)
            .filter(|w| w.iter().all(|x| x.abs() < 0.01))
            .count();
        assert!(silent > 1_000, "the gate closes: {silent}");
    }

    #[test]
    fn a_crush_has_few_levels_and_all_styles_stay_in_bounds() {
        let input = signal(96_000);
        let mut g = glitch(&[("crush", 1.0), ("rate", 1.0)]);
        let out = run(&mut g, &input, 512);
        // (Away from the edges, where the glitch eases in and out.)
        let mut levels: Vec<i32> = out[25_500..35_700]
            .iter()
            .map(|x| (x * 1000.0).round() as i32)
            .collect();
        levels.sort_unstable();
        levels.dedup();
        let dry_levels = {
            let mut l: Vec<i32> = input[25_500..35_700]
                .iter()
                .map(|x| (x * 1000.0).round() as i32)
                .collect();
            l.sort_unstable();
            l.dedup();
            l.len()
        };
        assert!(
            levels.len() < dry_levels / 4,
            "{} levels vs {}",
            levels.len(),
            dry_levels
        );
        // Every style at once, loud input, fast rate: finite and bounded (run() checks).
        let loud: Vec<f32> = signal(48_000).iter().map(|x| x * 2.0).collect();
        let all = [
            ("stutter", 1.0),
            ("reverse", 1.0),
            ("tape", 1.0),
            ("gate", 1.0),
            ("crush", 1.0),
            ("pitch", 1.0),
            ("rate", 3.0),
            ("length", 4.0),
        ];
        run(&mut glitch(&all), &loud, 300);
    }

    #[test]
    fn it_follows_the_song_position_and_runs_freely_when_stopped() {
        let input = signal(48_000);
        // Jumping to a position gives the pattern for that position.
        let mut a = glitch(&[("stutter", 1.0), ("reverse", 1.0), ("amount", 0.6)]);
        let mut b = glitch(&[("stutter", 1.0), ("reverse", 1.0), ("amount", 0.6)]);
        let (mut l1, mut r1) = (input.clone(), input.clone());
        a.sync(120.0, 8.0, true);
        a.process(&mut l1, &mut r1);
        let (mut l2, mut r2) = (input.clone(), input.clone());
        b.sync(120.0, 8.0, true);
        b.process(&mut l2, &mut r2);
        assert_eq!(l1, l2);
        // Stopped: the beat doesn't jump, but it keeps counting.
        let before = a.beat;
        a.sync(120.0, 100.0, false);
        assert_eq!(a.beat, before);
        let (mut l, mut r) = (input.clone(), input.clone());
        a.process(&mut l, &mut r);
        assert!(a.beat > before);
    }
}
