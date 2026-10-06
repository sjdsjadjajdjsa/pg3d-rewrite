//! Loads an exported map: visible geometry (map.gltf), collision (collision.gltf +
//! colliders.json) and markers such as spawn zones (info.json).

use avian3d::prelude::*;
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

#[derive(Deserialize)]
struct Info {
    #[serde(default)]
    markers: Vec<Marker>,
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
}

/// Physics layers: triggers live on their own layer so movement ignores them.
#[derive(PhysicsLayer, Default, Clone, Copy, Debug)]
pub enum Layer {
    #[default]
    World,
    Trigger,
}

/// A trigger volume from the original level, e.g. "DeadCollider" (kill zone).
#[derive(Component)]
pub struct Trigger {
    pub name: String,
}

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(Color::srgb(0.45, 0.65, 0.9)))
            // The original maps are mostly lit by pre-baked lightmaps, which we don't
            // import yet, so use a strong flat ambient light plus one "sun".
            .insert_resource(GlobalAmbientLight {
                color: Color::WHITE,
                brightness: 1500.0,
                ..default()
            })
            .init_resource::<MapMarkers>()
            .add_systems(PreStartup, load_markers)
            .add_systems(Startup, (spawn_map, spawn_colliders));
    }
}

fn map_dir(data: &DataDir, map: &CurrentMap) -> std::path::PathBuf {
    data.0.join("maps").join(&map.0)
}

fn load_markers(mut commands: Commands, data: Res<DataDir>, map: Res<CurrentMap>) {
    let path = map_dir(&data, &map).join("info.json");
    match std::fs::read_to_string(&path).map(|s| serde_json::from_str::<Info>(&s)) {
        Ok(Ok(info)) => commands.insert_resource(MapMarkers(info.markers)),
        Ok(Err(e)) => warn!("{}: {e}", path.display()),
        Err(e) => error!("{}: {e} (did you export the map?)", path.display()),
    }
}

fn spawn_map(mut commands: Commands, assets: Res<AssetServer>, map: Res<CurrentMap>) {
    let path = format!("maps/{}/map.gltf", map.0);
    info!("loading {path}");
    commands.spawn((
        Name::new(format!("map {}", map.0)),
        WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(path))),
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: 4_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(30.0, 60.0, 20.0).looking_at(Vec3::ZERO, Vec3::Y),
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

fn spawn_shape(commands: &mut Commands, d: &ColliderDef, shape: Collider, rot: Quat) {
    let world = Transform::from_matrix(Mat4::from_cols_array(&d.world));
    let mut e = commands.spawn((Name::new(d.name.clone()), world, RigidBody::Static));
    e.with_children(|c| {
        let mut child = c.spawn((shape, Transform::from_translation(d.center).with_rotation(rot)));
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

