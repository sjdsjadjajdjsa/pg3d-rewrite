//! Weapons: loadout, first-person view model, shooting, reloading, HUD.
//!
//! Data comes from <data>/weapons/ written by ../exporter/scripts/export_weapons.py:
//!   weapons.json          settings of every WeaponSounds prefab in the game
//!   <Weapon>/weapon.gltf  first-person model (arms + gun, static pose for now)
//!   <Weapon>/info.json    animation clip lengths (Shoot = fire interval, Reload = reload time)
//!
//! Controls: left mouse = fire (hold), R = reload, 1-6 or mouse wheel = switch weapon.

use std::collections::HashMap;

use avian3d::prelude::*;
#[allow(unused_imports)]
use crate::scene::{FixedUpdate, PostStartup, PostUpdate, PreStartup, Startup, Update};
use bevy::{
    camera::{ClearColorConfig, Exposure, visibility::RenderLayers},
    core_pipeline::tonemapping::Tonemapping,
    input::mouse::AccumulatedMouseScroll,
    prelude::*,
    window::{CursorGrabMode, CursorOptions},
    world_serialization::WorldInstanceReady,
};
use serde::Deserialize;

use crate::{
    controls::Action,
    DataDir,
    bots::{BotDamage, BotHead, BotPart},
    camera::PlayerCamera,
    map::Layer,
    player::Player,
    projectiles::{Look, Owner, SpawnProjectile},
};

/// Weapons given to the player, in slot order (keys 1..6). Only ones that were exported load.
/// One per original category (1 primary, 2 backup, 3 melee, 4 special, 5 sniper,
/// 6 heavy), then extras. PG3D_LOADOUT=Weapon2,Weapon61,... overrides it.
pub const DEFAULT_LOADOUT: [&str; 9] = [
    "Weapon3", "Weapon1", "Weapon9", "Weapon5", "Weapon61", "Weapon50", "Weapon2", "Weapon11", "Weapon75",
];
/// Render layer used for the first-person weapon so it never clips into walls.
pub const VIEW_MODEL_LAYER: usize = 1;

pub struct WeaponsPlugin;

impl crate::scene::ScenePlugin for WeaponsPlugin {
    fn build(&self, app: &mut crate::scene::SceneBuilder) {
        app.add_systems(Startup, (load_loadout, spawn_hud))
            .add_systems(PostStartup, spawn_view_model_camera)
            .add_systems(
                Update,
                (
                    switch_weapon,
                    (fire, reload).after(switch_weapon),
                    tag_view_model_meshes,
                    animate_view_model,
                    fade_impacts,
                    zoom,
                    track_muzzle,
                    refill_on_respawn,
                    crosshair_sprites,
                ),
            );
    }
}

/// The subset of the original WeaponSounds settings we use so far.
/// Field names are the original ones.
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
#[allow(non_snake_case)]
pub struct WeaponDef {
    /// 1 primary, 2 backup, 3 melee, 4 special, 5 sniper, 6 heavy
    pub categoryNabor: i32,
    pub ammoInClip: i32,
    pub InitialAmmo: i32,
    pub maxAmmo: i32,
    pub isMelee: i32,
    pub isShotGun: i32,
    pub bazooka: i32,
    /// railguns: the shot passes through walls and every enemy on its line
    pub railgun: i32,
    /// laser / freeze rays: a beam to what was hit
    pub freezer: i32,
    /// hits slow enemies to `slowdownCoeff` of their speed for `slowdownTime` seconds
    pub isSlowdown: i32,
    pub slowdownCoeff: f32,
    pub slowdownTime: f32,
    pub isZooming: i32,
    pub fieldOfViewZomm: f32,
    /// movement speed multiplier while holding this weapon
    pub speedModifier: f32,
    /// spread: current ("tek"), added per shot ("up"), recovery per second ("down")
    pub tekKoof: f32,
    pub upKoofFire: f32,
    pub downKoof: f32,
    pub radiusRoundMelee: f32,
    #[serde(default)]
    pub isRoundMelee: i32,
    /// two guns firing in turn (Player_move_c numShootInDouble: "Shoot1", "Shoot2", and the
    /// flash of that gun)
    #[serde(default)]
    pub isDoubleShot: i32,
    pub shootDistanceIfZoom: f32,
    /// pellets per shot for shotguns
    pub countShots: i32,
    /// rocket launchers: which "Rocket_<n>" prefab they fire and its blast radius
    pub rocketNum: i32,
    pub bazookaExplosionRadius: f32,
    /// your own rocket hurts you within this radius (0: never)
    pub bazookaExplosionRadiusSelf: f32,
    /// rocket jumps: the blast pushes its own shooter within this radius (0: never) with
    /// impulseForceSelf (Player_move_c.m_600289E)
    pub bazookaImpulseRadius: f32,
    pub impulseForceSelf: f32,
    /// damage per bullet; index = weapon upgrade tier (we use tier 0)
    pub damageByTier: Vec<f32>,
    /// random damage variation in percent (x = min, y = max)
    pub damageRange: UnityVec2,
    /// offset of the weapon prefab from the camera (Unity coordinates)
    pub gunPosition: UnityVec3,
}

#[derive(Deserialize, Clone, Copy, Debug, Default)]
pub struct UnityVec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Deserialize, Clone, Copy, Debug, Default)]
pub struct UnityVec2 {
    pub x: f32,
    pub y: f32,
}

impl UnityVec3 {
    /// Unity (left-handed) -> Bevy, same X-mirroring as the exporter.
    pub fn to_bevy(self) -> Vec3 {
        Vec3::new(-self.x, self.y, self.z)
    }
}

#[derive(Deserialize, Default)]
struct WeaponInfo {
    #[serde(default)]
    animations: HashMap<String, f32>,
    /// glTF animation names, in file order (GltfAssetLabel::Animation(i))
    #[serde(default)]
    clips: Vec<String>,
    /// two-gun weapons: "Shoot1" / "Shoot2" -> "left" / "right" (export_weapons.py)
    #[serde(default)]
    shot_sides: HashMap<String, String>,
    /// original sound field name -> wav file (shoot, reload, empty, ...)
    #[serde(default)]
    sounds: HashMap<String, String>,
}

pub struct Weapon {
    pub name: String,
    pub def: WeaponDef,
    /// seconds between shots (length of the original "Shoot" animation)
    pub fire_interval: f32,
    /// its "Loading" (draw) clip, played when it is taken out; no shooting until it ends
    pub draw_time: Option<f32>,
    /// seconds (length of the original "Reload" animation)
    pub reload_time: f32,
    pub clip: i32,
    pub reserve: i32,
    pub spread: f32,
    /// animation clips in weapon.gltf (Idle, Walk, Shoot, Reload, ...)
    pub clips: Vec<String>,
    pub shot_sides: HashMap<String, String>,
    /// rocket launchers: the projectile they fire
    pub rocket: Option<RocketDef>,
    /// damage multiplier for hits on a head (BalanceWeaponsCore "H", default HeadShotCoef 1.4)
    pub head_coef: f32,
    /// speed the Reload clip plays at (reload_time = clip length / this)
    pub reload_speed: f32,
    pub sounds: HashMap<String, Handle<AudioSource>>,
}

/// Headshot multipliers from weapons/balance.json (export_balance.py).
#[derive(Deserialize, Default)]
struct HeadCoefs {
    #[serde(default)]
    head: HashMap<String, f32>,
    #[serde(default)]
    head_default: Option<f32>,
    /// Weapons "R": speed of the Reload animation (BalanceController; 1 if absent)
    #[serde(default)]
    reload_speed: HashMap<String, f32>,
}

impl HeadCoefs {
    fn get(&self, weapon: &str) -> f32 {
        let num = weapon.trim_start_matches("Weapon");
        self.head.get(num).copied().or(self.head_default).unwrap_or(1.4).max(1.0)
    }
    fn reload_speed(&self, weapon: &str) -> f32 {
        self.reload_speed.get(weapon.trim_start_matches("Weapon")).copied().filter(|v| *v > 0.05).unwrap_or(1.0)
    }
}

#[derive(Resource, Default)]
pub struct Loadout {
    pub weapons: Vec<Weapon>,
    pub current: usize,
    cooldown: f32,
    /// seconds of reloading left (the HUD's "Reloading" ring: ingame_ui.rs)
    pub reloading: f32,
    kick: f32,
    view_model: Option<Entity>,
    /// one-shot animation the view model should play next ("Shoot", "Reload")
    play: Option<&'static str>,
    /// degrees the weapon is dipped below the view: a freshly drawn weapon comes up from
    /// 40 (Player_move_c's weapon change coroutine, field 0x62c)
    pub lower: f32,
    /// where the gun's barrel (BulletSpawnPoint) shows on screen, as a point in the main
    /// camera's space: rays start there (track_muzzle)
    muzzle: Option<Vec3>,
    /// two-gun weapons: the second gun fires next
    barrel: bool,
    /// each GunFlash of the gun in hand, in the camera's space (left of the view < 0)
    flash_x: Vec<f32>,
}

impl Loadout {
    pub fn current(&self) -> Option<&Weapon> {
        self.weapons.get(self.current)
    }
}

/// Root of the first-person weapon model.
#[derive(Component)]
pub struct ViewModel;

/// The view model's animation graph: clip name -> node.
#[derive(Component)]
struct ViewModelAnims {
    graph: Handle<AnimationGraph>,
    nodes: HashMap<String, AnimationNodeIndex>,
    /// entity holding the AnimationPlayer, found once the glTF scene is spawned
    player: Option<Entity>,
}

/// Bullet hole marker, fades out.
#[derive(Component)]
struct Impact(f32);

/// Original RocketSettings fields we use.
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
#[allow(non_snake_case)]
pub struct RocketDef {
    pub lifeTime: f32,
    pub startForce: f32,
    pub useGravity: i32,
    /// RocketSettings.dontDoSelfDamage
    pub dontDoSelfDamage: i32,
    #[serde(skip)]
    pub model: String,
}

impl RocketDef {
    /// The game computes velocity = Time.fixedDeltaTime * startForce / mass
    /// (RocketSettings.CalcVelocity @ 0x181E33770). The rigidbody mass is not recovered;
    /// 0.2 gives believable speeds (GUESS): 350 -> 35 m/s.
    pub fn speed(&self) -> f32 {
        (0.02 * self.startForce / 0.2).max(5.0)
    }
}

fn rocket_def(data: &std::path::Path, num: i32) -> Option<RocketDef> {
    let name = format!("Rocket_{num}");
    let all: HashMap<String, RocketDef> =
        serde_json::from_str(&std::fs::read_to_string(data.join("rockets/rockets.json")).ok()?).ok()?;
    let mut r = all.get(&name)?.clone();
    r.model = format!("rockets/{name}/rocket.gltf");
    if !data.join(&r.model).exists() {
        warn!("{name} model not exported (run export_rockets.py)");
        return None;
    }
    Some(r)
}

fn load_loadout(mut commands: Commands, data: Res<DataDir>, assets: Res<AssetServer>) {
    let dir = data.0.join("weapons");
    let all: HashMap<String, WeaponDef> = match std::fs::read_to_string(dir.join("weapons.json"))
        .map(|s| serde_json::from_str(&s))
    {
        Ok(Ok(w)) => w,
        other => {
            error!("weapons.json: {:?} (run export_weapons.py)", other.err());
            commands.insert_resource(Loadout::default());
            return;
        }
    };
    info!("{} weapon definitions", all.len());
    let heads: HeadCoefs = std::fs::read_to_string(dir.join("balance.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let mut loadout = Loadout::default();
    let custom = std::env::var("PG3D_LOADOUT").ok();
    // the weapons picked in the Armory replace the default of their category (slots 1-6)
    let saved = crate::armory::SavedLoadout::load();
    let mut picked: Vec<String> = DEFAULT_LOADOUT.iter().map(|s| s.to_string()).collect();
    for (k, slot) in picked.iter_mut().enumerate().take(6) {
        if let Some(w) = saved.equipped.get(&(k as i32 + 1)).filter(|w| all.contains_key(w.as_str())) {
            *slot = w.clone();
        }
    }
    let first: Vec<String> = picked[..6].to_vec();
    // every default weapon not in slots 1-6 stays available on the extra keys
    let extras: Vec<String> =
        DEFAULT_LOADOUT.iter().map(|s| s.to_string()).filter(|w| !first.contains(w)).collect();
    let picked: Vec<String> = first.into_iter().chain(extras).collect();
    let names: Vec<&str> = match &custom {
        Some(c) => c.split(',').map(str::trim).collect(),
        None => picked.iter().map(String::as_str).collect(),
    };
    for name in names {
        let Some(def) = all.get(name) else { continue };
        if !dir.join(name).join("weapon.gltf").exists() {
            continue;
        }
        let info: WeaponInfo = std::fs::read_to_string(dir.join(name).join("info.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        // clips of a few frames are placeholders (e.g. heavy weapons without a reload animation)
        let anim = |k: &str, default: f32| info.animations.get(k).copied().filter(|v| *v > 0.05).unwrap_or(default);
        let clip = if def.isMelee != 0 { 0 } else { def.ammoInClip };
        loadout.weapons.push(Weapon {
            name: name.to_string(),
            def: def.clone(),
            // two-barrelled weapons have Shoot1 / Shoot2 instead of Shoot
            fire_interval: anim("Shoot", anim("Shoot1", 0.3)),
            draw_time: info.animations.get("Loading").copied().filter(|v| *v > 0.05),
            reload_time: anim("Reload", 1.5) / heads.reload_speed(name),
            reload_speed: heads.reload_speed(name),
            clip,
            reserve: (def.maxAmmo - clip).max(0),
            spread: 0.0,
            clips: info.clips.clone(),
            shot_sides: info.shot_sides.clone(),
            rocket: (def.bazooka != 0).then(|| rocket_def(&data.0, def.rocketNum)).flatten(),
            head_coef: heads.get(name),
            sounds: crate::sound::load_sounds(&assets, &format!("weapons/{name}"), &info.sounds),
        });
    }
    info!(
        "loadout: {}",
        loadout.weapons.iter().map(|w| format!("{} ({:.2}s)", w.name, w.fire_interval)).collect::<Vec<_>>().join(", ")
    );
    commands.insert_resource(loadout);
}

/// The camera that draws the weapon (off while the pause Menu is open).
#[derive(Component)]
pub struct ViewModelCamera;

/// The "+" crosshair (hidden while the pause Menu is open).
#[derive(Component)]
pub struct Crosshair;

/// A second camera, attached to the main one, draws only the weapon on top of the world.
fn spawn_view_model_camera(
    mut commands: Commands,
    main: Single<Entity, With<PlayerCamera>>,
    mut loadout: ResMut<Loadout>,
    assets: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    commands.entity(*main).with_children(|c| {
        c.spawn((
            Camera3d::default(),
            Camera { order: 1, clear_color: ClearColorConfig::None, ..default() },
            Projection::Perspective(PerspectiveProjection { fov: 60f32.to_radians(), near: 0.01, ..default() }),
            Exposure { ev100: -0.263_034_4 },
            Tonemapping::None,
            RenderLayers::layer(VIEW_MODEL_LAYER),
            ViewModelCamera,
        ));
    });
    if let Some(w) = loadout.current() {
        let e = spawn_view_model(&mut commands, *main, &assets, &mut graphs, w);
        loadout.view_model = Some(e);
    }
}

/// Where the weapon model sits relative to the camera: the original places the prefab
/// at `gunPosition` (about 1.7 m below the eye, i.e. the prefab origin is at the feet).
/// The prefabs face Unity's +Z, which after our X-mirroring is Bevy's +Z, while a Bevy
/// camera looks down -Z, so turn them around.
fn view_model_transform(gun_position: Vec3) -> Transform {
    let rot = Quat::from_rotation_y(std::f32::consts::PI);
    Transform::from_translation(rot * gun_position).with_rotation(rot)
}

fn spawn_view_model(
    commands: &mut Commands,
    camera: Entity,
    assets: &AssetServer,
    graphs: &mut Assets<AnimationGraph>,
    w: &Weapon,
) -> Entity {
    let name = &w.name;
    let path = format!("weapons/{name}/weapon.gltf");
    let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(path.clone()));
    let (graph, indices) = AnimationGraph::from_clips(
        (0..w.clips.len()).map(|i| assets.load(GltfAssetLabel::Animation(i).from_asset(path.clone()))),
    );
    let anims = ViewModelAnims {
        graph: graphs.add(graph),
        nodes: w.clips.iter().cloned().zip(indices).collect(),
        player: None,
    };
    let e = commands
        .spawn((
            Name::new(format!("view model {name}")),
            ViewModel,
            crate::particles::AssetDir(format!("weapons/{name}")),
            anims,
            WorldAssetRoot(scene),
            view_model_transform(w.def.gunPosition.to_bevy()),
            RenderLayers::layer(VIEW_MODEL_LAYER),
        ))
        .observe(hook_up_animation_player)
        .id();
    commands.entity(camera).add_child(e);
    e
}

/// When the weapon's glTF scene has spawned, give its AnimationPlayer our graph and
/// start the idle loop.
fn hook_up_animation_player(
    ready: On<WorldInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    mut roots: Query<&mut ViewModelAnims>,
    mut players: Query<&mut AnimationPlayer>,
) {
    let Ok(mut anims) = roots.get_mut(ready.entity) else { return };
    for child in children.iter_descendants(ready.entity) {
        if let Ok(mut player) = players.get_mut(child) {
            commands.entity(child).insert(AnimationGraphHandle(anims.graph.clone()));
            if let Some(idle) = anims.nodes.get("Idle") {
                player.play(*idle).repeat();
            }
            anims.player = Some(child);
            info!("view model animations ready: {:?}", anims.nodes.keys().collect::<Vec<_>>());
            break;
        }
    }
}

/// glTF meshes are spawned later as children; move them onto the view model layer.
/// The weapon is lit like the zombies: by the sun and the map's light probes. Maps
/// without probes draw it unlit so it is never pitch black.
/// How much the first-person gun lights itself on maps with light probes (x its texture).
const GUN_GLOW: f32 = 0.6;

fn tag_view_model_meshes(
    mut commands: Commands,
    added: Query<(Entity, Option<&MeshMaterial3d<StandardMaterial>>), (Added<Mesh3d>, Without<RenderLayers>)>,
    parents: Query<&ChildOf>,
    roots: Query<(), With<ViewModel>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    probes: Res<crate::lighting::HasProbes>,
) {
    for (e, mat) in &added {
        if !parents.iter_ancestors(e).any(|a| roots.contains(a)) {
            continue;
        }
        let mut ec = commands.entity(e);
        ec.insert(RenderLayers::layer(VIEW_MODEL_LAYER));
        if probes.0 {
            // lit by the map's light probes (the gun's Optimized/MultiShader has
            // LIGHTING_ENABLE; the arms' Cubic/Universal is UNLIT and stays so), with a glow
            // of its own colours so it never sinks into a dark corner (OUR CHOICE)
            if let Some(m) = mat.and_then(|m| materials.get(&m.0)).filter(|m| !m.unlit && m.alpha_mode == AlphaMode::Opaque).cloned() {
                let f = std::env::var("PG3D_GUN_GLOW").ok().and_then(|v| v.parse().ok()).unwrap_or(GUN_GLOW);
                let emissive_texture = m.base_color_texture.clone();
                ec.insert(MeshMaterial3d(materials.add(StandardMaterial { emissive: LinearRgba::rgb(f, f, f), emissive_texture, ..m })));
            }
            continue;
        }
        if let Some(m) = mat.and_then(|m| materials.get(&m.0)).cloned() {
            ec.insert(MeshMaterial3d(materials.add(StandardMaterial { unlit: true, ..m })));
        }
    }
}

fn switch_weapon(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    input: crate::controls::Input,
    scroll: Res<AccumulatedMouseScroll>,
    mut loadout: ResMut<Loadout>,
    camera: Single<Entity, With<PlayerCamera>>,
    assets: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    time: Res<Time>,
    game_sounds: Option<Res<crate::sound::GameSounds>>,
) {
    let n = loadout.weapons.len();
    if n == 0 {
        return;
    }
    // PG3D_SOAK (automated stability test): switch to the next weapon every 2.5 s
    let soak = std::env::var("PG3D_SOAK").is_ok() && (time.elapsed_secs() % 2.5) < time.delta_secs();
    let mut next = loadout.current;
    // bar slots 1-6 follow the key bindings; extra weapons 7-9 are on the number keys
    for (i, a) in crate::controls::Action::SLOTS.into_iter().enumerate() {
        if input.just_pressed(a) && i < n {
            next = i;
        }
    }
    for (i, k) in [KeyCode::Digit7, KeyCode::Digit8, KeyCode::Digit9].into_iter().enumerate() {
        if keys.just_pressed(k) && i + 6 < n {
            next = i + 6;
        }
    }
    // the wheel goes round the six weapons on the bar (your Armory picks); the extra
    // starter weapons on keys 7-9 stay out of it (PG3D_SOAK tests cycle through all)
    let ring = if soak { n } else { n.min(6) };
    let from = loadout.current.min(ring - 1);
    if scroll.delta.y > 0.0 {
        next = (from + ring - 1) % ring;
    } else if scroll.delta.y < 0.0 || soak {
        next = (from + 1) % ring;
    }
    if next == loadout.current {
        return;
    }
    loadout.current = next;
    loadout.reloading = 0.0;
    // Player_move_c's weapon change: the category's ChangeWeaponClip, then the weapon's
    // "Loading" (draw) clip if it has one (no shooting while it plays), otherwise it rises
    // from 40 degrees down at 200 a second
    let w = &loadout.weapons[next];
    let sound = match w.def.categoryNabor {
        1 => "Primary",
        2 => "Backup",
        3 => "Melee",
        4 => "Special",
        5 => "Sniper",
        6 => "Heavy",
        _ => "",
    };
    let clip = game_sounds.as_ref().and_then(|g| g.get(&format!("player/ChangeWeaponClip_{sound}"), 0).or_else(|| g.get("player/ChangeWeaponClip", 0)));
    crate::sound::play(&mut commands, clip, None, 0.8);
    if let Some(t) = w.draw_time {
        loadout.cooldown = t;
        loadout.lower = 0.0;
        loadout.play = Some("Loading");
    } else {
        loadout.cooldown = RAISE_FROM / RAISE_SPEED;
        loadout.lower = RAISE_FROM;
    }
    if let Some(old) = loadout.view_model.take() {
        commands.entity(old).despawn();
    }
    let e = spawn_view_model(&mut commands, *camera, &assets, &mut graphs, &loadout.weapons[next]);
    loadout.view_model = Some(e);
}

#[allow(clippy::too_many_arguments)]
fn fire(
    mut commands: Commands,
    time: Res<Time>,
    input: crate::controls::Input,
    cursor: Single<&CursorOptions>,
    mut loadout: ResMut<Loadout>,
    camera: Single<&GlobalTransform, With<PlayerCamera>>,
    spatial: SpatialQuery,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut impact_assets: Local<Option<(Handle<Mesh>, Handle<StandardMaterial>)>>,
    bot_parts: Query<(&BotPart, Has<BotHead>)>,
    mut hits: MessageWriter<BotDamage>,
    mut rockets: MessageWriter<SpawnProjectile>,
    mut flashes: MessageWriter<crate::particles::Burst>,
    (alive, game_sounds, part_globals): (Res<crate::bots::PlayerStats>, Option<Res<crate::sound::GameSounds>>, Query<(&GlobalTransform, &CollisionLayers, Has<ColliderDisabled>), With<crate::bots::BotPart>>),
    (mut beams, mut kicks, mut marks, mut impacts): (
        MessageWriter<crate::beams::SpawnBeam>,
        MessageWriter<crate::feel::ShotKick>,
        MessageWriter<crate::feel::HitMark>,
        MessageWriter<crate::feel::ShotImpact>,
    ),
) {
    let dt = time.delta_secs();
    if alive.health <= 0.0 {
        return;
    }
    // reborrow as a plain struct so we can use different fields at the same time
    let loadout = &mut *loadout;
    loadout.cooldown = (loadout.cooldown - dt).max(0.0);
    loadout.kick = (loadout.kick - dt * 6.0).max(0.0);
    let cur = loadout.current;
    let Some(w) = loadout.weapons.get_mut(cur) else { return };
    w.spread = (w.spread - w.def.downKoof * dt).max(0.0);

    let autofire = std::env::var("PG3D_AUTOFIRE").is_ok(); // automated tests
    let wants = autofire || (input.pressed(Action::Fire) && cursor.grab_mode != CursorGrabMode::None);
    if !wants || loadout.cooldown > 0.0 || loadout.reloading > 0.0 {
        return;
    }
    let melee = w.def.isMelee != 0;
    if !melee && w.clip <= 0 {
        if w.reserve > 0 {
            loadout.reloading = w.reload_time;
            loadout.play = Some("Reload");
            crate::sound::play(&mut commands, w.sounds.get("reload"), None, 0.8);
        } else if input.just_pressed(Action::Fire) {
            crate::sound::play(&mut commands, w.sounds.get("empty"), None, 0.8);
        }
        return;
    }
    crate::sound::play(&mut commands, w.sounds.get("shoot"), None, 0.7);
    if !melee {
        w.clip -= 1;
    }
    // Shotguns fire `countShots` pellets (15 for the starter shotgun: 15 x 0.215 damage
    // = one full hit kills a 3 HP campaign zombie, which matches the original).
    let pellets = if w.def.isShotGun != 0 { w.def.countShots.max(1) as u32 } else { 1 };
    let base_damage = w.def.damageByTier.first().copied().unwrap_or(1.0);
    let (dmin, dmax) = (w.def.damageRange.x, w.def.damageRange.y);
    // melee reach: WeaponContainer.Range (balance "_range", default 3 m); round melee
    // (isRoundMelee) hits everyone within radiusRoundMelee instead
    let range = if melee {
        if w.def.isRoundMelee != 0 { w.def.radiusRoundMelee.max(crate::tdm_bots::MELEE_RANGE) } else { crate::tdm_bots::MELEE_RANGE }
    } else {
        300.0
    };
    // spread in degrees: base + accumulated recoil (GUESS: unit of tekKoof/upKoofFire)
    let spread_deg = w.def.tekKoof + w.spread + if pellets > 1 { 4.0 } else { 0.0 };
    // the scatter grows by upKoofFire a shot up to the weapon balance's _maxCoef (default 4,
    // T_2003C6A .cctor); before, holding the trigger on an automatic grew it without end
    // and long bursts sprayed far off the crosshair
    w.spread = (w.spread + w.def.upKoofFire).min((MAX_SCATTER - w.def.tekKoof).max(0.0));
    let interval = w.fire_interval;
    loadout.cooldown = interval;
    loadout.kick = 1.0;
    loadout.play = Some("Shoot");
    // camera kick (GUESS, by feel): slow, heavy guns kick more; shotguns and launchers most
    if !melee {
        let mut deg = (0.45 + interval * 1.6).clamp(0.45, 2.5);
        if pellets > 1 {
            deg = deg.max(2.2);
        }
        if w.rocket.is_some() {
            deg = deg.max(2.8);
        }
        kicks.write(crate::feel::ShotKick(deg));
    }
    if let Some(vm) = loadout.view_model {
        let double = w.def.isDoubleShot != 0 || (w.clips.iter().any(|c| c == "Shoot1") && !w.clips.iter().any(|c| c == "Shoot"));
        if double {
            loadout.barrel = !loadout.barrel;
            // the clip that plays now (animate_view_model) and the hand it fires
            let clip = if loadout.barrel { "Shoot2" } else { "Shoot1" };
            let sides = (w.shot_sides.get("Shoot1"), w.shot_sides.get("Shoot2"));
            let side = if sides.0.is_some() && sides.1.is_some() && sides.0 != sides.1 { w.shot_sides.get(clip).cloned() } else { None };
            let xs = &loadout.flash_x;
            let pick = |left: bool| {
                xs.iter().enumerate().min_by(|a, b| if left { a.1.total_cmp(b.1) } else { b.1.total_cmp(a.1) }).map(|(i, _)| i)
            };
            let k = match side.as_deref() {
                Some("left") => pick(true),
                Some("right") => pick(false),
                _ => None,
            }
            .unwrap_or(loadout.barrel as usize);
            commands.entity(vm).insert(crate::particles::Barrel(k));
        }
        flashes.write(crate::particles::Burst(vm));
    }

    let (mesh, mat) = impact_assets
        .get_or_insert_with(|| {
            (
                meshes.add(Sphere::new(0.04)),
                materials.add(StandardMaterial { base_color: Color::srgb(0.05, 0.05, 0.05), unlit: true, ..default() }),
            )
        })
        .clone();
    let origin = camera.translation();
    if let Some(r) = &w.rocket {
        let aim = spread_dir(camera.rotation(), spread_deg * 0.3, time.elapsed_secs(), 0);
        // out of the launcher's barrel, towards what the crosshair is on
        let from = camera.transform_point(loadout.muzzle.unwrap_or(Vec3::new(0.0, -0.15, -0.8)));
        let world = SpatialQueryFilter::from_mask([Layer::World, Layer::Enemy]);
        let target = origin + *aim * spatial.cast_ray(origin, aim, 200.0, true, &world).map_or(200.0, |h| h.distance);
        let dir = Dir3::new(target - from).unwrap_or(aim);
        rockets.write(SpawnProjectile {
            owner: Owner::Player,
            from,
            dir,
            speed: r.speed(),
            life: if r.lifeTime > 0.0 { r.lifeTime } else { 5.0 },
            gravity: r.useGravity != 0,
            damage: base_damage,
            explosion_radius: w.def.bazookaExplosionRadius.max(1.0),
            self_impulse: (w.def.bazookaImpulseRadius, w.def.impulseForceSelf),
            self_damage_radius: Some(if r.dontDoSelfDamage != 0 { 0.0 } else { w.def.bazookaExplosionRadiusSelf }),
            look: Look::Model(r.model.clone()),
        });
        return;
    }
    // ray weapons draw their beam from the gun (a little right of and below the eye)
    let muzzle_local = loadout.muzzle.unwrap_or(Vec3::new(0.14, -0.12, -0.6));
    let muzzle = camera.transform_point(muzzle_local);
    if w.def.railgun != 0 {
        // the railgun's shot goes through walls and hits every enemy on its line, once each
        let dir = spread_dir(camera.rotation(), spread_deg, time.elapsed_secs(), 0);
        let enemies = SpatialQueryFilter::from_mask([Layer::Enemy]);
        // each bot once; through its head if the line crosses it
        let mut done: Vec<(Entity, bool)> = vec![];
        for hit in spatial.ray_hits(origin, dir, range, 64, true, &enemies) {
            if let Ok((part, head)) = bot_parts.get(hit.entity) {
                match done.iter_mut().find(|(b, _)| *b == part.0) {
                    Some(d) => d.1 |= head,
                    None => done.push((part.0, head)),
                }
            }
        }
        for (bot, head) in done {
            let amount = base_damage * if head { w.head_coef } else { 1.0 };
            hits.write(BotDamage { bot, amount, slow: None });
        }
        beams.write(crate::beams::SpawnBeam { weapon: w.name.clone(), from: muzzle, to: origin + *dir * 150.0, life: None, follow: None });
        return;
    }
    let filter = SpatialQueryFilter::from_mask([Layer::World, Layer::Enemy]);
    let slow = (w.def.isSlowdown != 0).then_some((w.def.slowdownCoeff.clamp(0.05, 1.0), w.def.slowdownTime.max(0.1)));
    if w.def.freezer != 0 {
        // a ray to the first thing in the way, redrawn each shot while firing
        let dir = spread_dir(camera.rotation(), spread_deg, time.elapsed_secs(), 0);
        let end = spatial.cast_ray(origin, dir, range, true, &filter).map_or(range.min(120.0), |h| h.distance);
        beams.write(crate::beams::SpawnBeam {
            weapon: w.name.clone(),
            from: muzzle,
            to: origin + *dir * end,
            life: Some(interval * 1.6),
            follow: Some((muzzle_local, end)),
        });
    }
    for i in 0..pellets {
        let dir = spread_dir(camera.rotation(), spread_deg, time.elapsed_secs(), i);
        let hit = spatial.cast_ray(origin, dir, range, true, &filter);
        if std::env::var("PG3D_TRACE_SHOTS").is_ok() {
            // a miss that passed within 0.5 m of a bot part's centre before stopping
            let reach = hit.map_or(range, |h| h.distance);
            let near = part_globals.iter().filter(|(_, l, off)| !off && l.memberships.has_all(Layer::Enemy)).filter_map(|(g, _, _)| {
                let c = g.translation() - origin;
                let along = c.dot(*dir);
                (along > 0.0 && along < reach).then(|| (c - *dir * along).length())
            }).fold(f32::MAX, f32::min);
            debug!("shot {}: {} near {:.2}", w.name, match hit { Some(h) if bot_parts.contains(h.entity) => "bot", Some(_) => "world", None => "nothing" }, near);
        }
        let Some(hit) = hit else { continue };
        if let Ok((part, head)) = bot_parts.get(hit.entity) {
            // random variation of +-damageRange percent, like the original
            let r = (time.elapsed_secs() * 91.7 + i as f32 * 3.3).sin() * 0.5 + 0.5;
            let mut amount = base_damage * (1.0 + (dmin + (dmax - dmin) * r) / 100.0);
            if head {
                amount *= w.head_coef;
                debug!("headshot: {} x{}", w.name, w.head_coef);
                // Player_move_c.headShotSound
                crate::sound::play(&mut commands, game_sounds.as_ref().and_then(|g| g.get("player/headShotSound", 0)), None, 0.8);
                marks.write(crate::feel::HitMark::Head);
            }
            hits.write(BotDamage { bot: part.0, amount, slow });
        } else {
            let p = origin + dir * hit.distance + hit.normal * 0.01;
            if i < 3 {
                impacts.write(crate::feel::ShotImpact { point: p, normal: hit.normal });
            }
            commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone()), Transform::from_translation(p), Impact(8.0)));
        }
    }
}

/// Random direction inside a cone of `deg` degrees around the camera's forward.
/// Coming back to life: every weapon full again (clip and reserve), no reload under way.
fn refill_on_respawn(stats: Res<crate::bots::PlayerStats>, mut loadout: ResMut<Loadout>, mut was_dead: Local<bool>) {
    let dead = stats.health <= 0.0;
    if *was_dead && !dead {
        for w in &mut loadout.weapons {
            w.clip = if w.def.isMelee != 0 { 0 } else { w.def.ammoInClip };
            w.reserve = (w.def.maxAmmo - w.clip).max(0);
            w.spread = 0.0;
        }
        loadout.reloading = 0.0;
        loadout.cooldown = 0.0;
        info!("respawn: weapons refilled");
    }
    *was_dead = dead;
}

/// The weapon's own crosshair (WeaponCustomCrosshair._dataId -> CrosshairsSo entry,
/// export_crosshairs.py): atlas sprites for the centre, the top / bottom / left arms (the
/// right one is the left mirrored) and corners, each at its offset from the centre; the arms
/// move out with the gun's current scatter (AimCrosshairController). Sizes are NGUI pixels
/// of a 768 high screen.
#[derive(Deserialize, Default)]
struct CrosshairFile {
    crosshairs: HashMap<String, HashMap<String, CrosshairPart>>,
    weapons: HashMap<String, i32>,
}

#[derive(Deserialize, Clone)]
struct CrosshairPart {
    sprite: String,
    offset: [f32; 2],
    size: [f32; 2],
}

/// The screen-centre point the crosshair's sprites hang off.
#[derive(Component)]
struct CrosshairCenter;

/// One sprite of the crosshair: its part ("up", "left", ...), mirrored copy or not.
#[derive(Component)]
struct CrosshairSprite {
    part: String,
    mirrored: bool,
    offset: Vec2,
    size: Vec2,
}

#[allow(clippy::too_many_arguments)]
fn crosshair_sprites(
    mut commands: Commands,
    loadout: Res<Loadout>,
    data: Res<DataDir>,
    assets: Res<AssetServer>,
    cross: Query<(Entity, Option<&Children>), With<CrosshairCenter>>,
    mut parts: Query<(&CrosshairSprite, &mut Node)>,
    window: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut file: Local<Option<CrosshairFile>>,
    mut shown: Local<Option<String>>,
) {
    let Some(w) = loadout.current() else { return };
    let defs = file.get_or_insert_with(|| {
        std::fs::read_to_string(data.0.join("ui/crosshairs/crosshairs.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    });
    let scale = window.single().map_or(1.0, |w| w.height() / 768.0);
    let Ok((root, kids)) = cross.single() else { return };
    if shown.as_deref() != Some(w.name.as_str()) {
        *shown = Some(w.name.clone());
        if let Some(k) = kids {
            for c in k.iter() {
                commands.entity(c).despawn();
            }
        }
        let id = defs.weapons.get(&w.name).copied().unwrap_or(1).to_string();
        let Some(set) = defs.crosshairs.get(&id).or_else(|| defs.crosshairs.get("1")) else { return };
        commands.entity(root).with_children(|c| {
            for (part, p) in set {
                let mirrors: &[bool] = if matches!(part.as_str(), "left" | "leftUp" | "leftDown") { &[false, true] } else { &[false] };
                for &m in mirrors {
                    c.spawn((
                        CrosshairSprite { part: part.clone(), mirrored: m, offset: Vec2::from(p.offset), size: Vec2::from(p.size) },
                        ImageNode { image: assets.load(format!("ui/crosshairs/{}.png", p.sprite)), flip_x: m, flip_y: part == "down" || part == "leftDown", ..default() },
                        Node { position_type: PositionType::Absolute, ..default() },
                    ));
                }
            }
        });
    }
    // arms out by 6 NGUI px per unit of scatter beyond the weapon's resting tekKoof
    let push = (w.spread * 6.0).min(30.0);
    for (sp, mut node) in &mut parts {
        let o = sp.offset + if sp.part == "center" { Vec2::ZERO } else { Vec2::splat(push) };
        // centre of the sprite from the screen centre, NGUI y up
        let (dx, dy) = match sp.part.as_str() {
            "up" => (0.0, o.y + sp.size.y / 2.0),
            "down" => (0.0, -(o.y + sp.size.y / 2.0)),
            "left" => (-(o.x + sp.size.x / 2.0), 0.0),
            "leftUp" => (-(o.x + sp.size.x / 2.0), o.y + sp.size.y / 2.0),
            "leftDown" => (-(o.x + sp.size.x / 2.0), -(o.y + sp.size.y / 2.0)),
            _ => (0.0, 0.0),
        };
        let dx = if sp.mirrored { -dx } else { dx };
        let (cx, cy) = (dx * scale, -dy * scale);
        let (w, h) = (sp.size.x * scale, sp.size.y * scale);
        if std::env::var("PG3D_TRACE_FX").is_ok() {
            debug!("crosshair {} {}: centre {cx:.0},{cy:.0} size {w:.0}x{h:.0}", sp.part, sp.mirrored);
        }
        node.left = px(cx - w / 2.0);
        node.top = px(cy - h / 2.0);
        node.width = px(w);
        node.height = px(h);
    }
}

/// Most scatter a weapon builds up (weapon balance _maxCoef default).
const MAX_SCATTER: f32 = 4.0;

fn spread_dir(rot: Quat, deg: f32, t: f32, i: u32) -> Dir3 {
    // cheap deterministic noise; good enough for spread
    let h = |x: f32| (x.sin() * 43758.547).fract();
    let a = h(t * 12.9898 + i as f32 * 78.233) * std::f32::consts::TAU;
    let r = h(t * 4.1414 + i as f32 * 12.345).sqrt() * deg.to_radians();
    let local = Vec3::new(r.sin() * a.cos(), r.sin() * a.sin(), -r.cos());
    Dir3::new(rot * local).unwrap_or(Dir3::NEG_Z)
}

fn reload(mut commands: Commands, input: crate::controls::Input, time: Res<Time>, mut loadout: ResMut<Loadout>) {
    let loadout = &mut *loadout;
    let cur = loadout.current;
    if loadout.reloading > 0.0 {
        loadout.reloading -= time.delta_secs();
        if loadout.reloading <= 0.0 {
            loadout.reloading = 0.0;
            if let Some(w) = loadout.weapons.get_mut(cur) {
                let take = (w.def.ammoInClip - w.clip).min(w.reserve);
                w.clip += take;
                w.reserve -= take;
            }
        }
        return;
    }
    let Some(w) = loadout.weapons.get(cur) else { return };
    if input.just_pressed(Action::Reload) && w.def.isMelee == 0 && w.clip < w.def.ammoInClip && w.reserve > 0 {
        loadout.reloading = w.reload_time;
        loadout.play = Some("Reload");
        crate::sound::play(&mut commands, w.sounds.get("reload"), None, 0.8);
    }
}

/// Drives the original weapon animations: one-shot Shoot/Reload when requested, Walk
/// while moving on the ground, Idle otherwise. Weapons without clips get a small
/// procedural kick instead.
const RAISE_FROM: f32 = 40.0;
const RAISE_SPEED: f32 = 200.0;

fn animate_view_model(
    mut loadout: ResMut<Loadout>,
    player: Single<&Player>,
    mut roots: Query<(Entity, &ViewModelAnims, &mut Transform), With<ViewModel>>,
    mut players: Query<&mut AnimationPlayer>,
    time: Res<Time>,
) {
    let request = loadout.play.take();
    loadout.lower = (loadout.lower - RAISE_SPEED * time.delta_secs()).max(0.0);
    let Some(w) = loadout.current() else { return };
    let base = view_model_transform(w.def.gunPosition.to_bevy());
    // a weapon being drawn turns up into view about the camera
    let dip = Quat::from_rotation_x(-loadout.lower.to_radians());
    let base = Transform { translation: dip * base.translation, rotation: dip * base.rotation, ..base };
    let reload_speed = w.reload_speed;
    let double = w.def.isDoubleShot != 0;
    let barrel = loadout.barrel;
    // a clip asked for before the new model has loaded is kept for when it has
    let mut waiting = request.is_some();
    let current_model = loadout.view_model;
    let moving = player.grounded && Vec2::new(player.velocity.x, player.velocity.z).length() > 0.5;
    for (e, anims, mut t) in &mut roots {
        // (the model just switched away from is still here this frame)
        if current_model != Some(e) {
            continue;
        }
        let animated = anims.player.and_then(|p| players.get_mut(p).ok());
        if animated.is_some() {
            waiting = false;
        }
        let Some(mut ap) = animated else {
            // no clips (yet): procedural kick / reload dip
            let dip = if loadout.reloading > 0.0 { 0.15 } else { 0.0 };
            t.translation = base.translation + Vec3::new(0.0, -dip, 0.06 * loadout.kick);
            t.rotation = base.rotation * Quat::from_rotation_x(-0.08 * loadout.kick);
            continue;
        };
        *t = base;
        let node = |n: &str| anims.nodes.get(n).copied();
        if let Some(mut name) = request {
            // Shoot1 / Shoot2 in turn for two-barrelled weapons (dual pistols, shotguns)
            if name == "Shoot" && node("Shoot1").is_some() && (double || node("Shoot").is_none()) {
                name = if barrel && node("Shoot2").is_some() { "Shoot2" } else { "Shoot1" };
            }
            if let Some(n) = node(name) {
                ap.stop_all();
                // the Shoot clip's length is the fire interval; Reload plays at the weapon's
                // balance reload speed (so it lasts reload_time)
                let speed = if name == "Reload" { reload_speed } else { 1.0 };
                ap.play(n).replay();
                if let Some(a) = ap.animation_mut(n) {
                    a.set_speed(speed);
                }
                continue;
            }
        }
        let busy = ["Shoot", "Shoot1", "Shoot2", "Reload", "Loading"].iter().filter_map(|n| node(n)).any(|n| {
            ap.animation(n).is_some_and(|a| !a.is_finished())
        });
        if busy {
            continue;
        }
        let want = if moving { node("Walk").or(node("Idle")) } else { node("Idle") };
        if let Some(n) = want {
            if !ap.is_playing_animation(n) || ap.animation(n).is_some_and(|a| a.is_finished()) {
                ap.stop_all();
                ap.play(n).repeat();
            }
        }
    }
    if waiting {
        loadout.play = request;
    }
}

/// The barrel of the gun in your hands: its BulletSpawnPoint (or GunFlash). The gun is drawn
/// by its own camera with a 60 degree view; the world camera's is wider, so the point is
/// moved out to where it shows on screen at the same depth.
fn track_muzzle(
    mut loadout: ResMut<Loadout>,
    main: Single<(&GlobalTransform, &Projection), With<PlayerCamera>>,
    gun_cam: Query<&Projection, With<ViewModelCamera>>,
    children: Query<&Children>,
    names: Query<(&Name, &GlobalTransform)>,
) {
    let Some(vm) = loadout.view_model else { return };
    let find = |want: &str| children.iter_descendants(vm).find_map(|d| names.get(d).ok().filter(|(n, _)| n.as_str() == want).map(|(_, g)| g.translation()));
    let flashes: Vec<f32> = children
        .iter_descendants(vm)
        .filter_map(|d| names.get(d).ok().filter(|(n, _)| n.as_str() == "GunFlash").map(|(_, g)| g.translation()))
        .map(|p| main.0.affine().inverse().transform_point3(p).x)
        .collect();
    if std::env::var("PG3D_TRACE_FX").is_ok() && flashes.len() > 1 {
        debug!("gun flashes at x {flashes:.2?}");
    }
    loadout.flash_x = flashes;
    let Some(p) = find("BulletSpawnPoint").or_else(|| find("GunFlash")) else {
        loadout.muzzle = None;
        return;
    };
    let (cam, proj) = *main;
    let local = cam.affine().inverse().transform_point3(p);
    let fov = |p: &Projection| match p {
        Projection::Perspective(p) => Some(p.fov),
        _ => None,
    };
    let k = match (fov(proj), gun_cam.single().ok().and_then(fov)) {
        (Some(world), Some(gun)) => (world / 2.0).tan() / (gun / 2.0).tan(),
        _ => 1.0,
    };
    loadout.muzzle = Some(Vec3::new(local.x * k, local.y * k, local.z));
    if std::env::var("PG3D_TRACE_FX").is_ok() {
        debug!("muzzle: local {local:.2}, k {k:.2}");
    }
}

fn fade_impacts(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut Impact)>) {
    for (e, mut imp) in &mut q {
        imp.0 -= time.delta_secs();
        if imp.0 <= 0.0 {
            commands.entity(e).despawn();
        }
    }
}

fn spawn_hud(mut commands: Commands) {
    // crosshair
    commands.spawn((
        Crosshair,
        Node { position_type: PositionType::Absolute, width: percent(100.0), height: percent(100.0), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() },
        // a zero-size point at the screen centre; crosshair_sprites hangs the parts off it
        children![(CrosshairCenter, Node { width: px(0.0), height: px(0.0), ..default() })],
    ));
}

/// Right mouse on a scoped weapon (isZooming): narrow the field of view to the weapon's
/// fieldOfViewZomm (degrees, vertical like ours) and hide the gun model.
#[allow(clippy::type_complexity)]
fn zoom(
    input: crate::controls::Input,
    mut commands: Commands,
    mut loadout: ResMut<Loadout>,
    mut cam: Single<&mut Projection, (With<PlayerCamera>, Without<ViewModel>)>,
    mut view_models: Query<&mut Visibility, With<ViewModel>>,
    mut zoomed: Local<bool>,
    time: Res<Time>,
    settings: Res<crate::settings::Settings>,
    (stats, paused): (Option<Res<crate::bots::PlayerStats>>, Option<Res<crate::pause::Paused>>),
) {
    let Some(w) = loadout.current() else { return };
    // the one place that shows or hides the gun in your hands: hidden while scoped, dead
    // (the kill cam) or paused
    let gone = stats.is_some_and(|s| s.health <= 0.0) || paused.is_some_and(|p| p.0);
    let want = w.def.isZooming != 0 && input.pressed(Action::Aim) && loadout.reloading <= 0.0;
    let target = if want { w.def.fieldOfViewZomm } else { settings.fov };
    if want != *zoomed {
        let key = if want { "zoomIn" } else { "zoomOut" };
        crate::sound::play(&mut commands, w.sounds.get(key), None, 0.7);
        *zoomed = want;
    }
    if let Projection::Perspective(p) = &mut **cam {
        let cur = p.fov.to_degrees();
        p.fov = (cur + (target - cur) * (time.delta_secs() * 15.0).min(1.0)).to_radians();
    }
    let show = if want || gone { Visibility::Hidden } else { Visibility::Inherited };
    for mut v in &mut view_models {
        if *v != show {
            *v = show;
        }
    }
    // scoped shots are accurate: no spread while zoomed (sniper behaviour)
    let cur = loadout.current;
    if want {
        if let Some(w) = loadout.weapons.get_mut(cur) {
            w.spread = 0.0;
        }
    }
}
