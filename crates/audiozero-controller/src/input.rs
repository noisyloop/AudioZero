//! Controller inputs, named by where they are on a DualShock 4 (other pads map onto the same
//! names).

use serde::{Deserialize, Serialize};

/// A button. `Cross`, `Circle`, `Square` and `Triangle` are the face buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Btn {
    Cross,
    Circle,
    Square,
    Triangle,
    DpadLeft,
    DpadUp,
    DpadRight,
    DpadDown,
    L1,
    R1,
    L2,
    R2,
    L3,
    R3,
    Options,
    Share,
    Ps,
}

/// A stick axis, from -1 to 1. Up and right are positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Ax {
    LeftX,
    LeftY,
    RightX,
    RightY,
}

/// What a controller reports, already named. `pad` tells controllers apart.
#[derive(Debug, Clone, PartialEq)]
pub enum RawEvent {
    Connected {
        pad: usize,
        name: String,
    },
    Disconnected {
        pad: usize,
    },
    /// A button went down or up. For L2 and R2 `value` is how far they're pulled (0 to 1).
    Button {
        pad: usize,
        button: Btn,
        pressed: bool,
        value: f32,
    },
    /// A trigger moved (L2 and R2 are analog).
    Trigger {
        pad: usize,
        button: Btn,
        value: f32,
    },
    Axis {
        pad: usize,
        axis: Ax,
        value: f32,
    },
}

impl RawEvent {
    pub fn pad(&self) -> usize {
        match *self {
            Self::Connected { pad, .. }
            | Self::Disconnected { pad }
            | Self::Button { pad, .. }
            | Self::Trigger { pad, .. }
            | Self::Axis { pad, .. } => pad,
        }
    }
}

impl std::fmt::Display for RawEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connected { pad, name } => write!(f, "pad {pad} connected: {name}"),
            Self::Disconnected { pad } => write!(f, "pad {pad} disconnected"),
            Self::Button {
                button, pressed, ..
            } => write!(f, "{button:?} {}", if *pressed { "down" } else { "up" }),
            Self::Trigger { button, value, .. } => write!(f, "{button:?} {value:.2}"),
            Self::Axis { axis, value, .. } => write!(f, "{axis:?} {value:+.2}"),
        }
    }
}
