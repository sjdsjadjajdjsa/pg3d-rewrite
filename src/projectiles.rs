//! Projectiles: player rockets (weapons with `bazooka = 1`) and enemy bullets
//! (RilisoftBot.ShootingBot). They fly in a straight line (or arc, with gravity), check
//! for hits with a ray along each frame's movement, and explode or deal direct damage.

use avian3d::prelude::*;
#[allow(unused_imports)]
use crate::scene::{FixedUpdate, PostStartup, PostUpdate, PreStartup, Startup, Update};
use bevy::prelude::*;

use crate::{
    bots::{BotDamage, BotPart, PlayerStats},
    map::Layer,
};

pub struct ProjectilesPlugin;

impl crate::scene::ScenePlugin for ProjectilesPlugin {
    fn build(&self, app: &mut crate::scene::SceneBuilder) {
        app.add_systems(Update, (spawn_projectiles, fly, explosions).chain());
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Owner {
    Player,
    Enemy,
}

#[derive(Clone, Debug)]
pub enum Look {
    /// glTF model path relative to the data folder (rockets)
    Model(String),
    /// glowing ball (enemy bullets)
    Ball { color: Color, radius: f32 },
}

#[derive(Message, Clone, Debug)]
pub struct SpawnProjectile {
    pub owner: Owner,
    pub from: Vec3,
    pub dir: Dir3,
    pub speed: f32,
    pub life: f32,
    pub gravity: bool,
    /// damage on direct hit (and at the centre of an explosion)
    pub damage: f32,
    /// > 0: explodes, damaging everything within this radius
    pub explosion_radius: f32,
    /// your own rockets push you away (rocket jumps): (radius, force) from the weapon's
    /// bazookaImpulseRadius / impulseForceSelf; radius 0: no push
    pub self_impulse: (f32, f32),
    pub look: Look,
}

#[derive(Component)]
struct Projectile {
    spec: SpawnProjectile,
    velocity: Vec3,
    age: f32,
}

#[derive(Component)]
pub struct Explosion {
    age: f32,
    radius: f32,
}

fn spawn_projectiles(
    mut commands: Commands,
    mut msgs: MessageReader<SpawnProjectile>,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for m in msgs.read() {
        let t = Transform::from_translation(m.from).looking_to(m.dir, Vec3::Y);
        let mut e = commands.spawn((
            Name::new("projectile"),
            Projectile { spec: m.clone(), velocity: *m.dir * m.speed, age: 0.0 },
            t,
            Visibility::default(),
        ));
        match &m.look {
            // the original models point down Unity's +Z = our +Z; Bevy "forward" is -Z
            Look::Model(path) => {
                e.with_children(|c| {
                    c.spawn((
                        WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(path.clone()))),
                        Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
                    ));
                });
            }
            Look::Ball { color, radius } => {
                e.insert((
                    Mesh3d(meshes.add(Sphere::new(*radius))),
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color: *color,
                        emissive: color.to_linear() * 3.0,
                        unlit: true,
                        ..default()
                    })),
                ));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fly(
    mut commands: Commands,
    time: Res<Time>,
    spatial: SpatialQuery,
    mut q: Query<(Entity, &mut Transform, &mut Projectile)>,
    bot_parts: Query<&BotPart>,
    bots: Query<(Entity, &GlobalTransform), With<crate::bots::BotMarker>>,
    mut player: Single<(&Transform, &mut crate::player::Player), Without<Projectile>>,
    mut stats: ResMut<PlayerStats>,
    mut hits: MessageWriter<BotDamage>,
    mut player_hits: MessageWriter<crate::bots::PlayerHit>,
) {
    let dt = time.delta_secs();
    for (e, mut t, mut p) in &mut q {
        p.age += dt;
        if p.spec.gravity {
            p.velocity.y -= crate::player::GRAVITY * dt;
        }
        let step = p.velocity * dt;
        let Ok(dir) = Dir3::new(step) else { continue };
        let mask = match p.spec.owner {
            Owner::Player => LayerMask::from([Layer::World, Layer::Enemy]),
            Owner::Enemy => LayerMask::from([Layer::World, Layer::Player]),
        };
        let hit = spatial.cast_ray(t.translation, dir, step.length(), true, &SpatialQueryFilter::from_mask(mask));
        let mut impact = None;
        if let Some(h) = hit {
            let point = t.translation + *dir * h.distance;
            impact = Some(point);
            if p.spec.explosion_radius <= 0.0 {
                // direct hit only
                match p.spec.owner {
                    Owner::Player => {
                        if let Ok(part) = bot_parts.get(h.entity) {
                            hits.write(BotDamage { bot: part.0, amount: p.spec.damage, slow: None });
                        }
                    }
                    Owner::Enemy => {
                        if h.entity != Entity::PLACEHOLDER && point.distance(player.0.translation) < 1.5 {
                            hurt_player(&mut stats, p.spec.damage, point - *dir * 10.0, &mut player_hits);
                        }
                    }
                }
            }
        } else if p.age >= p.spec.life {
            impact = Some(t.translation);
        }
        let Some(point) = impact else {
            t.translation += step;
            if p.spec.gravity {
                t.look_to(dir, Vec3::Y);
            }
            continue;
        };
        if p.spec.explosion_radius > 0.0 {
            let r = p.spec.explosion_radius;
            // linear falloff to 30% at the edge (GUESS: the original curve is not recovered)
            let falloff = |d: f32| if d > r { 0.0 } else { 1.0 - 0.7 * d / r };
            match p.spec.owner {
                Owner::Player => {
                    let feet = player.0.translation;
                    rocket_jump(&mut player.1, feet, point, p.spec.self_impulse);
                    for (bot, gt) in &bots {
                        let d = gt.translation().distance(point).min((gt.translation() + Vec3::Y).distance(point));
                        let k = falloff(d);
                        if k > 0.0 {
                            hits.write(BotDamage { bot, amount: p.spec.damage * k, slow: None });
                        }
                    }
                }
                Owner::Enemy => {
                    let k = falloff(player.0.translation.distance(point));
                    if k > 0.0 {
                        hurt_player(&mut stats, p.spec.damage * k, point, &mut player_hits);
                    }
                }
            }
            commands.spawn((Name::new("explosion"), Explosion { age: 0.0, radius: r }, Transform::from_translation(point)));
        }
        commands.entity(e).despawn();
    }
}

/// Player_move_c.m_600289E (the shooter's own blast) -> ImpactReceiver.AddImpact: within
/// the impulse radius the push is impulseForceSelf x dynamicSettings 0.7 x (1 - distance /
/// radius), away from the blast with the upward part always up (|y|), added to the
/// player's impact (max 100: maxImpactMagnitude of the Player prefab). player.rs plays it.
fn rocket_jump(p: &mut crate::player::Player, feet: Vec3, blast: Vec3, (radius, force): (f32, f32)) {
    if radius <= 0.0 || force <= 0.0 {
        return;
    }
    // Player_move_c's transform: the CharacterController's origin, 1 m above the feet
    let body = feet + Vec3::Y * 1.0;
    let d = body.distance(blast);
    if d >= radius {
        return;
    }
    let k = (1.0 - d / radius).max(0.0);
    let mut dir = (body - blast).normalize_or(Vec3::Y);
    dir.y = dir.y.abs();
    p.impact += dir * force * crate::player::IMPULSE_SETTING * k;
    p.impact = p.impact.clamp_length_max(crate::player::MAX_IMPACT);
    info!("rocket jump: {:.1} m from the blast, push {:.1}", d, p.impact.length());
}

fn hurt_player(stats: &mut PlayerStats, amount: f32, from: Vec3, hits: &mut MessageWriter<crate::bots::PlayerHit>) {
    if stats.protected <= 0.0 && stats.health > 0.0 {
        stats.health -= amount;
        hits.write(crate::bots::PlayerHit { from });
    }
}

/// Simple explosion: an orange ball that grows to the blast radius and fades out.
fn explosions(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut Explosion, &mut Transform, Option<&MeshMaterial3d<StandardMaterial>>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    const LIFE: f32 = 0.35;
    for (e, mut x, mut t, mat) in &mut q {
        let Some(mat) = mat else {
            commands.entity(e).insert((
                Mesh3d(meshes.add(Sphere::new(1.0))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::srgba(1.0, 0.6, 0.15, 0.9),
                    unlit: true,
                    alpha_mode: AlphaMode::Add,
                    ..default()
                })),
            ));
            continue;
        };
        x.age += time.delta_secs();
        let k = (x.age / LIFE).min(1.0);
        t.scale = Vec3::splat(x.radius * (0.3 + 0.7 * k));
        if let Some(mut m) = materials.get_mut(&mat.0) {
            m.base_color.set_alpha(0.9 * (1.0 - k));
        }
        if x.age >= LIFE {
            commands.entity(e).despawn();
        }
    }
}
