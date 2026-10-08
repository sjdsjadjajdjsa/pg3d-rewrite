//! Explosive TNT boxes. The original places them with ExplosionObjectRespawnController
//! (markers "Bomb_TNT_Respawn"); the exporter hangs the prefab (Bomb_TNT) under each marker
//! and tags it with glTF extras `{"explosive": {radius, radius_max, damage_percent, respawn,
//! box_center, box_size}}` (DamagedExplosionObject + BoxCollider fields).
//! Shooting one blows it up, hurting zombies around it; it comes back after `respawn` s.

use avian3d::prelude::*;
#[allow(unused_imports)]
use crate::scene::{FixedUpdate, PostStartup, PostUpdate, PreStartup, Startup, Update};
use bevy::{gltf::GltfExtras, prelude::*};
use serde::Deserialize;

use crate::{
    bots::{BotDamage, BotPart},
    map::Layer,
    projectiles::{Look, Owner, SpawnProjectile},
};

pub struct ExplosivesPlugin;

impl crate::scene::ScenePlugin for ExplosivesPlugin {
    fn build(&self, app: &mut crate::scene::SceneBuilder) {
        app.add_systems(Update, (tag_explosives, explode, respawn));
    }
}

#[derive(Deserialize, Clone)]
struct ExplosiveDef {
    radius: f32,
    /// full damage inside this distance (DamagedExplosionObject.radiusMax)
    #[serde(default)]
    radius_max: f32,
    #[serde(default)]
    respawn: f32,
    #[serde(default)]
    damage_percent: f32,
    #[serde(default)]
    box_center: Option<Vec3>,
    #[serde(default)]
    box_size: Option<Vec3>,
}

#[derive(Deserialize)]
struct Extras {
    explosive: ExplosiveDef,
}

#[derive(Component)]
struct Explosive {
    def: ExplosiveDef,
    /// seconds until it is back (while blown up)
    gone: f32,
    hitbox: Entity,
}

/// Bot damage the original deals at the centre of the blast is `damagePercent` of the
/// bot's health; our explosions deal absolute damage, so use this much per percent (GUESS:
/// 90% -> 27, enough to kill all early zombie types).
const DAMAGE_PER_PERCENT: f32 = 0.3;

fn tag_explosives(mut commands: Commands, added: Query<(Entity, &GltfExtras), Added<GltfExtras>>) {
    for (e, x) in &added {
        let Ok(Extras { explosive: def }) = serde_json::from_str::<Extras>(&x.value) else { continue };
        let size = def.box_size.unwrap_or(Vec3::splat(0.6));
        let hitbox = commands
            .spawn((
                BotPart(e),
                Collider::cuboid(size.x, size.y, size.z),
                Transform::from_translation(def.box_center.unwrap_or(Vec3::Y * 0.3)),
                CollisionLayers::new(Layer::Enemy, LayerMask::ALL),
            ))
            .id();
        commands.entity(e).add_child(hitbox).insert(Explosive { def, gone: 0.0, hitbox });
    }
}

fn explode(
    mut commands: Commands,
    mut hits: MessageReader<BotDamage>,
    mut q: Query<(&mut Explosive, &GlobalTransform)>,
    mut shots: MessageWriter<SpawnProjectile>,
    mut bursts: MessageWriter<crate::particles::Burst>,
    mut stats: ResMut<crate::bots::PlayerStats>,
    mut player_hits: MessageWriter<crate::bots::PlayerHit>,
    player: Single<&GlobalTransform, With<crate::player::Player>>,
) {
    for h in hits.read() {
        let Ok((mut x, gt)) = q.get_mut(h.bot) else { continue };
        if x.gone > 0.0 {
            continue;
        }
        x.gone = if x.def.respawn > 0.0 { x.def.respawn } else { 90.0 };
        let centre = gt.translation() + Vec3::Y * 0.3;
        info!("TNT exploded at {centre:.1}");
        // a projectile that bursts on the spot: same blast + damage as rockets
        shots.write(SpawnProjectile {
            owner: Owner::Player,
            from: centre,
            dir: Dir3::Y,
            speed: 0.01,
            life: 0.0,
            gravity: false,
            damage: x.def.damage_percent.max(30.0) * DAMAGE_PER_PERCENT,
            explosion_radius: x.def.radius.clamp(1.0, 10.0),
            look: Look::Ball { color: Color::srgb(1.0, 0.6, 0.1), radius: 0.05 },
        });
        // the blast hurts the player too: damagePercent of max health within radiusMax,
        // fading to nothing at radius. OBSERVED (clip d_rec4): ~3-4 m away took 100% -> 22%.
        let d = player.translation().distance(centre);
        let r0 = x.def.radius_max.max(0.0);
        let r1 = x.def.radius.max(r0 + 0.01);
        let k = 1.0 - ((d - r0) / (r1 - r0)).clamp(0.0, 1.0);
        if k > 0.0 && stats.protected <= 0.0 && stats.health > 0.0 {
            stats.health -= crate::bots::PLAYER_MAX_HEALTH * x.def.damage_percent / 100.0 * k;
            info!("TNT hurt the player {d:.1} m away: {:.0}% of max health", x.def.damage_percent * k);
            player_hits.write(crate::bots::PlayerHit { from: centre });
        }
        // the prefab's own blast effect (flash, sparks, fire), exported under the TNT node
        bursts.write(crate::particles::Burst(h.bot));
        commands.entity(h.bot).insert(Visibility::Hidden);
        commands.entity(x.hitbox).insert(CollisionLayers::NONE);
    }
}

fn respawn(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut Explosive)>) {
    for (e, mut x) in &mut q {
        if x.gone <= 0.0 {
            continue;
        }
        x.gone -= time.delta_secs();
        if x.gone <= 0.0 {
            commands.entity(e).insert(Visibility::Inherited);
            commands.entity(x.hitbox).insert(CollisionLayers::new(Layer::Enemy, LayerMask::ALL));
        }
    }
}
