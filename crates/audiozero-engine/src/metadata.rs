//! Song info for exported files: the credits and identifiers a music release carries.
//!
//! The fields follow what distributors and the standards ask for: ISRC (ISO 3901), UPC/EAN
//! (GTIN), ISWC (ISO 15707), composer, lyricist, producer, label, the © and ℗ lines, language
//! (ISO 639-2), release date (ISO 8601) and an explicit-content flag. They're kept in the project,
//! so a song remembers its own credits, and written into exported WAV and MP3 files by
//! [`crate::tags`]. Tempo and key come from the project itself.
//!
//! [`Metadata::sanitize`] makes any text safe to store and write (no control characters, bounded
//! length, identifiers normalised); [`Metadata::validate`] says what's wrong with an identifier
//! instead of quietly writing a bad one.

use serde::{Deserialize, Serialize};

/// The longest a one-line field can be, in characters.
pub const MAX_FIELD_CHARS: usize = 200;
/// The longest the comment can be, in characters.
pub const MAX_COMMENT_CHARS: usize = 1000;

/// Whether the song has explicit lyrics, as distributors ask for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Explicit {
    /// Not said either way: nothing is written.
    #[default]
    Unspecified,
    NotExplicit,
    Explicit,
    /// A clean version of an explicit song.
    Clean,
}

impl Explicit {
    pub const ALL: [Explicit; 4] = [
        Explicit::Unspecified,
        Explicit::NotExplicit,
        Explicit::Explicit,
        Explicit::Clean,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Explicit::Unspecified => "Not set",
            Explicit::NotExplicit => "Not explicit",
            Explicit::Explicit => "Explicit",
            Explicit::Clean => "Clean version",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Metadata {
    pub title: String,
    pub artist: String,
    pub album_artist: String,
    pub album: String,
    /// 0 when not set.
    pub track_number: u32,
    pub track_total: u32,
    /// `YYYY`, `YYYY-MM` or `YYYY-MM-DD`.
    pub release_date: String,
    pub genre: String,
    pub composer: String,
    pub lyricist: String,
    pub producer: String,
    /// Mixed by.
    pub mixer: String,
    /// The record label (or publisher).
    pub label: String,
    /// International Standard Recording Code, `CCXXXYYNNNNN` (hyphens are fine when typing).
    pub isrc: String,
    /// The release's barcode: UPC (12 digits) or EAN (13).
    pub upc: String,
    /// International Standard Musical Work Code, `T-123.456.789-C`.
    pub iswc: String,
    /// The © line: the year and owner of the composition, like `2026 Jane Doe`.
    pub copyright: String,
    /// The ℗ line: the year and owner of the recording, like `2026 Jane Doe Records`.
    pub phonogram: String,
    /// Three letters, ISO 639-2 (`eng`, `spa`, `jpn`).
    pub language: String,
    pub explicit: Explicit,
    pub comment: String,
    /// Write the song's tempo and key too.
    pub include_tempo_and_key: bool,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            title: String::new(),
            artist: String::new(),
            album_artist: String::new(),
            album: String::new(),
            track_number: 0,
            track_total: 0,
            release_date: String::new(),
            genre: String::new(),
            composer: String::new(),
            lyricist: String::new(),
            producer: String::new(),
            mixer: String::new(),
            label: String::new(),
            isrc: String::new(),
            upc: String::new(),
            iswc: String::new(),
            copyright: String::new(),
            phonogram: String::new(),
            language: String::new(),
            explicit: Explicit::Unspecified,
            comment: String::new(),
            include_tempo_and_key: true,
        }
    }
}

/// Facts that come from the project and the app, not from the user.
#[derive(Debug, Clone, PartialEq)]
pub struct Auto {
    pub bpm: f64,
    /// The key as ID3 writes it (`Am`, `C#`), when the scale is a plain major or minor one.
    pub key: Option<String>,
    pub software: String,
    /// When it was made, UTC.
    pub when: Utc,
}

/// A UTC date and time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Utc {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl Utc {
    /// The current time. (Days since 1970 to a calendar date: Howard Hinnant's algorithm.)
    pub fn now() -> Self {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Self::from_unix(secs)
    }

    pub fn from_unix(secs: u64) -> Self {
        let days = (secs / 86_400) as i64;
        let rest = secs % 86_400;
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let year = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        Self {
            year: (year + i64::from(month <= 2)) as i32,
            month,
            day,
            hour: (rest / 3600) as u32,
            minute: (rest % 3600 / 60) as u32,
            second: (rest % 60) as u32,
        }
    }

    /// `2026-11-15T21:30:00`, the ISO 8601 form ID3 uses.
    pub fn iso(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

impl Auto {
    /// The automatic facts for `project`, made by `software`.
    pub fn for_project(project: &crate::project::Project, software: &str) -> Self {
        let minor = matches!(
            project.scale.as_str(),
            "minor" | "harmonic_minor" | "melodic_minor" | "minor_pentatonic"
        );
        let major = matches!(project.scale.as_str(), "major" | "major_pentatonic");
        let key = (major || minor).then(|| {
            format!(
                "{}{}",
                crate::music::KEYS[usize::from(project.key % 12)],
                if minor { "m" } else { "" }
            )
        });
        Self {
            bpm: project.bpm,
            key,
            software: software.to_string(),
            when: Utc::now(),
        }
    }
}

/// Removes characters that don't belong in a tag: control characters (newlines too, unless
/// `multiline`) and the invisible direction and zero-width marks that can disguise text.
fn clean(text: &str, max: usize, multiline: bool) -> String {
    let mut out = String::with_capacity(text.len().min(max * 4));
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let c = match c {
            // A Windows line break (\r\n) is one line break, not two.
            '\r' if multiline => {
                if chars.peek() == Some(&'\n') {
                    continue;
                }
                '\n'
            }
            '\n' if multiline => '\n',
            '\t' | '\n' | '\r' => ' ',
            c if c.is_control() => continue,
            '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}' => {
                continue;
            }
            c => c,
        };
        out.push(c);
        if out.chars().count() >= max {
            break;
        }
    }
    out.trim().to_string()
}

/// Drops a leading ©, ℗, "(c)" or "(p)" from a copyright line, so the year comes first as ID3
/// wants it.
fn without_symbol(line: &str) -> String {
    let t = line.trim_start();
    let lower = t.to_lowercase();
    let rest = if let Some(r) = t.strip_prefix(['©', '℗']) {
        r
    } else if lower.starts_with("(c)") || lower.starts_with("(p)") {
        &t[3..]
    } else {
        t
    };
    rest.trim_start().to_string()
}

impl Metadata {
    /// Whether nothing has been filled in (the tempo-and-key switch doesn't count).
    pub fn is_empty(&self) -> bool {
        let blank = Self::default();
        Self {
            include_tempo_and_key: true,
            ..self.clone()
        } == blank
    }

    /// Makes every field safe to store and write: no control or invisible characters, bounded
    /// lengths, identifiers in their plain form. It never refuses; [`Metadata::validate`] says
    /// what's wrong with what's left.
    pub fn sanitize(&mut self) {
        for field in [
            &mut self.title,
            &mut self.artist,
            &mut self.album_artist,
            &mut self.album,
            &mut self.genre,
            &mut self.composer,
            &mut self.lyricist,
            &mut self.producer,
            &mut self.mixer,
            &mut self.label,
        ] {
            *field = clean(field, MAX_FIELD_CHARS, false);
        }
        self.comment = clean(&self.comment, MAX_COMMENT_CHARS, true);
        self.release_date = clean(&self.release_date, 24, false);
        self.copyright = without_symbol(&clean(&self.copyright, MAX_FIELD_CHARS, false));
        self.phonogram = without_symbol(&clean(&self.phonogram, MAX_FIELD_CHARS, false));
        self.isrc = clean(&self.isrc, 20, false)
            .chars()
            .filter(|c| !matches!(c, '-' | ' '))
            .collect::<String>()
            .to_uppercase();
        self.upc = clean(&self.upc, 20, false)
            .chars()
            .filter(char::is_ascii_digit)
            .collect();
        self.iswc = clean(&self.iswc, 20, false)
            .chars()
            .filter(|c| !matches!(c, '-' | ' ' | '.'))
            .collect::<String>()
            .to_uppercase();
        self.language = clean(&self.language, 24, false).to_lowercase();
        self.track_number = self.track_number.min(9999);
        self.track_total = self.track_total.min(9999);
    }

    /// The first thing wrong with the identifiers and dates, in plain words.
    pub fn validate(&self) -> Result<(), String> {
        if !self.isrc.is_empty() && !valid_isrc(&self.isrc) {
            return Err(format!(
                "\"{}\" isn't an ISRC. It's 12 characters: 2 letters for the country, 3 for \
                 the registrant, 2 digits for the year and 5 digits (like US-RC1-26-00001).",
                self.isrc
            ));
        }
        if !self.upc.is_empty() && !valid_gtin(&self.upc) {
            return Err(
                "The UPC/EAN isn't a valid barcode: it has to be 12 or 13 digits with \
                        a correct check digit."
                    .into(),
            );
        }
        if !self.iswc.is_empty() && !valid_iswc(&self.iswc) {
            return Err(format!(
                "\"{}\" isn't an ISWC. It's a T and 10 digits, like T-034.524.680-1.",
                self.iswc
            ));
        }
        if !self.release_date.is_empty() && !valid_date(&self.release_date) {
            return Err("The release date has to be YYYY, YYYY-MM or YYYY-MM-DD.".into());
        }
        if !self.language.is_empty()
            && !(self.language.len() == 3 && self.language.chars().all(|c| c.is_ascii_lowercase()))
        {
            return Err("The language is three letters, like eng, spa or jpn.".into());
        }
        for (name, line) in [("©", &self.copyright), ("℗", &self.phonogram)] {
            if !line.is_empty() && !starts_with_year(line) {
                return Err(format!(
                    "The {name} line starts with the year, then the owner: 2026 Jane Doe."
                ));
            }
        }
        if self.track_total != 0 && self.track_number > self.track_total {
            return Err("The track number is higher than the number of tracks.".into());
        }
        Ok(())
    }
}

fn starts_with_year(line: &str) -> bool {
    let b = line.as_bytes();
    b.len() >= 5 && b[..4].iter().all(u8::is_ascii_digit) && b[4] == b' '
}

/// ISO 3901: two letters, three letters or digits, two digits, five digits.
pub fn valid_isrc(isrc: &str) -> bool {
    let b = isrc.as_bytes();
    b.len() == 12
        && b[..2].iter().all(u8::is_ascii_uppercase)
        && b[2..5]
            .iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && b[5..].iter().all(u8::is_ascii_digit)
}

/// A UPC-A (12 digits) or EAN-13 with its check digit right.
pub fn valid_gtin(code: &str) -> bool {
    let digits: Vec<u32> = code.chars().filter_map(|c| c.to_digit(10)).collect();
    if digits.len() != code.len() || !(digits.len() == 12 || digits.len() == 13) {
        return false;
    }
    // From the right, skipping the check digit: weights 3, 1, 3, 1, ...
    let sum: u32 = digits[..digits.len() - 1]
        .iter()
        .rev()
        .enumerate()
        .map(|(i, d)| if i % 2 == 0 { d * 3 } else { *d })
        .sum();
    (10 - sum % 10) % 10 == digits[digits.len() - 1]
}

/// ISO 15707: a T, nine digits and a check digit (1 + the digits weighted 1 to 9, mod 10).
pub fn valid_iswc(iswc: &str) -> bool {
    let Some(rest) = iswc.strip_prefix('T') else {
        return false;
    };
    let digits: Vec<u32> = rest.chars().filter_map(|c| c.to_digit(10)).collect();
    if digits.len() != 10 || rest.len() != 10 {
        return false;
    }
    let sum: u32 = 1 + digits[..9]
        .iter()
        .enumerate()
        .map(|(i, d)| (i as u32 + 1) * d)
        .sum::<u32>();
    (10 - sum % 10) % 10 == digits[9]
}

fn valid_date(date: &str) -> bool {
    let parts: Vec<&str> = date.split('-').collect();
    let number = |s: &str, len: usize| {
        (s.len() == len && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse::<u32>().ok())
    };
    let (year, month, day) = match parts.as_slice() {
        [y] => (number(y, 4), Some(Some(1)), Some(Some(1))),
        [y, m] => (number(y, 4), number(m, 2), Some(Some(1))),
        [y, m, d] => (number(y, 4), number(m, 2), number(d, 2)),
        _ => return false,
    };
    let (Some(Some(y)), Some(Some(m)), Some(Some(d))) = (year, month, day) else {
        return false;
    };
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        _ => return false,
    };
    (1900..=2200).contains(&y) && (1..=days).contains(&d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_checked_against_their_standards() {
        assert!(valid_isrc("USRC12600001"));
        assert!(
            !valid_isrc("US-RC1-26-00001"),
            "sanitize strips hyphens first"
        );
        assert!(!valid_isrc("USRC1260000"), "too short");
        assert!(!valid_isrc("usrc12600001"), "lower case");
        assert!(!valid_isrc("1SRC12600001"), "country is letters");
        // GTIN check digits: a real UPC-A and EAN-13.
        assert!(valid_gtin("036000291452"));
        assert!(!valid_gtin("036000291453"));
        assert!(valid_gtin("4006381333931"));
        assert!(!valid_gtin("12345"));
        // ISWC: T-034.524.680-1 is the standard's own example.
        assert!(valid_iswc("T0345246801"));
        assert!(!valid_iswc("T0345246802"));
        assert!(!valid_iswc("0345246801"));
    }

    #[test]
    fn dates_are_real_dates() {
        for ok in ["2026", "2026-11", "2026-11-15", "2024-02-29"] {
            assert!(valid_date(ok), "{ok}");
        }
        for bad in [
            "26",
            "2026-13",
            "2026-02-30",
            "2025-02-29",
            "2026-1-5",
            "2026/11/15",
            "",
        ] {
            assert!(!valid_date(bad), "{bad}");
        }
    }

    #[test]
    fn sanitize_cleans_text_and_normalises_identifiers() {
        let mut m = Metadata {
            title: "  Song\u{0}\ttitle\n".into(),
            artist: "A\u{202E}B".into(),
            isrc: "us-rc1-26-00001".into(),
            upc: "0 36000-291452".into(),
            iswc: "t-034.524.680-1".into(),
            copyright: "© 2026 Jane".into(),
            phonogram: "(P) 2026 Jane Records".into(),
            language: "ENG".into(),
            comment: "line one\r\nline two\u{7}".into(),
            track_number: 99999,
            ..Default::default()
        };
        m.sanitize();
        assert_eq!(m.title, "Song title");
        assert_eq!(m.artist, "AB", "direction overrides are dropped");
        assert_eq!(
            (m.isrc.as_str(), m.upc.as_str(), m.iswc.as_str()),
            ("USRC12600001", "036000291452", "T0345246801")
        );
        assert_eq!(
            (m.copyright.as_str(), m.phonogram.as_str()),
            ("2026 Jane", "2026 Jane Records")
        );
        assert_eq!(m.language, "eng");
        assert_eq!(m.comment, "line one\nline two");
        assert_eq!(m.track_number, 9999);
        assert!(m.validate().is_ok());
    }

    #[test]
    fn sanitize_bounds_the_length() {
        let mut m = Metadata {
            title: "x".repeat(100_000),
            comment: "y".repeat(100_000),
            ..Default::default()
        };
        m.sanitize();
        assert_eq!(m.title.chars().count(), MAX_FIELD_CHARS);
        assert_eq!(m.comment.chars().count(), MAX_COMMENT_CHARS);
    }

    #[test]
    fn validation_says_what_is_wrong() {
        let bad = |f: &dyn Fn(&mut Metadata)| {
            let mut m = Metadata::default();
            f(&mut m);
            m.sanitize();
            m.validate().unwrap_err()
        };
        assert!(bad(&|m| m.isrc = "nope".into()).contains("ISRC"));
        assert!(bad(&|m| m.upc = "123".into()).contains("barcode"));
        assert!(bad(&|m| m.iswc = "T1".into()).contains("ISWC"));
        assert!(bad(&|m| m.release_date = "soon".into()).contains("release date"));
        assert!(bad(&|m| m.language = "english".into()).contains("three letters"));
        assert!(bad(&|m| m.copyright = "Jane".into()).contains("year"));
        assert!(bad(&|m| (m.track_number, m.track_total) = (5, 3)).contains("track number"));
        assert!(Metadata::default().validate().is_ok());
    }

    #[test]
    fn time_converts_from_unix_seconds() {
        assert_eq!(Utc::from_unix(0).iso(), "1970-01-01T00:00:00");
        assert_eq!(Utc::from_unix(1_700_000_000).iso(), "2023-11-14T22:13:20");
        assert_eq!(
            Utc::from_unix(951_782_400).iso(),
            "2000-02-29T00:00:00",
            "a leap day"
        );
    }

    #[test]
    fn empty_ignores_the_tempo_switch() {
        let mut m = Metadata::default();
        assert!(m.is_empty());
        m.include_tempo_and_key = false;
        assert!(m.is_empty());
        m.title = "x".into();
        assert!(!m.is_empty());
    }
}
