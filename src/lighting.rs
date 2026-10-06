//! Makes maps look like the original game.
//!
//! The original is a Unity *gamma colour space* project: textures, lightmaps and lights
//! are multiplied together without converting to linear light, and the result is shown
//! as-is (no tone mapping). Bevy is physically based and linear. To match:
//!   * camera exposure = 1 and tone mapping off, so "1.0" in a shader is "1.0" on screen
//!   * baked lightmaps are converted by the exporter (value^2.2) and attached per mesh
//!   * ambient light / fog / sun come from the scene's RenderSettings and Lights

use bevy::{
    camera::Exposure,
    core_pipeline::tonemapping::Tonemapping,
    gltf::GltfExtras,
    pbr::{DistanceFog, FogFalloff, Lightmap},
    prelude::*,
};
use serde::Deserialize;

use crate::map::{CurrentMap, MapInfo};

pub struct LightingPlugin;

impl Plugin for LightingPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (setup_lights, setup_camera_look).after(crate::camera::spawn_camera))
            .add_systems(Update, (attach_lightmaps, set_lightmap_exposure));
    }
}

/// Bevy exposure: 1 / (2^ev100 * 1.2). ev100 = -log2(1.2) gives exactly 1.
const UNITY_LIKE_EV100: f32 = -0.263_034_4;

fn gamma(c: [f32; 3]) -> Color {
    // gamma-space colour from Unity == sRGB-encoded colour in Bevy
    Color::srgb(c[0], c[1], c[2])
}

fn setup_camera_look(
    mut commands: Commands,
    info: Res<MapInfo>,
    camera: Single<Entity, With<Camera3d>>,
    mut clear: ResMut<ClearColor>,
) {
    let r = &info.render;
    let mut e = commands.entity(*camera);
    e.insert((Exposure { ev100: UNITY_LIKE_EV100 }, Tonemapping::None));
    if r.fog {
        let color = gamma(r.fog_color.unwrap_or([0.5; 3]));
        let falloff = match r.fog_mode {
            Some(1) => FogFalloff::Linear { start: r.fog_start.unwrap_or(0.0), end: r.fog_end.unwrap_or(300.0) },
            Some(3) => FogFalloff::ExponentialSquared { density: r.fog_density.unwrap_or(0.01) },
            _ => FogFalloff::Exponential { density: r.fog_density.unwrap_or(0.01) },
        };
        e.insert(DistanceFog { color, falloff, ..default() });
        clear.0 = color;
    }
}

fn setup_lights(mut commands: Commands, info: Res<MapInfo>) {
    let r = &info.render;
    // Ambient only lights things that have no lightmap (the lightmap already has it).
    commands.insert_resource(GlobalAmbientLight {
        color: gamma(r.ambient_sky.unwrap_or([0.4; 3])),
        brightness: r.ambient_intensity.unwrap_or(1.0),
        affects_lightmapped_meshes: false,
    });
    // Sun: the map's directional light, or a default one.
    let sun = info.lights.iter().find(|l| l.kind == 1);
    let (color, intensity, dir) = sun
        .map(|l| (gamma(l.color), l.intensity, l.direction))
        .unwrap_or((Color::WHITE, 0.8, Vec3::new(-0.4, -1.0, -0.3)));
    commands.spawn((
        Name::new(sun.map(|l| l.name.clone()).unwrap_or_else(|| "sun".into())),
        DirectionalLight {
            color,
            // Bevy's diffuse term divides by pi, Unity's legacy one does not.
            illuminance: intensity * std::f32::consts::PI,
            shadow_maps_enabled: true,
            // lightmapped surfaces already contain this light
            affects_lightmapped_mesh_diffuse: false,
            ..default()
        },
        Transform::default().looking_to(dir.normalize_or(Vec3::NEG_Y), Vec3::Y),
        // also light the first-person weapon (drawn on render layer 1)
        bevy::camera::visibility::RenderLayers::from_layers(&[0, 1]),
    ));
}

#[derive(Deserialize)]
struct LightmapExtras {
    lightmap: String,
    lightmap_rect: [f32; 4],
}

/// The exporter tags glTF nodes with `{"lightmap": "...png", "lightmap_rect": [...]}`.
/// Bevy puts node extras on the node entity; its mesh primitives are the entity itself
/// or its children.
fn attach_lightmaps(
    mut commands: Commands,
    assets: Res<AssetServer>,
    map: Res<CurrentMap>,
    added: Query<(Entity, &GltfExtras), Added<GltfExtras>>,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
) {
    for (entity, extras) in &added {
        let Ok(x) = serde_json::from_str::<LightmapExtras>(&extras.value) else { continue };
        let [x0, y0, x1, y1] = x.lightmap_rect;
        let lightmap = Lightmap {
            image: assets.load(format!("maps/{}/{}", map.0, x.lightmap)),
            uv_rect: Rect::new(x0, y0, x1, y1),
            bicubic_sampling: true,
        };
        let targets = std::iter::once(entity).chain(children.get(entity).into_iter().flat_map(|c| c.iter()));
        for t in targets {
            if meshes.contains(t) {
                commands.entity(t).insert(lightmap.clone());
            }
        }
    }
}

/// Lightmap PNGs are stored scaled down by `lightmap_scale`; every material scales back.
fn set_lightmap_exposure(
    mut events: MessageReader<AssetEvent<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    info: Res<MapInfo>,
) {
    for ev in events.read() {
        if let AssetEvent::Added { id } = ev {
            if let Some(mut m) = materials.get_mut(*id) {
                m.lightmap_exposure = info.lightmap_scale;
            }
        }
    }
}
