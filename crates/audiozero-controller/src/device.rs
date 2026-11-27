//! Finds controllers and reads them on a thread of their own, with `gilrs` (evdev on Linux).
//!
//! This thread is the only place a device is polled. It sends [`RawEvent`]s over a channel and
//! never touches the audio engine, so a slow or unplugged controller can't disturb sound.
//! Controllers plugged in or unplugged later are noticed while it runs.

use crate::input::{Ax, Btn, RawEvent};
use gilrs::{Axis, Button, EventType, GamepadId, Gilrs};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

/// Sony's USB vendor id, and the DualShock 4's product ids (v1, v2 and the USB dongle).
const SONY: u16 = 0x054c;
const DS4_PRODUCTS: [u16; 3] = [0x05c4, 0x09cc, 0x0ba0];
/// Analog events are dropped, not queued, once this many wait unread (button events never are).
const MAX_PENDING_ANALOG: usize = 1024;

/// A controller the system knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PadInfo {
    pub id: usize,
    pub name: String,
    pub vendor: Option<u16>,
    pub product: Option<u16>,
    pub dualshock4: bool,
}

impl PadInfo {
    /// "Wireless Controller (054c:09cc, DualShock 4)".
    pub fn label(&self) -> String {
        let ids = match (self.vendor, self.product) {
            (Some(v), Some(p)) => format!("{v:04x}:{p:04x}"),
            _ => "unknown id".into(),
        };
        if self.dualshock4 {
            format!("{} ({ids}, DualShock 4)", self.name)
        } else {
            format!("{} ({ids})", self.name)
        }
    }
}

/// The DS4's touchpad and motion sensors appear as input devices of their own; they aren't
/// controllers to play.
fn is_sensor(name: &str) -> bool {
    let name = name.to_lowercase();
    name.contains("touchpad") || name.contains("motion sensor")
}

fn info(id: GamepadId, gamepad: &gilrs::Gamepad<'_>) -> PadInfo {
    let (vendor, product) = (gamepad.vendor_id(), gamepad.product_id());
    PadInfo {
        id: usize::from(id),
        name: gamepad.name().to_string(),
        vendor,
        product,
        dualshock4: vendor == Some(SONY) && product.is_some_and(|p| DS4_PRODUCTS.contains(&p)),
    }
}

/// The controllers connected now (opens the system's input devices once, then lets go).
pub fn list() -> Result<Vec<PadInfo>, String> {
    let gilrs = Gilrs::new().map_err(|e| e.to_string())?;
    Ok(gilrs
        .gamepads()
        .filter(|(_, g)| !is_sensor(g.name()))
        .map(|(id, g)| info(id, &g))
        .collect())
}

fn button(b: Button) -> Option<Btn> {
    Some(match b {
        Button::South => Btn::Cross,
        Button::East => Btn::Circle,
        Button::West => Btn::Square,
        Button::North => Btn::Triangle,
        Button::DPadLeft => Btn::DpadLeft,
        Button::DPadUp => Btn::DpadUp,
        Button::DPadRight => Btn::DpadRight,
        Button::DPadDown => Btn::DpadDown,
        Button::LeftTrigger => Btn::L1,
        Button::RightTrigger => Btn::R1,
        Button::LeftTrigger2 => Btn::L2,
        Button::RightTrigger2 => Btn::R2,
        Button::LeftThumb => Btn::L3,
        Button::RightThumb => Btn::R3,
        Button::Start => Btn::Options,
        Button::Select => Btn::Share,
        Button::Mode => Btn::Ps,
        _ => return None,
    })
}

/// Controllers being read. Dropping it stops the thread.
pub struct Controllers {
    events: mpsc::Receiver<RawEvent>,
    pending: Arc<AtomicUsize>,
    pads: Arc<Mutex<Vec<PadInfo>>>,
    error: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// What the reading thread shares with [`Controllers`].
struct Shared {
    tx: mpsc::Sender<RawEvent>,
    pending: Arc<AtomicUsize>,
    pads: Arc<Mutex<Vec<PadInfo>>>,
    error: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
}

impl Controllers {
    /// Starts reading. `wake` is called when events arrive (to redraw the window).
    pub fn start(wake: impl Fn() + Send + 'static) -> Self {
        let (tx, events) = mpsc::channel();
        let pending = Arc::new(AtomicUsize::new(0));
        let pads = Arc::new(Mutex::new(Vec::new()));
        let error = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Shared {
            tx,
            pending: Arc::clone(&pending),
            pads: Arc::clone(&pads),
            error: Arc::clone(&error),
            stop: Arc::clone(&stop),
        };
        let thread = std::thread::Builder::new()
            .name("controller".into())
            .spawn(move || run(&shared, &wake))
            .ok();
        if thread.is_none() {
            *error.lock().expect("not poisoned") = Some("can't start the controller thread".into());
        }
        Self {
            events,
            pending,
            pads,
            error,
            stop,
            thread,
        }
    }

    /// Events that arrived since the last call.
    pub fn events(&self) -> Vec<RawEvent> {
        let events: Vec<RawEvent> = self.events.try_iter().collect();
        // Every event was counted before it was sent, so this can't go below zero.
        self.pending.fetch_sub(events.len(), Relaxed);
        events
    }

    /// The controllers connected now.
    pub fn pads(&self) -> Vec<PadInfo> {
        self.pads.lock().map_or_else(|_| Vec::new(), |p| p.clone())
    }

    /// Why controllers can't be read at all (no input system, for example), if they can't.
    pub fn error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|e| e.clone())
    }
}

impl Drop for Controllers {
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Shared {
    fn send(&self, event: RawEvent, droppable: bool) {
        if droppable && self.pending.load(Relaxed) > MAX_PENDING_ANALOG {
            return;
        }
        self.pending.fetch_add(1, Relaxed);
        if self.tx.send(event).is_err() {
            self.pending.fetch_sub(1, Relaxed);
        }
    }

    fn connect(&self, gilrs: &Gilrs, id: GamepadId, known: &mut HashSet<usize>) {
        let gamepad = gilrs.gamepad(id);
        let pad = usize::from(id);
        if is_sensor(gamepad.name()) || !known.insert(pad) {
            return;
        }
        let pad_info = info(id, &gamepad);
        self.send(
            RawEvent::Connected {
                pad,
                name: pad_info.label(),
            },
            false,
        );
        if let Ok(mut list) = self.pads.lock() {
            list.retain(|p| p.id != pad);
            list.push(pad_info);
        }
    }
}

fn run(shared: &Shared, wake: &(dyn Fn() + Send)) {
    let mut gilrs = match Gilrs::new() {
        Ok(g) => g,
        Err(e) => {
            if let Ok(mut slot) = shared.error.lock() {
                *slot = Some(e.to_string());
            }
            return;
        }
    };
    let mut known: HashSet<usize> = HashSet::new();
    let mut dpad = [0.0f32; 2];
    // Controllers already plugged in.
    let present: Vec<GamepadId> = gilrs.gamepads().map(|(id, _)| id).collect();
    for id in present {
        shared.connect(&gilrs, id, &mut known);
    }
    wake();
    while !shared.stop.load(Relaxed) {
        let Some(event) = gilrs.next_event_blocking(Some(Duration::from_millis(100))) else {
            continue;
        };
        let pad = usize::from(event.id);
        match event.event {
            EventType::Connected => shared.connect(&gilrs, event.id, &mut known),
            EventType::Disconnected => {
                if known.remove(&pad) {
                    if let Ok(mut list) = shared.pads.lock() {
                        list.retain(|p| p.id != pad);
                    }
                    shared.send(RawEvent::Disconnected { pad }, false);
                }
            }
            _ if !known.contains(&pad) => continue,
            EventType::ButtonPressed(b, _) | EventType::ButtonReleased(b, _) => {
                let pressed = matches!(event.event, EventType::ButtonPressed(..));
                if let Some(button) = button(b) {
                    shared.send(
                        RawEvent::Button {
                            pad,
                            button,
                            pressed,
                            value: f32::from(u8::from(pressed)),
                        },
                        false,
                    );
                }
            }
            EventType::ButtonChanged(b @ (Button::LeftTrigger2 | Button::RightTrigger2), v, _) => {
                if let Some(button) = button(b) {
                    shared.send(
                        RawEvent::Trigger {
                            pad,
                            button,
                            value: v,
                        },
                        true,
                    );
                }
            }
            EventType::AxisChanged(axis, value, _) => match axis {
                Axis::LeftStickX | Axis::LeftStickY | Axis::RightStickX | Axis::RightStickY => {
                    let axis = match axis {
                        Axis::LeftStickX => Ax::LeftX,
                        Axis::LeftStickY => Ax::LeftY,
                        Axis::RightStickX => Ax::RightX,
                        _ => Ax::RightY,
                    };
                    shared.send(RawEvent::Axis { pad, axis, value }, true);
                }
                Axis::DPadX | Axis::DPadY => {
                    // A pad whose D-pad is an axis: report it as the four buttons.
                    let i = usize::from(axis == Axis::DPadY);
                    let (neg, pos) = if i == 0 {
                        (Btn::DpadLeft, Btn::DpadRight)
                    } else {
                        (Btn::DpadDown, Btn::DpadUp)
                    };
                    let before = dpad[i];
                    dpad[i] = value;
                    for (button, was, now) in [
                        (neg, before < -0.5, value < -0.5),
                        (pos, before > 0.5, value > 0.5),
                    ] {
                        if was != now {
                            shared.send(
                                RawEvent::Button {
                                    pad,
                                    button,
                                    pressed: now,
                                    value: f32::from(u8::from(now)),
                                },
                                false,
                            );
                        }
                    }
                }
                _ => continue,
            },
            _ => continue,
        }
        wake();
    }
}
