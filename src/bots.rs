//! Campaign-style enemies ("bots"): zombies that spawn at the map's EnemyCreationZone
//! markers, chase the player, hit them, take damage and die.
//!
//! Data: <data>/bots/ from ../pixel-gun-3d-re/scripts/export_bots.py
//!   bots.json          the original BaseBot settings (health, damagePerHit, ...)
//!   <Bot>/bot.gltf     model, skeleton and animations (Zombie_Walk, Zombie_Attack, ...)
//!
//! Behaviour is our own simple version of RilisoftBot.BaseBot / MeleeBot:
//!   idle -> (player within detectRadius) chase -> (within attackDistance) attack
//!   -> health <= 0: death animation, removed, respawn later.

use std::collections::HashMap;

use avian3d::prelude::*;
#[allow(unused_imports)]
use crate::scene::{FixedUpdate, PostStartup, PostUpdate, PreStartup, Startup, Update};
use bevy::{prelude::*, world_serialization::WorldInstanceReady};
use serde::Deserialize;

use crate::{
    DataDir,
    projectiles::{Look, Owner, SpawnProjectile},
    map::{Layer, MapMarkers},
    player::{GRAVITY, Player},
};

/// Bot types we spawn are every exported bot folder in <data>/bots/ (picked at random).
/// Zombies per wave. OBSERVED: the original Arena HUD shows 45 at the start of waves 1-3.
pub const WAVE_SIZE: u32 = 45;
/// How many bots are alive at once (GUESS: about 10 were on screen in the reference clips).
const MAX_ALIVE: usize = 10;
/// Pause between waves, seconds. OBSERVED (reference clip d_rec7, wave 2 -> 3): "Wave is
/// over! Pick up the prize!" for 5 s, then a 5-4-3-2-1 countdown, then "New wave!".
pub const WAVE_BREAK: f32 = 10.25;
/// Seconds of "Invincibility" at the start (and after a respawn). OBSERVED in clip d_rec2:
/// the HUD bar empties at 1/3 of its width per second and the panel vanishes with ~0.7 s of
/// bar left, so the protection itself lasts ~2.3 s (the bar is drawn as if it were 3 s).
pub const SPAWN_PROTECTION: f32 = 2.3;
pub const SPAWN_PROTECTION_BAR: f32 = 3.0;
/// Seconds between spawns while below MAX_ALIVE (GUESS: a 45-zombie wave took ~75 s).
const SPAWN_INTERVAL: f32 = 1.2;
/// Player health. GUESS: the original value is obfuscated; classic campaign had 9 hearts.
pub const PLAYER_MAX_HEALTH: f32 = 9.0;
/// PG3D_HUNT=1 (automated tests): zombies always know where you are instead of only
/// chasing within the original detectRadius.
fn hunt() -> bool {
    std::env::var("PG3D_HUNT").is_ok()
}
/// Seconds the dead body stays before it is removed.
const CORPSE_TIME: f32 = 2.0;

pub struct BotsPlugin;

impl crate::scene::ScenePlugin for BotsPlugin {
    fn build(&self, app: &mut crate::scene::SceneBuilder) {
        app.init_resource::<Wave>()
            .insert_resource(PlayerStats { health: PLAYER_MAX_HEALTH, protected: SPAWN_PROTECTION, score: 0, kills: 0, respawn_in: 0.0 })
            .add_systems(Startup, load_bot_defs)
            .add_systems(FixedUpdate, (bot_brain, bot_move).chain())
            .add_systems(
                Update,
                (spawner, apply_damage, bot_death, bot_animation, player_death, auto_aim),
            );
    }
}

/// The original BaseBot fields we use (names kept).
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
#[allow(non_snake_case)]
pub struct BotDef {
    pub health: f32,
    pub damagePerHit: f32,
    pub attackDistance: f32,
    /// movement speed while chasing (m/s, see notes)
    pub attackingSpeed: f32,
    /// movement speed while wandering
    pub notAttackingSpeed: f32,
    pub detectRadius: f32,
    pub scorePerKill: i32,
    /// NavMeshAgent.baseOffset: how far above the ground the bot floats (flying zombies)
    pub agentBaseOffset: f32,
    /// MeleeBot / ShootingBot / ...
    pub class: String,
    /// ShootingBot: shoots from this distance, bullet speed (m/s), explosive bullets
    pub rangeShootingDistance: f32,
    pub speedBullet: f32,
    pub isProjectileExplosion: i32,
    pub radiusExplosion: f32,
    /// colour / size of the bullet's particle in the original prefab
    pub bulletColor: Option<[f32; 4]>,
    pub bulletSize: f32,
    /// colliders of the prefab, relative to its root in Unity axes (export_bots.py hit_boxes):
    /// head sphere [x, y, z, radius] (hits there are headshots), body box [x, y, z, sx, sy, sz]
    pub head: Option<[f32; 4]>,
    pub body: Option<[f32; 6]>,
}

impl BotDef {
    fn shooter(&self) -> bool {
        self.class.contains("Shooting") && self.speedBullet > 0.0
    }
    fn attack_range(&self) -> f32 {
        if self.shooter() { self.rangeShootingDistance.max(3.0) } else { self.attackDistance.max(1.0) * 0.8 }
    }
}

#[derive(Deserialize, Default)]
struct BotInfo {
    #[serde(default)]
    animations: HashMap<String, f32>,
    #[serde(default)]
    clips: Vec<String>,
    /// attackSound, takeDamageSound (bite lands), damageSound (bot hurt), deathSound
    #[serde(default)]
    sounds: HashMap<String, String>,
}

struct BotType {
    name: String,
    def: BotDef,
    clips: Vec<String>,
    /// "Zombie_Attack", or "<Enemy>_shooting" for shooting bots
    attack_clip: String,
    attack_time: f32,
    sounds: HashMap<String, Handle<AudioSource>>,
}

#[derive(Resource, Default)]
struct BotTypes(Vec<BotType>);

#[derive(Resource, Default)]
pub struct Wave {
    pub number: u32,
    /// zombies of this wave not spawned yet
    pub to_spawn: u32,
    /// seconds until the next wave starts (during a break)
    pub break_left: f32,
    /// zombies of this wave still alive or not spawned yet (the HUD counter)
    pub left: u32,
    /// seconds since this wave started
    pub age: f32,
}

#[derive(Resource)]
pub struct PlayerStats {
    pub health: f32,
    /// seconds of spawn protection left
    pub protected: f32,
    pub score: i32,
    pub kills: i32,
    /// seconds until respawn while dead
    pub respawn_in: f32,
}

/// "The player was hit by something at `from`" (drives the HUD's red flash and arrows).
#[derive(Message)]
pub struct PlayerHit {
    pub from: Vec3,
}

/// "This bot took `amount` damage" (sent by weapons).
#[derive(Message)]
pub struct BotDamage {
    pub bot: Entity,
    pub amount: f32,
    /// slow the bot down: (speed factor, seconds), e.g. the Freeze Ray (0.7 for 1.5 s)
    pub slow: Option<(f32, f32)>,
}

/// Put on bot colliders; points at the bot's root entity.
#[derive(Component)]
pub struct BotPart(pub Entity);

/// The head collider of a bot: weapon hits on it do headshot damage.
#[derive(Component)]
pub struct BotHead;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BotState {
    Idle,
    Chase,
    Attack,
    Dead,
}

/// Marks living-or-dying bot roots (used by explosions to find targets).
#[derive(Component)]
pub struct BotMarker;

#[derive(Component)]
struct Bot {
    kind: usize,
    health: f32,
    state: BotState,
    velocity: Vec3,
    grounded: bool,
    /// time until the next hit lands (while attacking) / corpse is removed (dead)
    timer: f32,
    /// wander target while idle
    wander: Vec3,
    /// we have no navigation mesh yet: when blocked, walk sideways for a moment
    stuck: f32,
    detour: Vec3,
    detour_time: f32,
    /// slowed by a freeze weapon: speed factor and seconds left
    slow: (f32, f32),
    /// current height above the walking position (agentBaseOffset; drops to 0 on death)
    hover: f32,
    /// waypoints from the navigation mesh towards the current target
    path: Vec<Vec3>,
    /// seconds until the path is recalculated
    repath: f32,
}

#[derive(Component)]
struct BotAnims {
    graph: Handle<AnimationGraph>,
    nodes: HashMap<String, AnimationNodeIndex>,
    player: Option<Entity>,
    playing: Option<String>,
}


fn load_bot_defs(mut commands: Commands, data: Res<DataDir>, assets: Res<AssetServer>) {
    let dir = data.0.join("bots");
    let all: HashMap<String, BotDef> = std::fs::read_to_string(dir.join("bots.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let mut types = BotTypes::default();
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map(|r| r.filter_map(|e| e.ok()).filter(|e| e.path().join("bot.gltf").exists()))
        .into_iter()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    // PG3D_BOTS=Enemy3_go,Enemy6_go limits which bots spawn (testing)
    if let Ok(only) = std::env::var("PG3D_BOTS") {
        let only: Vec<&str> = only.split(',').map(str::trim).collect();
        names.retain(|n| only.contains(&n.as_str()));
    }
    for name in &names {
        let name = name.as_str();
        let Some(def) = all.get(name) else { continue };
        let Some(info) = std::fs::read_to_string(dir.join(name).join("info.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<BotInfo>(&s).ok())
        else {
            continue;
        };
        let attack_clip = info
            .clips
            .iter()
            .find(|c| c.contains("Attack") || c.to_lowercase().contains("shooting"))
            .cloned()
            .unwrap_or_else(|| "Zombie_Attack".into());
        let attack_time = info.animations.get(&attack_clip).copied().filter(|v| *v > 0.1).unwrap_or(1.5);
        let sounds = crate::sound::load_sounds(&assets, &format!("bots/{name}"), &info.sounds);
        types.0.push(BotType { name: name.to_string(), def: def.clone(), clips: info.clips, attack_clip, attack_time, sounds });
    }
    if types.0.is_empty() {
        warn!("no bots exported (run export_bots.py)");
    } else {
        info!("bot types: {}", types.0.iter().map(|t| format!("{} hp={}", t.name, t.def.health)).collect::<Vec<_>>().join(", "));
    }
    commands.insert_resource(types);
}

fn spawn_points(markers: &MapMarkers) -> Vec<Vec3> {
    let pts: Vec<Vec3> =
        markers.0.iter().filter(|m| m.name.starts_with("EnemyCreationZone")).map(|m| m.position).collect();
    if !pts.is_empty() {
        return pts;
    }
    // maps without enemy zones: use the multiplayer spawn points
    markers.0.iter().filter(|m| m.name.contains("CreateZone")).map(|m| m.position).collect()
}

/// Nothing solid between a bot's eyes and the player's eyes?
fn can_see(spatial: &SpatialQuery, bot: Vec3, player: Vec3) -> bool {
    let from = bot + Vec3::Y * 1.5;
    let to = player + Vec3::Y * 0.6;
    let Ok(dir) = Dir3::new(to - from) else { return true };
    spatial.cast_ray(from, dir, from.distance(to), true, &SpatialQueryFilter::from_mask(Layer::World)).is_none()
}

fn rand01(seed: f32) -> f32 {
    (seed.sin() * 43758.547).fract().abs()
}

#[allow(clippy::too_many_arguments)]
fn spawner(
    mut commands: Commands,
    time: Res<Time>,
    types: Res<BotTypes>,
    markers: Res<MapMarkers>,
    assets: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    bots: Query<&Bot>,
    player: Single<&Transform, With<Player>>,
    mut next: Local<f32>,
    mut wave: ResMut<Wave>,
    nav: Res<crate::navmesh::NavMesh>,
    spatial: SpatialQuery,
    collision: Res<crate::map::CollisionReady>,
) {
    if types.0.is_empty() || std::env::var("PG3D_NOBOTS").is_ok() || crate::team_match::team_mode() {
        return;
    }
    // nothing spawns before the map's floors are in the physics scene (they'd land under them)
    if !collision.ready() {
        return;
    }
    let dt = time.delta_secs();
    let alive = bots.iter().filter(|b| b.state != BotState::Dead).count();
    wave.left = alive as u32 + wave.to_spawn;
    wave.age += dt;
    // waves: every wave sends WAVE_SIZE zombies; when they are all dead there is a short
    // break, then the next, harder wave starts
    if wave.to_spawn == 0 && alive == 0 {
        if wave.break_left <= 0.0 {
            // the in-match HUD shows the original wave messages (ingame_ui.rs)
            wave.break_left = if wave.number == 0 { 2.0 } else { WAVE_BREAK };
        }
        wave.break_left -= dt;
        if wave.break_left <= 0.0 {
            wave.number += 1;
            // PG3D_WAVE_SIZE=n (automated tests): smaller waves to reach the breaks quickly
            wave.to_spawn = std::env::var("PG3D_WAVE_SIZE").ok().and_then(|v| v.parse().ok()).unwrap_or(WAVE_SIZE);
            wave.break_left = 0.0;
            wave.age = 0.0;
            info!("wave {} ({} zombies)", wave.number, wave.to_spawn);
        }
        return;
    }
    *next -= dt;
    if *next > 0.0 || alive >= MAX_ALIVE || wave.to_spawn == 0 {
        return;
    }
    *next = SPAWN_INTERVAL;
    wave.to_spawn -= 1;
    if std::env::var("PG3D_TRACE").is_ok() {
        info!("spawning bot ({} alive)", alive);
    }
    let pts = spawn_points(&markers);
    // pick a spawn point not right next to the player
    let t = time.elapsed_secs();
    let mut candidates: Vec<Vec3> = pts.iter().copied().filter(|p| p.distance(player.translation) > 8.0).collect();
    if candidates.is_empty() {
        candidates = pts;
    }
    if candidates.is_empty() {
        return;
    }
    let pos = candidates[(rand01(t * 1.7) * candidates.len() as f32) as usize % candidates.len()];
    // some zone markers sit inside the floor or off the walkable area (Hill's is 21 m under
    // the ground): move to the navmesh, then stand on the collider below
    let floor = SpatialQueryFilter::from_mask(Layer::World);
    let below = |p: Vec3| spatial.cast_ray(p + Vec3::Y * 1.5, Dir3::NEG_Y, 4.0, false, &floor).map(|h| p + Vec3::Y * (1.5 - h.distance));
    let pos = below(pos).or_else(|| nav.walkable_point(pos).map(|p| below(p).unwrap_or(p))).unwrap_or(pos);
    // early waves only use the weaker zombie types (sorted by health), later ones all
    let mut order: Vec<usize> = (0..types.0.len()).collect();
    order.sort_by(|a, b| types.0[*a].def.health.total_cmp(&types.0[*b].def.health));
    let allowed = (2 + wave.number as usize * 2).min(order.len()).max(1);
    let kind = order[(rand01(t * 3.1) * allowed as f32) as usize % allowed];
    let ty = &types.0[kind];

    let path = format!("bots/{}/bot.gltf", ty.name);
    let (graph, indices) = AnimationGraph::from_clips(
        (0..ty.clips.len()).map(|i| assets.load(GltfAssetLabel::Animation(i).from_asset(path.clone()))),
    );
    let anims = BotAnims {
        graph: graphs.add(graph),
        nodes: ty.clips.iter().cloned().zip(indices).collect(),
        player: None,
        playing: None,
    };
    let root = commands
        .spawn((
            Name::new(ty.name.clone()),
            Bot {
                kind,
                health: ty.def.health,
                state: BotState::Idle,
                velocity: Vec3::ZERO,
                grounded: false,
                timer: 0.0,
                wander: pos,
                stuck: 0.0,
                detour: Vec3::ZERO,
                detour_time: 0.0,
                slow: (1.0, 0.0),
                hover: ty.def.agentBaseOffset,
                path: Vec::new(),
                repath: 0.0,
            },
            anims,
            BotMarker,
            WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(path))),
            Transform::from_translation(pos + Vec3::Y * (0.5 + ty.def.agentBaseOffset)),
            RigidBody::Kinematic,
            CustomPositionIntegration,
            TransformInterpolation,
        ))
        .observe(hook_up_bot_animations)
        .id();
    // hit boxes of the prefab (Unity x is mirrored in glTF); without them a capsule the size
    // of the NavMeshAgent (0.5 m radius, 2 m tall, origin at the feet)
    let layers = CollisionLayers::new(Layer::Enemy, LayerMask::ALL);
    commands.entity(root).with_children(|c| {
        match ty.def.body {
            Some([x, y, z, sx, sy, sz]) => {
                c.spawn((BotPart(root), Collider::cuboid(sx, sy, sz), Transform::from_xyz(-x, y, z), layers))
            }
            None => c.spawn((BotPart(root), Collider::capsule(0.45, 1.0), Transform::from_xyz(0.0, 1.0, 0.0), layers)),
        };
        if let Some([x, y, z, r]) = ty.def.head {
            c.spawn((BotPart(root), BotHead, Collider::sphere(r), Transform::from_xyz(-x, y, z), layers));
        }
    });
}

fn hook_up_bot_animations(
    ready: On<WorldInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    mut roots: Query<&mut BotAnims>,
    players: Query<(), With<AnimationPlayer>>,
) {
    let Ok(mut anims) = roots.get_mut(ready.entity) else { return };
    for child in children.iter_descendants(ready.entity) {
        if players.contains(child) {
            commands.entity(child).insert(AnimationGraphHandle(anims.graph.clone()));
            anims.player = Some(child);
            break;
        }
    }
}

/// Decide what each bot wants to do.
fn bot_brain(
    mut commands: Commands,
    time: Res<Time>,
    types: Res<BotTypes>,
    mut stats: ResMut<PlayerStats>,
    player: Single<&Transform, (With<Player>, Without<Bot>)>,
    mut bots: Query<(&Transform, &mut Bot)>,
    spatial: SpatialQuery,
    mut shots: MessageWriter<SpawnProjectile>,
    mut hit: MessageWriter<PlayerHit>,
) {
    let dt = time.delta_secs();
    stats.protected = (stats.protected - dt).max(0.0);
    let trace = std::env::var("PG3D_TRACE").is_ok() && (time.elapsed_secs() % 2.0) < dt;
    if trace {
        info!("player health {:.1} score {}", stats.health, stats.score);
    }
    for (t, mut b) in &mut bots {
        if trace {
            info!("bot {} {:?} hp={:.1} pos={:.1} grounded={}", types.0[b.kind].name, b.state, b.health, t.translation, b.grounded);
        }
        if b.state == BotState::Dead {
            continue;
        }
        let def = &types.0[b.kind].def;
        let to_player = player.translation - t.translation;
        let flat = Vec2::new(to_player.x, to_player.z).length();
        let player_alive = stats.health > 0.0;
        b.state = if !player_alive {
            BotState::Idle
        } else if flat <= def.attack_range() && (!def.shooter() || can_see(&spatial, t.translation, player.translation)) {
            BotState::Attack
        } else if flat <= def.detectRadius || b.state == BotState::Chase || hunt() {
            BotState::Chase // once a zombie has seen you it keeps coming
        } else {
            BotState::Idle
        };
        if b.state == BotState::Attack {
            b.timer -= dt;
            if b.timer <= 0.0 {
                b.timer = types.0[b.kind].attack_time;
                let snd = &types.0[b.kind].sounds;
                if def.shooter() {
                    // fire a bullet from about chest height at the player's chest
                    let from = t.translation + Vec3::Y * 1.3;
                    let aim = player.translation + Vec3::Y * 0.3;
                    if let Ok(dir) = Dir3::new(aim - from) {
                        let c = def.bulletColor.unwrap_or([1.0, 0.3, 0.1, 1.0]);
                        shots.write(SpawnProjectile {
                            owner: Owner::Enemy,
                            from: from + *dir * 0.6,
                            dir,
                            speed: def.speedBullet,
                            life: 5.0,
                            gravity: false,
                            damage: def.damagePerHit,
                            explosion_radius: if def.isProjectileExplosion != 0 { def.radiusExplosion.max(1.0) } else { 0.0 },
                            self_impulse: (0.0, 0.0),
                            look: Look::Ball {
                                color: Color::srgba(c[0], c[1], c[2], 1.0),
                                // particle size 1 looks like a ~0.25 m glowing ball in game (GUESS)
                                radius: (def.bulletSize.max(0.3) * 0.18).clamp(0.08, 0.3),
                            },
                        });
                        crate::sound::play(&mut commands, snd.get("shootingSound"), Some(t.translation), 1.0);
                    }
                    continue;
                }
                crate::sound::play(&mut commands, snd.get("attackSound"), Some(t.translation), 1.0);
                if stats.protected <= 0.0 {
                    stats.health -= def.damagePerHit;
                    hit.write(PlayerHit { from: t.translation });
                    crate::sound::play(&mut commands, snd.get("takeDamageSound"), None, 0.8);
                }
            }
        } else {
            b.timer = types.0[b.kind].attack_time * 0.5; // first hit lands mid-swing
        }
        if b.state == BotState::Idle && t.translation.xz().distance(b.wander.xz()) < 1.0 {
            let s = time.elapsed_secs() + t.translation.x;
            b.wander = t.translation + Vec3::new(rand01(s) * 10.0 - 5.0, 0.0, rand01(s * 1.3) * 10.0 - 5.0);
        }
    }
}

/// Walk towards the target with the same move-and-slide as the player.
#[allow(clippy::type_complexity)]
fn bot_move(
    time: Res<Time>,
    types: Res<BotTypes>,
    player: Single<&Transform, (With<Player>, Without<Bot>)>,
    mut bots: Query<(Entity, &mut Transform, &mut Bot, &Children)>,
    colliders: Query<(&Collider, &Transform), Without<Bot>>,
    move_and_slide: MoveAndSlide,
    nav: Res<crate::navmesh::NavMesh>,
) {
    let dt = time.delta_secs();
    for (e, mut t, mut b, children) in &mut bots {
        // the body hit box moves the bot; it sits at its own offset above the feet (a cuboid
        // centred 0.35 m up for the small zombies)
        let Some((ce, col, off)) = children.iter().find_map(|c| colliders.get(c).ok().map(|(x, ct)| (c, x, ct.translation))) else {
            continue;
        };
        let def = &types.0[b.kind].def;
        // flying bots: physics and path finding work on the point below them
        let hover_before = b.hover;
        if b.state == BotState::Dead {
            b.hover = (b.hover - dt * 6.0).max(0.0); // fall when killed
        }
        t.translation.y -= hover_before;
        let (target, speed) = match b.state {
            BotState::Chase => (player.translation, def.attackingSpeed.max(0.5)),
            BotState::Idle => (b.wander, def.notAttackingSpeed.max(0.5) * 0.5),
            BotState::Attack => (player.translation, 0.0),
            BotState::Dead => (t.translation, 0.0),
        };
        b.slow.1 = (b.slow.1 - dt).max(0.0);
        let speed = if b.slow.1 > 0.0 { speed * b.slow.0 } else { speed };
        // follow the navigation mesh; recompute the route twice a second
        let mut aim = target;
        if speed > 0.0 && !nav.is_empty() {
            b.repath -= dt;
            if b.repath <= 0.0 {
                b.repath = 0.5;
                b.path = nav.find_path(t.translation, target).unwrap_or_default();
            }
            while let Some(&wp) = b.path.first() {
                let d = Vec2::new(wp.x - t.translation.x, wp.z - t.translation.z).length();
                if d < 0.4 && b.path.len() > 1 {
                    b.path.remove(0);
                } else {
                    break;
                }
            }
            if let Some(&wp) = b.path.first() {
                aim = wp;
            }
        }
        let mut dir = aim - t.translation;
        dir.y = 0.0;
        let mut dir = dir.normalize_or_zero();
        if b.detour_time > 0.0 {
            b.detour_time -= dt;
            dir = b.detour;
        }
        if dir != Vec3::ZERO {
            // the models face +Z
            let want = Quat::from_rotation_arc(Vec3::Z, dir);
            t.rotation = t.rotation.slerp(want, (dt * 8.0).min(1.0));
        }
        b.velocity.x = dir.x * speed;
        b.velocity.z = dir.z * speed;
        b.velocity.y = if b.grounded { -2.0 } else { (b.velocity.y - GRAVITY * dt).max(-17.0) };
        let mut grounded = false;
        let out = move_and_slide.move_and_slide(
            col,
            t.translation + t.rotation * off,
            t.rotation,
            b.velocity,
            time.delta(),
            &MoveAndSlideConfig::default(),
            &SpatialQueryFilter::from_mask(Layer::World).with_excluded_entities([e, ce]),
            |hit| {
                if hit.normal.y > 0.5 {
                    grounded = true;
                }
                MoveAndSlideHitResponse::Accept
            },
        );
        let before = t.translation;
        t.translation = out.position - t.rotation * off;
        b.grounded = grounded;
        let bob = if b.hover > 0.0 && b.state != BotState::Dead {
            (time.elapsed_secs() * 2.0 + e.index().index() as f32).sin() * 0.15
        } else {
            0.0
        };
        let foot = t.translation;
        t.translation.y += b.hover + bob;
        // blocked by a wall? (moved less than a third of what we wanted)
        let moved = Vec2::new(foot.x - before.x, foot.z - before.z).length();
        if speed > 0.0 && moved < speed * dt * 0.33 {
            b.stuck += dt;
            if b.stuck > 0.4 && b.detour_time <= 0.0 {
                // turn 70-110 degrees left or right and walk that way for a bit
                let s = time.elapsed_secs() * 7.3 + e.index().index() as f32;
                let angle = (if rand01(s) < 0.5 { 1.0 } else { -1.0 }) * (1.2 + rand01(s * 2.1) * 0.7);
                b.detour = Quat::from_rotation_y(angle) * dir;
                b.detour_time = 1.0 + rand01(s * 3.7);
                b.stuck = 0.0;
            }
        } else {
            b.stuck = 0.0;
        }
        if t.translation.y < -100.0 {
            b.health = 0.0; // fell out of the map
        }
    }
}

fn apply_damage(
    mut commands: Commands,
    mut hits: MessageReader<BotDamage>,
    types: Res<BotTypes>,
    mut stats: ResMut<PlayerStats>,
    mut bots: Query<(&mut Bot, &GlobalTransform)>,
    mut hurt_sound_cooldown: Local<f32>,
    time: Res<Time>,
    mut marks: MessageWriter<crate::feel::HitMark>,
) {
    *hurt_sound_cooldown -= time.delta_secs();
    for hit in hits.read() {
        let Ok((mut b, gt)) = bots.get_mut(hit.bot) else { continue };
        if b.state == BotState::Dead {
            continue;
        }
        b.health -= hit.amount;
        if let Some((k, secs)) = hit.slow {
            b.slow = (k, secs);
        }
        b.state = BotState::Chase; // shot bots always come for you
        marks.write(if b.health <= 0.0 { crate::feel::HitMark::Kill } else { crate::feel::HitMark::Hit });
        let snd = &types.0[b.kind].sounds;
        if b.health > 0.0 && *hurt_sound_cooldown <= 0.0 {
            // shotguns hit 15 times at once: one hurt sound is enough
            *hurt_sound_cooldown = 0.1;
            crate::sound::play(&mut commands, snd.get("damageSound"), Some(gt.translation()), 1.0);
        }
        if b.health <= 0.0 {
            crate::sound::play(&mut commands, snd.get("deathSound"), Some(gt.translation()), 1.0);
            b.state = BotState::Dead;
            b.timer = CORPSE_TIME;
            stats.score += types.0[b.kind].def.scorePerKill;
            stats.kills += 1;
            info!("killed {} (score {})", types.0[b.kind].name, stats.score);
        }
    }
}

fn bot_death(
    mut commands: Commands,
    time: Res<Time>,
    mut bots: Query<(Entity, &mut Bot, &Children)>,
    parts: Query<(), With<BotPart>>,
) {
    for (e, mut b, children) in &mut bots {
        if b.health <= 0.0 && b.state != BotState::Dead {
            b.state = BotState::Dead;
            b.timer = CORPSE_TIME;
        }
        if b.state != BotState::Dead {
            continue;
        }
        // stop being shootable as soon as it dies
        for c in children.iter().filter(|c| parts.contains(*c)) {
            commands.entity(c).despawn();
        }
        b.timer -= time.delta_secs();
        if b.timer <= 0.0 {
            commands.entity(e).despawn();
        }
    }
}

fn bot_animation(
    types: Res<BotTypes>,
    mut bots: Query<(&Bot, &mut BotAnims)>,
    mut players: Query<&mut AnimationPlayer>,
) {
    for (b, mut a) in &mut bots {
        let Some(mut ap) = a.player.and_then(|p| players.get_mut(p).ok()) else { continue };
        let moving = Vec2::new(b.velocity.x, b.velocity.z).length() > 0.1;
        let (want, repeat) = match b.state {
            BotState::Dead => ("Zombie_Dead", false),
            BotState::Attack => (types.0[b.kind].attack_clip.as_str(), true),
            _ if moving => ("Zombie_Walk", true),
            _ => ("Idle", true),
        };
        if a.playing.as_deref() == Some(want) {
            continue;
        }
        let Some(node) = a.nodes.get(want).copied() else { continue };
        ap.stop_all();
        let anim = ap.play(node);
        if repeat {
            anim.repeat();
        }
        a.playing = Some(want.to_string());
    }
}

/// At 0 health: wait 3 seconds, then respawn at the spawn point with full health.
fn player_death(
    time: Res<Time>,
    mut stats: ResMut<PlayerStats>,
    player: Single<(&mut Transform, &mut Player)>,
    over: Res<crate::results::MatchOver>,
) {
    if stats.health > 0.0 || over.0 {
        return;
    }
    if stats.respawn_in <= 0.0 {
        // Team Fight: the respawn window's auto timer ("To Battle!" can cut it short)
        stats.respawn_in = if crate::team_match::team_mode() { crate::tdm_bots::AUTO_RESPAWN } else { 3.0 };
        info!("player died (score {}, kills {})", stats.score, stats.kills);
    }
    stats.respawn_in -= time.delta_secs();
    if stats.respawn_in <= 0.0 {
        let (mut t, mut p) = player.into_inner();
        t.translation = p.spawn;
        p.velocity = Vec3::ZERO;
        stats.health = PLAYER_MAX_HEALTH;
        stats.respawn_in = 0.0;
        stats.protected = SPAWN_PROTECTION;
    }
}

/// PG3D_AUTOAIM=1 (automated tests): turn the camera to the nearest living bot
/// (PG3D_AUTOAIM=head: to its head collider).
fn auto_aim(
    bots: Query<(&GlobalTransform, &Bot, Option<&Children>)>,
    heads: Query<&GlobalTransform, With<BotHead>>,
    bodies: Query<&GlobalTransform, (With<BotPart>, Without<BotHead>)>,
    mut camera: Single<&mut Transform, With<crate::camera::PlayerCamera>>,
) {
    let Ok(mode) = std::env::var("PG3D_AUTOAIM") else { return };
    let eye = camera.translation;
    let target = bots
        .iter()
        .filter(|(_, b, _)| b.state != BotState::Dead)
        .map(|(t, _, kids)| {
            let head = kids.and_then(|k| k.iter().find_map(|c| heads.get(c).ok()));
            let body = kids.and_then(|k| k.iter().find_map(|c| bodies.get(c).ok()));
            match (head, body) {
                (Some(h), _) if mode == "head" => h.translation(),
                (_, Some(b)) => b.translation(),
                _ => t.translation() + Vec3::Y * 1.2,
            }
        })
        .min_by(|a, b| a.distance(eye).total_cmp(&b.distance(eye)));
    if let Some(p) = target {
        camera.look_at(p, Vec3::Y);
    }
}
