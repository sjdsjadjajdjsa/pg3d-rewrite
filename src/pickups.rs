//! Pickups ("bonuses"): health and ammo boxes at the map's BonusCreationZone points.
//!
//! Model and sounds: <data>/pickups/ from ../pixel-gun-3d-re/scripts/export_pickups.py
//! (the original BonusGeneric prefab, which holds one child model per bonus type).

use bevy::{prelude::*, world_serialization::WorldInstanceReady};

use crate::{
    DataDir,
    bots::{PLAYER_MAX_HEALTH, PlayerStats},
    map::MapMarkers,
    player::Player,
    weapons::Loadout,
};

/// OBSERVED (reference clip d_rec8): a health pickup gives back half of max health
/// (32% -> 82%, and 51% -> full).
const HEALTH_AMOUNT: f32 = PLAYER_MAX_HEALTH * 0.5;
/// GUESS: respawn time is not recovered yet.
const RESPAWN_SECS: f32 = 15.0;
const PICKUP_RADIUS: f32 = 1.3;

pub struct PickupsPlugin;

impl Plugin for PickupsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_pickups)
            .add_systems(Update, (spin, collect, respawn));
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Health,
    Ammo,
}

impl Kind {
    /// child model name in the prefab and index of its sound in BonusItem.itemSounds
    fn model(self) -> (&'static str, usize) {
        match self {
            Kind::Health => ("health", 1),
            Kind::Ammo => ("ammo", 0),
        }
    }
}

#[derive(Component)]
struct Pickup {
    kind: Kind,
    home: Vec3,
    /// seconds until it comes back after being taken (0 = present)
    respawn: f32,
}

#[derive(Resource)]
struct PickupSounds(Vec<Handle<AudioSource>>);

fn spawn_pickups(mut commands: Commands, markers: Res<MapMarkers>, data: Res<DataDir>, assets: Res<AssetServer>) {
    if !data.0.join("pickups/pickups.gltf").exists() {
        return;
    }
    let sounds = (0..2).map(|i| assets.load(format!("pickups/sounds/{i}.wav"))).collect();
    commands.insert_resource(PickupSounds(sounds));
    let points: Vec<Vec3> =
        markers.0.iter().filter(|m| m.name.starts_with("BonusCreationZone")).map(|m| m.position).collect();
    for (i, p) in points.iter().enumerate() {
        let kind = if i % 2 == 0 { Kind::Health } else { Kind::Ammo };
        commands
            .spawn((
                Name::new(format!("pickup {kind:?}")),
                Pickup { kind, home: *p, respawn: 0.0 },
                WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset("pickups/pickups.gltf"))),
                Transform::from_translation(*p),
            ))
            .observe(show_only_kind);
    }
    info!("{} pickups", points.len());
}

/// The prefab contains every bonus model; keep only this pickup's one.
fn show_only_kind(
    ready: On<WorldInstanceReady>,
    pickups: Query<&Pickup>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut commands: Commands,
) {
    let Ok(p) = pickups.get(ready.entity) else { return };
    let keep = p.kind.model().0;
    let models = ["ammo", "health", "armor", "grenade", "mech"];
    for d in children.iter_descendants(ready.entity) {
        if let Ok(n) = names.get(d) {
            if models.contains(&n.as_str()) && n.as_str() != keep {
                commands.entity(d).insert(Visibility::Hidden);
            }
        }
    }
}

fn spin(time: Res<Time>, mut q: Query<(&mut Transform, &Pickup)>) {
    let t = time.elapsed_secs();
    for (mut tr, p) in &mut q {
        tr.rotation = Quat::from_rotation_y(t * 1.5);
        tr.translation = p.home + Vec3::Y * (0.15 * (t * 2.0).sin());
    }
}

fn collect(
    mut commands: Commands,
    player: Single<&Transform, (With<Player>, Without<Pickup>)>,
    mut q: Query<(&Transform, &mut Pickup, &mut Visibility)>,
    mut stats: ResMut<PlayerStats>,
    mut loadout: ResMut<Loadout>,
    sounds: Option<Res<PickupSounds>>,
) {
    if stats.health <= 0.0 {
        return;
    }
    for (t, mut p, mut vis) in &mut q {
        if p.respawn > 0.0 || t.translation.distance(player.translation) > PICKUP_RADIUS + 0.9 {
            continue;
        }
        let used = match p.kind {
            Kind::Health if stats.health < PLAYER_MAX_HEALTH => {
                stats.health = (stats.health + HEALTH_AMOUNT).min(PLAYER_MAX_HEALTH);
                true
            }
            Kind::Ammo => {
                // one extra magazine for the gun in hand only (OBSERVED in clip d_rec8:
                // 50/50 -> 50/100 on the selected gun, the other slots unchanged)
                let cur = loadout.current;
                match loadout.weapons.get_mut(cur) {
                    Some(w) if w.def.isMelee == 0 && w.reserve < w.def.maxAmmo => {
                        w.reserve = (w.reserve + w.def.ammoInClip).min(w.def.maxAmmo);
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        };
        if used {
            p.respawn = RESPAWN_SECS;
            *vis = Visibility::Hidden;
            if let Some(s) = &sounds {
                crate::sound::play(&mut commands, s.0.get(p.kind.model().1), None, 1.0);
            }
            info!("picked up {:?}", p.kind);
        }
    }
}

fn respawn(time: Res<Time>, mut q: Query<(&mut Pickup, &mut Visibility)>) {
    for (mut p, mut vis) in &mut q {
        if p.respawn > 0.0 {
            p.respawn -= time.delta_secs();
            if p.respawn <= 0.0 {
                p.respawn = 0.0;
                *vis = Visibility::Inherited;
            }
        }
    }
}
