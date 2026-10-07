//! The player character on the right of the Armory, holding the selected weapon.
//!
//! Drawn by its own camera into a picture that the Armory screen shows behind its stats
//! panel (as in the original, where the panel covers the legs). The character and weapon
//! live on their own render layer far below the lobby so the lobby's lights and models
//! don't mix in. The original fits the character's arms to the weapon's hand points (IK);
//! we only place the weapon at the character's Weapon_point for now.

use bevy::{
    camera::{visibility::RenderLayers, RenderTarget},
    prelude::*,
    render::render_resource::TextureFormat,
};

pub struct ArmoryPreviewPlugin;

impl Plugin for ArmoryPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PreviewWeapon>()
            .add_systems(Startup, spawn)
            .add_systems(Update, (tag_meshes, play_idle, attach_weapon, hide_weapon_extras));
    }
}

const LAYER: usize = 3;
/// Where the preview stage sits (out of sight of the lobby camera).
const STAGE: Vec3 = Vec3::new(0.0, -500.0, 0.0);
/// Picture size; shown at the right of the Armory (OBSERVED: the character stands in
/// x 960..1280, y 60..660 px of 1280x720 screenshots).
pub const SIZE: UVec2 = UVec2::new(400, 750);

/// The weapon the preview should hold (set by the Armory), and the one it holds.
#[derive(Resource, Default)]
pub struct PreviewWeapon {
    pub wanted: Option<String>,
    held: Option<(String, Entity)>,
}

#[derive(Resource)]
pub struct PreviewImage(pub Handle<Image>);

#[derive(Component)]
struct PreviewRoot;

/// The preview's camera (only drawn while the Armory is open).
#[derive(Component)]
pub struct PreviewCamera;

#[derive(Component)]
struct PreviewChar {
    graph: Handle<AnimationGraph>,
    idle: AnimationNodeIndex,
    playing: bool,
}

#[derive(Component)]
struct HeldWeapon {
    cleaned: bool,
}

fn spawn(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    assets: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    let mut image = Image::new_target_texture(SIZE.x, SIZE.y, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= bevy::render::render_resource::TextureUsages::COPY_SRC;
    let handle = images.add(image);
    commands.insert_resource(PreviewImage(handle.clone()));
    let layer = RenderLayers::layer(LAYER);
    commands.spawn((
        Name::new("armory preview camera"),
        PreviewCamera,
        Camera3d::default(),
        Camera { order: -1, clear_color: ClearColorConfig::Custom(Color::NONE), ..default() },
        RenderTarget::from(handle),
        Projection::Perspective(PerspectiveProjection { fov: 30f32.to_radians(), ..default() }),
        Transform::from_translation(STAGE + Vec3::new(0.0, 1.25, 5.2)).looking_at(STAGE + Vec3::new(0.0, 0.88, 0.0), Vec3::Y),
        layer.clone(),
    ));
    let path = "characters/default/character.gltf";
    let (graph, idle) = AnimationGraph::from_clip(assets.load(GltfAssetLabel::Animation(0).from_asset(path)));
    commands.spawn((
        Name::new("armory preview character"),
        PreviewRoot,
        PreviewChar { graph: graphs.add(graph), idle, playing: false },
        WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(path))),
        Transform::from_translation(STAGE).with_rotation(Quat::from_rotation_y(0.45)),
        layer,
    ));
}

/// Everything under the preview goes on its layer, drawn unlit like the original's flat
/// shop lighting.
fn tag_meshes(
    mut commands: Commands,
    added: Query<(Entity, Option<&MeshMaterial3d<StandardMaterial>>), Added<Mesh3d>>,
    parents: Query<&ChildOf>,
    roots: Query<(), With<PreviewRoot>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (e, mat) in &added {
        if !parents.iter_ancestors(e).any(|a| roots.contains(a)) {
            continue;
        }
        let mut ec = commands.entity(e);
        ec.insert(RenderLayers::layer(LAYER));
        if let Some(m) = mat.and_then(|m| materials.get(&m.0)).cloned() {
            ec.insert(MeshMaterial3d(materials.add(StandardMaterial { unlit: true, ..m })));
        }
    }
}

fn play_idle(
    mut commands: Commands,
    mut chars: Query<(Entity, &mut PreviewChar)>,
    children: Query<&Children>,
    mut players: Query<&mut AnimationPlayer>,
) {
    for (e, mut c) in &mut chars {
        if c.playing {
            continue;
        }
        let Some(p) = children.iter_descendants(e).find(|d| players.contains(*d)) else { continue };
        if let Ok(mut player) = players.get_mut(p) {
            player.play(c.idle).repeat();
        }
        commands.entity(p).insert(AnimationGraphHandle(c.graph.clone()));
        c.playing = true;
    }
}

/// Put the wanted weapon at the character's Weapon_point.
fn attach_weapon(
    mut commands: Commands,
    mut want: ResMut<PreviewWeapon>,
    chars: Query<Entity, With<PreviewChar>>,
    children: Query<&Children>,
    names: Query<&Name>,
    assets: Res<AssetServer>,
) {
    if want.held.as_ref().map(|h| &h.0) == want.wanted.as_ref() {
        return;
    }
    let Ok(ch) = chars.single() else { return };
    // wait until the character is loaded
    if !children.iter_descendants(ch).any(|d| names.get(d).is_ok_and(|n| n.as_str() == "Weapon_point")) {
        return;
    }
    if let Some((_, old)) = want.held.take() {
        commands.entity(old).despawn();
    }
    let Some(name) = want.wanted.clone() else { return };
    let w = commands
        .spawn((
            Name::new(format!("preview {name}")),
            HeldWeapon { cleaned: false },
            WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(format!("weapons/{name}/weapon.gltf")))),
            // held low at the right hip, pointing ahead (OBSERVED pose); the weapon files
            // are first-person, with the gun 1.5 up and 0.8 ahead of their origin
            weapon_transform(),
            ChildOf(ch),
        ))
        .id();
    want.held = Some((name, w));
}

/// The weapon files are first-person: hide their arms and muzzle effects.
fn hide_weapon_extras(
    mut commands: Commands,
    mut weapons: Query<(Entity, &mut HeldWeapon)>,
    children: Query<&Children>,
    names: Query<&Name>,
    meshes: Query<(), With<Mesh3d>>,
) {
    for (w, mut h) in &mut weapons {
        if h.cleaned {
            continue;
        }
        let all: Vec<Entity> = children.iter_descendants(w).collect();
        if all.is_empty() {
            continue;
        }
        for e in all {
            let Ok(n) = names.get(e) else { continue };
            if n.as_str() == "BulletSpawnPoint" {
                commands.entity(e).insert(Visibility::Hidden);
            }
            if n.as_str() == "Arms_Mesh" {
                // the arm mesh itself (its primitives), not the gun mesh below it
                for c in children.get(e).into_iter().flatten() {
                    if meshes.contains(*c) {
                        commands.entity(*c).insert(Visibility::Hidden);
                    }
                }
            }
        }
        h.cleaned = true;
    }
}

fn weapon_transform() -> Transform {
    let rot = Quat::from_rotation_z(1.0) * Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2);
    let gun = Vec3::new(-0.68, 0.92, 0.45);
    Transform::from_translation(gun - rot * Vec3::new(0.0, 1.5, 0.82)).with_rotation(rot)
}
