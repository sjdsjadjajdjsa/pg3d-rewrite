//! The player's level and XP, saved next to the settings
//! (~/.config/pixel-gun-3d-rs/progress.json). The XP each level needs is the game's
//! (BalanceLeveling.LevellingV3: `XP` of level L+1 is what level L needs to go up).

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
#[serde(default)]
pub struct Progress {
    pub level: i32,
    /// XP gathered on the current level
    pub xp: i64,
}

impl Default for Progress {
    fn default() -> Self {
        Self { level: 1, xp: 0 }
    }
}

#[derive(Deserialize)]
struct Level {
    #[serde(rename = "L")]
    l: i32,
    #[serde(rename = "XP")]
    xp: i64,
}

#[derive(Deserialize, Default)]
struct Balance {
    #[serde(default)]
    levels: Vec<Level>,
}

impl Progress {
    fn path() -> std::path::PathBuf {
        crate::settings::Settings::config_dir().join("progress.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn save(&self) {
        let p = Self::path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&p, serde_json::to_string_pretty(self).unwrap_or_default());
    }

    /// XP needed to go from `level` to the next one (None at the top).
    pub fn needed(data: &std::path::Path, level: i32) -> Option<i64> {
        let b: Balance = std::fs::read_to_string(data.join("weapons/balance.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        b.levels.iter().find(|l| l.l == level + 1).map(|l| l.xp)
    }

    /// Add XP and save; true if the player went up a level.
    pub fn add_xp(data: &std::path::Path, xp: i64) -> bool {
        let mut p = Self::load();
        let start = p.level;
        p.xp += xp;
        while let Some(n) = Self::needed(data, p.level).filter(|n| p.xp >= *n) {
            p.xp -= n;
            p.level += 1;
        }
        p.save();
        p.level > start
    }
}
