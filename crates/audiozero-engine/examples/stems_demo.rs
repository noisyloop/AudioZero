//! Splits an audio file into Drums, Bass, Vocals and Other, to hear (or measure) the separation:
//!
//!     cargo run --release -p audiozero-engine --example stems_demo -- song.wav out_folder
//!
//! Any file AudioZero can open works (WAV, AIFF, FLAC, MP3, Ogg Vorbis).

use audiozero_engine::{sample, stems, wav};

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let input = args.next().ok_or("usage: stems_demo song.wav out_folder")?;
    let folder = std::path::PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let song = sample::decode_file(std::path::Path::new(&input))?;
    println!("{:.1} s at {} Hz", song.seconds(), song.rate());
    let start = std::time::Instant::now();
    let parts = stems::separate(&song, |_| true)?;
    println!("separated in {:.2} s", start.elapsed().as_secs_f64());
    for (name, part) in &parts {
        let path = folder.join(format!("{}.wav", name.to_lowercase()));
        let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        wav::write_stereo_16(
            std::io::BufWriter::new(file),
            part.rate().round() as u32,
            part.left(),
            part.right(),
        )
        .map_err(|e| e.to_string())?;
        println!("{name}: {}", path.display());
    }
    Ok(())
}
