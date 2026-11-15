//! Writing song info into audio files.
//!
//! - **MP3:** an ID3v2.4 tag at the start of the file, UTF-8, with the standard frames: `TIT2`
//!   title, `TPE1` artist, `TPE2` album artist, `TALB` album, `TRCK` track, `TDRC` date, `TCON`
//!   genre, `TCOM` composer, `TEXT` lyricist, `TPUB` label, `TCOP` © line, `TPRO` ℗ line, `TSRC`
//!   ISRC, `TBPM` tempo, `TKEY` key, `TLAN` language, `TIPL` producer and mixer, `COMM` comment,
//!   `TSSE`/`TDEN` encoder and time. Identifiers with no standard frame (barcode, ISWC, the
//!   explicit flag) use `TXXX` the way MusicBrainz Picard and iTunes do.
//! - **WAV:** the same ID3 tag in an `id3 ` chunk (read by foobar2000, Picard, mutagen, Reaper
//!   and most tag editors), a `LIST`/`INFO` chunk (the baseline that Windows and many players
//!   read), and an EBU Tech 3285 `bext` chunk (what broadcast and archive tools read).
//!
//! Every text value is sanitised first ([`Metadata::sanitize`]), and sizes are checked, so a
//! hostile or silly value can't produce a malformed file.

use crate::metadata::{Auto, Explicit, Metadata};

/// Frame and chunk payloads are limited to what a syncsafe 28-bit size can say.
const MAX_FRAME: usize = (1 << 28) - 1;

fn syncsafe(n: usize) -> [u8; 4] {
    [
        ((n >> 21) & 0x7F) as u8,
        ((n >> 14) & 0x7F) as u8,
        ((n >> 7) & 0x7F) as u8,
        (n & 0x7F) as u8,
    ]
}

/// One ID3v2.4 frame.
fn frame(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(10 + body.len());
    out.extend_from_slice(id);
    out.extend_from_slice(&syncsafe(body.len().min(MAX_FRAME)));
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(body);
    out
}

/// UTF-8 (encoding 3) text, with any stray NUL removed.
fn utf8(text: &str) -> Vec<u8> {
    let mut v = vec![3u8];
    v.extend(text.bytes().filter(|&b| b != 0));
    v
}

fn text_frame(id: &[u8; 4], text: &str) -> Option<Vec<u8>> {
    (!text.is_empty()).then(|| frame(id, &utf8(text)))
}

fn txxx(description: &str, value: &str) -> Vec<u8> {
    let mut body = vec![3u8];
    body.extend(description.bytes().filter(|&b| b != 0));
    body.push(0);
    body.extend(value.bytes().filter(|&b| b != 0));
    frame(b"TXXX", &body)
}

/// The ID3v2.4 tag for `meta`: empty when there's nothing to say.
pub fn id3v24(meta: &Metadata, auto: &Auto) -> Vec<u8> {
    let mut frames: Vec<Vec<u8>> = Vec::new();
    let mut add = |f: Option<Vec<u8>>| frames.extend(f);
    add(text_frame(b"TIT2", &meta.title));
    add(text_frame(b"TPE1", &meta.artist));
    add(text_frame(b"TPE2", &meta.album_artist));
    add(text_frame(b"TALB", &meta.album));
    if meta.track_number > 0 {
        let track = if meta.track_total > 0 {
            format!("{}/{}", meta.track_number, meta.track_total)
        } else {
            meta.track_number.to_string()
        };
        add(text_frame(b"TRCK", &track));
    }
    add(text_frame(b"TDRC", &meta.release_date));
    add(text_frame(b"TCON", &meta.genre));
    add(text_frame(b"TCOM", &meta.composer));
    add(text_frame(b"TEXT", &meta.lyricist));
    add(text_frame(b"TPUB", &meta.label));
    add(text_frame(b"TCOP", &meta.copyright));
    add(text_frame(b"TPRO", &meta.phonogram));
    add(text_frame(b"TSRC", &meta.isrc));
    add(text_frame(b"TLAN", &meta.language));
    if meta.include_tempo_and_key {
        add(text_frame(b"TBPM", &format!("{}", auto.bpm.round() as i64)));
        if let Some(key) = &auto.key {
            add(text_frame(b"TKEY", key));
        }
    }
    // Involved people: role, then name.
    let mut people = vec![3u8];
    for (role, name) in [("producer", &meta.producer), ("mix", &meta.mixer)] {
        if !name.is_empty() {
            people.extend(role.bytes());
            people.push(0);
            people.extend(name.bytes().filter(|&b| b != 0));
            people.push(0);
        }
    }
    if people.len() > 1 {
        add(Some(frame(b"TIPL", &people)));
    }
    if !meta.upc.is_empty() {
        add(Some(txxx("BARCODE", &meta.upc)));
    }
    if !meta.iswc.is_empty() {
        add(Some(txxx("ISWC", &meta.iswc)));
    }
    match meta.explicit {
        Explicit::Unspecified => {}
        Explicit::NotExplicit => add(Some(txxx("ITUNESADVISORY", "0"))),
        Explicit::Explicit => add(Some(txxx("ITUNESADVISORY", "1"))),
        Explicit::Clean => add(Some(txxx("ITUNESADVISORY", "2"))),
    }
    if !meta.comment.is_empty() {
        let mut body = vec![3u8];
        body.extend_from_slice(b"eng");
        body.push(0); // an empty short description
        body.extend(meta.comment.bytes().filter(|&b| b != 0));
        add(Some(frame(b"COMM", &body)));
    }
    add(text_frame(b"TSSE", &auto.software));
    add(text_frame(b"TDEN", &auto.when.iso()));

    let size: usize = frames.iter().map(Vec::len).sum();
    if size == 0 || size > MAX_FRAME {
        return Vec::new();
    }
    let mut tag = Vec::with_capacity(10 + size);
    tag.extend_from_slice(b"ID3");
    tag.extend_from_slice(&[4, 0, 0]);
    tag.extend_from_slice(&syncsafe(size));
    for f in frames {
        tag.extend(f);
    }
    tag
}

/// A RIFF chunk: id, size, body, and a pad byte if the size is odd.
fn riff_chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(9 + body.len());
    out.extend_from_slice(id);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    if body.len() % 2 == 1 {
        out.push(0);
    }
    out
}

/// ASCII, `?` for anything else, padded with zeros or cut to exactly `len` bytes.
fn ascii_field(text: &str, len: usize) -> Vec<u8> {
    let mut v: Vec<u8> = text
        .chars()
        .map(|c| {
            if c.is_ascii() && !c.is_ascii_control() {
                c as u8
            } else {
                b'?'
            }
        })
        .take(len)
        .collect();
    v.resize(len, 0);
    v
}

/// The EBU Tech 3285 `bext` chunk (version 1): description, originator, date and time, and the
/// coding history.
fn bext(meta: &Metadata, auto: &Auto, sample_rate: u32, bits: u16) -> Vec<u8> {
    let description = match (meta.title.is_empty(), meta.artist.is_empty()) {
        (false, false) => format!("{} - {}", meta.title, meta.artist),
        (false, true) => meta.title.clone(),
        (true, false) => meta.artist.clone(),
        (true, true) => String::new(),
    };
    let when = auto.when;
    let mut body = Vec::new();
    body.extend(ascii_field(&description, 256));
    body.extend(ascii_field(&auto.software, 32)); // originator
    body.extend(ascii_field("", 32)); // originator reference
    body.extend(ascii_field(
        &format!("{:04}-{:02}-{:02}", when.year, when.month, when.day),
        10,
    ));
    body.extend(ascii_field(
        &format!("{:02}-{:02}-{:02}", when.hour, when.minute, when.second),
        8,
    ));
    body.extend_from_slice(&0u64.to_le_bytes()); // time reference
    body.extend_from_slice(&1u16.to_le_bytes()); // BWF version
    body.extend(std::iter::repeat_n(0u8, 64)); // UMID
    body.extend(std::iter::repeat_n(0u8, 190)); // reserved
    let history = format!("A=PCM,F={sample_rate},W={bits},M=stereo,T=AudioZero\r\n");
    body.extend(history.bytes().filter(u8::is_ascii));
    riff_chunk(b"bext", &body)
}

/// A `LIST`/`INFO` chunk with the fields players read.
fn list_info(meta: &Metadata, auto: &Auto) -> Vec<u8> {
    let mut body = b"INFO".to_vec();
    let copyright = if meta.copyright.is_empty() {
        String::new()
    } else {
        format!("\u{a9} {}", meta.copyright)
    };
    let track = (meta.track_number > 0).then(|| meta.track_number.to_string());
    for (id, text) in [
        (b"INAM", meta.title.as_str()),
        (b"IART", meta.artist.as_str()),
        (b"IPRD", meta.album.as_str()),
        (b"ICRD", meta.release_date.as_str()),
        (b"IGNR", meta.genre.as_str()),
        (b"ICMT", meta.comment.as_str()),
        (b"ICOP", copyright.as_str()),
        (b"ITRK", track.as_deref().unwrap_or("")),
        (b"ISFT", auto.software.as_str()),
    ] {
        if !text.is_empty() {
            let mut value: Vec<u8> = text.bytes().filter(|&b| b != 0).collect();
            value.push(0);
            body.extend(riff_chunk(id, &value));
        }
    }
    riff_chunk(b"LIST", &body)
}

/// The extra chunks for a WAV: `before_data` goes between `fmt ` and `data`, `after_data`
/// follows the audio. Both are empty when there's nothing to write.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WavChunks {
    pub before_data: Vec<u8>,
    pub after_data: Vec<u8>,
}

pub fn wav_chunks(meta: &Metadata, auto: &Auto, sample_rate: u32, bits: u16) -> WavChunks {
    if meta.is_empty() {
        return WavChunks::default();
    }
    let mut after = list_info(meta, auto);
    let id3 = id3v24(meta, auto);
    if !id3.is_empty() {
        after.extend(riff_chunk(b"id3 ", &id3));
    }
    WavChunks {
        before_data: bext(meta, auto, sample_rate, bits),
        after_data: after,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::Utc;

    fn auto() -> Auto {
        Auto {
            bpm: 110.4,
            key: Some("Am".into()),
            software: "AudioZero 0.0.0".into(),
            when: Utc::from_unix(1_700_000_000),
        }
    }

    fn full() -> Metadata {
        let mut m = Metadata {
            title: "Night Drive".into(),
            artist: "Jane Doe".into(),
            album: "Roads".into(),
            track_number: 2,
            track_total: 9,
            release_date: "2026-11-15".into(),
            genre: "Electronic".into(),
            composer: "Jane Doe".into(),
            lyricist: "J. Writer".into(),
            producer: "Sam Producer".into(),
            mixer: "Max Mixer".into(),
            label: "Noisy Records".into(),
            isrc: "US-RC1-26-00001".into(),
            upc: "036000291452".into(),
            iswc: "T-034.524.680-1".into(),
            copyright: "© 2026 Jane Doe".into(),
            phonogram: "℗ 2026 Noisy Records".into(),
            language: "eng".into(),
            explicit: Explicit::Explicit,
            comment: "Made in AudioZero".into(),
            ..Default::default()
        };
        m.sanitize();
        m
    }

    /// Reads the frames of an ID3v2.4 tag back: (id, body).
    fn frames(tag: &[u8]) -> Vec<(String, Vec<u8>)> {
        assert_eq!(&tag[..3], b"ID3");
        assert_eq!(&tag[3..6], &[4, 0, 0]);
        let size = tag[6..10]
            .iter()
            .fold(0usize, |n, b| (n << 7) | usize::from(*b));
        assert_eq!(
            size,
            tag.len() - 10,
            "the header's size is the frames' size"
        );
        let mut out = Vec::new();
        let mut i = 10;
        while i < tag.len() {
            let n = tag[i + 4..i + 8]
                .iter()
                .fold(0usize, |n, b| (n << 7) | usize::from(*b));
            out.push((
                String::from_utf8(tag[i..i + 4].to_vec()).unwrap(),
                tag[i + 10..i + 10 + n].to_vec(),
            ));
            i += 10 + n;
        }
        assert_eq!(i, tag.len());
        out
    }

    fn text(frames: &[(String, Vec<u8>)], id: &str) -> Option<String> {
        frames
            .iter()
            .find(|(i, _)| i == id)
            .map(|(_, b)| String::from_utf8(b[1..].to_vec()).unwrap())
    }

    #[test]
    fn the_id3_tag_has_the_standard_frames() {
        let tag = id3v24(&full(), &auto());
        let f = frames(&tag);
        for (id, want) in [
            ("TIT2", "Night Drive"),
            ("TPE1", "Jane Doe"),
            ("TALB", "Roads"),
            ("TRCK", "2/9"),
            ("TDRC", "2026-11-15"),
            ("TCON", "Electronic"),
            ("TCOM", "Jane Doe"),
            ("TEXT", "J. Writer"),
            ("TPUB", "Noisy Records"),
            ("TCOP", "2026 Jane Doe"),
            ("TPRO", "2026 Noisy Records"),
            ("TSRC", "USRC12600001"),
            ("TLAN", "eng"),
            ("TBPM", "110"),
            ("TKEY", "Am"),
            ("TSSE", "AudioZero 0.0.0"),
            ("TDEN", "2023-11-14T22:13:20"),
        ] {
            assert_eq!(text(&f, id).as_deref(), Some(want), "{id}");
        }
        let people = &f.iter().find(|(i, _)| i == "TIPL").unwrap().1;
        assert_eq!(&people[1..], b"producer\0Sam Producer\0mix\0Max Mixer\0");
        let barcode = &f.iter().find(|(i, _)| i == "TXXX").unwrap().1;
        assert_eq!(&barcode[1..], b"BARCODE\x00036000291452");
        assert!(
            f.iter()
                .any(|(i, b)| i == "TXXX" && b.ends_with(b"ITUNESADVISORY\x001"))
        );
        let comment = &f.iter().find(|(i, _)| i == "COMM").unwrap().1;
        assert_eq!(&comment[1..], b"eng\0Made in AudioZero");
    }

    #[test]
    fn unset_fields_write_nothing_and_tempo_can_be_left_out() {
        let nothing = Metadata {
            include_tempo_and_key: false,
            ..Default::default()
        };
        let quiet = Auto {
            software: String::new(),
            ..auto()
        };
        assert_eq!(
            frames(&id3v24(&nothing, &quiet)).len(),
            1,
            "only the time is left"
        );
        let m = Metadata {
            title: "Only a title".into(),
            include_tempo_and_key: false,
            ..Default::default()
        };
        let f = frames(&id3v24(&m, &auto()));
        let ids: Vec<&str> = f.iter().map(|(i, _)| i.as_str()).collect();
        assert_eq!(
            ids,
            ["TIT2", "TSSE", "TDEN"],
            "no tempo, key, artist or anything else"
        );
    }

    #[test]
    fn the_wav_chunks_are_well_formed() {
        let m = full();
        let chunks = wav_chunks(&m, &auto(), 48_000, 24);
        // bext: a fixed 602 bytes plus the coding history.
        let b = &chunks.before_data;
        assert_eq!(&b[..4], b"bext");
        let size = u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize;
        assert_eq!(b.len(), 8 + size + size % 2);
        assert!(size >= 602);
        assert_eq!(&b[8..8 + 20], b"Night Drive - Jane D");
        assert_eq!(&b[8 + 256..8 + 256 + 15], b"AudioZero 0.0.0");
        assert_eq!(&b[8 + 320..8 + 330], b"2023-11-14");
        assert_eq!(&b[8 + 330..8 + 338], b"22-13-20");
        assert_eq!(
            u16::from_le_bytes([b[8 + 346], b[8 + 347]]),
            1,
            "BWF version 1"
        );
        assert!(b[8 + 602..].starts_with(b"A=PCM,F=48000,W=24,M=stereo"));

        // After the audio: LIST/INFO, then the id3 chunk. Walk them.
        let a = &chunks.after_data;
        let mut i = 0;
        let mut ids = Vec::new();
        while i < a.len() {
            let n = u32::from_le_bytes(a[i + 4..i + 8].try_into().unwrap()) as usize;
            ids.push(String::from_utf8(a[i..i + 4].to_vec()).unwrap());
            i += 8 + n + n % 2; // chunks are padded to an even size
        }
        assert_eq!(i, a.len());
        assert_eq!(ids, ["LIST", "id3 "]);
        let list = &a[8..8 + u32::from_le_bytes(a[4..8].try_into().unwrap()) as usize];
        assert_eq!(&list[..4], b"INFO");
        let info = String::from_utf8_lossy(list);
        for want in [
            "INAM",
            "Night Drive",
            "IART",
            "Jane Doe",
            "IPRD",
            "Roads",
            "ICRD",
            "2026-11-15",
            "ITRK",
        ] {
            assert!(info.contains(want), "{want}");
        }
        assert!(info.contains("\u{a9} 2026 Jane Doe"));
    }

    #[test]
    fn empty_metadata_adds_no_chunks() {
        assert_eq!(
            wav_chunks(&Metadata::default(), &auto(), 48_000, 16),
            WavChunks::default()
        );
    }

    #[test]
    fn odd_sized_text_is_padded_to_even_chunks() {
        let mut m = Metadata {
            title: "abc".into(),
            ..Default::default()
        };
        m.sanitize();
        let c = wav_chunks(&m, &auto(), 44_100, 16);
        assert_eq!(c.before_data.len() % 2, 0);
        assert_eq!(c.after_data.len() % 2, 0);
    }
}
