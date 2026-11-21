//! Built-in MP4 video: H.264 (Cisco's OpenH264, compiled into the app) and AAC-LC (a pure-Rust
//! encoder), written to an `.mp4` with no FFmpeg needed. This is what makes the exported file
//! something anyone can upload: the result is a plain H.264 + AAC MP4.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use bytes::Bytes;
use mp4::{
    AacConfig, AudioObjectType, AvcConfig, ChannelConfig, MediaConfig, Mp4Config, Mp4Sample,
    Mp4Writer, SampleFreqIndex, TrackConfig, TrackType,
};
use openh264::OpenH264API;
use openh264::encoder::{BitRate, Encoder, EncoderConfig, FrameRate, IntraFramePeriod};
use openh264::formats::{RgbaSliceU8, YUVBuffer};
use oxideav_aac::encoder::{EncoderConfig as AacEncoderConfig, StreamEncoder};

/// Samples in one AAC frame.
const AAC_FRAME: usize = 1024;
/// AAC bitrate, bits per second (stereo).
const AAC_BITRATE: u32 = 192_000;

/// Why a frame callback stopped the export.
pub enum Next {
    /// The frame was drawn into the buffer.
    Frame,
    /// The export was cancelled.
    Cancel,
}

/// What [`write_mp4`] needs besides the frames.
pub struct Mp4Job<'a> {
    pub width: usize,
    pub height: usize,
    pub fps: u32,
    /// Sample rate of `left` and `right`.
    pub rate: u32,
    pub left: &'a [f32],
    pub right: &'a [f32],
    pub frames: usize,
}

/// Encodes the song to raw AAC-LC frames (no ADTS headers), 1024 samples each.
fn encode_aac(job: &Mp4Job) -> Result<Vec<Vec<u8>>, String> {
    let mut encoder = StreamEncoder::new(AacEncoderConfig {
        sample_rate: job.rate,
        channels: 2,
        bitrate: AAC_BITRATE,
    })
    .map_err(|e| format!("AAC encoder: {e:?}"))?;
    let to_i16 = |v: f32| (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
    let mut out = Vec::with_capacity(job.left.len() / AAC_FRAME + 2);
    let mut pcm = Vec::with_capacity(AAC_FRAME * 2);
    for (l, r) in job.left.chunks(AAC_FRAME).zip(job.right.chunks(AAC_FRAME)) {
        pcm.clear();
        for (&a, &b) in l.iter().zip(r) {
            pcm.push(to_i16(a));
            pcm.push(to_i16(b));
        }
        let frame = encoder
            .encode_frame(&pcm)
            .map_err(|e| format!("AAC encoder: {e:?}"))?;
        out.push(strip_adts(&frame)?);
    }
    let tail = encoder
        .finish()
        .map_err(|e| format!("AAC encoder: {e:?}"))?;
    if !tail.is_empty() {
        out.push(strip_adts(&tail)?);
    }
    Ok(out)
}

/// The AAC payload of one ADTS frame (MP4 stores frames without the ADTS header).
fn strip_adts(frame: &[u8]) -> Result<Vec<u8>, String> {
    let bad = || "AAC encoder gave an unexpected frame".to_string();
    if frame.len() < 7 || frame[0] != 0xFF || frame[1] & 0xF0 != 0xF0 {
        return Err(bad());
    }
    let header = if frame[1] & 1 == 1 { 7 } else { 9 };
    let length = (usize::from(frame[3] & 3) << 11)
        | (usize::from(frame[4]) << 3)
        | (usize::from(frame[5]) >> 5);
    if length != frame.len() || length < header {
        return Err(bad());
    }
    Ok(frame[header..].to_vec())
}

fn freq_index(rate: u32) -> Result<SampleFreqIndex, String> {
    Ok(match rate {
        48_000 => SampleFreqIndex::Freq48000,
        44_100 => SampleFreqIndex::Freq44100,
        32_000 => SampleFreqIndex::Freq32000,
        _ => return Err(format!("unsupported sample rate {rate} Hz for MP4 audio")),
    })
}

/// Splits an Annex B stream (start codes) into its NAL units.
fn nal_units(stream: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= stream.len() {
        if stream[i] == 0 && stream[i + 1] == 0 && stream[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut units = Vec::new();
    for (n, &s) in starts.iter().enumerate() {
        let mut end = starts.get(n + 1).map_or(stream.len(), |next| next - 3);
        // A four-byte start code leaves a zero on the end of the previous unit.
        while end > s && stream[end - 1] == 0 {
            end -= 1;
        }
        if end > s {
            units.push(&stream[s..end]);
        }
    }
    units
}

/// One H.264 access unit as MP4 wants it: length-prefixed NAL units without the parameter sets.
struct Access {
    sample: Vec<u8>,
    key: bool,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
}

fn access_unit(stream: &[u8]) -> Access {
    let mut access = Access {
        sample: Vec::new(),
        key: false,
        sps: None,
        pps: None,
    };
    for nal in nal_units(stream) {
        match nal[0] & 0x1F {
            7 => access.sps = Some(nal.to_vec()),
            8 => access.pps = Some(nal.to_vec()),
            // Access unit delimiters, SEI and filler aren't needed in the file.
            6 | 9 | 12 => {}
            kind => {
                if kind == 5 {
                    access.key = true;
                }
                access
                    .sample
                    .extend_from_slice(&(nal.len() as u32).to_be_bytes());
                access.sample.extend_from_slice(nal);
            }
        }
    }
    access
}

/// Writes `job` to `path` as H.264 + AAC in an MP4. `draw` fills the buffer with the RGBA pixels
/// of frame `i` and says whether to go on. `progress(done)` is called after each frame.
pub fn write_mp4(
    path: &Path,
    job: &Mp4Job,
    mut draw: impl FnMut(usize, &mut Vec<u8>) -> Next,
    mut progress: impl FnMut(usize),
) -> Result<Next, String> {
    let (w, h, fps) = (job.width, job.height, job.fps);
    let audio = encode_aac(job)?;

    // Roughly 0.1 bit per pixel per frame: 6 Mbit/s for 1080p30, 25 for 4K30.
    let bps = (w as f64 * h as f64 * f64::from(fps) * 0.1).clamp(1.0e6, 60.0e6) as u32;
    let config = EncoderConfig::new()
        .bitrate(BitRate::from_bps(bps))
        .max_frame_rate(FrameRate::from_hz(fps as f32))
        .intra_frame_period(IntraFramePeriod::from_num_frames(fps * 2))
        .skip_frames(false);
    let mut encoder = Encoder::with_api_config(OpenH264API::from_source(), config)
        .map_err(|e| format!("H.264 encoder: {e}"))?;
    let mut yuv = YUVBuffer::new(w, h);
    let mut rgba = Vec::new();

    let file = File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let brand = |s: &str| s.parse().expect("a four-character code");
    let mut writer = Mp4Writer::write_start(
        BufWriter::new(file),
        &Mp4Config {
            major_brand: brand("isom"),
            minor_version: 512,
            compatible_brands: vec![brand("isom"), brand("iso2"), brand("avc1"), brand("mp41")],
            timescale: 1000,
        },
    )
    .map_err(|e| e.to_string())?;

    let mut next_audio = 0usize;
    let mut result = Ok(Next::Frame);
    let mut tracks_added = false;
    for i in 0..job.frames {
        if matches!(draw(i, &mut rgba), Next::Cancel) {
            result = Ok(Next::Cancel);
            break;
        }
        yuv.read_rgba8(RgbaSliceU8::new(&rgba, (w, h)));
        let stream = encoder
            .encode(&yuv)
            .map_err(|e| format!("H.264 encoder: {e}"))?
            .to_vec();
        let access = access_unit(&stream);
        if !tracks_added {
            let (Some(sps), Some(pps)) = (access.sps.clone(), access.pps.clone()) else {
                return Err("H.264 encoder gave no parameter sets".into());
            };
            writer
                .add_track(&TrackConfig {
                    track_type: TrackType::Video,
                    timescale: fps,
                    language: "und".into(),
                    media_conf: MediaConfig::AvcConfig(AvcConfig {
                        width: w as u16,
                        height: h as u16,
                        seq_param_set: sps,
                        pic_param_set: pps,
                    }),
                })
                .map_err(|e| e.to_string())?;
            writer
                .add_track(&TrackConfig {
                    track_type: TrackType::Audio,
                    timescale: job.rate,
                    language: "und".into(),
                    media_conf: MediaConfig::AacConfig(AacConfig {
                        bitrate: AAC_BITRATE,
                        profile: AudioObjectType::AacLowComplexity,
                        freq_index: freq_index(job.rate)?,
                        chan_conf: ChannelConfig::Stereo,
                    }),
                })
                .map_err(|e| e.to_string())?;
            tracks_added = true;
        }
        if access.sample.is_empty() {
            return Err("H.264 encoder skipped a frame".into());
        }
        // Audio up to this frame first, so the file interleaves and can start playing early.
        let now = i as f64 / f64::from(fps);
        while next_audio < audio.len()
            && (next_audio * AAC_FRAME) as f64 / f64::from(job.rate) <= now
        {
            write_audio(&mut writer, &audio[next_audio], next_audio)?;
            next_audio += 1;
        }
        writer
            .write_sample(
                1,
                &Mp4Sample {
                    start_time: i as u64,
                    duration: 1,
                    rendering_offset: 0,
                    is_sync: access.key,
                    bytes: Bytes::from(access.sample),
                },
            )
            .map_err(|e| e.to_string())?;
        progress(i + 1);
    }
    if matches!(result, Ok(Next::Frame)) {
        while next_audio < audio.len() {
            write_audio(&mut writer, &audio[next_audio], next_audio)?;
            next_audio += 1;
        }
        writer.write_end().map_err(|e| e.to_string())?;
    }
    result
}

fn write_audio(
    writer: &mut Mp4Writer<BufWriter<File>>,
    frame: &[u8],
    index: usize,
) -> Result<(), String> {
    writer
        .write_sample(
            2,
            &Mp4Sample {
                start_time: (index * AAC_FRAME) as u64,
                duration: AAC_FRAME as u32,
                rendering_offset: 0,
                is_sync: true,
                bytes: Bytes::copy_from_slice(frame),
            },
        )
        .map_err(|e| e.to_string())
}
