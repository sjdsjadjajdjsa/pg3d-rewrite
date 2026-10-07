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
            .add_systems(Update, (spin, collect, respawn, wave_prize));
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Health,
    Ammo,
    /// the prize after each Arena wave
    Coin,
}

impl Kind {
    /// child model name in the prefab and index of its sound in BonusItem.itemSounds
    fn model(self) -> (&'static str, usize) {
        match self {
            Kind::Health => ("health", 1),
            Kind::Ammo => ("ammo", 0),
            Kind::Coin => ("Coin", 17),
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
struct PickupSounds(Vec<Option<Handle<AudioSource>>>);

fn spawn_pickups(mut commands: Commands, markers: Res<MapMarkers>, data: Res<DataDir>, assets: Res<AssetServer>) {
    if !data.0.join("pickups/pickups.gltf").exists() {
        return;
    }
    // BonusItem.itemSounds has gaps (no sound for some bonus types)
    let sounds = (0..18)
        .map(|i| format!("pickups/sounds/{i}.wav"))
        .map(|f| data.0.join(&f).exists().then(|| assets.load(f)))
        .collect();
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
    let models = ["ammo", "health", "armor", "grenade", "mech", "Coin"];
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
    mut q: Query<(Entity, &Transform, &mut Pickup, &mut Visibility)>,
    mut stats: ResMut<PlayerStats>,
    mut loadout: ResMut<Loadout>,
    sounds: Option<Res<PickupSounds>>,
    mut wallet: ResMut<crate::wallet::Wallet>,
) {
    if stats.health <= 0.0 {
        return;
    }
    for (e, t, mut p, mut vis) in &mut q {
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
            Kind::Coin => true,
            _ => false,
        };
        if used {
            p.respawn = RESPAWN_SECS;
            *vis = Visibility::Hidden;
            if p.kind == Kind::Coin {
                // the prize is one coin (OBSERVED: 5 076 -> 5 077 in clip e_rec2)
                wallet.add_coins(1);
                commands.entity(e).despawn();
            }
            if let Some(s) = &sounds {
                crate::sound::play(&mut commands, s.0.get(p.kind.model().1).and_then(Option::as_ref), None, 1.0);
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

/// Between Arena waves a coin (the "prize") appears in the middle of the map. OBSERVED in
/// clip d_rec7: it is there as "Wave is over! Pick up the prize!" shows, stays until taken,
/// and taking it adds 1 coin. Its exact spot is not recovered yet: we use the middle of the
/// map's BonusCreationZone points.
fn wave_prize(
    mut commands: Commands,
    wave: Res<crate::bots::Wave>,
    markers: Res<MapMarkers>,
    data: Res<DataDir>,
    assets: Res<AssetServer>,
    coins: Query<(Entity, &Pickup)>,
    mut given: Local<u32>,
) {
    let between = wave.number > 0 && wave.left == 0 && wave.break_left > 0.0;
    // PG3D_PRIZE (automated screenshots): a prize right at the start
    let test = *given == 0 && std::env::var("PG3D_PRIZE").is_ok();
    if !test && (!between || *given == wave.number) {
        return;
    }
    *given = wave.number.max(1);
    // one prize at a time: an old one not taken is replaced
    for (e, p) in &coins {
        if p.kind == Kind::Coin {
            commands.entity(e).despawn();
        }
    }
    if !data.0.join("pickups/pickups.gltf").exists() {
        return;
    }
    let pts: Vec<Vec3> =
        markers.0.iter().filter(|m| m.name.starts_with("BonusCreationZone")).map(|m| m.position).collect();
    if pts.is_empty() {
        return;
    }
    let home = pts.iter().sum::<Vec3>() / pts.len() as f32;
    commands
        .spawn((
            Name::new("pickup Coin"),
            Pickup { kind: Kind::Coin, home, respawn: 0.0 },
            WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset("pickups/pickups.gltf"))),
            Transform::from_translation(home),
        ))
        .observe(show_only_kind);
    info!("wave prize at {home}");
}
