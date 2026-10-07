//! Weapons: loadout, first-person view model, shooting, reloading, HUD.
//!
//! Data comes from <data>/weapons/ written by ../pixel-gun-3d-re/scripts/export_weapons.py:
//!   weapons.json          settings of every WeaponSounds prefab in the game
//!   <Weapon>/weapon.gltf  first-person model (arms + gun, static pose for now)
//!   <Weapon>/info.json    animation clip lengths (Shoot = fire interval, Reload = reload time)
//!
//! Controls: left mouse = fire (hold), R = reload, 1-6 or mouse wheel = switch weapon.

use std::collections::HashMap;

use avian3d::prelude::*;
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
    bots::{BotDamage, BotPart},
    camera::PlayerCamera,
    map::Layer,
    player::Player,
    projectiles::{Look, Owner, SpawnProjectile},
};

/// Weapons given to the player, in slot order (keys 1..6). Only ones that were exported load.
/// One per original category (1 primary, 2 backup, 3 melee, 4 special, 5 sniper,
/// 6 heavy), then extras. PG3D_LOADOUT=Weapon2,Weapon61,... overrides it.
const LOADOUT: [&str; 9] = [
    "Weapon3", "Weapon1", "Weapon9", "Weapon5", "Weapon61", "Weapon50", "Weapon2", "Weapon11", "Weapon75",
];
/// Render layer used for the first-person weapon so it never clips into walls.
const VIEW_MODEL_LAYER: usize = 1;

pub struct WeaponsPlugin;

impl Plugin for WeaponsPlugin {
    fn build(&self, app: &mut App) {
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
    pub railgun: i32,
    pub isZooming: i32,
    pub fieldOfViewZomm: f32,
    /// movement speed multiplier while holding this weapon
    pub speedModifier: f32,
    /// spread: current ("tek"), added per shot ("up"), recovery per second ("down")
    pub tekKoof: f32,
    pub upKoofFire: f32,
    pub downKoof: f32,
    pub radiusRoundMelee: f32,
    pub shootDistanceIfZoom: f32,
    /// pellets per shot for shotguns
    pub countShots: i32,
    /// rocket launchers: which "Rocket_<n>" prefab they fire and its blast radius
    pub rocketNum: i32,
    pub bazookaExplosionRadius: f32,
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
    /// original sound field name -> wav file (shoot, reload, empty, ...)
    #[serde(default)]
    sounds: HashMap<String, String>,
}

pub struct Weapon {
    pub name: String,
    pub def: WeaponDef,
    /// seconds between shots (length of the original "Shoot" animation)
    pub fire_interval: f32,
    /// seconds (length of the original "Reload" animation)
    pub reload_time: f32,
    pub clip: i32,
    pub reserve: i32,
    pub spread: f32,
    /// animation clips in weapon.gltf (Idle, Walk, Shoot, Reload, ...)
    pub clips: Vec<String>,
    /// rocket launchers: the projectile they fire
    pub rocket: Option<RocketDef>,
    pub sounds: HashMap<String, Handle<AudioSource>>,
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
    let mut loadout = Loadout::default();
    let custom = std::env::var("PG3D_LOADOUT").ok();
    let names: Vec<&str> = match &custom {
        Some(c) => c.split(',').map(str::trim).collect(),
        None => LOADOUT.to_vec(),
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
            fire_interval: anim("Shoot", 0.3),
            reload_time: anim("Reload", 1.5),
            clip,
            reserve: (def.maxAmmo - clip).max(0),
            spread: 0.0,
            clips: info.clips.clone(),
            rocket: (def.bazooka != 0).then(|| rocket_def(&data.0, def.rocketNum)).flatten(),
            sounds: crate::sound::load_sounds(&assets, &format!("weapons/{name}"), &info.sounds),
        });
    }
    info!(
        "loadout: {}",
        loadout.weapons.iter().map(|w| format!("{} ({:.2}s)", w.name, w.fire_interval)).collect::<Vec<_>>().join(", ")
    );
    commands.insert_resource(loadout);
}

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
    scroll: Res<AccumulatedMouseScroll>,
    mut loadout: ResMut<Loadout>,
    camera: Single<Entity, With<PlayerCamera>>,
    assets: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    time: Res<Time>,
) {
    let n = loadout.weapons.len();
    if n == 0 {
        return;
    }
    // PG3D_SOAK (automated stability test): switch to the next weapon every 2.5 s
    let soak = std::env::var("PG3D_SOAK").is_ok() && (time.elapsed_secs() % 2.5) < time.delta_secs();
    let digits = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ];
    let mut next = loadout.current;
    for (i, k) in digits.iter().enumerate() {
        if keys.just_pressed(*k) && i < n {
            next = i;
        }
    }
    if scroll.delta.y > 0.0 {
        next = (loadout.current + n - 1) % n;
    } else if scroll.delta.y < 0.0 || soak {
        next = (loadout.current + 1) % n;
    }
    if next == loadout.current {
        return;
    }
    loadout.current = next;
    loadout.cooldown = 0.3; // draw time (GUESS)
    loadout.reloading = 0.0;
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
    bot_parts: Query<&BotPart>,
    mut hits: MessageWriter<BotDamage>,
    mut rockets: MessageWriter<SpawnProjectile>,
    mut flashes: MessageWriter<crate::particles::Burst>,
    alive: Res<crate::bots::PlayerStats>,
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
    let range = if melee { w.def.radiusRoundMelee.max(2.5) } else { 300.0 };
    // spread in degrees: base + accumulated recoil (GUESS: unit of tekKoof/upKoofFire)
    let spread_deg = w.def.tekKoof + w.spread + if pellets > 1 { 4.0 } else { 0.0 };
    w.spread += w.def.upKoofFire;
    let interval = w.fire_interval;
    loadout.cooldown = interval;
    loadout.kick = 1.0;
    loadout.play = Some("Shoot");
    if let Some(vm) = loadout.view_model {
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
        let dir = spread_dir(camera.rotation(), spread_deg * 0.3, time.elapsed_secs(), 0);
        rockets.write(SpawnProjectile {
            owner: Owner::Player,
            // start a little in front and below the eye, like from the launcher
            from: origin + *dir * 0.8 - camera.up() * 0.15,
            dir,
            speed: r.speed(),
            life: if r.lifeTime > 0.0 { r.lifeTime } else { 5.0 },
            gravity: r.useGravity != 0,
            damage: base_damage,
            explosion_radius: w.def.bazookaExplosionRadius.max(1.0),
            look: Look::Model(r.model.clone()),
        });
        return;
    }
    let filter = SpatialQueryFilter::from_mask([Layer::World, Layer::Enemy]);
    for i in 0..pellets {
        let dir = spread_dir(camera.rotation(), spread_deg, time.elapsed_secs(), i);
        let Some(hit) = spatial.cast_ray(origin, dir, range, true, &filter) else { continue };
        if let Ok(part) = bot_parts.get(hit.entity) {
            // random variation of +-damageRange percent, like the original
            let r = (time.elapsed_secs() * 91.7 + i as f32 * 3.3).sin() * 0.5 + 0.5;
            let amount = base_damage * (1.0 + (dmin + (dmax - dmin) * r) / 100.0);
            hits.write(BotDamage { bot: part.0, amount });
        } else {
            let p = origin + dir * hit.distance + hit.normal * 0.01;
            commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone()), Transform::from_translation(p), Impact(8.0)));
        }
    }
}

/// Random direction inside a cone of `deg` degrees around the camera's forward.
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
fn animate_view_model(
    mut loadout: ResMut<Loadout>,
    player: Single<&Player>,
    mut roots: Query<(&ViewModelAnims, &mut Transform), With<ViewModel>>,
    mut players: Query<&mut AnimationPlayer>,
) {
    let request = loadout.play.take();
    let Some(w) = loadout.current() else { return };
    let base = view_model_transform(w.def.gunPosition.to_bevy());
    let moving = player.grounded && Vec2::new(player.velocity.x, player.velocity.z).length() > 0.5;
    for (anims, mut t) in &mut roots {
        let animated = anims.player.and_then(|p| players.get_mut(p).ok());
        let Some(mut ap) = animated else {
            // no clips (yet): procedural kick / reload dip
            let dip = if loadout.reloading > 0.0 { 0.15 } else { 0.0 };
            t.translation = base.translation + Vec3::new(0.0, -dip, 0.06 * loadout.kick);
            t.rotation = base.rotation * Quat::from_rotation_x(-0.08 * loadout.kick);
            continue;
        };
        *t = base;
        let node = |n: &str| anims.nodes.get(n).copied();
        if let Some(name) = request {
            if let Some(n) = node(name) {
                ap.stop_all();
                // clip lengths already equal the gameplay timings (fire interval, reload time)
                ap.play(n).replay();
                continue;
            }
        }
        let busy = ["Shoot", "Reload"].iter().filter_map(|n| node(n)).any(|n| {
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
        Node { position_type: PositionType::Absolute, width: percent(100.0), height: percent(100.0), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() },
        children![(Text::new("+"), TextFont { font_size: FontSize::Px(28.0), ..default() }, TextColor(Color::WHITE))],
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
) {
    let Some(w) = loadout.current() else { return };
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
    for mut v in &mut view_models {
        *v = if want { Visibility::Hidden } else { Visibility::Inherited };
    }
    // scoped shots are accurate: no spread while zoomed (sniper behaviour)
    let cur = loadout.current;
    if want {
        if let Some(w) = loadout.weapons.get_mut(cur) {
            w.spread = 0.0;
        }
    }
}
