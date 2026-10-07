//! Makes maps look like the original game.
//!
//! The original is a Unity *gamma colour space* project: textures, lightmaps and lights
//! are multiplied together without converting to linear light, and the result is shown
//! as-is (no tone mapping). Bevy is physically based and linear. To match:
//!   * camera exposure = 1 and tone mapping off, so "1.0" in a shader is "1.0" on screen
//!   * baked lightmaps are converted by the exporter (value^2.2) and attached per mesh
//!   * ambient light / fog / sun come from the scene's RenderSettings and Lights
//!   * moving things (zombies, weapons, pickups) are lit by the map's baked light probes,
//!     resampled by ../pixel-gun-3d-re/scripts/export_probes.py into a Bevy irradiance
//!     volume (probes.json + probes.bin). Like Unity, they then get no flat ambient light.

use bevy::{
    asset::RenderAssetUsages,
    camera::{Exposure, visibility::RenderLayers},
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    core_pipeline::tonemapping::Tonemapping,
    gltf::GltfExtras,
    light::IrradianceVolume,
    pbr::{DistanceFog, FogFalloff, Lightmap},
    prelude::*,
};
use serde::Deserialize;

use crate::{
    DataDir,
    map::{CurrentMap, MapInfo},
};

pub struct LightingPlugin;

impl Plugin for LightingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HasProbes>()
            .add_systems(Startup, (setup_lights, setup_camera_look).after(crate::camera::spawn_camera))
            .add_systems(Update, (attach_lightmaps, set_lightmap_exposure, gamma_blend_glass));
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

/// True when the current map has baked light probes (an irradiance volume).
#[derive(Resource, Default)]
pub struct HasProbes(pub bool);

#[derive(Deserialize)]
struct ProbeGrid {
    resolution: [u32; 3],
    min: Vec3,
    max: Vec3,
}

/// The map's light probes as an irradiance volume entity (see export_probes.py).
fn spawn_probes(commands: &mut Commands, images: &mut Assets<Image>, data: &DataDir, map: &CurrentMap) -> bool {
    let dir = crate::map::map_dir(data, map);
    let Some(grid) =
        std::fs::read_to_string(dir.join("probes.json")).ok().and_then(|s| serde_json::from_str::<ProbeGrid>(&s).ok())
    else {
        return false;
    };
    let [rx, ry, rz] = grid.resolution;
    let size = Extent3d { width: rx, height: 2 * ry, depth_or_array_layers: 3 * rz };
    let Ok(bytes) = std::fs::read(dir.join("probes.bin")) else { return false };
    if bytes.len() != (size.width * size.height * size.depth_or_array_layers * 8) as usize {
        warn!("probes.bin has the wrong size; re-run export_probes.py");
        return false;
    }
    let image =
        Image::new(size, TextureDimension::D3, bytes, TextureFormat::Rgba16Float, RenderAssetUsages::RENDER_WORLD);
    let centre = (grid.min + grid.max) / 2.0;
    // voxel centres sit on the grid's corners: the volume reaches half a voxel further
    let voxel = (grid.max - grid.min) / (Vec3::new(rx as f32, ry as f32, rz as f32) - 1.0).max(Vec3::ONE);
    let extent = grid.max - grid.min + voxel;
    commands.spawn((
        Name::new("light probes"),
        LightProbe { falloff: Vec3::ZERO },
        IrradianceVolume { voxels: images.add(image), intensity: 1.0, affects_lightmapped_meshes: false },
        Transform::from_translation(centre).with_scale(extent),
        RenderLayers::from_layers(&[0, 1]),
    ));
    info!("light probes: {rx}x{ry}x{rz} irradiance volume, {extent:.1} m");
    true
}

fn setup_lights(
    mut commands: Commands,
    info: Res<MapInfo>,
    data: Res<DataDir>,
    map: Res<CurrentMap>,
    mut images: ResMut<Assets<Image>>,
) {
    let r = &info.render;
    let probes = std::env::var("PG3D_NOPROBES").is_err() && spawn_probes(&mut commands, &mut images, &data, &map);
    commands.insert_resource(HasProbes(probes));
    // Ambient only lights things that have no lightmap (the lightmap already has it) and,
    // when the map has light probes, nothing at all: Unity bakes the ambient into them.
    commands.insert_resource(GlobalAmbientLight {
        color: gamma(r.ambient_sky.unwrap_or([0.4; 3])),
        brightness: if probes { 0.0 } else { r.ambient_intensity.unwrap_or(1.0) },
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
        RenderLayers::from_layers(&[0, 1]),
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
                // Unity's legacy Diffuse/Lambert shaders have no specular highlight or
                // Fresnel reflection; Bevy's default 0.5 reflectance whitens glass and
                // anything seen at a grazing angle.
                m.reflectance = 0.0;
            }
        }
    }
}

/// Unity (gamma colour space) mixes see-through surfaces with the scene in *sRGB* values;
/// Bevy mixes in linear light, where a faint bright layer (rink glass, the space dome's
/// green panes) shows up about twice as strongly. Once each blended material's texture
/// has loaded, lower its alpha so the linear mix matches the gamma mix for its average
/// colour, assuming it's lit to about 0.6 and seen against a darkish scene (sRGB 0.15).
fn gamma_blend_glass(
    mut materials: ResMut<Assets<StandardMaterial>>,
    images: Res<Assets<Image>>,
    mut done: Local<std::collections::HashSet<AssetId<StandardMaterial>>>,
) {
    const LIGHT: f32 = 0.6;
    const BACK: f32 = 0.15;
    let lin = |v: f32| v.powf(2.2);
    let mut fixes = Vec::new();
    for (id, m) in materials.iter() {
        if m.alpha_mode != AlphaMode::Blend || done.contains(&id) {
            continue;
        }
        let (mut col, mut alpha) = (1.0, 1.0);
        if let Some(h) = &m.base_color_texture {
            let Some(img) = images.get(h) else { continue };
            let Some(data) = img.data.as_ref() else {
                done.insert(id);
                continue;
            };
            if img.texture_descriptor.format.block_copy_size(None) != Some(4) {
                done.insert(id);
                continue;
            }
            let n = (data.len() / 4).max(1) as f32;
            let (mut l, mut a) = (0.0, 0.0);
            for px in data.chunks_exact(4) {
                let pa = px[3] as f32 / 255.0;
                l += (0.21 * px[0] as f32 + 0.72 * px[1] as f32 + 0.07 * px[2] as f32) / 255.0 * pa;
                a += pa;
            }
            col = if a > 0.0 { l / a } else { 0.0 };
            alpha = a / n;
        }
        done.insert(id);
        let tint = m.base_color.to_srgba();
        let c = (col * (0.21 * tint.red + 0.72 * tint.green + 0.07 * tint.blue) * LIGHT).min(1.0);
        let a = alpha * tint.alpha;
        let d = lin(c) - lin(BACK);
        if a <= 0.0 || a >= 1.0 || d.abs() < 0.02 {
            continue;
        }
        let a2 = ((lin(a * c + (1.0 - a) * BACK) - lin(BACK)) / d).clamp(0.0, 1.0);
        fixes.push((id, a2 / a));
    }
    for (id, k) in fixes {
        if let Some(mut m) = materials.get_mut(id) {
            let a = m.base_color.alpha();
            m.base_color.set_alpha((a * k).min(1.0));
        }
    }
}
