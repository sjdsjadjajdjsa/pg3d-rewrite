//! Tiny stand-in for Unity's particle systems: enough for muzzle flashes and smoke.
//!
//! The exporter tags glTF nodes that had a ParticleSystem with
//! `{"particle_texture": "...png", "particle": {life, size, color, tiles, row, ...}}`.
//! Those nodes get a [`ParticleEmitter`]. When something calls [`burst`] on a prefab, every
//! emitter under it spawns one camera-facing quad that plays its row of the texture sheet
//! over its lifetime.

use bevy::{
    camera::visibility::RenderLayers,
    gltf::GltfExtras,
    math::Affine2,
    prelude::*,
};
use serde::Deserialize;

pub struct ParticlesPlugin;

impl Plugin for ParticlesPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Burst>().add_systems(Update, (attach_emitters, spawn_bursts, update_particles).chain());
    }
}

/// Put on a prefab root: asset folder its glTF lives in (texture paths are relative to it).
#[derive(Component)]
pub struct AssetDir(pub String);

/// "Fire all particle emitters under this entity once."
#[derive(Message)]
pub struct Burst(pub Entity);

#[derive(Deserialize, Clone)]
pub struct ParticleSettings {
    pub life: f32,
    pub size: f32,
    pub color: [f32; 4],
    pub tiles: [u32; 2],
    pub row: u32,
    pub single_row: bool,
}

#[derive(Deserialize)]
struct Extras {
    particle_texture: String,
    particle: ParticleSettings,
}

#[derive(Component)]
pub struct ParticleEmitter {
    texture: Handle<Image>,
    settings: ParticleSettings,
}

#[derive(Component)]
struct Particle {
    age: f32,
    settings: ParticleSettings,
    material: Handle<StandardMaterial>,
}

#[derive(Resource)]
struct Quad(Handle<Mesh>);

fn attach_emitters(
    mut commands: Commands,
    added: Query<(Entity, &GltfExtras), Added<GltfExtras>>,
    parents: Query<&ChildOf>,
    dirs: Query<&AssetDir>,
    assets: Res<AssetServer>,
) {
    for (e, extras) in &added {
        let Ok(x) = serde_json::from_str::<Extras>(&extras.value) else { continue };
        let Some(dir) = parents.iter_ancestors(e).find_map(|a| dirs.get(a).ok()) else { continue };
        commands.entity(e).insert(ParticleEmitter {
            texture: assets.load(format!("{}/{}", dir.0, x.particle_texture)),
            settings: x.particle,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_bursts(
    mut commands: Commands,
    mut bursts: MessageReader<Burst>,
    children: Query<&Children>,
    emitters: Query<(&ParticleEmitter, &GlobalTransform)>,
    layers: Query<&RenderLayers>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    quad: Option<Res<Quad>>,
) {
    let quad = match quad {
        Some(q) => q.0.clone(),
        None => {
            let h = meshes.add(Rectangle::new(1.0, 1.0));
            commands.insert_resource(Quad(h.clone()));
            h
        }
    };
    for Burst(root) in bursts.read() {
        // draw on the same render layer as the prefab (the view model has its own camera)
        let layer = layers.get(*root).cloned().unwrap_or_default();
        for e in children.iter_descendants(*root) {
            let Ok((em, gt)) = emitters.get(e) else { continue };
            let [r, g, b, a] = em.settings.color;
            let material = materials.add(StandardMaterial {
                base_color: Color::srgba(r, g, b, a),
                base_color_texture: Some(em.texture.clone()),
                unlit: true,
                alpha_mode: AlphaMode::Add,
                cull_mode: None,
                ..default()
            });
            commands.spawn((
                Particle { age: 0.0, settings: em.settings.clone(), material: material.clone() },
                Mesh3d(quad.clone()),
                MeshMaterial3d(material),
                Transform::from_translation(gt.translation()).with_scale(Vec3::splat(em.settings.size)),
                layer.clone(),
            ));
        }
    }
}

/// Cameras particles turn to face: the match camera, or the Armory preview's.
type FacingCamera = Or<(With<crate::camera::PlayerCamera>, With<crate::armory_preview::PreviewCamera>)>;

fn update_particles(
    mut commands: Commands,
    time: Res<Time>,
    cameras: Query<&GlobalTransform, FacingCamera>,
    mut q: Query<(Entity, &mut Particle, &mut Transform)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // the match camera, or the Armory preview's in the menu
    let Some(camera) = cameras.iter().next() else { return };
    let cam_rot = camera.compute_transform().rotation;
    for (e, mut p, mut t) in &mut q {
        p.age += time.delta_secs();
        let k = p.age / p.settings.life.max(0.01);
        if k >= 1.0 {
            materials.remove(&p.material);
            commands.entity(e).despawn();
            continue;
        }
        t.rotation = cam_rot; // face the camera
        // pick the texture-sheet frame for this point in the particle's life
        let [tx, ty] = p.settings.tiles;
        let frames = if p.settings.single_row { tx } else { tx * ty };
        let f = ((k * frames as f32) as u32).min(frames - 1);
        let (col, row) = if p.settings.single_row { (f, p.settings.row) } else { (f % tx, f / tx) };
        if let Some(mut m) = materials.get_mut(&p.material) {
            let size = Vec2::new(1.0 / tx as f32, 1.0 / ty as f32);
            m.uv_transform = Affine2::from_scale_angle_translation(size, 0.0, Vec2::new(col as f32, row as f32) * size);
            m.base_color.set_alpha(p.settings.color[3] * (1.0 - k));
        }
    }
}
