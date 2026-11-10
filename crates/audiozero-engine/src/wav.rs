//! A minimal WAV writer (16- or 24-bit PCM stereo), so exporting needs no extra dependency.

use std::io::{self, Write};

pub fn write_stereo_16(
    out: impl Write,
    sample_rate: u32,
    left: &[f32],
    right: &[f32],
) -> io::Result<()> {
    write_stereo(out, sample_rate, 16, left, right)
}

/// 24-bit, for recordings, which keep more of the input's resolution.
pub fn write_stereo_24(
    out: impl Write,
    sample_rate: u32,
    left: &[f32],
    right: &[f32],
) -> io::Result<()> {
    write_stereo(out, sample_rate, 24, left, right)
}

fn write_stereo(
    mut out: impl Write,
    sample_rate: u32,
    bits: u16,
    left: &[f32],
    right: &[f32],
) -> io::Result<()> {
    let frames = left.len().min(right.len());
    let bytes_per_sample = usize::from(bits / 8);
    let frame_bytes = 2 * bytes_per_sample;
    let data_bytes = u32::try_from(frames * frame_bytes)
        .ok()
        .filter(|n| *n <= u32::MAX - 36)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "audio is too long for a WAV file",
            )
        })?;
    out.write_all(b"RIFF")?;
    out.write_all(&(36 + data_bytes).to_le_bytes())?;
    out.write_all(b"WAVEfmt ")?;
    out.write_all(&16u32.to_le_bytes())?; // fmt chunk size
    out.write_all(&1u16.to_le_bytes())?; // PCM
    out.write_all(&2u16.to_le_bytes())?; // channels
    out.write_all(&sample_rate.to_le_bytes())?;
    out.write_all(&(sample_rate * frame_bytes as u32).to_le_bytes())?; // bytes per second
    out.write_all(&(frame_bytes as u16).to_le_bytes())?; // bytes per frame
    out.write_all(&bits.to_le_bytes())?;
    out.write_all(b"data")?;
    out.write_all(&data_bytes.to_le_bytes())?;
    let full_scale = ((1i32 << (bits - 1)) - 1) as f32;
    let mut buffer = Vec::with_capacity(frames * frame_bytes);
    for i in 0..frames {
        for x in [left[i], right[i]] {
            let x = if x.is_finite() {
                x.clamp(-1.0, 1.0)
            } else {
                0.0
            };
            let v = (x * full_scale).round() as i32;
            buffer.extend_from_slice(&v.to_le_bytes()[..bytes_per_sample]);
        }
    }
    out.write_all(&buffer)
}

#[cfg(test)]
mod tests {
    #[test]
    fn writes_a_valid_header() {
        let mut bytes = Vec::new();
        super::write_stereo_16(&mut bytes, 48_000, &[0.0, 1.0], &[-1.0, 0.5]).unwrap();
        assert_eq!(bytes.len(), 44 + 8);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 8);
        assert_eq!(i16::from_le_bytes([bytes[48], bytes[49]]), 32767);
    }

    #[test]
    fn writes_24_bit_that_reads_back() {
        let dir = std::env::temp_dir().join(format!("audiozero-wav24-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.wav");
        let left = [0.0, 0.5, -0.25, 1.0];
        let right = [-1.0, 0.1, 0.2, 0.3];
        super::write_stereo_24(std::fs::File::create(&path).unwrap(), 44_100, &left, &right)
            .unwrap();
        let back = crate::sample::decode_file(&path).unwrap();
        assert_eq!(back.frames(), 4);
        for (a, b) in back
            .left()
            .iter()
            .zip(left)
            .chain(back.right().iter().zip(right))
        {
            assert!((a - b).abs() < 1e-5, "{a} vs {b}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
