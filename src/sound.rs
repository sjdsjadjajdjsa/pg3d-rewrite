//! Playing the original sound effects (exported as .wav next to each weapon / bot).

use std::collections::HashMap;

use bevy::{
    audio::{PlaybackMode, SpatialScale, Volume},
    prelude::*,
};

pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, add_listener);
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
pub fn play(commands: &mut Commands, sound: Option<&Handle<AudioSource>>, at: Option<Vec3>, volume: f32) {
    let Some(sound) = sound else { return };
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
