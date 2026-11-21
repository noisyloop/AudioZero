//! Writes a short demo song built with the chord pack: a drum beat, a chord progression and a
//! bass line under it, in A minor. It also writes the project, so it opens in the app.
//!
//!     cargo run --release -p audiozero-engine --example chords_demo -- some/folder [genre] [nodrums]
//!
//! `genre` is a drum kit id (techno, house, dubstep, hiphop, lofi, trap, dnb, halftime, jazz).

use audiozero_engine::drums;
use audiozero_engine::engine::Engine;
use audiozero_engine::progressions::{self, genre_style};
use audiozero_engine::sample::SamplePool;
use audiozero_engine::session::{ChordRequest, ChordSource, Session};
use audiozero_engine::{Project, wav};

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let folder = std::path::PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
    let genre = args.next().unwrap_or_else(|| "lofi".into());
    // Pass "nodrums" to leave the drums out, to hear (or measure) the chords and the bass alone.
    let drums_on = args.next().as_deref() != Some("nodrums");
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;

    let mut session = Session::new(Project::default()).map_err(|e| e.to_string())?;
    session.set_key(9, "minor").map_err(|e| e.to_string())?;
    let beat = drums::beats_for(&genre).next().ok_or("unknown genre")?;
    if drums_on {
        session
            .add_drum_beat(None, beat.id, 4, None)
            .map_err(|e| e.to_string())?;
    }
    // The first minor progression of the genre.
    let scale = audiozero_engine::music::scale_by_id("minor");
    let progression = progressions::for_genre(&genre)
        .find(|p| progressions::suits(p, scale))
        .ok_or("no progression")?;
    let request = ChordRequest {
        source: ChordSource::Library(progression.id.clone()),
        repeats: ((16.0 / progression.beats()).ceil() as usize).max(1),
        start: Some(0.0),
        pattern: None,
        seed: 1,
    };
    let ((chords, name), _) = session
        .add_chords(None, request)
        .map_err(|e| e.to_string())?;
    let style = genre_style(&genre);
    let (bass, _) = session
        .add_bass_line(chords, style.bass, 1, Some(style.bass_sound))
        .map_err(|e| e.to_string())?;
    println!(
        "{genre}: {name} in A minor, {:?} chords on {}, {} bass on {}",
        style.pattern,
        style.sound,
        style.bass.name(),
        style.bass_sound
    );
    println!(
        "{} chord notes, {} bass notes, {} bars",
        session.clip(chords).map_or(0, |c| c.notes.len()),
        session.clip(bass).map_or(0, |c| c.notes.len()),
        session.clip(chords).map_or(0.0, |c| c.length / 4.0)
    );
    session.set_bpm(beat.bpm).ok();

    let project = session.project().clone();
    project
        .save(&folder.join(format!(
            "{genre}-chords{}.azproj",
            if drums_on { "" } else { "-nodrums" }
        )))
        .map_err(|e| e.to_string())?;
    let mut pool = SamplePool::new(48_000.0);
    let mut engine =
        Engine::with_samples(project, 48_000.0, 512, &mut pool).map_err(|e| e.to_string())?;
    engine.play();
    let frames = ((engine.seconds_to_end() + 2.0) * 48_000.0) as usize;
    let (left, right) = engine.render(frames);
    let path = folder.join(format!(
        "{genre}-chords{}.wav",
        if drums_on { "" } else { "-nodrums" }
    ));
    let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    wav::write_stereo_16(std::io::BufWriter::new(file), 48_000, &left, &right)
        .map_err(|e| e.to_string())?;
    let peak = left
        .iter()
        .chain(&right)
        .fold(0.0f32, |m, x| m.max(x.abs()));
    let rms = (left.iter().map(|x| x * x).sum::<f32>() / left.len() as f32).sqrt();
    println!(
        "{} ({:.1} s, peak {peak:.2}, rms {rms:.3})",
        path.display(),
        frames as f64 / 48_000.0
    );
    Ok(())
}
