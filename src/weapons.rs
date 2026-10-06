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
};
use serde::Deserialize;

use crate::{DataDir, camera::PlayerCamera, map::Layer};

/// Weapons given to the player, in slot order (keys 1..6). Only ones that were exported load.
const LOADOUT: [&str; 6] = ["Weapon2", "Weapon1", "Weapon9", "Weapon3", "Weapon5", "Weapon11"];
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
                    update_hud,
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
    /// offset of the weapon prefab from the camera (Unity coordinates)
    pub gunPosition: UnityVec3,
}

#[derive(Deserialize, Clone, Copy, Debug, Default)]
pub struct UnityVec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
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
}

#[derive(Resource, Default)]
pub struct Loadout {
    pub weapons: Vec<Weapon>,
    pub current: usize,
    cooldown: f32,
    reloading: f32,
    kick: f32,
    view_model: Option<Entity>,
}

impl Loadout {
    pub fn current(&self) -> Option<&Weapon> {
        self.weapons.get(self.current)
    }
}

/// Root of the first-person weapon model.
#[derive(Component)]
struct ViewModel;

/// Bullet hole marker, fades out.
#[derive(Component)]
struct Impact(f32);

#[derive(Component)]
struct HudText;

fn load_loadout(mut commands: Commands, data: Res<DataDir>) {
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
    for name in LOADOUT {
        let Some(def) = all.get(name) else { continue };
        if !dir.join(name).join("weapon.gltf").exists() {
            continue;
        }
        let info: WeaponInfo = std::fs::read_to_string(dir.join(name).join("info.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let anim = |k: &str, default: f32| info.animations.get(k).copied().filter(|v| *v > 0.01).unwrap_or(default);
        let clip = if def.isMelee != 0 { 0 } else { def.ammoInClip };
        loadout.weapons.push(Weapon {
            name: name.to_string(),
            def: def.clone(),
            fire_interval: anim("Shoot", 0.3),
            reload_time: anim("Reload", 1.5),
            clip,
            reserve: (def.maxAmmo - clip).max(0),
            spread: 0.0,
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
        let e = spawn_view_model(&mut commands, *main, &assets, w);
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

fn spawn_view_model(commands: &mut Commands, camera: Entity, assets: &AssetServer, w: &Weapon) -> Entity {
    let name = &w.name;
    let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(format!("weapons/{name}/weapon.gltf")));
    let e = commands
        .spawn((
            Name::new(format!("view model {name}")),
            ViewModel,
            WorldAssetRoot(scene),
            view_model_transform(w.def.gunPosition.to_bevy()),
            RenderLayers::layer(VIEW_MODEL_LAYER),
        ))
        .id();
    commands.entity(camera).add_child(e);
    e
}

/// glTF meshes are spawned later as children; move them onto the view model layer and
/// make them unlit (the original draws the first-person weapon with its own simple
/// lighting, so it is never pitch black on night maps).
fn tag_view_model_meshes(
    mut commands: Commands,
    added: Query<(Entity, Option<&MeshMaterial3d<StandardMaterial>>), (Added<Mesh3d>, Without<RenderLayers>)>,
    parents: Query<&ChildOf>,
    roots: Query<(), With<ViewModel>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (e, mat) in &added {
        if !parents.iter_ancestors(e).any(|a| roots.contains(a)) {
            continue;
        }
        let mut ec = commands.entity(e);
        ec.insert(RenderLayers::layer(VIEW_MODEL_LAYER));
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
) {
    let n = loadout.weapons.len();
    if n == 0 {
        return;
    }
    let digits = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6];
    let mut next = loadout.current;
    for (i, k) in digits.iter().enumerate() {
        if keys.just_pressed(*k) && i < n {
            next = i;
        }
    }
    if scroll.delta.y > 0.0 {
        next = (loadout.current + n - 1) % n;
    } else if scroll.delta.y < 0.0 {
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
    let e = spawn_view_model(&mut commands, *camera, &assets, &loadout.weapons[next]);
    loadout.view_model = Some(e);
}

#[allow(clippy::too_many_arguments)]
fn fire(
    mut commands: Commands,
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: Single<&CursorOptions>,
    mut loadout: ResMut<Loadout>,
    camera: Single<&GlobalTransform, With<PlayerCamera>>,
    spatial: SpatialQuery,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut impact_assets: Local<Option<(Handle<Mesh>, Handle<StandardMaterial>)>>,
) {
    let dt = time.delta_secs();
    // reborrow as a plain struct so we can use different fields at the same time
    let loadout = &mut *loadout;
    loadout.cooldown = (loadout.cooldown - dt).max(0.0);
    loadout.kick = (loadout.kick - dt * 6.0).max(0.0);
    let cur = loadout.current;
    let Some(w) = loadout.weapons.get_mut(cur) else { return };
    w.spread = (w.spread - w.def.downKoof * dt).max(0.0);

    let wants = mouse.pressed(MouseButton::Left) && cursor.grab_mode != CursorGrabMode::None;
    if !wants || loadout.cooldown > 0.0 || loadout.reloading > 0.0 {
        return;
    }
    let melee = w.def.isMelee != 0;
    if !melee && w.clip <= 0 {
        if w.reserve > 0 {
            loadout.reloading = w.reload_time;
        }
        return;
    }
    if !melee {
        w.clip -= 1;
    }
    let pellets = if w.def.isShotGun != 0 { 8 } else { 1 }; // GUESS: pellet count not found yet
    let range = if melee { w.def.radiusRoundMelee.max(2.5) } else { 300.0 };
    // spread in degrees: base + accumulated recoil (GUESS: unit of tekKoof/upKoofFire)
    let spread_deg = w.def.tekKoof + w.spread + if pellets > 1 { 4.0 } else { 0.0 };
    w.spread += w.def.upKoofFire;
    let interval = w.fire_interval;
    loadout.cooldown = interval;
    loadout.kick = 1.0;

    let (mesh, mat) = impact_assets
        .get_or_insert_with(|| {
            (
                meshes.add(Sphere::new(0.04)),
                materials.add(StandardMaterial { base_color: Color::srgb(0.05, 0.05, 0.05), unlit: true, ..default() }),
            )
        })
        .clone();
    let origin = camera.translation();
    let filter = SpatialQueryFilter::from_mask(Layer::World);
    for i in 0..pellets {
        let dir = spread_dir(camera.rotation(), spread_deg, time.elapsed_secs(), i);
        if let Some(hit) = spatial.cast_ray(origin, dir, range, true, &filter) {
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

fn reload(keys: Res<ButtonInput<KeyCode>>, time: Res<Time>, mut loadout: ResMut<Loadout>) {
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
    if keys.just_pressed(KeyCode::KeyR) && w.def.isMelee == 0 && w.clip < w.def.ammoInClip && w.reserve > 0 {
        loadout.reloading = w.reload_time;
    }
}

/// Simple procedural motion until the original animations are imported:
/// kick back when firing, dip down while reloading.
fn animate_view_model(loadout: Res<Loadout>, mut q: Query<&mut Transform, With<ViewModel>>) {
    let Some(w) = loadout.current() else { return };
    let base = view_model_transform(w.def.gunPosition.to_bevy());
    let reload_dip = if loadout.reloading > 0.0 { 0.15 } else { 0.0 };
    for mut t in &mut q {
        t.translation = base.translation + Vec3::new(0.0, -reload_dip, 0.06 * loadout.kick);
        t.rotation = base.rotation * Quat::from_rotation_x(-0.08 * loadout.kick);
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
    // weapon + ammo, bottom right
    commands.spawn((
        Node { position_type: PositionType::Absolute, right: px(24.0), bottom: px(20.0), ..default() },
        children![(HudText, Text::new(""), TextFont { font_size: FontSize::Px(26.0), ..default() }, TextColor(Color::WHITE))],
    ));
}

fn update_hud(loadout: Res<Loadout>, mut text: Single<&mut Text, With<HudText>>) {
    let Some(w) = loadout.current() else { return };
    let ammo = if w.def.isMelee != 0 { "melee".to_string() } else { format!("{} / {}", w.clip, w.reserve) };
    let state = if loadout.reloading > 0.0 { "  reloading…" } else { "" };
    text.0 = format!("[{}] {}   {}{}", loadout.current + 1, w.name, ammo, state);
}
