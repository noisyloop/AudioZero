//! A minimal WAV writer (16-bit PCM stereo), so exporting needs no extra dependency.

use std::io::{self, Write};

pub fn write_stereo_16(
    mut out: impl Write,
    sample_rate: u32,
    left: &[f32],
    right: &[f32],
) -> io::Result<()> {
    let frames = left.len().min(right.len());
    let data_bytes = u32::try_from(frames * 4)
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
    out.write_all(&(sample_rate * 4).to_le_bytes())?; // bytes per second
    out.write_all(&4u16.to_le_bytes())?; // bytes per frame
    out.write_all(&16u16.to_le_bytes())?; // bits per sample
    out.write_all(b"data")?;
    out.write_all(&data_bytes.to_le_bytes())?;
    let to_i16 = |x: f32| (x.clamp(-1.0, 1.0) * 32767.0).round() as i16;
    let mut buffer = Vec::with_capacity(frames * 4);
    for i in 0..frames {
        buffer.extend_from_slice(&to_i16(left[i]).to_le_bytes());
        buffer.extend_from_slice(&to_i16(right[i]).to_le_bytes());
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
}
