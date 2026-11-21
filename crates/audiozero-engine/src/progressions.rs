//! The chord progression library: 100 progressions for each of the drum rack's nine genres, and
//! each genre's usual sound, pattern and bass line.
//!
//! The progressions are in `progressions.json`, made by `scripts/chords/make_progressions.py`.
//! Each is written in roman numerals relative to the key ("i:8 bVI:8 bIII:4 bVII:4": a chord
//! and how many beats it lasts), so it plays in whatever key the song is in.

use crate::chords::{BassStyle, Comp, Mood, Numeral, Pattern, Step, Voicing, comp_clip};
use crate::music::Scale;
use crate::project::Clip;
use std::sync::LazyLock;

/// One progression.
#[derive(Debug, Clone, PartialEq)]
pub struct Progression {
    /// "techno-001": the genre (a drum kit's id) and a number.
    pub id: String,
    pub genre: String,
    /// The numerals, as in "i – bVI – bIII – bVII".
    pub name: String,
    /// A word for how it sounds ("Dark", "Mellow", "Classic"...), for grouping in menus.
    pub tag: String,
    pub mood: Mood,
    pub steps: Vec<(Numeral, f64)>,
}

impl Progression {
    /// How many beats one pass takes.
    pub fn beats(&self) -> f64 {
        self.steps.iter().map(|s| s.1).sum()
    }

    /// The chords in `key`.
    pub fn steps_in(&self, key: u8) -> Vec<Step> {
        self.steps
            .iter()
            .map(|&(numeral, beats)| Step {
                chord: numeral.resolve(key),
                beats,
            })
            .collect()
    }

    /// The progression as a clip of notes in `key`, `repeats` times over, from `start`.
    pub fn clip(
        &self,
        key: u8,
        repeats: usize,
        start: f64,
        beats_per_bar: u32,
        comp: Comp,
    ) -> Clip {
        comp_clip(
            &self.name,
            &self.steps_in(key),
            repeats,
            start,
            beats_per_bar,
            comp,
        )
    }
}

fn parse(entry: &serde_json::Value) -> Option<Progression> {
    let text = |k: &str| entry.get(k)?.as_str().map(str::to_owned);
    let mut steps = Vec::new();
    for part in entry.get("chords")?.as_str()?.split_whitespace() {
        let (numeral, beats) = part.split_once(':')?;
        let beats: f64 = beats
            .parse()
            .ok()
            .filter(|b: &f64| *b > 0.0 && *b <= 64.0)?;
        steps.push((Numeral::parse(numeral).ok()?, beats));
    }
    if steps.is_empty() || steps.len() > 16 {
        return None;
    }
    Some(Progression {
        id: text("id")?,
        genre: text("genre")?,
        name: text("name")?,
        tag: text("tag")?,
        mood: match entry.get("mood")?.as_str()? {
            "major" => Mood::Major,
            "minor" => Mood::Minor,
            _ => return None,
        },
        steps,
    })
}

/// Every progression, genre by genre.
pub fn all() -> &'static [Progression] {
    static ALL: LazyLock<Vec<Progression>> = LazyLock::new(|| {
        serde_json::from_str::<Vec<serde_json::Value>>(include_str!("progressions.json"))
            .unwrap_or_default()
            .iter()
            .filter_map(parse)
            .collect()
    });
    &ALL
}

pub fn by_id(id: &str) -> Option<&'static Progression> {
    all().iter().find(|p| p.id == id)
}

/// The progressions of a genre (a drum kit's id, such as "lofi").
pub fn for_genre(genre: &str) -> impl Iterator<Item = &'static Progression> {
    all().iter().filter(move |p| p.genre == genre)
}

/// The tags used in a genre, most common first.
pub fn tags_for(genre: &str) -> Vec<&'static str> {
    let mut counts: Vec<(&'static str, usize)> = Vec::new();
    for p in for_genre(genre) {
        match counts.iter_mut().find(|(t, _)| *t == p.tag) {
            Some(c) => c.1 += 1,
            None => counts.push((p.tag.as_str(), 1)),
        }
    }
    counts.sort_by_key(|c| std::cmp::Reverse(c.1));
    counts.into_iter().map(|c| c.0).collect()
}

/// Whether a progression suits a song in `scale`: major progressions for major-sounding scales,
/// minor ones for minor-sounding scales.
pub fn suits(progression: &Progression, scale: Scale) -> bool {
    progression.mood == Mood::of_scale(scale)
}

/// How a genre's chords are usually played.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GenreStyle {
    pub pattern: Pattern,
    pub voicing: Voicing,
    /// The name of the chord pad sound ([`crate::chord_pads::SOUNDS`]).
    pub sound: &'static str,
    pub bass: BassStyle,
    /// A Soundgineer preset for the bass.
    pub bass_sound: &'static str,
}

/// A genre's usual way of playing chords (a drum kit's id); anything else gets a neutral style.
pub fn genre_style(genre: &str) -> GenreStyle {
    let (pattern, voicing, sound, bass, bass_sound) = match genre {
        "techno" => (
            Pattern::Offbeat,
            Voicing::Root,
            "Rave stab",
            BassStyle::Offbeat,
            "Acid Squelch",
        ),
        "house" => (
            Pattern::Offbeat,
            Voicing::Smooth,
            "Organ",
            BassStyle::Offbeat,
            "Pulse Sub",
        ),
        "dubstep" => (
            Pattern::Block,
            Voicing::Open,
            "Dark pad",
            BassStyle::Sub808,
            "Wobble Bass",
        ),
        "hiphop" => (
            Pattern::Syncopated,
            Voicing::Smooth,
            "Rhodes",
            BassStyle::Syncopated,
            "Rumble Sine",
        ),
        "lofi" => (
            Pattern::Strum,
            Voicing::Smooth,
            "Lo-fi keys",
            BassStyle::RootFifth,
            "Rumble Sine",
        ),
        "trap" => (
            Pattern::ArpUp,
            Voicing::Open,
            "Ice pluck",
            BassStyle::Sub808,
            "808 Drop",
        ),
        "dnb" => (
            Pattern::Block,
            Voicing::Smooth,
            "Strings",
            BassStyle::Rolling,
            "Reese Bass",
        ),
        "halftime" => (
            Pattern::Block,
            Voicing::Open,
            "Warm pad",
            BassStyle::Sub808,
            "Reese Bass",
        ),
        "jazz" => (
            Pattern::Syncopated,
            Voicing::Smooth,
            "Rhodes",
            BassStyle::Walking,
            "Rumble Sine",
        ),
        _ => (
            Pattern::Block,
            Voicing::Smooth,
            "Warm pad",
            BassStyle::Root,
            "Deep Saw Bass",
        ),
    };
    GenreStyle {
        pattern,
        voicing,
        sound,
        bass,
        bass_sound,
    }
}

/// A Soundgineer preset that suits a bass style, for a bass line added without a genre.
pub fn bass_preset(style: BassStyle) -> &'static str {
    match style {
        BassStyle::Root => "Deep Saw Bass",
        BassStyle::RootFifth => "Solid Square",
        BassStyle::Octave => "Solid Square",
        BassStyle::Offbeat => "Pulse Sub",
        BassStyle::Walking => "Rumble Sine",
        BassStyle::Sub808 => "808 Drop",
        BassStyle::Rolling => "Reese Bass",
        BassStyle::Syncopated => "Deep Saw Bass",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chord_pads::SOUNDS;
    use crate::drums::KITS;
    use crate::music::scale_by_id;
    use std::collections::HashSet;

    #[test]
    fn every_genre_has_a_hundred_valid_unique_progressions() {
        let raw: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("progressions.json")).unwrap();
        assert_eq!(raw.len(), 900);
        assert_eq!(all().len(), 900, "every entry parses");
        let ids: HashSet<_> = all().iter().map(|p| &p.id).collect();
        assert_eq!(ids.len(), 900, "ids are unique");
        for kit in &KITS {
            let list: Vec<_> = for_genre(kit.id).collect();
            assert_eq!(list.len(), 100, "{}", kit.id);
            let distinct: HashSet<_> = list
                .iter()
                .map(|p| {
                    p.steps
                        .iter()
                        .map(|(n, b)| (*n, b.to_bits()))
                        .collect::<Vec<_>>()
                })
                .collect();
            assert_eq!(distinct.len(), 100, "{} has no repeats", kit.id);
            for (i, p) in list.iter().enumerate() {
                assert_eq!(p.id, format!("{}-{:03}", kit.id, i + 1));
                assert!(!p.name.is_empty() && !p.tag.is_empty());
                let beats = p.beats();
                assert!(
                    (4.0..=64.0).contains(&beats),
                    "{} lasts {beats} beats",
                    p.id
                );
                assert_eq!(beats % 4.0, 0.0, "{} fills whole bars", p.id);
            }
            assert!(tags_for(kit.id).len() >= 4, "{} has some variety", kit.id);
        }
        assert!(by_id("jazz-001").is_some() && by_id("nope").is_none());
    }

    #[test]
    fn progressions_play_in_any_key() {
        let p = by_id("lofi-003").unwrap();
        let in_c: Vec<String> = p.steps_in(0).iter().map(|s| s.chord.name()).collect();
        let in_d: Vec<String> = p.steps_in(2).iter().map(|s| s.chord.name()).collect();
        assert_ne!(in_c, in_d);
        assert_eq!(p.steps_in(0).len(), p.steps_in(7).len());
        // A clip has real notes, whole bars, and starts where asked.
        let clip = p.clip(5, 2, 16.0, 4, Comp::default());
        assert_eq!(clip.start, 16.0);
        assert_eq!(clip.length % 4.0, 0.0);
        assert!(!clip.notes.is_empty());
        assert_eq!(clip.name, p.name);
    }

    #[test]
    fn the_mood_matches_the_scale_and_the_style_is_known() {
        let minor_ones = all().iter().filter(|p| p.mood == Mood::Minor).count();
        assert!(
            (300..700).contains(&minor_ones),
            "{minor_ones} of 900 are minor"
        );
        let a_minor_one = all().iter().find(|p| p.mood == Mood::Minor).unwrap();
        assert!(
            suits(a_minor_one, scale_by_id("minor")) && !suits(a_minor_one, scale_by_id("major"))
        );
        assert!(suits(a_minor_one, scale_by_id("blues")));
        for kit in &KITS {
            let style = genre_style(kit.id);
            assert!(SOUNDS.iter().any(|s| s.0 == style.sound), "{}", kit.id);
            assert!(
                crate::soundgineer::preset(style.bass_sound).is_some(),
                "{}",
                kit.id
            );
        }
        for style in BassStyle::ALL {
            assert!(crate::soundgineer::preset(bass_preset(style)).is_some());
        }
    }
}
