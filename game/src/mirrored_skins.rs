//! Skinned meshes whose bone is mirrored (negative scale). The character's right leg is
//! the left leg mirrored by its animation (Foot_right is keyed at scale -1.588 in the lobby
//! Idle clip). A mirror turns the triangles inside out; Unity notices the negative
//! determinant and flips the culling, Bevy only does that for an entity's own transform, not
//! for its bones, so the leg was drawn from the inside (dark, "glitching" legs).
//!
//! While a skinned mesh's first bone (times its bind pose) mirrors, it gets a copy of the
//! mesh with the triangle winding reversed.

#[allow(unused_imports)]
use crate::scene::{FixedUpdate, PostStartup, PostUpdate, PreStartup, Startup, Update};
use bevy::{
    mesh::{Indices, skinning::{SkinnedMesh, SkinnedMeshInverseBindposes}},
    prelude::*,
};

pub struct MirroredSkinsPlugin;

impl crate::scene::ScenePlugin for MirroredSkinsPlugin {
    fn build(&self, app: &mut crate::scene::SceneBuilder) {
        app.add_systems(PostUpdate, flip_mirrored.after(TransformSystems::Propagate));
    }
}

/// The mesh as loaded and its reversed-winding twin.
#[derive(Component)]
struct Winding {
    original: Handle<Mesh>,
    flipped: Option<Handle<Mesh>>,
    mirrored: bool,
}

fn flip_mirrored(
    mut commands: Commands,
    mut skins: Query<(Entity, &SkinnedMesh, &mut Mesh3d, &GlobalTransform, Option<&mut Winding>)>,
    joints: Query<&GlobalTransform>,
    bindposes: Res<Assets<SkinnedMeshInverseBindposes>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    for (e, skin, mut mesh, own, winding) in &mut skins {
        let Some(&j) = skin.joints.first() else { continue };
        let Ok(jt) = joints.get(j) else { continue };
        let Some(ibp) = bindposes.get(&skin.inverse_bindposes).and_then(|b| b.first().copied()) else { continue };
        let skin_det = (jt.to_matrix() * ibp).determinant();
        let mirrored = (skin_det < 0.0) != (own.to_matrix().determinant() < 0.0);
        let Some(mut w) = winding else {
            if mirrored {
                commands.entity(e).insert(Winding { original: mesh.0.clone(), flipped: None, mirrored: false });
            }
            continue;
        };
        if w.mirrored == mirrored {
            continue;
        }
        if mirrored && w.flipped.is_none() {
            let Some(src) = meshes.get(&w.original) else { continue };
            let mut m = src.clone();
            if let Some(ind) = m.indices_mut() {
                match ind {
                    Indices::U16(v) => v.chunks_exact_mut(3).for_each(|t| t.swap(1, 2)),
                    Indices::U32(v) => v.chunks_exact_mut(3).for_each(|t| t.swap(1, 2)),
                }
            }
            w.flipped = Some(meshes.add(m));
        }
        w.mirrored = mirrored;
        debug!("mirrored skin {e}: {mirrored}");
        mesh.0 = if mirrored { w.flipped.clone().unwrap_or_else(|| w.original.clone()) } else { w.original.clone() };
    }
}
