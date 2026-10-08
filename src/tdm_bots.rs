//! Player bots for Team Fight: your team (blue) and the rival team (red) filled with bots,
//! like the original's PlayerBot system (PlayerBotsManager / AIBotController /
//! AIBotTargetDetector / PlayerBotMovement). The tuning comes from the game's PlayerBot
//! prefab (export_player_bot.py -> bots/player_bot.json):
//!
//! * target detection: every `timeBetweenLOSChecks` s; an enemy is noticed within
//!   `maxDistToNoticeTarget` m inside the `effectiveFov` cone (or within `aroundDistToTarget`
//!   m in any direction) with nothing in between; it is forgotten past
//!   `distanceToLoseAwarness` m
//! * aiming: turns at `rotationSpeedMin..Max` (per bot), fires when lined up
//! * fighting: dodges sideways for `dodgingTime` s at `dodgingSpeed`
//! * moving: the navmesh; jumps up ledges higher than `jumpNeededDeltaHeight`, otherwise
//!   `jumpWhenPossibleChance` % of the time, with `jumpSpeed`
//!
//! Bots look like players (the character model holding a real weapon from the game) and
//! use that weapon's damage and fire rate. Kills and deaths go into `TdmScores`, which the
//! Team Fight score box and table show.

use avian3d::prelude::*;
use bevy::{
    prelude::*,
    window::{CursorGrabMode, CursorOptions},
};
use crate::ngui::NguiScreen;
use serde::Deserialize;

#[allow(unused_imports)]
use crate::scene::{FixedUpdate, PostStartup, PostUpdate, PreStartup, Startup, Update};
use crate::{
    DataDir,
    bots::{BotDamage, BotPart, PLAYER_MAX_HEALTH, PlayerHit, PlayerStats, SPAWN_PROTECTION},
    map::{Layer, MapMarkers},
    player::Player,
};

pub struct TdmBotsPlugin;

impl crate::scene::ScenePlugin for TdmBotsPlugin {
    fn build(&self, app: &mut crate::scene::SceneBuilder) {
        if !crate::team_match::team_mode() || std::env::var("PG3D_NOBOTS").is_ok() {
            app.init_resource::<TdmScores>();
            return;
        }
        app.init_resource::<TdmScores>()
            .add_systems(Startup, (spawn_bots.after(crate::player::spawn_player), spawn_respawn_screen))
            .add_systems(FixedUpdate, (brain, movement).chain())
            .add_systems(Update, (hook_up, animate_bodies, take_damage, player_killed, respawn, name_tags, death_view))
            .add_systems(PostUpdate, bot_view);
    }
}

/// Bots per team (the original's Team Fight is 5 against 5).
const TEAM_SIZE: usize = 5;
/// Seconds dead before respawning (as the player).
const RESPAWN: f32 = 3.0;
const GRAVITY: f32 = 9.81;
/// Bots run at the player's speed.
const RUN_SPEED: f32 = crate::player::WALK_SPEED;
/// Height of the bots' first-person weapon rig's eye (at the chest; GUESS, looks right).
const BOT_EYE: f32 = 1.3;
/// Turn rate while roaming, rad/s (aiming uses the bot's own rotationSpeed).
const ROAM_TURN: f32 = 4.0;
/// Horizontal acceleration, m/s² (full speed in about a quarter second).
const ACCEL: f32 = 22.0;
/// Strafing tops out at this share of running speed.
const STRAFE_MAX: f32 = 0.65;
/// Weapons bots carry (exported primaries).
const BOT_WEAPONS: [&str; 4] = ["Weapon3", "Weapon2", "Weapon9", "Weapon1"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Team {
    Blue,
    Red,
}

/// One line of the Team Fight table.
#[derive(Clone, Debug)]
pub struct ScoreRow {
    pub name: String,
    pub team: Team,
    pub kills: i32,
    pub deaths: i32,
    pub you: bool,
}

/// Team kills and every player's line (rows[0] is you).
#[derive(Resource, Default)]
pub struct TdmScores {
    pub blue: i32,
    pub red: i32,
    pub rows: Vec<ScoreRow>,
    /// the bot that last hurt the player (gets the kill)
    last_hit_player: Option<Entity>,
    /// per bot row: (your kills of it, its kills of you) — the respawn window's "You VS Rival"
    vs: std::collections::HashMap<usize, (i32, i32)>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default, rename_all = "camelCase")]
struct Controller {
    rotation_speed_min: f32,
    rotation_speed_max: f32,
    dodging_time: f32,
    dodging_speed: f32,
    max_move_distance: f32,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default, rename_all = "camelCase")]
struct Detector {
    time_between_l_o_s_checks: f32,
    effective_fov: f32,
    max_dist_to_notice_target: f32,
    around_dist_to_target: f32,
    distance_to_lose_awarness: f32,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default, rename_all = "camelCase")]
struct Movement {
    jump_needed_delta_height: f32,
    jump_when_possible_chance: f32,
    jump_speed: f32,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct Tuning {
    #[serde(rename = "AIBotController")]
    controller: Controller,
    #[serde(rename = "AIBotTargetDetector")]
    detector: Detector,
    #[serde(rename = "PlayerBotMovement")]
    movement: Movement,
}

impl Tuning {
    fn load(data: &std::path::Path) -> Self {
        let mut t: Tuning = std::fs::read_to_string(data.join("bots/player_bot.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        // the prefab's values (in case the export is missing)
        let d = |v: &mut f32, x: f32| {
            if *v <= 0.0 {
                *v = x
            }
        };
        d(&mut t.controller.rotation_speed_min, 3.0);
        d(&mut t.controller.rotation_speed_max, 9.0);
        d(&mut t.controller.dodging_time, 0.7);
        d(&mut t.controller.dodging_speed, 5.0);
        d(&mut t.controller.max_move_distance, 10.0);
        d(&mut t.detector.time_between_l_o_s_checks, 0.5);
        d(&mut t.detector.effective_fov, 70.0);
        d(&mut t.detector.max_dist_to_notice_target, 20.0);
        d(&mut t.detector.around_dist_to_target, 4.0);
        d(&mut t.detector.distance_to_lose_awarness, 20.0);
        d(&mut t.movement.jump_needed_delta_height, 0.5);
        d(&mut t.movement.jump_when_possible_chance, 30.0);
        d(&mut t.movement.jump_speed, 5.0);
        t
    }
}

#[derive(Resource)]
struct BotSetup {
    tuning: Tuning,
    blue_spawns: Vec<Vec3>,
    red_spawns: Vec<Vec3>,
    roam: Vec<Vec3>,
    /// the player body's clips (export_player_anims.py: RoyalePlayerAnimController)
    anims: std::collections::HashMap<String, Clip>,
}

/// One body animation (same layout as the weapons' profile.json).
#[derive(Deserialize, Clone, Default)]
struct Clip {
    length: f32,
    fps: f32,
    tracks: Vec<ClipTrack>,
}

#[derive(Deserialize, Clone, Default)]
struct ClipTrack {
    path: String,
    #[serde(default)]
    rotation: Vec<[f32; 4]>,
    #[serde(default)]
    translation: Vec<[f32; 3]>,
}

impl Clip {
    /// Bone rotation / position of track `k` at `t` seconds (looping).
    fn sample(&self, k: usize, t: f32) -> (Option<Quat>, Option<Vec3>) {
        let tr = &self.tracks[k];
        let t = if self.length > 0.0 { t.rem_euclid(self.length) } else { 0.0 };
        let f = t * self.fps.max(1.0);
        let (i, a) = (f.floor() as usize, f.fract());
        let pick = |n: usize| (i.min(n.saturating_sub(1)), (i + 1).min(n.saturating_sub(1)));
        let rot = (!tr.rotation.is_empty()).then(|| {
            let (i0, i1) = pick(tr.rotation.len());
            Quat::from_array(tr.rotation[i0]).slerp(Quat::from_array(tr.rotation[i1]), a)
        });
        let pos = (!tr.translation.is_empty()).then(|| {
            let (i0, i1) = pick(tr.translation.len());
            Vec3::from(tr.translation[i0]).lerp(Vec3::from(tr.translation[i1]), a)
        });
        (rot, pos)
    }
}

/// A bot's body bones by path (below Royale_Player_body) and the animation state.
#[derive(Component, Default)]
struct BodyAnim {
    bones: std::collections::HashMap<String, Entity>,
    /// bones' rest pose, for tracks a clip doesn't key
    rest: std::collections::HashMap<Entity, (Quat, Vec3)>,
    clip: String,
    prev: String,
    fade: f32,
    time: f32,
    prev_time: f32,
}

/// What a bot is aiming at.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Target {
    Player,
    Bot(Entity),
}

#[derive(Component)]
pub struct PlayerBot {
    pub team: Team,
    row: usize,
    health: f32,
    velocity: Vec3,
    grounded: bool,
    yaw: f32,
    turn_speed: f32,
    target: Option<Target>,
    los_timer: f32,
    goal: Vec3,
    path: Vec<Vec3>,
    repath: f32,
    dodge_dir: f32,
    next_shot: f32,
    respawn_in: f32,
    rng: u32,
    /// seconds in the air (short hops don't switch to the jump pose)
    air: f32,
    /// seconds left of the "hit" flinch (TakeDamage)
    hurt: f32,
    /// seconds left of a strafe burst (0: not strafing)
    strafe: f32,
    /// next moment to consider strafing while shooting
    strafe_check: f32,
    /// seconds before the first shot at a newly seen target (reaction time)
    react: f32,
    /// 0.6 .. 1.0: how well this bot aims and how fast it reacts
    skill: f32,
    /// distance it likes to fight at
    fight_dist: f32,
    /// seconds without getting anywhere, and a detour being walked (direction, seconds)
    stuck: f32,
    detour: (Vec3, f32),
    // weapon
    damage: f32,
    interval: f32,
    shoot_sound: Option<Handle<AudioSource>>,
    weapon: Option<Entity>,
    weapon_name: &'static str,
}

impl PlayerBot {
    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng % 10000) as f32 / 10000.0
    }
    fn alive(&self) -> bool {
        self.health > 0.0
    }
}

/// Name label above a bot (blue: your team, red: enemies).
#[derive(Component)]
struct NameTag(Entity);

#[derive(Component)]
struct Hooked;

#[derive(Deserialize, Default)]
struct WeaponInfo {
    #[serde(default)]
    animations: std::collections::HashMap<String, f32>,
    #[serde(default)]
    sounds: std::collections::HashMap<String, String>,
}

#[derive(Deserialize, Default)]
#[allow(non_snake_case)]
struct WeaponStats {
    #[serde(default)]
    damageByTier: Vec<f32>,
    #[serde(default)]
    isShotGun: i32,
    #[serde(default)]
    countShots: i32,
}

/// Bot names (the game's own bots use player-like nicks; these are ours).
const NAMES: [&str; 10] = ["Blocky", "Pixelator", "Crafty", "Boomer", "Sniperoo", "Cubix", "Zapper", "Rex", "Nova", "Tank"];

fn spawn_points(markers: &MapMarkers) -> (Vec<Vec3>, Vec<Vec3>, Vec<Vec3>) {
    let pts = |f: &dyn Fn(&str) -> bool| markers.0.iter().filter(|m| f(&m.name)).map(|m| m.position).collect::<Vec<_>>();
    let mut blue = pts(&|n| n.starts_with("MultyPlayerCreateZoneCommand1"));
    let mut red = pts(&|n| n.starts_with("MultyPlayerCreateZoneCommand2"));
    let all = pts(&|n| n.starts_with("MultyPlayerCreateZone") && !n.contains("Flag") && !n.contains("Duel") && !n.contains("COOP"));
    if blue.is_empty() || red.is_empty() {
        // no team zones: split the free-for-all ones by side of the map
        let c = all.iter().copied().sum::<Vec3>() / all.len().max(1) as f32;
        blue = all.iter().copied().filter(|p| p.x >= c.x).collect();
        red = all.iter().copied().filter(|p| p.x < c.x).collect();
        if blue.is_empty() || red.is_empty() {
            blue = all.clone();
            red = all.clone();
        }
    }
    (blue, red, all)
}

#[allow(clippy::too_many_arguments)]
fn spawn_bots(
    mut commands: Commands,
    data: Res<DataDir>,
    assets: Res<AssetServer>,
    markers: Res<MapMarkers>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut scores: ResMut<TdmScores>,
    mut player: Query<(&mut Transform, &mut Player)>,
    nav: Res<crate::navmesh::NavMesh>,
    spatial: SpatialQuery,
) {
    let (blue_spawns, red_spawns, roam) = spawn_points(&markers);
    let _ = &mut graphs;
    let anims: std::collections::HashMap<String, Clip> = std::fs::read_to_string(data.0.join("characters/player_body/anims.json"))
        .ok()
        .and_then(|t| serde_json::from_value::<serde_json::Map<String, serde_json::Value>>(serde_json::from_str(&t).ok()?).ok())
        .map(|m| m.into_iter().filter_map(|(k, v)| serde_json::from_value(v).ok().map(|c| (k, c))).collect())
        .unwrap_or_default();
    let setup = BotSetup {
        tuning: Tuning::load(&data.0),
        blue_spawns: blue_spawns.clone(),
        red_spawns: red_spawns.clone(),
        roam: if roam.is_empty() { blue_spawns.iter().chain(&red_spawns).copied().collect() } else { roam },
        anims,
    };
    // you start with your team
    if let (Ok((mut t, mut p)), Some(&s)) = (player.single_mut(), blue_spawns.first()) {
        let s = nav.walkable_point(s).unwrap_or(s) + Vec3::Y;
        t.translation = s;
        p.spawn = s;
    }
    scores.rows = vec![ScoreRow { name: "Player".into(), team: Team::Blue, kills: 0, deaths: 0, you: true }];
    let stats: std::collections::HashMap<String, WeaponStats> = std::fs::read_to_string(data.0.join("weapons/weapons.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let mut k = 0usize;
    for (team, n) in [(Team::Blue, TEAM_SIZE - 1), (Team::Red, TEAM_SIZE)] {
        for _ in 0..n {
            let row = scores.rows.len();
            let name = NAMES[k % NAMES.len()].to_string();
            scores.rows.push(ScoreRow { name: name.clone(), team, kills: 0, deaths: 0, you: false });
            let wname = BOT_WEAPONS[k % BOT_WEAPONS.len()];
            let info: WeaponInfo = std::fs::read_to_string(data.0.join(format!("weapons/{wname}/info.json")))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default();
            let ws = stats.get(wname);
            let pellets = ws.filter(|w| w.isShotGun != 0).map_or(1, |w| w.countShots.max(1)) as f32;
            let damage = ws.and_then(|w| w.damageByTier.first().copied()).unwrap_or(1.0) * pellets;
            let interval = info.animations.get("Shoot").copied().filter(|v| *v > 0.05).unwrap_or(0.3).max(0.08);
            let spawns = if team == Team::Blue { &setup.blue_spawns } else { &setup.red_spawns };
            let at = spawns.get(k % spawns.len().max(1)).copied().unwrap_or(Vec3::ZERO);
            let at = stand_at(&spatial, at);
            let tc = &setup.tuning.controller;
            let mut bot = PlayerBot {
                team,
                row,
                health: PLAYER_MAX_HEALTH,
                velocity: Vec3::ZERO,
                grounded: false,
                yaw: 0.0,
                turn_speed: 0.0,
                target: None,
                los_timer: 0.0,
                goal: at,
                path: vec![],
                repath: 0.0,
                dodge_dir: 1.0,
                next_shot: 1.0,
                respawn_in: 0.0,
                air: 0.0,
                hurt: 0.0,
                strafe: 0.0,
                strafe_check: 1.0,
                react: 0.0,
                skill: 1.0,
                fight_dist: 10.0,
                stuck: 0.0,
                detour: (Vec3::ZERO, 0.0),
                rng: 0x9E37_79B9 ^ (k as u32 * 7919 + 1),
                damage,
                interval,
                shoot_sound: info.sounds.get("shoot").map(|r| assets.load(format!("weapons/{wname}/{r}"))),
                weapon: None,
                weapon_name: wname,
            };
            bot.turn_speed = tc.rotation_speed_min + (tc.rotation_speed_max - tc.rotation_speed_min) * bot.rand();
            // GUESS (for fun): mixed skill, each bot with its own favourite range
            bot.skill = 0.6 + 0.4 * bot.rand();
            bot.fight_dist = 7.0 + 9.0 * bot.rand();
            let layers = if team == Team::Red {
                // enemies: hit by your bullets
                CollisionLayers::new(Layer::Enemy, LayerMask::ALL)
            } else {
                CollisionLayers::new(Layer::Trigger, LayerMask::NONE)
            };
            let root = commands
                .spawn((
                    Name::new(format!("bot {name}")),
                    Transform::from_translation(at + Vec3::Y * 0.1),
                    Visibility::Inherited,
                    RigidBody::Kinematic,
                    CustomPositionIntegration,
                    TransformInterpolation,
                    WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset("characters/player_body/body.gltf"))),
                    BodyAnim::default(),
                ))
                .id();
            commands.entity(root).with_children(|c| {
                c.spawn((BotPart(root), Collider::capsule(0.4, 1.0), Transform::from_xyz(0.0, 0.9, 0.0), layers));
            });
            let w = commands
                .spawn((
                    Name::new(format!("bot weapon {wname}")),
                    crate::particles::AssetDir(format!("weapons/{wname}")),
                    WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(format!("weapons/{wname}/weapon.gltf")))),
                    Transform::IDENTITY,
                    Visibility::Inherited,
                ))
                .id();
            bot.weapon = Some(w);
            commands.entity(root).insert(bot);
            commands.spawn((
                NameTag(root),
                Text::new(name),
                TextFont { font_size: FontSize::Px(14.0), ..default() },
                TextColor(if team == Team::Blue { Color::srgb(0.3, 0.6, 1.0) } else { Color::srgb(1.0, 0.25, 0.2) }),
                TextShadow { offset: Vec2::splat(1.0), color: Color::BLACK },
                Node { position_type: PositionType::Absolute, ..default() },
                Visibility::Hidden,
            ));
            k += 1;
        }
    }
    commands.insert_resource(setup);
}

/// Once a bot's body has loaded: its bones by path, the weapon (with its own arms) at the
/// body's WeaponPoint (the original's weaponPointNotMine).
#[allow(clippy::too_many_arguments)]
fn hook_up(
    mut commands: Commands,
    mut bots: Query<(Entity, &PlayerBot, &mut BodyAnim), Without<Hooked>>,
    children: Query<&Children>,
    names: Query<&Name>,
    transforms: Query<&Transform>,
    weapons: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
) {
    for (e, b, mut anim) in &mut bots {
        let all: Vec<Entity> = children.iter_descendants(e).collect();
        let Some(root) = all.iter().copied().find(|d| names.get(*d).is_ok_and(|n| n.as_str() == "Royale_Player_body")) else { continue };
        let _ = &transforms;
        // the weapon must have loaded too (its arm mesh is hidden below)
        let Some(w) = b.weapon else { continue };
        let wall: Vec<Entity> = weapons.iter_descendants(w).collect();
        let Some(arms) = wall.iter().copied().find(|d| names.get(*d).is_ok_and(|n| n.as_str() == "Arms_Mesh")) else { continue };
        let mut bones = std::collections::HashMap::new();
        let mut stack = vec![(root, String::new())];
        while let Some((n, path)) = stack.pop() {
            for c in children.get(n).into_iter().flatten() {
                let Ok(name) = names.get(*c) else { continue };
                let p = if path.is_empty() { name.to_string() } else { format!("{path}/{name}") };
                bones.insert(p.clone(), *c);
                if let Ok(t) = transforms.get(*c) {
                    anim.rest.insert(*c, (t.rotation, t.translation));
                }
                stack.push((*c, p));
            }
        }
        anim.bones = bones;
        anim.clip = "Idle_v02".into();
        // as in the Armory (CharacterInterfaceV2): the weapon brings its own arms; the
        // body's arms mesh is hidden
        let _ = (&meshes, arms);
        for d in &all {
            if names.get(*d).is_ok_and(|n| n.as_str() == "rp_arms_mesh") {
                commands.entity(*d).insert(Visibility::Hidden);
            }
        }
        // the weapon prefab is a first-person rig whose origin is gunPosition (y -1.7) below
        // the eye; put that "eye" at the chest so its arms come out at the shoulders
        commands.entity(w).insert((ChildOf(e), Transform::from_translation(Vec3::new(0.0, BOT_EYE - 1.7, 0.12))));
        commands.entity(e).insert(Hooked);
    }
}

/// Pick the body clip from how the bot moves relative to where it faces (the original's
/// locomotion blend: run forward / 45 / sideways / backward, standing idle, in the air) and
/// play it, cross-fading 0.15 s; "TakeDamage" plays over it for a moment when hit.
fn animate_bodies(
    time: Res<Time>,
    setup: Option<Res<BotSetup>>,
    mut bots: Query<(&PlayerBot, &mut BodyAnim)>,
    mut tf: Query<&mut Transform, Without<PlayerBot>>,
) {
    let Some(setup) = setup else { return };
    let dt = time.delta_secs();
    for (b, mut a) in &mut bots {
        if a.bones.is_empty() || !b.alive() || std::env::var("PG3D_NOBODYANIM").is_ok() {
            continue;
        }
        let v = Vec3::new(b.velocity.x, 0.0, b.velocity.z);
        let speed = v.length();
        let fwd = Quat::from_rotation_y(b.yaw) * Vec3::Z;
        // the model's right is -X (Unity's +X, mirrored by the export)
        let right = Quat::from_rotation_y(b.yaw) * Vec3::NEG_X;
        let want = if !b.grounded && b.air > 0.25 {
            "Jump_idle"
        } else if speed < 0.6 {
            "Idle_v02"
        } else {
            let ang = v.dot(right).atan2(v.dot(fwd)).to_degrees();
            match ang {
                a if a.abs() < 22.5 => "run_forward",
                a if a.abs() > 157.5 => "run_backward",
                a if (22.5..67.5).contains(&a) => "run_forward_right_45",
                a if (67.5..112.5).contains(&a) => "run_right",
                a if a >= 112.5 => "walk_backward_right_v2",
                a if (-67.5..-22.5).contains(&a) => "run_forward_left_45",
                a if (-112.5..-67.5).contains(&a) => "run_left",
                _ => "walk_backward_left_v2",
            }
        };
        if want != a.clip && setup.anims.contains_key(want) {
            a.prev = std::mem::replace(&mut a.clip, want.to_string());
            a.prev_time = a.time;
            a.time = 0.0;
            a.fade = 0.0;
        }
        // run clips are made for full running speed
        let rate = if a.clip.starts_with("run") || a.clip.starts_with("walk") { (speed / RUN_SPEED).clamp(0.5, 1.4) } else { 1.0 };
        a.time += dt * rate;
        a.prev_time += dt * rate;
        a.fade = (a.fade + dt / 0.15).min(1.0);
        let mut pose: std::collections::HashMap<Entity, (Quat, Vec3)> = std::collections::HashMap::new();
        let apply = |name: &str, t: f32, w: f32, pose: &mut std::collections::HashMap<Entity, (Quat, Vec3)>| {
            let Some(c) = setup.anims.get(name) else { return };
            for (k, tr) in c.tracks.iter().enumerate() {
                let Some(&e) = a.bones.get(&tr.path) else { continue };
                let rest = a.rest.get(&e).copied().unwrap_or((Quat::IDENTITY, Vec3::ZERO));
                let (r, p) = c.sample(k, t);
                let cur = pose.entry(e).or_insert(rest);
                if let Some(r) = r {
                    cur.0 = cur.0.slerp(r, w);
                }
                if let Some(p) = p {
                    cur.1 = cur.1.lerp(p, w);
                }
            }
        };
        if a.fade < 1.0 && !a.prev.is_empty() {
            let prev = a.prev.clone();
            apply(&prev, a.prev_time, 1.0, &mut pose);
        }
        let clip = a.clip.clone();
        apply(&clip, a.time, if a.prev.is_empty() { 1.0 } else { a.fade }, &mut pose);
        if b.hurt > 0.0 {
            apply("TakeDamage", 0.27 - b.hurt, 0.8, &mut pose);
        }
        for (e, (r, p)) in pose {
            if let Ok(mut t) = tf.get_mut(e) {
                t.rotation = r;
                t.translation = p;
            }
        }
    }
}

/// Where to stand for a spawn marker: on the floor under it (markers float a little;
/// the navmesh point can be inside a block).
fn stand_at(spatial: &SpatialQuery, p: Vec3) -> Vec3 {
    let world = SpatialQueryFilter::from_mask(Layer::World);
    spatial.cast_ray(p + Vec3::Y * 1.0, Dir3::NEG_Y, 6.0, true, &world).map_or(p, |h| p + Vec3::Y * (1.0 - h.distance)) + Vec3::Y * 0.05
}

fn eye(p: Vec3) -> Vec3 {
    p + Vec3::Y * 1.5
}

fn line_clear(spatial: &SpatialQuery, from: Vec3, to: Vec3) -> bool {
    let Ok(dir) = Dir3::new(to - from) else { return true };
    spatial.cast_ray(from, dir, from.distance(to), true, &SpatialQueryFilter::from_mask(Layer::World)).is_none()
}

/// Targets, aiming and shooting.
#[allow(clippy::too_many_arguments)]
fn brain(
    mut commands: Commands,
    time: Res<Time>,
    setup: Option<Res<BotSetup>>,
    mut bots: Query<(Entity, &Transform, &mut PlayerBot)>,
    player: Query<&Transform, (With<Player>, Without<PlayerBot>)>,
    mut stats: ResMut<PlayerStats>,
    mut scores: ResMut<TdmScores>,
    mut hit: MessageWriter<PlayerHit>,
    mut bursts: MessageWriter<crate::particles::Burst>,
    spatial: SpatialQuery,
    nav: Res<crate::navmesh::NavMesh>,
) {
    let Some(setup) = setup else { return };
    let dt = time.delta_secs();
    let tu = &setup.tuning;
    let player_pos = player.single().ok().map(|t| t.translation);
    let player_alive = stats.health > 0.0;
    // positions of everyone this frame
    let everyone: Vec<(Entity, Vec3, Team, bool)> = bots.iter().map(|(e, t, b)| (e, t.translation, b.team, b.alive())).collect();
    let mut damage_bots: Vec<(Entity, Entity, f32)> = vec![];
    for (e, t, mut b) in &mut bots {
        if !b.alive() {
            continue;
        }
        let pos = t.translation;
        b.hurt = (b.hurt - dt).max(0.0);
        b.strafe = (b.strafe - dt).max(0.0);
        let forward = Quat::from_rotation_y(b.yaw) * Vec3::Z;
        // where a target is (None: gone / dead)
        let where_is = |tg: Target| match tg {
            Target::Player => player_pos.filter(|_| player_alive),
            Target::Bot(x) => everyone.iter().find(|o| o.0 == x && o.3).map(|o| o.1),
        };
        // AIBotTargetDetector: look around every timeBetweenLOSChecks
        b.los_timer -= dt;
        if b.los_timer <= 0.0 {
            b.los_timer = tu.detector.time_between_l_o_s_checks;
            // keep the current target while in reach and in sight
            let keep = b.target.and_then(|tg| where_is(tg).map(|p| (tg, p))).filter(|(_, p)| {
                p.distance(pos) < tu.detector.distance_to_lose_awarness && line_clear(&spatial, eye(pos), eye(*p))
            });
            let had = b.target;
            b.target = keep.map(|(tg, _)| tg).or_else(|| {
                let mut cands: Vec<(Target, Vec3)> = everyone
                    .iter()
                    .filter(|o| o.3 && o.2 != b.team)
                    .map(|o| (Target::Bot(o.0), o.1))
                    .collect();
                if b.team == Team::Red && player_alive && let Some(p) = player_pos {
                    cands.push((Target::Player, p));
                }
                cands
                    .into_iter()
                    .filter(|(_, p)| {
                        let to = *p - pos;
                        let d = to.length();
                        let ang = forward.angle_between(Vec3::new(to.x, 0.0, to.z).normalize_or_zero()).to_degrees();
                        (d < tu.detector.around_dist_to_target
                            || (d < tu.detector.max_dist_to_notice_target && ang < tu.detector.effective_fov))
                            && line_clear(&spatial, eye(pos), eye(*p))
                    })
                    .min_by(|a, b2| a.1.distance(pos).total_cmp(&b2.1.distance(pos)))
                    .map(|(tg, _)| tg)
            });
            // a new target: a moment to react before the first shot (GUESS 0.25-0.65 s)
            if b.target.is_some() && b.target != had {
                b.react = 0.25 + 0.4 * (1.0 - b.skill) / 0.4 * b.rand();
            }
        }
        let target_pos = b.target.and_then(where_is);
        if target_pos.is_none() {
            b.target = None;
        }
        match target_pos {
            Some(tp) => {
                // turn towards the target, fire when lined up
                let to = tp - pos;
                let want = to.x.atan2(to.z);
                let diff = (want - b.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                b.yaw += diff.clamp(-b.turn_speed * dt, b.turn_speed * dt);
                b.next_shot -= dt;
                b.react -= dt;
                if diff.abs() < 0.15 && b.next_shot <= 0.0 && b.react <= 0.0 {
                    b.next_shot = b.interval;
                    if let Some(w) = b.weapon {
                        bursts.write(crate::particles::Burst(w));
                    }
                    // the weapon's own Shoot sound, from where the bot stands
                    crate::sound::play_scaled(&mut commands, b.shoot_sound.as_ref(), Some(pos), 0.7, 0.7);
                    // hit chance (GUESS, tuned for fun): falls off with distance, worse
                    // while strafing, scaled by the bot's skill; the player is a bit harder
                    // to hit than bots so fights against you last
                    let mut chance = (0.75 - to.length() * 0.018).clamp(0.2, 0.75) * b.skill;
                    if b.strafe > 0.0 {
                        chance *= 0.75;
                    }
                    if b.target == Some(Target::Player) {
                        chance *= 0.7;
                    }
                    if b.rand() < chance && line_clear(&spatial, eye(pos), eye(tp)) {
                        match b.target {
                            Some(Target::Player) => {
                                if stats.protected <= 0.0 {
                                    stats.health -= b.damage;
                                    scores.last_hit_player = Some(e);
                                    hit.write(PlayerHit { from: pos });
                                }
                            }
                            Some(Target::Bot(x)) => damage_bots.push((x, e, b.damage)),
                            None => {}
                        }
                    }
                }
                // AIBotController dodging, in bursts like players: now and then while
                // shooting (every 1.5-3 s, 35 %), and often right after being hit (hurt
                // starts one in take_damage); a burst lasts dodgingTime x 1..1.8
                b.strafe_check -= dt;
                if b.strafe <= 0.0 && b.strafe_check <= 0.0 && b.react <= 0.0 {
                    b.strafe_check = 1.5 + 1.5 * b.rand();
                    if b.rand() < 0.35 {
                        b.strafe = tu.controller.dodging_time * (1.0 + 0.8 * b.rand());
                        b.dodge_dir = if b.rand() < 0.5 { -1.0 } else { 1.0 };
                    }
                }
                b.goal = tp;
            }
            None => {
                b.next_shot = b.next_shot.max(0.3);
                // look for a fight: head for a random living enemy (most of the time) or a
                // point near a random spawn zone, within maxMoveDistance of it
                if pos.distance(b.goal) < 2.0 || b.path.is_empty() {
                    let enemies: Vec<Vec3> = everyone
                        .iter()
                        .filter(|o| o.3 && o.2 != b.team)
                        .map(|o| o.1)
                        .chain(if b.team == Team::Red && player_alive { player_pos } else { None })
                        .collect();
                    let base = if !enemies.is_empty() && b.rand() < 0.7 {
                        enemies[(b.rand() * enemies.len() as f32) as usize % enemies.len()]
                    } else {
                        let i = (b.rand() * setup.roam.len() as f32) as usize;
                        setup.roam.get(i).copied().unwrap_or(pos)
                    };
                    let off = Vec3::new(b.rand() - 0.5, 0.0, b.rand() - 0.5) * tu.controller.max_move_distance;
                    b.goal = nav.walkable_point(base + off).unwrap_or(base);
                    b.repath = 0.0;
                }
            }
        }
    }
    let _ = damage_bots.len();
    for (victim, shooter, amount) in damage_bots {
        apply_bot_damage(&mut commands, &mut bots, &mut scores, victim, Some(shooter), amount, &mut stats);
    }
}

/// Damage to a bot; on death, the kill counts for the shooter's team.
fn apply_bot_damage(
    commands: &mut Commands,
    bots: &mut Query<(Entity, &Transform, &mut PlayerBot)>,
    scores: &mut TdmScores,
    victim: Entity,
    shooter: Option<Entity>,
    amount: f32,
    stats: &mut PlayerStats,
) {
    let Ok((_, t, mut b)) = bots.get_mut(victim) else { return };
    if !b.alive() {
        return;
    }
    b.health -= amount;
    if b.alive() {
        // flinch; most of the time start dodging (GUESS 60 %)
        if b.hurt <= 0.0 {
            b.hurt = 0.27;
            if b.strafe <= 0.0 && b.rand() < 0.6 {
                b.strafe = 0.7 * (1.0 + 0.8 * b.rand());
                b.dodge_dir = if b.rand() < 0.5 { -1.0 } else { 1.0 };
            }
        }
        return;
    }
    b.respawn_in = RESPAWN;
    let (team, row, pos) = (b.team, b.row, t.translation);
    commands.entity(victim).insert(Visibility::Hidden);
    let _ = pos;
    if let Some(r) = scores.rows.get_mut(row) {
        r.deaths += 1;
    }
    let killer_row = match shooter {
        Some(s) => bots.get(s).ok().map(|(_, _, k)| k.row),
        None => {
            // you
            scores.vs.entry(row).or_default().0 += 1;
            stats.kills += 1;
            stats.score += 100;
            Some(0)
        }
    };
    if let Some(r) = killer_row.and_then(|r| scores.rows.get_mut(r)) {
        r.kills += 1;
    }
    match team {
        Team::Red => scores.blue += 1,
        Team::Blue => scores.red += 1,
    }
    info!("tdm kill: {:?} bot down, score {} : {}", team, scores.blue, scores.red);
}

/// Your bullets on red bots (BotDamage from weapons.rs).
fn take_damage(
    mut commands: Commands,
    mut hits: MessageReader<BotDamage>,
    mut bots: Query<(Entity, &Transform, &mut PlayerBot)>,
    mut scores: ResMut<TdmScores>,
    mut stats: ResMut<PlayerStats>,
) {
    for h in hits.read() {
        if bots.contains(h.bot) {
            apply_bot_damage(&mut commands, &mut bots, &mut scores, h.bot, None, h.amount, &mut stats);
        }
    }
}

/// Your death counts for the red team and the bot that hit you last.
fn player_killed(stats: Res<PlayerStats>, mut scores: ResMut<TdmScores>, bots: Query<&PlayerBot>, mut was_dead: Local<bool>) {
    let dead = stats.health <= 0.0;
    if dead && !*was_dead {
        scores.red += 1;
        info!("tdm kill: you died, score {} : {}", scores.blue, scores.red);
        if let Some(r) = scores.rows.get_mut(0) {
            r.deaths += 1;
        }
        if let Some(row) = scores.last_hit_player.and_then(|e| bots.get(e).ok()).map(|b| b.row)
            && let Some(r) = scores.rows.get_mut(row)
        {
            r.kills += 1;
            scores.vs.entry(row).or_default().1 += 1;
        }
    }
    *was_dead = dead;
}

/// Dead bots come back at their team's spawn after RESPAWN seconds.
fn respawn(
    mut commands: Commands,
    time: Res<Time>,
    setup: Option<Res<BotSetup>>,
    mut bots: Query<(Entity, &mut Transform, &mut PlayerBot)>,
    spatial: SpatialQuery,
) {
    let Some(setup) = setup else { return };
    for (e, mut t, mut b) in &mut bots {
        // hopelessly stuck (inside a block): start again from a spawn point
        if b.alive() && b.stuck > 6.0 {
            let spawns = if b.team == Team::Blue { &setup.blue_spawns } else { &setup.red_spawns };
            let i = (b.rand() * spawns.len() as f32) as usize;
            if let Some(&at) = spawns.get(i) {
                t.translation = stand_at(&spatial, at);
            }
            b.stuck = 0.0;
            b.path.clear();
        }
        if b.alive() {
            continue;
        }
        b.respawn_in -= time.delta_secs();
        if b.respawn_in > 0.0 {
            continue;
        }
        let spawns = if b.team == Team::Blue { &setup.blue_spawns } else { &setup.red_spawns };
        let i = (b.rand() * spawns.len() as f32) as usize;
        let at = spawns.get(i).copied().unwrap_or(t.translation);
        t.translation = stand_at(&spatial, at);
        b.health = PLAYER_MAX_HEALTH;
        b.velocity = Vec3::ZERO;
        b.target = None;
        b.path.clear();
        b.next_shot = SPAWN_PROTECTION;
        commands.entity(e).insert(Visibility::Inherited);
    }
}

/// Walk along the navmesh to the goal (strafing while fighting), gravity, jumps.
#[allow(clippy::type_complexity)]
fn movement(
    spatial: SpatialQuery,
    time: Res<Time>,
    setup: Option<Res<BotSetup>>,
    mut bots: Query<(Entity, &mut Transform, &mut PlayerBot, &Children)>,
    colliders: Query<(&Collider, &Transform), Without<PlayerBot>>,
    move_and_slide: MoveAndSlide,
    nav: Res<crate::navmesh::NavMesh>,
) {
    let Some(setup) = setup else { return };
    let mv = &setup.tuning.movement;
    let dt = time.delta_secs();
    for (e, mut t, mut b, children) in &mut bots {
        if !b.alive() {
            continue;
        }
        let Some((ce, col, off)) = children.iter().find_map(|c| colliders.get(c).ok().map(|(x, ct)| (c, x, ct.translation))) else {
            continue;
        };
        let fighting = b.target.is_some();
        // path to the goal, recomputed twice a second
        b.repath -= dt;
        if b.repath <= 0.0 && !nav.is_empty() {
            b.repath = 0.5;
            b.path = nav.find_path(t.translation, b.goal).unwrap_or_default();
        }
        while let Some(&wp) = b.path.first() {
            if Vec2::new(wp.x - t.translation.x, wp.z - t.translation.z).length() < 0.6 && b.path.len() > 1 {
                b.path.remove(0);
            } else {
                break;
            }
        }
        let aim = b.path.first().copied().unwrap_or(b.goal);
        let mut dir = Vec3::new(aim.x - t.translation.x, 0.0, aim.z - t.translation.z);
        let close = dir.length() < 1.0;
        dir = dir.normalize_or_zero();
        // roam a little slower than full speed (players don't sprint everywhere)
        let mut speed = RUN_SPEED * 0.85;
        if fighting {
            // close in to its favourite range, back off when too close, otherwise stand
            // and shoot; strafe only in bursts (dodgingSpeed x 0.8)
            let to = Vec3::new(b.goal.x - t.translation.x, 0.0, b.goal.z - t.translation.z);
            let d = to.length();
            let face = to.normalize_or_zero();
            let side = Vec3::new(face.z, 0.0, -face.x) * b.dodge_dir;
            let approach = if d > b.fight_dist + 3.0 { 1.0 } else if d < b.fight_dist - 4.0 { -0.6 } else { 0.0 };
            let path_dir = dir;
            dir = if approach > 0.0 { path_dir } else { face * approach };
            speed = RUN_SPEED * 0.7;
            if b.strafe > 0.0 {
                dir = (dir * 0.4 + side).normalize_or_zero();
                speed = (setup.tuning.controller.dodging_speed * 0.8).min(RUN_SPEED * STRAFE_MAX);
            } else if approach == 0.0 {
                dir = Vec3::ZERO;
            }
        } else {
            // face where it walks
            // turn smoothly towards where it walks (no snapping round corners)
            if dir != Vec3::ZERO {
                let diff = (dir.x.atan2(dir.z) - b.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                let rate = b.turn_speed.min(ROAM_TURN);
                b.yaw += diff.clamp(-rate * dt, rate * dt);
            }
            if close {
                dir = Vec3::ZERO;
            }
        }
        // look ahead (the maps are made of blocks: steps, crates and walls): a block at the
        // knees with room above it is jumped onto; a wall at chest height is walked around,
        // trying 45 and 90 degrees to either side for the first clear way
        let world = SpatialQueryFilter::from_mask(Layer::World).with_excluded_entities([e, ce]);
        let ray = |from: Vec3, d: Vec3, len: f32| Dir3::new(d).ok().and_then(|d| spatial.cast_ray(from, d, len, true, &world)).is_some();
        let feet = t.translation;
        let mut want_jump = false;
        if dir != Vec3::ZERO {
            let knee = ray(feet + Vec3::Y * 0.35, dir, 0.9);
            let chest = ray(feet + Vec3::Y * 1.3, dir, 0.9);
            if knee && !chest {
                want_jump = true;
            } else if chest && b.detour.1 <= 0.0 {
                let turns = [0.8f32, -0.8, 1.57, -1.57];
                if let Some(a) = turns.iter().copied().find(|a| {
                    let d = Quat::from_rotation_y(*a) * dir;
                    !ray(feet + Vec3::Y * 1.3, d, 1.5) && !ray(feet + Vec3::Y * 0.35, d, 1.0)
                }) {
                    b.detour = (Quat::from_rotation_y(a) * dir, 0.6);
                }
            }
        }
        // jump up a ledge on the path (PlayerBotMovement)
        if let Some(&wp) = b.path.first() {
            let up = wp.y - t.translation.y;
            if up > mv.jump_needed_delta_height || (up > 0.3 && b.rand() * 100.0 < mv.jump_when_possible_chance * dt) {
                want_jump = true;
            }
        }
        // stuck (blocked, or hanging on an edge): jump, then walk a detour, then pick a new goal
        if b.stuck > 0.3 {
            want_jump = true;
        }
        if b.stuck > 1.0 && b.detour.1 <= 0.0 {
            let a = if b.rand() < 0.5 { 1.57 } else { -1.57 };
            b.detour = (Quat::from_rotation_y(a) * if dir == Vec3::ZERO { Vec3::Z } else { dir }, 0.8);
        }
        if b.stuck > 2.5 && b.stuck < 2.5 + dt * 1.5 {
            b.path.clear();
            b.goal = t.translation + Quat::from_rotation_y(b.rand() * std::f32::consts::TAU) * Vec3::Z * 6.0;
            b.repath = 0.0;
        }
        if b.detour.1 > 0.0 {
            b.detour.1 -= dt;
            dir = b.detour.0;
        }
        let standing = b.grounded || (b.velocity.y.abs() < 0.5 && b.stuck > 0.2);
        if want_jump && standing {
            b.velocity.y = mv.jump_speed;
            b.grounded = false;
        }
        // speed up and slow down like a player (no instant direction flips)
        let want_v = Vec2::new(dir.x, dir.z) * speed;
        let cur_v = Vec2::new(b.velocity.x, b.velocity.z);
        let accel = if b.grounded { ACCEL } else { ACCEL * 0.3 };
        let new_v = cur_v + (want_v - cur_v).clamp_length_max(accel * dt);
        b.velocity.x = new_v.x;
        b.velocity.z = new_v.y;
        if b.grounded && b.velocity.y <= 0.0 {
            b.velocity.y = -2.0;
        } else {
            b.velocity.y = (b.velocity.y - GRAVITY * dt).max(-17.0);
        }
        let mut grounded = false;
        let rot = Quat::from_rotation_y(b.yaw);
        let out = move_and_slide.move_and_slide(
            col,
            t.translation + off,
            Quat::IDENTITY,
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
        t.translation = out.position - off;
        b.grounded = grounded;
        b.air = if grounded { 0.0 } else { b.air + dt };
        // progress check (horizontal): under a third of the wanted move counts as stuck
        let moved = Vec2::new(t.translation.x - before.x, t.translation.z - before.z).length();
        let wanted = Vec2::new(b.velocity.x, b.velocity.z).length() * dt;
        if wanted > 0.01 && moved < wanted * 0.33 {
            b.stuck += dt;
        } else {
            b.stuck = (b.stuck - dt * 2.0).max(0.0);
        }
        if grounded && b.velocity.y < 0.0 {
            b.velocity.y = 0.0;
        }
        t.rotation = rot;
        if std::env::var("PG3D_TRACE").is_ok() && (time.elapsed_secs() % 3.0) < dt {
            info!("tdm bot {:?} pos={:.1} goal={:.1} path={} grounded={} target={:?} vel={:.1}", b.team, t.translation, b.goal, b.path.len(), b.grounded, b.target, b.velocity);
        }
        if t.translation.y < -100.0 {
            b.health = 0.0;
            b.respawn_in = RESPAWN;
        }
    }
}

/// Names above the bots' heads (only those you can see).
fn name_tags(
    mut tags: Query<(&NameTag, &mut Node, &mut Visibility, &ComputedNode)>,
    bots: Query<(&GlobalTransform, &PlayerBot)>,
    camera: Query<(&Camera, &GlobalTransform), With<crate::camera::PlayerCamera>>,
    spatial: SpatialQuery,
) {
    let Ok((cam, ct)) = camera.single() else { return };
    for (tag, mut node, mut vis, cn) in &mut tags {
        let Ok((gt, b)) = bots.get(tag.0) else { continue };
        let head = gt.translation() + Vec3::Y * 2.2;
        let mut shown = false;
        if b.alive()
            && head.distance(ct.translation()) < 40.0
            && line_clear(&spatial, ct.translation(), head)
            && let Ok(p) = cam.world_to_viewport(ct, head)
        {
            let w = cn.size().x * cn.inverse_scale_factor();
            node.left = px(p.x - w / 2.0);
            node.top = px(p.y);
            shown = true;
        }
        *vis = if shown { Visibility::Inherited } else { Visibility::Hidden };
    }
}

/// Seconds after dying before "To Battle!" can be pressed (RespawnWindow's "CAN RESPAWN IN:";
/// GUESS 3 s, the old fixed respawn time).
pub const CAN_RESPAWN: f32 = 3.0;
/// Seconds until the automatic respawn (RespawnWindow's "To respawn" timer; GUESS 10 s).
pub const AUTO_RESPAWN: f32 = 10.0;

#[derive(Resource)]
struct RespawnScreen(Option<Entity>);

fn spawn_respawn_screen(mut commands: Commands, assets: Res<AssetServer>, data: Res<DataDir>) {
    let e = crate::ngui::spawn_screen(&mut commands, &assets, &data, "respawn");
    if let Some(e) = e {
        commands.entity(e).insert(Visibility::Hidden);
    }
    commands.insert_resource(RespawnScreen(e));
}

/// The parts of the original RespawnWindow we have nothing for (clans, kill cam, shop
/// offers, loadouts, battle pass, spawn picker, report, champion classes, armor).
const RESPAWN_OFF: [&str; 27] = [
    "KillerClanName_Label",
    "Killer_veteranbadge",
    "ProgressBarArmor",
    "DragArea",
    "KillerView",
    "DragKillCam",
    "ItemToBuyScaler",
    "ClanWeapon",
    "EventWeapon",
    "KillerWeaponModulesIcons",
    "RW_RivalItems_container",
    "RW_BtnRivalItems (obs)",
    "Btn_Report",
    "RW_UpgradeHolder (OFF)",
    "LoadoutHolder (OFF)",
    "OnUpgradeEnable_cont (OFF)",
    "Progress_IfUpgrade",
    "KillerItems (obsolete)",
    "itemToBuyScaler",
    "Bottom",
    "CharacterViewHolder",
    "ScoreConat (OFF)",
    "RW_LoadoutPanel (OFF)",
    "NetworkStartTable_FM_SelectSpawn (OFF)",
    "ChangeClass_Btn (OFF)",
    "BtnGoBattleImm",
    "CanRespawnSniper",
];

fn respawn_layout(s: &mut NguiScreen) {
    for name in RESPAWN_OFF {
        if let Some(i) = s.find(name) {
            s.set_active(i, false);
        }
    }
    // how the player died: shot (no burn / bleed / poison icons)
    for i in 0..s.nodes.len() {
        let n = s.nodes[i].name.as_str();
        if n.starts_with("Icon_dead_by_") || n.starts_with("bg_dead_by_") || n == "arrow_up" || n == "arrow_down" {
            s.set_active(i, false);
        }
    }
    if let Some(h) = s.find_path("KillerWeapon/Header_Label") {
        let t = s.string("Key_7756");
        s.set_text(h, t);
    }
    if let Some(h) = s.find_path("AutoRespawnLabel") {
        let t = s.string("Key_4937");
        s.set_text(h, t);
    }
}

/// While you are dead: the original's RespawnWindow — "You were killed by:" with the
/// killer's name and rank, your kills of each other (You VS Rival), the killer's health,
/// its weapon next to yours with their damage bars, and "To Battle!" once the
/// "CAN RESPAWN IN" count is over (Space works too); otherwise the auto respawn timer.
#[allow(clippy::too_many_arguments)]
fn death_view(
    mut stats: ResMut<PlayerStats>,
    scores: Res<TdmScores>,
    bots: Query<&PlayerBot>,
    data: Res<DataDir>,
    assets: Res<AssetServer>,
    screen: Option<Res<RespawnScreen>>,
    mut screens: Query<&mut NguiScreen>,
    mut vis: Query<&mut Visibility>,
    mut clicks: MessageReader<crate::ngui::NguiClick>,
    keys: Res<ButtonInput<KeyCode>>,
    loadout: Option<Res<crate::weapons::Loadout>>,
    mut cursor: Single<&mut CursorOptions>,
    mut shown: Local<bool>,
    mut weapons: Local<Option<std::collections::HashMap<String, serde_json::Value>>>,
) {
    let Some(e) = screen.and_then(|s| s.0) else { return };
    let dead = stats.health <= 0.0;
    let pressed = clicks.read().any(|c| c.name == "BtnGoBattle") || keys.just_pressed(KeyCode::Space);
    if dead != *shown {
        *shown = dead;
        if let Ok(mut v) = vis.get_mut(e) {
            *v = if dead { Visibility::Inherited } else { Visibility::Hidden };
        }
        cursor.grab_mode = if dead { CursorGrabMode::None } else { CursorGrabMode::Locked };
        cursor.visible = dead;
        if !dead {
            return;
        }
        let Ok(mut s) = screens.get_mut(e) else { return };
        respawn_layout(&mut s);
        let w = weapons.get_or_insert_with(|| {
            std::fs::read_to_string(data.0.join("weapons/weapons.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
        });
        let wname = |s: &NguiScreen, n: &str| {
            w.get(n).and_then(|d| d.get("localizeWeaponKey")).and_then(|k| k.as_str()).map(|k| s.string(k)).unwrap_or_else(|| n.to_string())
        };
        let damage = |n: &str| {
            let d = w.get(n);
            let tier = d.and_then(|d| d.get("damageByTier")).and_then(|v| v.get(0)).and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
            let shot = d.and_then(|d| d.get("isShotGun")).and_then(|v| v.as_i64()).unwrap_or(0) != 0;
            let pellets = if shot { d.and_then(|d| d.get("countShots")).and_then(|v| v.as_i64()).unwrap_or(1).max(1) as f32 } else { 1.0 };
            tier * pellets
        };
        let killer = scores.last_hit_player.and_then(|k| bots.get(k).ok());
        let row = killer.map(|b| b.row);
        let name = row.and_then(|r| scores.rows.get(r)).map(|r| r.name.clone()).unwrap_or_default();
        if let Some(i) = s.find("KillerNicknameLabel") {
            s.set_text(i, name);
        }
        let (mine, theirs) = row.and_then(|r| scores.vs.get(&r)).copied().unwrap_or_default();
        if let Some(i) = s.find("PlayerKillCount") {
            s.set_text(i, mine.to_string());
        }
        if let Some(i) = s.find("RivalKillCount") {
            s.set_text(i, theirs.to_string());
        }
        // the killer's health left
        let hp = killer.map_or(0.0, |b| b.health.max(0.0));
        if let Some(bar) = s.find("ProgressBarHealth") {
            if let Some(f) = s.find_in(bar, "fill") {
                s.set_fill(f, hp / PLAYER_MAX_HEALTH);
            }
            if let Some(l) = s.find_in(bar, "Amount_Label") {
                s.set_text(l, (hp.ceil() as i32).to_string());
            }
        }
        // the two weapons and their damage per second, as bars of the stronger one
        let theirs_w = killer.map(|b| (b.weapon_name.to_string(), b.damage / b.interval.max(0.05)));
        let mine_w = loadout.as_ref().and_then(|l| l.current()).map(|cw| (cw.name.clone(), damage(&cw.name) / cw.fire_interval.max(0.05)));
        let top = theirs_w.iter().chain(mine_w.iter()).map(|w| w.1).fold(0.01f32, f32::max);
        for (panel, wp) in [("KillerWeapon", theirs_w), ("YourWeapon", mine_w)] {
            let Some(p) = s.find(panel) else { continue };
            let Some((n, dps)) = wp else {
                s.set_active(p, false);
                continue;
            };
            s.set_active(p, true);
            let label = wname(&s, &n);
            for l in ["ItemName_Label", "ItemName"] {
                if let Some(i) = s.find_in(p, l) {
                    s.set_text(i, label.clone());
                }
            }
            if let Some(i) = s.find_in(p, "ItemImage") {
                s.set_texture(i, assets.load(format!("weapons/{n}/icon.png")));
            }
            if let Some(i) = s.find_in(p, "ProgressArmor") {
                s.set_fill(i, dps / top);
            }
        }
    }
    if !dead {
        return;
    }
    // the timers
    let since = AUTO_RESPAWN - stats.respawn_in;
    let can = since >= CAN_RESPAWN;
    if let Ok(mut s) = screens.get_mut(e) {
        for (name, on) in [("CanRespawnLabel", !can), ("BtnGoBattle", can), ("AutoRespawnLabel", can)] {
            if let Some(i) = s.find(name) {
                s.set_active(i, on);
            }
        }
        if let Some(i) = s.find_path("CanRespawnLabel/canRespawnTimerLabel") {
            s.set_text(i, ((CAN_RESPAWN - since).max(0.0).ceil() as i32).to_string());
        }
        if let Some(i) = s.find("autoRespawnTimerLabel") {
            s.set_text(i, (stats.respawn_in.max(0.0).ceil() as i32).to_string());
        }
    }
    if std::env::var("PG3D_TRACE").is_ok()
        && let Ok(s) = screens.get(e)
    {
        let shown: Vec<String> = (0..s.nodes.len()).filter(|&i| s.nodes[i].label.is_some() && s.visible(i)).map(|i| format!("{}={:?}", s.nodes[i].name, s.text(i))).collect();
        info!("respawn window: {}", shown.join(", "));
    }
    if can && pressed && stats.respawn_in > 0.0 {
        stats.respawn_in = 0.0001;
    }
}

/// PG3D_BOT_VIEW=k[,angle] (automated screenshots): the camera follows bot k from 3.5 m,
/// seen from `angle` degrees off its front (0 = facing you).
fn bot_view(
    bots: Query<(&Transform, &PlayerBot)>,
    mut cam: Query<&mut Transform, (With<crate::camera::PlayerCamera>, Without<PlayerBot>)>,
    spatial: SpatialQuery,
) {
    let Ok(v) = std::env::var("PG3D_BOT_VIEW") else { return };
    let mut it = v.split(',');
    let k: usize = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let ang: f32 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0.0);
    let Some((t, b)) = bots.iter().filter(|(_, b)| b.alive()).nth(k) else { return };
    let Ok(mut c) = cam.single_mut() else { return };
    let dir = Quat::from_rotation_y(b.yaw + ang.to_radians()) * Vec3::Z;
    let focus = t.translation + Vec3::Y * 1.0;
    let want = dir * 3.5 + Vec3::Y * 0.6;
    // closer if something is in the way
    let d = Dir3::new(want).ok().and_then(|d| spatial.cast_ray(focus, d, want.length(), true, &SpatialQueryFilter::from_mask(Layer::World))).map_or(want.length(), |h| (h.distance - 0.3).max(0.8));
    c.translation = focus + want.normalize() * d;
    c.look_at(focus, Vec3::Y);
}
