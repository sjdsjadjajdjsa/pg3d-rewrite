//! Playing the original sound effects (exported as .wav next to each weapon / bot).

use std::collections::HashMap;

use bevy::{
    audio::{PlaybackMode, SpatialScale, Volume},
    prelude::*,
};

pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, add_listener).add_systems(Update, player_sounds);
    }
}

/// The camera is the "ears" for 3D sounds.
fn add_listener(mut commands: Commands, camera: Single<Entity, With<crate::camera::PlayerCamera>>) {
    commands.entity(*camera).insert(SpatialListener::new(0.3));
}

/// Load `sounds` from info.json: {field: "sounds/<field>.wav"} under `dir` (asset path).
pub fn load_sounds(assets: &AssetServer, dir: &str, sounds: &HashMap<String, String>) -> HashMap<String, Handle<AudioSource>> {
    sounds.iter().map(|(k, rel)| (k.clone(), assets.load(format!("{dir}/{rel}")))).collect()
}

/// Play a sound once. `at = None` plays it "in your head" (your own weapon); `Some(pos)`
/// plays it in the world so it gets quieter with distance and comes from that side.
/// Sound effect volume from the settings (0..1), applied to every effect.
static SFX_VOLUME: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f800000); // 1.0

pub fn set_sfx_volume(v: f32) {
    SFX_VOLUME.store(v.clamp(0.0, 1.0).to_bits(), std::sync::atomic::Ordering::Relaxed);
}

pub fn play(commands: &mut Commands, sound: Option<&Handle<AudioSource>>, at: Option<Vec3>, volume: f32) {
    let Some(sound) = sound else { return };
    debug!("sound {:?}", sound.path());
    let volume = volume * f32::from_bits(SFX_VOLUME.load(std::sync::atomic::Ordering::Relaxed));
    let settings = PlaybackSettings {
        mode: PlaybackMode::Despawn,
        volume: Volume::Linear(volume),
        spatial: at.is_some(),
        // distances are divided by 4 before the 1/distance fall-off: audible across a map
        spatial_scale: at.map(|_| SpatialScale::new(0.25)),
        ..PlaybackSettings::DESPAWN
    };
    let mut e = commands.spawn((AudioPlayer(sound.clone()), settings));
    if let Some(p) = at {
        e.insert(Transform::from_translation(p));
    }
}

/// The game's own non-weapon sounds (export_game_sounds.py -> sounds/index.json):
/// "player/jumpAudio", "ui/Click", "lobby/arsenalLobbyButtonSound", ... Lists (footsteps,
/// monster roars) keep all their variants.
#[derive(Resource, Default)]
pub struct GameSounds(pub HashMap<String, Vec<Handle<AudioSource>>>);

impl GameSounds {
    /// One of the sound's variants (picked by `seed`).
    pub fn get(&self, key: &str, seed: usize) -> Option<&Handle<AudioSource>> {
        let v = self.0.get(key)?;
        v.get(seed % v.len().max(1))
    }
}

pub fn load_game_sounds(mut commands: Commands, assets: Res<AssetServer>, data: Res<crate::DataDir>) {
    let index: HashMap<String, HashMap<String, Vec<String>>> = std::fs::read_to_string(data.0.join("sounds/index.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let mut out = HashMap::new();
    for (group, fields) in index {
        for (field, files) in fields {
            out.insert(format!("{group}/{field}"), files.iter().map(|f| assets.load(format!("sounds/{f}"))).collect());
        }
    }
    commands.insert_resource(GameSounds(out));
}

/// Every NGUI button clicks like the original (ButtonClickSound "snd_button_click"); the
/// lobby's big buttons have their own (LobbyMainMenuSounds).
pub fn ui_click_sounds(mut commands: Commands, mut clicks: MessageReader<crate::ngui::NguiClick>, sounds: Option<Res<GameSounds>>) {
    let Some(sounds) = sounds else { return };
    for c in clicks.read() {
        let own = match c.name.as_str() {
            "BtnArmory" => "lobby/arsenalLobbyButtonSound",
            "BtnSettings" => "lobby/settingsLobbyButtonSound",
            "BtnFriends" => "lobby/friendsLobbyButtonSound",
            "btn_Profile" => "lobby/profileLobbyButtonSound",
            "BtnClan" => "lobby/socialLobbyButtonSound",
            "BtnLobby_Craft" => "lobby/craftLobbyButtonSound",
            _ => "ui/Click",
        };
        play(&mut commands, sounds.get(own, 0).or_else(|| sounds.get("ui/Click", 0)), None, 1.0);
    }
}

/// The player's own sounds (Player prefab: SkinName / PlayerMovementsSoundsController /
/// Player_move_c): footsteps while walking, jump and landing, getting hurt, dying, and the
/// weapon-switch sound of the new weapon's category.
#[allow(clippy::too_many_arguments)]
pub fn player_sounds(
    mut commands: Commands,
    time: Res<Time>,
    sounds: Option<Res<GameSounds>>,
    player: Query<&crate::player::Player>,
    stats: Option<Res<crate::bots::PlayerStats>>,
    loadout: Option<Res<crate::weapons::Loadout>>,
    mut st: Local<(bool, f32, Option<usize>, f32, usize, bool)>,
    mut air: Local<f32>,
) {
    let Some(sounds) = sounds else { return };
    let Ok(p) = player.single() else { return };
    let (was_grounded, last_health, last_weapon, step_timer, step_n, started) = &mut *st;
    let mut s = |key: &str, seed: usize, vol: f32| play(&mut commands, sounds.get(key, seed), None, vol);
    if *started {
        // took off / landed
        if *was_grounded && !p.grounded && p.velocity.y > 1.0 {
            s("player/jumpAudio", 0, 0.8);
        } else if !*was_grounded && p.grounded && *air > 0.3 {
            // a real fall, not a step down a ledge
            s("player/jumpDownAudio", 0, 0.8);
        }
    }
    *air = if p.grounded { 0.0 } else { *air + time.delta_secs() };
    *was_grounded = p.grounded;
    // footsteps: a random one of the 9 walk steps every ~0.35 s while moving on the ground
    let speed = Vec2::new(p.velocity.x, p.velocity.z).length();
    if p.grounded && speed > 1.0 {
        *step_timer -= time.delta_secs() * (speed / 5.0).clamp(0.6, 1.6);
        if *step_timer <= 0.0 {
            *step_timer = 0.35;
            *step_n = step_n.wrapping_mul(1103515245).wrapping_add(12345);
            s("player/stepWalkAudioClips", *step_n >> 8, 0.5);
        }
    } else {
        *step_timer = 0.0;
    }
    if let Some(stats) = stats {
        if *started && stats.health < *last_health - 0.01 {
            if stats.health <= 0.0 {
                s("player/deadPlayerSound", 0, 1.0);
                // Player_move_c.killedByMonster: the enemies here are all monsters
                s("player/killedByMonster", 0, 1.0);
            } else {
                s("player/damagePlayerSound", 0, 0.9);
            }
        }
        *last_health = stats.health;
    }
    if let Some(l) = loadout {
        if *started && last_weapon.is_some() && *last_weapon != Some(l.current) && !l.weapons.is_empty() {
            // slots 1-6: Primary, Backup, Melee, Special, Sniper, Heavy
            let key = match l.current {
                0 => "player/ChangeWeaponClip_Primary",
                1 => "player/ChangeWeaponClip_Backup",
                2 => "player/ChangeWeaponClip_Melee",
                3 => "player/ChangeWeaponClip_Special",
                4 => "player/ChangeWeaponClip_Sniper",
                5 => "player/ChangeWeaponClip_Heavy",
                _ => "player/ChangeWeaponClip",
            };
            s(key, 0, 0.8);
        }
        *last_weapon = Some(l.current);
    }
    *started = true;
}
