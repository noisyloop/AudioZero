//! `audiozero`: the AudioZero desktop app (Rust rewrite). With no options it opens the window;
//! the options play, render or inspect projects from the command line.

mod audio;
mod gui;

use audiozero_engine::{Command, Engine, PluginFormat, Project, wav};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
Usage: audiozero [PROJECT] [OPTIONS]

Opens the AudioZero window with PROJECT (an AudioZero .json project), or the demo song.

Options:
  --play             Play from the command line instead of opening the window
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
    play: bool,
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
            Some("--play") => args.play = true,
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

    if !args.play && args.render.is_none() && args.save.is_none() {
        return gui::run(args.project);
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
    let mut output = audio::Output::open();
    println!("{}", output.description);
    if !output.has_device() {
        return Err("no audio output device (use --render to write a WAV file instead)".into());
    }
    let seconds = project.end_beats() * 60.0 / project.bpm + 1.5;
    for warning in output.controller.load(project).map_err(|e| e.to_string())? {
        eprintln!("warning: {warning}");
    }
    output.controller.send(Command::Play);
    println!("Playing {seconds:.1} s. Press Ctrl+C to stop.");
    std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
    Ok(())
}
