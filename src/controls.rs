//! Key bindings: which key or mouse button triggers each game action. Saved as JSON next to
//! the settings (~/.config/pixel-gun-3d-rs/controls.json) and edited in the settings window's
//! key binding tab (settings_ui.rs). Game code asks [`Controls`] instead of reading keys.

use std::collections::HashMap;

use bevy::{ecs::system::SystemParam, prelude::*};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Action {
    Forward,
    Back,
    Left,
    Right,
    Jump,
    Fire,
    Aim,
    Reload,
    FreeCamera,
}

impl Action {
    pub const ALL: [Action; 9] = [
        Action::Forward,
        Action::Back,
        Action::Left,
        Action::Right,
        Action::Jump,
        Action::Fire,
        Action::Aim,
        Action::Reload,
        Action::FreeCamera,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Action::Forward => "forward",
            Action::Back => "back",
            Action::Left => "left",
            Action::Right => "right",
            Action::Jump => "jump",
            Action::Fire => "fire",
            Action::Aim => "aim",
            Action::Reload => "reload",
            Action::FreeCamera => "free_camera",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Action::Forward => "Move forward",
            Action::Back => "Move back",
            Action::Left => "Move left",
            Action::Right => "Move right",
            Action::Jump => "Jump",
            Action::Fire => "Shoot",
            Action::Aim => "Aim (zoom)",
            Action::Reload => "Reload",
            Action::FreeCamera => "Free camera",
        }
    }

    fn default_bind(self) -> Bind {
        match self {
            Action::Forward => Bind::Key(KeyCode::KeyW),
            Action::Back => Bind::Key(KeyCode::KeyS),
            Action::Left => Bind::Key(KeyCode::KeyA),
            Action::Right => Bind::Key(KeyCode::KeyD),
            Action::Jump => Bind::Key(KeyCode::Space),
            Action::Fire => Bind::Mouse(MouseButton::Left),
            Action::Aim => Bind::Mouse(MouseButton::Right),
            Action::Reload => Bind::Key(KeyCode::KeyR),
            Action::FreeCamera => Bind::Key(KeyCode::KeyV),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bind {
    Key(KeyCode),
    Mouse(MouseButton),
}

macro_rules! keys {
    ($($k:ident),* $(,)?) => {
        const KEYS: &[(KeyCode, &str)] = &[$((KeyCode::$k, stringify!($k))),*];
    };
}

// Keys that can be bound (Escape opens the pause screen, F10/F12 are reserved).
keys!(
    KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO, KeyP, KeyQ,
    KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ, Digit0, Digit1, Digit2, Digit3, Digit4, Digit5,
    Digit6, Digit7, Digit8, Digit9, Space, Tab, Enter, Backspace, CapsLock, ShiftLeft, ShiftRight,
    ControlLeft, ControlRight, AltLeft, AltRight, ArrowUp, ArrowDown, ArrowLeft, ArrowRight, Insert,
    Delete, Home, End, PageUp, PageDown, Backquote, Minus, Equal, BracketLeft, BracketRight, Backslash,
    Semicolon, Quote, Comma, Period, Slash, F1, F2, F3, F4, F5, F6, F7, F8, F9, F11, Numpad0, Numpad1,
    Numpad2, Numpad3, Numpad4, Numpad5, Numpad6, Numpad7, Numpad8, Numpad9, NumpadEnter, NumpadAdd,
    NumpadSubtract,
);

const MOUSE: &[(MouseButton, &str)] = &[
    (MouseButton::Left, "MouseLeft"),
    (MouseButton::Right, "MouseRight"),
    (MouseButton::Middle, "MouseMiddle"),
    (MouseButton::Back, "MouseBack"),
    (MouseButton::Forward, "MouseForward"),
];

impl Bind {
    fn id(self) -> Option<&'static str> {
        match self {
            Bind::Key(k) => KEYS.iter().find(|(c, _)| *c == k).map(|(_, n)| *n),
            Bind::Mouse(m) => MOUSE.iter().find(|(c, _)| *c == m).map(|(_, n)| *n),
        }
    }

    pub fn parse(s: &str) -> Option<Bind> {
        KEYS.iter()
            .find(|(_, n)| *n == s)
            .map(|(k, _)| Bind::Key(*k))
            .or_else(|| MOUSE.iter().find(|(_, n)| *n == s).map(|(m, _)| Bind::Mouse(*m)))
    }

    /// Short name shown in the settings window ("W", "Space", "Left Shift", "Mouse 1").
    pub fn label(self) -> String {
        let id = self.id().unwrap_or("?");
        match self {
            Bind::Mouse(MouseButton::Left) => "Mouse 1".into(),
            Bind::Mouse(MouseButton::Right) => "Mouse 2".into(),
            Bind::Mouse(MouseButton::Middle) => "Mouse 3".into(),
            Bind::Mouse(MouseButton::Back) => "Mouse 4".into(),
            Bind::Mouse(_) => "Mouse 5".into(),
            Bind::Key(_) => {
                if let Some(r) = id.strip_prefix("Key").or(id.strip_prefix("Digit")) {
                    return r.into();
                }
                for side in ["Left", "Right"] {
                    if let Some(r) = id.strip_suffix(side).filter(|r| !r.is_empty() && *r != "Arrow") {
                        return format!("{side} {}", r.replace("Control", "Ctrl"));
                    }
                }
                if let Some(r) = id.strip_prefix("Arrow") {
                    return format!("{r} arrow");
                }
                if let Some(r) = id.strip_prefix("Numpad") {
                    return format!("Num {r}");
                }
                id.into()
            }
        }
    }

    /// Bindable key/button pressed this frame, if any.
    pub fn just_pressed(keys: &ButtonInput<KeyCode>, mouse: &ButtonInput<MouseButton>) -> Option<Bind> {
        keys.get_just_pressed()
            .find(|k| KEYS.iter().any(|(c, _)| c == *k))
            .map(|k| Bind::Key(*k))
            .or_else(|| mouse.get_just_pressed().find(|m| MOUSE.iter().any(|(c, _)| c == *m)).map(|m| Bind::Mouse(*m)))
    }
}

#[derive(Resource, Clone, PartialEq, Debug)]
pub struct Controls(pub HashMap<Action, Bind>);

impl Default for Controls {
    fn default() -> Self {
        Self(Action::ALL.iter().map(|a| (*a, a.default_bind())).collect())
    }
}

impl Controls {
    fn path() -> std::path::PathBuf {
        crate::settings::Settings::config_dir().join("controls.json")
    }

    /// Saved bindings; missing or unknown entries fall back to the defaults.
    pub fn load() -> Self {
        let mut c = Self::default();
        let saved: HashMap<String, String> =
            std::fs::read_to_string(Self::path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        for a in Action::ALL {
            if let Some(b) = saved.get(a.id()).and_then(|s| Bind::parse(s)) {
                c.0.insert(a, b);
            }
        }
        c
    }

    pub fn save(&self) {
        let out: std::collections::BTreeMap<&str, &str> =
            self.0.iter().filter_map(|(a, b)| Some((a.id(), b.id()?))).collect();
        let p = Self::path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Err(e) = std::fs::write(&p, serde_json::to_string_pretty(&out).unwrap_or_default()) {
            warn!("could not save key bindings to {}: {e}", p.display());
        }
    }

    pub fn get(&self, a: Action) -> Bind {
        self.0.get(&a).copied().unwrap_or(a.default_bind())
    }

    /// The other action already using `bind`, if any.
    pub fn user_of(&self, bind: Bind, except: Action) -> Option<Action> {
        Action::ALL.into_iter().find(|a| *a != except && self.get(*a) == bind)
    }
}

/// Reads actions through the current bindings.
#[derive(SystemParam)]
pub struct Input<'w> {
    keys: Res<'w, ButtonInput<KeyCode>>,
    mouse: Res<'w, ButtonInput<MouseButton>>,
    controls: Res<'w, Controls>,
}

impl Input<'_> {
    pub fn pressed(&self, a: Action) -> bool {
        match self.controls.get(a) {
            Bind::Key(k) => self.keys.pressed(k),
            Bind::Mouse(m) => self.mouse.pressed(m),
        }
    }

    pub fn just_pressed(&self, a: Action) -> bool {
        match self.controls.get(a) {
            Bind::Key(k) => self.keys.just_pressed(k),
            Bind::Mouse(m) => self.mouse.just_pressed(m),
        }
    }
}
