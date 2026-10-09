//! Loads an exported map: visible geometry (map.gltf), collision (collision.gltf +
//! colliders.json) and markers such as spawn zones (info.json).

use avian3d::prelude::*;
#[allow(unused_imports)]
use crate::scene::{FixedUpdate, PostStartup, PostUpdate, PreStartup, Startup, Update};
use bevy::{asset::RenderAssetUsages, gltf::GltfLoaderSettings, prelude::*};
use serde::Deserialize;

use crate::DataDir;

pub struct MapPlugin;

/// Name of the map to load, e.g. "arena" -> <data>/maps/arena/
#[derive(Resource)]
pub struct CurrentMap(pub String);

/// Named points from the original level (spawn zones etc.), in Bevy world space.
#[derive(Resource, Default)]
pub struct MapMarkers(pub Vec<Marker>);

#[derive(Deserialize, Clone)]
pub struct Marker {
    pub name: String,
    pub position: Vec3,
}

/// Contents of info.json written by the exporter.
#[derive(Deserialize, Resource, Default, Clone)]
pub struct MapInfo {
    #[serde(default)]
    pub markers: Vec<Marker>,
    /// Lightmap PNGs are stored divided by this; multiply back via lightmap_exposure.
    #[serde(default = "one")]
    pub lightmap_scale: f32,
    #[serde(default)]
    pub render: RenderSettings,
    #[serde(default)]
    pub lights: Vec<LightDef>,
    /// a Skybox/Cubemap material (export_map.py export_cubemap)
    #[serde(default)]
    pub skybox: Option<SkyboxDef>,
}

/// The map's cubemap sky: six faces stacked in one PNG, and Unity's tint x 2 x exposure
/// (gamma space).
#[derive(Deserialize, Default, Clone)]
pub struct SkyboxDef {
    pub png: String,
    pub tint: [f32; 3],
    #[serde(default)]
    pub rotation: f32,
}

fn one() -> f32 {
    1.0
}

/// Unity RenderSettings of the scene (ambient light, fog). Colours are gamma-space.
#[derive(Deserialize, Default, Clone)]
pub struct RenderSettings {
    /// 0 skybox, 1 gradient, 3 flat colour
    #[serde(default)]
    pub ambient_mode: Option<i32>,
    pub ambient_sky: Option<[f32; 3]>,
    #[serde(default)]
    pub ambient_intensity: Option<f32>,
    #[serde(default)]
    pub fog: bool,
    pub fog_color: Option<[f32; 3]>,
    /// 1 = linear, 2 = exponential, 3 = exponential squared
    pub fog_mode: Option<u32>,
    pub fog_start: Option<f32>,
    pub fog_end: Option<f32>,
    pub fog_density: Option<f32>,
}

/// A Unity Light. type: 0 spot, 1 directional, 2 point.
#[derive(Deserialize, Clone)]
pub struct LightDef {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: u32,
    pub color: [f32; 3],
    pub intensity: f32,
    /// the way the light shines (glTF axes)
    pub direction: Vec3,
    #[serde(default)]
    pub position: Vec3,
    #[serde(default)]
    pub range: f32,
    /// spot cone, full angle in degrees
    #[serde(default)]
    pub spot_angle: Option<f32>,
    /// Unity LightmapBakeType: 1 mixed, 2 baked (lightmaps/probes only), 4 realtime
    #[serde(default)]
    pub baked: u32,
}

/// One collider from the original level (Unity BoxCollider, SphereCollider, ...).
#[derive(Deserialize)]
struct ColliderDef {
    name: String,
    kind: String,
    trigger: bool,
    /// object -> world matrix, column-major (same layout as glTF)
    world: [f32; 16],
    #[serde(default)]
    center: Vec3,
    size: Option<Vec3>,
    radius: Option<f32>,
    height: Option<f32>,
    /// 0 = X, 1 = Y, 2 = Z axis
    direction: Option<u32>,
    /// the GameObject's Unity tag ("Trampoline", ...)
    #[serde(default)]
    tag: Option<String>,
}

/// What a solid collider does to whoever touches it (the player's / bots' movement checks).
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Surface {
    /// tag "Trampoline": SkinName.OnControllerColliderHit throws you up along its normal
    Trampoline,
    /// a solid "DeadCollider" box (Matrix has them under the map): touching it kills
    Dead,
}

/// Physics layers: triggers live on their own layer so movement ignores them.
#[derive(PhysicsLayer, Default, Clone, Copy, Debug)]
pub enum Layer {
    #[default]
    World,
    Trigger,
    /// bots / enemies: hit by bullets, not part of the level
    Enemy,
    /// the local player's own capsule (bullets start inside it, so they must skip it)
    Player,
}

/// A trigger volume from the original level, e.g. "DeadCollider" (kill zone).
#[derive(Component)]
pub struct Trigger {
    pub name: String,
}

impl crate::scene::ScenePlugin for MapPlugin {
    fn build(&self, app: &mut crate::scene::SceneBuilder) {
        app.insert_resource(ClearColor(Color::srgb(0.45, 0.65, 0.9)))
            .init_resource::<MapMarkers>()
            .init_resource::<MapInfo>()
            .add_systems(PreStartup, load_markers)
            .add_systems(Startup, (spawn_map, spawn_colliders))
            .init_resource::<CollisionReady>()
            .add_systems(Update, (hide_collision_meshes, hide_campaign_teleports, collision_ready));
    }
}

pub fn map_dir(data: &DataDir, map: &CurrentMap) -> std::path::PathBuf {
    data.0.join("maps").join(&map.0)
}

/// The map's gravity (m/s^2, positive): SceneInfo.gravity (export_scene_info.py ->
/// maps/scenes.json), 9.81 when unknown.
pub fn scene_gravity(data: &std::path::Path, map: &str) -> f32 {
    std::fs::read_to_string(data.join("maps/scenes.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get(map.to_lowercase())?.get("gravity")?.as_f64())
        .map_or(crate::player::GRAVITY, |g| (-g as f32).abs().max(0.1))
}

fn load_markers(mut commands: Commands, data: Res<DataDir>, map: Res<CurrentMap>) {
    let g = scene_gravity(&data.0, &map.0);
    crate::player::set_gravity(g);
    if (g - crate::player::GRAVITY).abs() > 0.01 {
        info!("map gravity {g:.2} m/s^2");
    }
    let path = map_dir(&data, &map).join("info.json");
    match std::fs::read_to_string(&path).map(|s| serde_json::from_str::<MapInfo>(&s)) {
        Ok(Ok(info)) => {
            commands.insert_resource(MapMarkers(info.markers.clone()));
            commands.insert_resource(info);
        }
        Ok(Err(e)) => error!("{}: {e}", path.display()),
        Err(e) => error!("{}: {e} (did you export the map?)", path.display()),
    }
}

fn spawn_map(mut commands: Commands, assets: Res<AssetServer>, map: Res<CurrentMap>) {
    let path = format!("maps/{}/map.gltf", map.0);
    info!("loading {path}");
    commands.spawn((
        Name::new(format!("map {}", map.0)),
        WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(path))),
        // particle textures (explosion effects of map props) are relative to the map folder
        crate::particles::AssetDir(format!("maps/{}", map.0)),
    ));
}

fn spawn_colliders(
    mut commands: Commands,
    assets: Res<AssetServer>,
    data: Res<DataDir>,
    map: Res<CurrentMap>,
) {
    // Mesh colliders: an invisible copy of the collision meshes, turned into
    // triangle-mesh colliders once loaded.
    commands.spawn((
        Name::new("collision meshes"),
        WorldAssetRoot(
            assets
                .load_builder()
                // keep the vertex data on the CPU side so physics can read it
                .with_settings(|s: &mut GltfLoaderSettings| s.load_meshes = RenderAssetUsages::all())
                .load(GltfAssetLabel::Scene(0).from_asset(format!("maps/{}/collision.gltf", map.0))),
        ),
        ColliderConstructorHierarchy::new(ColliderConstructor::TrimeshFromMesh),
        RigidBody::Static,
        Visibility::Hidden,
        CollisionMeshes,
    ));

    // Simple shapes from colliders.json
    let path = map_dir(&data, &map).join("colliders.json");
    let defs: Vec<ColliderDef> = match std::fs::read_to_string(&path).map(|s| serde_json::from_str(&s)) {
        Ok(Ok(d)) => d,
        other => {
            error!("{}: {:?}", path.display(), other.err());
            return;
        }
    };
    let (mut solid, mut triggers) = (0, 0);
    for d in defs {
        let shape = match d.kind.as_str() {
            "box" => {
                let s = d.size.unwrap_or(Vec3::ONE);
                Collider::cuboid(s.x.abs(), s.y.abs(), s.z.abs())
            }
            "sphere" => Collider::sphere(d.radius.unwrap_or(0.5)),
            "capsule" => {
                let r = d.radius.unwrap_or(0.5);
                let len = (d.height.unwrap_or(2.0) - 2.0 * r).max(0.0);
                let c = Collider::capsule(r, len);
                // Unity capsules can point along X or Z too; rotate a child for that.
                let rot = match d.direction {
                    Some(0) => Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
                    Some(2) => Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
                    _ => Quat::IDENTITY,
                };
                spawn_shape(&mut commands, &d, c, rot);
                if d.trigger { triggers += 1 } else { solid += 1 }
                continue;
            }
            _ => continue, // mesh colliders come from collision.gltf
        };
        spawn_shape(&mut commands, &d, shape, Quat::IDENTITY);
        if d.trigger { triggers += 1 } else { solid += 1 }
    }
    info!("colliders: {solid} solid shapes, {triggers} triggers");
}

/// Whether the map's mesh colliders (collision.gltf, loaded in the background) are in the
/// physics scene: floors and walls from it don't exist before, so anything put "on the
/// floor" earlier ends up under it.
#[derive(Resource, Default)]
pub struct CollisionReady {
    frames: u32,
}

impl CollisionReady {
    pub fn ready(&self) -> bool {
        // a couple of frames for the physics step to take the new colliders in
        self.frames >= 3
    }
}

fn collision_ready(mut ready: ResMut<CollisionReady>, roots: Query<Has<ColliderConstructorHierarchy>, With<CollisionMeshes>>) {
    if roots.iter().all(|pending| !pending) && !roots.is_empty() {
        ready.frames = ready.frames.saturating_add(1);
    }
}

/// Root of the collision.gltf copy.
#[derive(Component)]
struct CollisionMeshes;

/// The root's Visibility::Hidden doesn't always reach the spawned glTF scene: in some runs
/// the collision meshes were drawn over the map (plain white walls and floors with black
/// shadows on Arena Castle, Mine, the ice rink). Hide every collision mesh itself.
fn hide_collision_meshes(
    mut commands: Commands,
    added: Query<Entity, Added<Mesh3d>>,
    parents: Query<&ChildOf>,
    roots: Query<(), With<CollisionMeshes>>,
) {
    for e in &added {
        if parents.iter_ancestors(e).any(|a| roots.contains(a)) {
            commands.entity(e).insert(Visibility::Hidden);
        }
    }
}

fn spawn_shape(commands: &mut Commands, d: &ColliderDef, shape: Collider, rot: Quat) {
    let world = Transform::from_matrix(Mat4::from_cols_array(&d.world));
    let mut e = commands.spawn((Name::new(d.name.clone()), world, RigidBody::Static));
    e.with_children(|c| {
        let mut child = c.spawn((shape, Transform::from_translation(d.center).with_rotation(rot)));
        if d.tag.as_deref() == Some("Trampoline") {
            child.insert(Surface::Trampoline);
        } else if !d.trigger && d.name.to_lowercase().contains("deadcoll") {
            child.insert(Surface::Dead);
        }
        if d.trigger {
            child.insert((
                Sensor,
                CollisionEventsEnabled,
                CollisionLayers::new(Layer::Trigger, LayerMask::ALL),
                Trigger { name: d.name.clone() },
            ));
        }
    });
}


/// GotToNextLevel: the campaign's "go to the next level" teleporter (a spinning blue
/// crystal) sits in many scenes because the same scenes serve the campaign. The original
/// switches it off at the start of every match: Initializer.Start when multiplayer, and
/// ZombieCreator.Start in survival / campaign, which switches it back on only once every
/// campaign enemy of the level is dead. We have no campaign levels, so it stays off.
fn hide_campaign_teleports(mut commands: Commands, added: Query<(Entity, &bevy::gltf::GltfExtras), Added<bevy::gltf::GltfExtras>>) {
    for (e, x) in &added {
        if x.value.contains("\"got_to_next_level\"") {
            commands.entity(e).insert(Visibility::Hidden);
            info!("campaign teleporter hidden");
        }
    }
}
