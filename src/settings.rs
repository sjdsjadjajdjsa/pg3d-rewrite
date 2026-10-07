//! Player settings (volumes, mouse sensitivity), saved as JSON in the user's config folder
//! (~/.config/pixel-gun-3d-rs/settings.json on Linux). Both the menu and the game read it.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Resource, Serialize, Deserialize, Clone, Copy, Debug)]
#[serde(default)]
pub struct Settings {
    /// 0..1
    pub music: f32,
    /// 0..1, sound effects
    pub sfx: f32,
    /// 0..1, mapped to mouse radians-per-pixel by `mouse_sensitivity()`
    pub sensitivity: f32,
    /// vertical field of view in degrees, as the original's Settings > Controls "Field of
    /// View" (it goes straight into Unity's Camera.fieldOfView, which is vertical too)
    pub fov: f32,
    /// Settings > "Invert cam": moving the mouse up looks down
    pub invert_y: bool,
    /// the check boxes next to the Music / Sound FX sliders (off = muted)
    pub music_on: bool,
    pub sfx_on: bool,
}

impl Default for Settings {
    fn default() -> Self {
        // 80: the value in the original's settings in our reference screenshots
        Self { music: 0.8, sfx: 0.8, sensitivity: 0.5, fov: 80.0, invert_y: false, music_on: true, sfx_on: true }
    }
}

impl Settings {
    /// ~/.config/pixel-gun-3d-rs (or $XDG_CONFIG_HOME/pixel-gun-3d-rs).
    pub fn config_dir() -> std::path::PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| ".".into());
        base.join("pixel-gun-3d-rs")
    }

    fn path() -> std::path::PathBuf {
        Self::config_dir().join("settings.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let p = Self::path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&p, serde_json::to_string_pretty(self).unwrap_or_default()) {
            warn!("could not save settings to {}: {e}", p.display());
        }
    }

    pub fn music_volume(&self) -> f32 {
        if self.music_on { self.music } else { 0.0 }
    }

    pub fn sfx_volume(&self) -> f32 {
        if self.sfx_on { self.sfx } else { 0.0 }
    }

    /// Radians of camera turn per pixel of mouse movement (0.0005 .. 0.0045).
    pub fn mouse_sensitivity(&self) -> f32 {
        0.0005 + self.sensitivity.clamp(0.0, 1.0) * 0.004
    }
}
