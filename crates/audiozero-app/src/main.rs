//! `audiozero`: the command-line front end of the Rust rewrite. It plays or renders projects
//! through the engine; the desktop UI comes later.

use audiozero_engine::{Engine, PluginFormat, Project, wav};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
Usage: audiozero [PROJECT] [OPTIONS]

Plays PROJECT (an AudioZero .json project), or the built-in demo song if none is given.

Options:
  --render OUT.wav   Render to a WAV file instead of playing
  --save OUT.json    Save the project (handy for getting a demo project to edit)
  --scan             List CLAP and VST3 plugins in the standard folders (without loading them)
  -h, --help         Show this help";

#[derive(Default)]
struct Args {
    project: Option<PathBuf>,
    render: Option<PathBuf>,
    save: Option<PathBuf>,
    scan: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args::default();
    let mut it = std::env::args_os().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .map(PathBuf::from)
                .ok_or(format!("{name} needs a file name"))
        };
        match arg.to_str() {
            Some("-h" | "--help") => return Err(String::new()),
            Some("--render") => args.render = Some(value("--render")?),
            Some("--save") => args.save = Some(value("--save")?),
            Some("--scan") => args.scan = true,
            Some(other) if other.starts_with('-') => return Err(format!("unknown option {other}")),
            _ if args.project.is_none() => args.project = Some(PathBuf::from(arg)),
            _ => return Err("only one project can be given".into()),
        }
    }
    Ok(args)
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            if !message.is_empty() {
                eprintln!("audiozero: {message}\n");
            }
            eprintln!("{USAGE}");
            return if message.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            };
        }
    };
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("audiozero: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> Result<(), String> {
    if args.scan {
        scan();
        return Ok(());
    }

    let project = match &args.project {
        Some(path) => Project::load(path).map_err(|e| format!("{}: {e}", path.display()))?,
        None => audiozero_engine::demo::project(),
    };
    if let Some(path) = &args.save {
        project
            .save(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        println!("Saved {}", path.display());
        if args.render.is_none() {
            return Ok(());
        }
    }

    if let Some(out) = &args.render {
        let sample_rate = 48_000;
        let mut engine =
            Engine::new(project, f64::from(sample_rate), 512).map_err(|e| e.to_string())?;
        print_warnings(&engine);
        engine.play();
        let frames = ((engine.seconds_to_end() + 2.0) * f64::from(sample_rate)) as usize;
        let (left, right) = engine.render(frames);
        let file = std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
        wav::write_stereo_16(std::io::BufWriter::new(file), sample_rate, &left, &right)
            .map_err(|e| format!("{}: {e}", out.display()))?;
        println!(
            "Rendered {:.1} s to {}",
            frames as f64 / f64::from(sample_rate),
            out.display()
        );
        return Ok(());
    }

    play(project)
}

fn print_warnings(engine: &Engine) {
    for warning in engine.warnings() {
        eprintln!("warning: {warning}");
    }
}

fn scan() {
    for format in [PluginFormat::Clap, PluginFormat::Vst3] {
        let paths = audiozero_host::default_search_paths(format);
        let found = audiozero_host::find_bundles(&paths, format);
        println!(
            "{} ({} found)",
            audiozero_host::extension(format).to_uppercase(),
            found.len()
        );
        for path in &paths {
            println!("  searched {}", path.display());
        }
        for bundle in found {
            println!("  {}", bundle.display());
        }
    }
}

fn play(project: Project) -> Result<(), String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("no audio output device found (use --render to write a WAV file instead)")?;
    let config = device
        .default_output_config()
        .map_err(|e| format!("audio device: {e}"))?;
    let format = config.sample_format();
    let config: cpal::StreamConfig = config.into();

    let mut engine =
        Engine::new(project, f64::from(config.sample_rate), 1024).map_err(|e| e.to_string())?;
    print_warnings(&engine);
    let seconds = engine.seconds_to_end() + 1.5;
    engine.play();

    use cpal::SampleFormat as F;
    let stream = match format {
        F::F32 => open::<f32>(&device, config, engine),
        F::I16 => open::<i16>(&device, config, engine),
        F::I32 => open::<i32>(&device, config, engine),
        F::U16 => open::<u16>(&device, config, engine),
        other => return Err(format!("unsupported audio sample format {other}")),
    }?;
    stream.play().map_err(|e| format!("audio device: {e}"))?;
    println!("Playing {seconds:.1} s. Press Ctrl+C to stop.");
    std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
    Ok(())
}

fn open<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut engine: Engine,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels).max(1);
    // Allocated once here, never in the callback.
    const CHUNK: usize = 4096;
    let (mut left, mut right) = (vec![0.0f32; CHUNK], vec![0.0f32; CHUNK]);
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                for frames in data.chunks_mut(CHUNK * channels) {
                    let n = frames.len() / channels;
                    engine.process(&mut left[..n], &mut right[..n]);
                    for (i, frame) in frames.chunks_mut(channels).enumerate() {
                        for (ch, sample) in frame.iter_mut().enumerate() {
                            let value = match ch {
                                0 => left[i],
                                1 => right[i],
                                _ => 0.0,
                            };
                            *sample = T::from_sample(if channels == 1 {
                                0.5 * (left[i] + right[i])
                            } else {
                                value
                            });
                        }
                    }
                }
            },
            |err| eprintln!("audio stream error: {err}"),
            None,
        )
        .map_err(|e| format!("audio device: {e}"))
}
