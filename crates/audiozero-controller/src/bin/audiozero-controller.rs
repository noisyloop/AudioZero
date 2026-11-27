//! `audiozero-controller`: a local diagnostic for game controllers, with no window and no audio.
//!
//! ```text
//! audiozero-controller list                 controllers the system sees
//! audiozero-controller monitor              press things; see events and the MIDI they make
//! audiozero-controller test                 play a scripted performance with no controller
//! options: --key C..B  --scale ID  --channel 1..16  --octave N
//! ```

use audiozero_controller::mapper::{BEND_CENTRE, CC_CUTOFF, CC_EXPRESSION, CC_MODULATION};
use audiozero_controller::{
    Action, Ax, Btn, Controllers, Layout, Mapper, Profile, RawEvent, device, midi,
};
use audiozero_engine::music;
use std::process::ExitCode;
use std::time::{Duration, Instant};

const USAGE: &str = "\
Usage: audiozero-controller <list|monitor|test> [options]

  list      List the controllers the system sees (DualShock 4s are marked)
  monitor   Print every button and stick event and the MIDI it makes (Ctrl+C to stop)
  test      Run a scripted performance through the same mapping, with no controller, and
            check note on/off, velocity, octave, pitch bend and control changes

Options:
  --key K       The key's root: C, C#, D ... B (default C)
  --scale ID    major, minor, dorian ... (default major)
  --channel N   MIDI channel shown, 1 to 16 (default 1)
  --octave N    Starting octave, -1 to 8 (default 4: the root is middle C)
  -h, --help    Show this help";

struct Options {
    key: u8,
    scale: music::Scale,
    profile: Profile,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        key: 0,
        scale: music::scale_by_id("major"),
        profile: Profile::default(),
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--key" => {
                let k = value("--key")?;
                o.key = music::KEYS
                    .iter()
                    .position(|n| n.eq_ignore_ascii_case(k))
                    .ok_or(format!("unknown key {k}: use C, C#, D ... B"))?
                    as u8;
            }
            "--scale" => {
                let id = value("--scale")?;
                o.scale = music::SCALES
                    .iter()
                    .copied()
                    .find(|s| s.id == id)
                    .ok_or(format!("unknown scale {id}"))?;
            }
            "--channel" => {
                o.profile.channel = value("--channel")?
                    .parse()
                    .map_err(|_| "--channel needs a number from 1 to 16")?;
            }
            "--octave" => {
                o.profile.octave = value("--octave")?
                    .parse()
                    .map_err(|_| "--octave needs a number from -1 to 8")?;
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    o.profile = o.profile.sanitized();
    Ok(o)
}

fn show(action: &Action, channel: u8) {
    match midi::message(action, channel) {
        Some(bytes) => println!("    MIDI  {}", midi::describe(bytes)),
        None => println!("    →     {action:?}"),
    }
}

fn list() -> Result<(), String> {
    let pads = device::list().map_err(|e| {
        format!(
            "can't read input devices: {e}\nOn Linux your user needs read access to /dev/input/event*; see docs/CONTROLLER.md."
        )
    })?;
    if pads.is_empty() {
        println!("No controllers found. Plug the DualShock 4 in with a USB data cable.");
    }
    for p in pads {
        println!("{}: {}", p.id, p.label());
    }
    Ok(())
}

fn monitor(o: Options) -> Result<(), String> {
    let controllers = Controllers::start(|| {});
    std::thread::sleep(Duration::from_millis(300));
    if let Some(e) = controllers.error() {
        return Err(format!("can't read controllers: {e}"));
    }
    let channel = o.profile.channel;
    let mut mapper = Mapper::new(o.profile);
    let layout = Layout::Scale {
        key: o.key,
        steps: o.scale.steps,
    };
    println!(
        "Listening in {}. Press buttons; Ctrl+C stops.",
        music::key_name(o.key, o.scale.id)
    );
    let mut last = Instant::now();
    let mut out = Vec::new();
    loop {
        for event in controllers.events() {
            // Stick noise would drown the rest: show axes only when they change a control.
            let quiet = matches!(event, RawEvent::Axis { .. } | RawEvent::Trigger { .. });
            if !quiet {
                println!("{event}");
            }
            mapper.handle(&event, &layout, &mut out);
            if quiet && !out.is_empty() {
                println!("{event}");
            }
            for a in out.drain(..) {
                show(&a, channel);
            }
        }
        let now = Instant::now();
        mapper.tick(now.duration_since(last).as_secs_f32(), &mut out);
        last = now;
        for a in out.drain(..) {
            show(&a, channel);
        }
        std::thread::sleep(Duration::from_millis(4));
    }
}

/// One scripted step: an event, and a check on what it produced.
type Check = Box<dyn Fn(&[Action]) -> Result<(), String>>;

fn test(o: Options) -> Result<(), String> {
    let channel = o.profile.channel;
    let mut mapper = Mapper::new(Profile {
        smoothing: 0.0,
        ..o.profile
    });
    let layout = Layout::Scale {
        key: o.key,
        steps: o.scale.steps,
    };
    let root = (i32::from(mapper.octave()) + 1) * 12 + i32::from(o.key);
    let button = |button, pressed| RawEvent::Button {
        pad: 0,
        button,
        pressed,
        value: 1.0,
    };
    let trigger = |button, value| RawEvent::Trigger {
        pad: 0,
        button,
        value,
    };
    let axis = |axis, value| RawEvent::Axis {
        pad: 0,
        axis,
        value,
    };
    let on = |pitch: i32, velocity: Option<f32>| -> Check {
        Box::new(move |a| match a {
            [
                Action::NoteOn {
                    pitch: p,
                    velocity: v,
                },
            ] if i32::from(*p) == pitch && velocity.is_none_or(|w| (v - w).abs() < 0.01) => Ok(()),
            other => Err(format!("expected note on {pitch}, got {other:?}")),
        })
    };
    let off = |pitch: i32| -> Check {
        Box::new(move |a| match a {
            [Action::NoteOff { pitch: p }] if i32::from(*p) == pitch => Ok(()),
            other => Err(format!("expected note off {pitch}, got {other:?}")),
        })
    };
    let one = |want: Action| -> Check {
        Box::new(move |a| {
            if a == [want] {
                Ok(())
            } else {
                Err(format!("expected {want:?}, got {a:?}"))
            }
        })
    };
    let third = i32::from(o.scale.steps.get(2).copied().unwrap_or(4));
    let steps: Vec<(&str, RawEvent, Check)> = vec![
        (
            "Cross down: the root",
            button(Btn::Cross, true),
            on(root, Some(0.8)),
        ),
        ("Cross up", button(Btn::Cross, false), off(root)),
        (
            "Square down: the third",
            button(Btn::Square, true),
            on(root + third, Some(0.8)),
        ),
        ("Square up", button(Btn::Square, false), off(root + third)),
        (
            "L2 pulled fully",
            trigger(Btn::L2, 1.0),
            Box::new(|a| {
                if a.is_empty() {
                    Ok(())
                } else {
                    Err(format!("L2 made {a:?}"))
                }
            }),
        ),
        (
            "Cross down, L2 held: full velocity",
            button(Btn::Cross, true),
            on(root, Some(1.0)),
        ),
        ("Cross up", button(Btn::Cross, false), off(root)),
        ("L2 released", trigger(Btn::L2, 0.0), Box::new(|_| Ok(()))),
        (
            "Left stick right: pitch bend up",
            axis(Ax::LeftX, 1.0),
            one(Action::PitchBend { value: 16383 }),
        ),
        (
            "Left stick centred: bend returns",
            axis(Ax::LeftX, 0.0),
            one(Action::PitchBend { value: BEND_CENTRE }),
        ),
        (
            "Left stick up: expression",
            axis(Ax::LeftY, 1.0),
            one(Action::Cc {
                controller: CC_EXPRESSION,
                value: 127,
            }),
        ),
        (
            "Right stick right: filter cutoff",
            axis(Ax::RightX, 1.0),
            one(Action::Cc {
                controller: CC_CUTOFF,
                value: 127,
            }),
        ),
        (
            "R2 pulled: modulation",
            trigger(Btn::R2, 1.0),
            one(Action::Cc {
                controller: CC_MODULATION,
                value: 127,
            }),
        ),
        (
            "R1: octave up",
            button(Btn::R1, true),
            one(Action::OctaveChanged(mapper.octave() + 1)),
        ),
        (
            "Cross down in the new octave",
            button(Btn::Cross, true),
            on(root + 12, None),
        ),
        (
            "Controller unplugged with a note held: nothing may hang",
            RawEvent::Disconnected { pad: 0 },
            Box::new(move |a| {
                let released = a.contains(&Action::NoteOff {
                    pitch: (root + 12) as u8,
                });
                if released {
                    Ok(())
                } else {
                    Err(format!("the held note wasn't released: {a:?}"))
                }
            }),
        ),
    ];
    let mut failures = 0;
    for (what, event, check) in steps {
        println!("{what}");
        let mut out = Vec::new();
        mapper.handle(&event, &layout, &mut out);
        for a in &out {
            show(a, channel);
        }
        if let Err(e) = check(&out) {
            println!("    FAIL  {e}");
            failures += 1;
        }
    }
    if mapper.held_pitches().count() > 0 {
        println!("FAIL  notes still held at the end");
        failures += 1;
    }
    if failures == 0 {
        println!("\nAll checks passed. (This tests the mapping, not a physical controller.)");
        Ok(())
    } else {
        Err(format!("{failures} check(s) failed"))
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("list") => list(),
        Some("monitor") => parse(&args[1..]).and_then(monitor),
        Some("test") => parse(&args[1..]).and_then(test),
        Some("-h" | "--help") | None => {
            eprintln!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some(other) => Err(format!("unknown command {other}\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("audiozero-controller: {e}");
            ExitCode::FAILURE
        }
    }
}
