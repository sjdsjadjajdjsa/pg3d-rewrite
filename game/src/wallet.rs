//! The player's coins and gems, saved as JSON next to the settings
//! (~/.config/pixel-gun-3d-rs/wallet.json). Shown by the HUD's coin/gem bar (BankShopView).

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Resource, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(default)]
pub struct Wallet {
    pub coins: i64,
    pub gems: i64,
}

impl Wallet {
    fn path() -> std::path::PathBuf {
        crate::settings::Settings::config_dir().join("wallet.json")
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
            warn!("could not save the wallet to {}: {e}", p.display());
        }
    }

    pub fn add_coins(&mut self, n: i64) {
        self.coins = self.coins.saturating_add(n).max(0);
        self.save();
    }

    pub fn add_gems(&mut self, n: i64) {
        self.gems = self.gems.saturating_add(n).max(0);
        self.save();
    }
}
