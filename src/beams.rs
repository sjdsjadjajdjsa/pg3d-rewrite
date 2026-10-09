//! Beams of ray weapons: railguns (Prototype) and laser / freeze rays (Freeze Ray Rifle,
//! Solar Ray Rifle). The original draws a LineRenderer from the "rays" bundle (exported by
//! export_rays.py into rays/rays.json): a textured strip from the gun to where the shot
//! ends, additive, fading out over `life` seconds.

use std::collections::HashMap;

#[allow(unused_imports)]
use crate::scene::{FixedUpdate, PostStartup, PostUpdate, PreStartup, Startup, Update};
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use serde::Deserialize;

use crate::{camera::PlayerCamera, DataDir};

pub struct BeamsPlugin;

impl crate::scene::ScenePlugin for BeamsPlugin {
    fn build(&self, app: &mut crate::scene::SceneBuilder) {
        app.add_systems(Startup, load).add_systems(Update, (spawn, update).chain());
    }
}

/// One ray prefab (LineRenderer settings).
#[derive(Deserialize, Clone, Default)]
#[serde(default)]
pub struct RayDef {
    /// seconds the beam stays (RayAndExplosionsStackItem.timeDeactivate)
    pub life: f32,
    /// FreezerRay.lifetime: continuous rays are re-drawn every shot for this long
    pub freezer_life: Option<f32>,
    pub width: f32,
    /// width at the start and end of the beam ([time, value] keys)
    pub width_curve: Vec<[f32; 2]>,
    pub texture: Option<String>,
}

#[derive(Resource, Default)]
pub struct RayDefs(pub HashMap<String, RayDef>);

/// "Draw this weapon's beam from `from` to `to`."
#[derive(Message, Clone)]
pub struct SpawnBeam {
    pub weapon: String,
    pub from: Vec3,
    pub to: Vec3,
    /// seconds to show it (None: the ray's own life)
    pub life: Option<f32>,
    /// your own continuous ray (freeze / laser rays): drawn from this point in the camera's
    /// space, `.1` metres along where you aim, every frame, so it stays on the gun and on
    /// the crosshair while you turn
    pub follow: Option<(Vec3, f32)>,
}

#[derive(Component)]
struct Beam {
    age: f32,
    life: f32,
    from: Vec3,
    to: Vec3,
    follow: Option<(Vec3, f32)>,
    material: Handle<StandardMaterial>,
}

fn load(mut commands: Commands, data: Res<DataDir>) {
    let defs: HashMap<String, RayDef> = std::fs::read_to_string(data.0.join("rays/rays.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    commands.insert_resource(RayDefs(defs));
}

/// A strip along +Z from 0 to 1, `w0` wide at the start and `w1` at the end; u along the
/// beam, v across (the LineRenderer's "stretch" texture mode).
fn strip(w0: f32, w1: f32) -> Mesh {
    let pos = vec![[-w0 / 2.0, 0.0, 0.0], [w0 / 2.0, 0.0, 0.0], [-w1 / 2.0, 0.0, 1.0], [w1 / 2.0, 0.0, 1.0]];
    let uv = vec![[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]];
    let n = vec![[0.0, 1.0, 0.0]; 4];
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, n)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(vec![0, 2, 1, 1, 2, 3]))
}

#[allow(clippy::too_many_arguments)]
fn spawn(
    mut commands: Commands,
    mut msgs: MessageReader<SpawnBeam>,
    defs: Option<Res<RayDefs>>,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut strips: Local<HashMap<String, Handle<Mesh>>>,
) {
    let Some(defs) = defs else { return };
    for m in msgs.read() {
        let Some(def) = defs.0.get(&m.weapon) else { continue };
        debug!("beam {} {:?} -> {:?}", m.weapon, m.from, m.to);
        let mesh = strips
            .entry(m.weapon.clone())
            .or_insert_with(|| {
                let at = |t: f32| {
                    let c = &def.width_curve;
                    let v = match c.len() {
                        0 => 0.05,
                        1 => c[0][1],
                        _ => {
                            let (a, b) = (c[0], c[c.len() - 1]);
                            a[1] + (b[1] - a[1]) * ((t - a[0]) / (b[0] - a[0]).max(1e-4)).clamp(0.0, 1.0)
                        }
                    };
                    v * def.width.max(0.01)
                };
                meshes.add(strip(at(0.0), at(1.0)))
            })
            .clone();
        let material = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: def.texture.as_ref().map(|t| assets.load(format!("rays/{t}"))),
            unlit: true,
            alpha_mode: AlphaMode::Add,
            cull_mode: None,
            double_sided: true,
            ..default()
        });
        commands.spawn((
            Name::new(format!("beam {}", m.weapon)),
            Beam {
                age: 0.0,
                // PG3D_BEAM_LIFE (tests): keep beams longer to photograph them
                life: std::env::var("PG3D_BEAM_LIFE").ok().and_then(|v| v.parse().ok()).unwrap_or(m.life.unwrap_or(def.life)).max(0.05),
                from: m.from,
                to: m.to,
                follow: m.follow,
                material: material.clone(),
            },
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::from_translation(m.from),
        ));
    }
}

/// Keep each beam turned to the camera (like a LineRenderer) and fade it out.
fn update(
    mut commands: Commands,
    time: Res<Time>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    mut beams: Query<(Entity, &mut Beam, &mut Transform)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Ok(cam) = camera.single() else { return };
    let eye = cam.translation();
    for (e, mut b, mut t) in &mut beams {
        b.age += time.delta_secs();
        if b.age >= b.life {
            materials.remove(&b.material);
            commands.entity(e).despawn();
            continue;
        }
        if let Some((local, dist)) = b.follow {
            b.from = cam.transform_point(local);
            b.to = eye + *cam.forward() * dist;
        }
        let d = b.to - b.from;
        let len = d.length();
        let Some(z) = d.try_normalize() else { continue };
        let to_eye = eye - b.from;
        let x = z.cross(to_eye).try_normalize().unwrap_or(Vec3::X);
        let y = z.cross(x);
        t.translation = b.from;
        t.rotation = Quat::from_mat3(&Mat3::from_cols(x, y, z));
        t.scale = Vec3::new(1.0, 1.0, len);
        if let Some(mut m) = materials.get_mut(&b.material) {
            m.base_color.set_alpha(1.0 - b.age / b.life);
        }
    }
}
