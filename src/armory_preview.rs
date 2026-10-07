//! The player character on the right of the Armory, holding the selected weapon.
//!
//! Drawn by its own camera into a picture that the Armory screen shows behind its stats
//! panel (as in the original, where the panel covers the legs). The character and weapon
//! live on their own render layer far below the lobby so the lobby's lights and models
//! don't mix in. As in the original (CharacterInterfaceV2), the weapon prefab sits at the
//! character's gun point with its own arms, posed by the weapon's profile animation, and
//! the character's arms are hidden.

use bevy::{
    camera::{visibility::RenderLayers, RenderTarget},
    core_pipeline::tonemapping::Tonemapping,
    input::mouse::MouseMotion,
    math::Affine2,
    prelude::*,
    render::render_resource::TextureFormat,
    window::PrimaryWindow,
};
use serde::Deserialize;

pub struct ArmoryPreviewPlugin;

impl Plugin for ArmoryPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PreviewWeapon>()
            .add_systems(Startup, spawn)
            .init_resource::<PreviewShoot>()
            .add_systems(Update, (tag_meshes, play_idle, attach_weapon, hide_weapon_extras, hide_char_arms, shoot, play_profile, drag_spin).chain());
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
    /// "Shoot" sound and the time between shots (weapon info.json)
    sound: Option<Handle<AudioSource>>,
    interval: f32,
    /// the Armory pose (export_profiles.py), its tracks resolved to entities once loaded
    profile: Option<Profile>,
    tracks: Vec<(Entity, usize)>,
    age: f32,
}

#[derive(Deserialize, Clone)]
struct Profile {
    length: f32,
    fps: f32,
    tracks: Vec<Track>,
}

#[derive(Deserialize, Clone)]
struct Track {
    path: String,
    #[serde(default)]
    rotation: Vec<[f32; 4]>,
    #[serde(default)]
    translation: Vec<[f32; 3]>,
    #[serde(default)]
    scale: Vec<[f32; 3]>,
}

/// The Armory's Shoot button: the character raises the weapon to the side and fires a
/// short burst (OBSERVED in clip h_rec2: gun level, pointing left, muzzle flashes).
#[derive(Resource, Default)]
pub struct PreviewShoot {
    /// seconds left of the burst
    pub time: f32,
    next_shot: f32,
    /// 0 = holding low, 1 = aiming
    aim: f32,
}

impl PreviewShoot {
    pub fn start(&mut self) {
        self.time = 1.6;
        self.next_shot = 0.25;
    }
}

#[derive(Deserialize, Default)]
struct WeaponInfo {
    #[serde(default)]
    animations: std::collections::HashMap<String, f32>,
    #[serde(default)]
    sounds: std::collections::HashMap<String, String>,
}

fn spawn(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    assets: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut image = Image::new_target_texture(SIZE.x, SIZE.y, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= bevy::render::render_resource::TextureUsages::COPY_SRC;
    let handle = images.add(image);
    commands.insert_resource(PreviewImage(handle.clone()));
    let layer = RenderLayers::layer(LAYER);
    let fov = 30f32.to_radians();
    let cam = commands
        .spawn((
            Name::new("armory preview camera"),
            PreviewCamera,
            Camera3d::default(),
            Camera { order: -1, clear_color: ClearColorConfig::Custom(Color::NONE), ..default() },
            RenderTarget::from(handle),
            Projection::Perspective(PerspectiveProjection { fov, ..default() }),
            // colours exactly as in the pictures (no film look)
            Tonemapping::None,
            Transform::from_translation(STAGE + Vec3::new(0.0, 1.25, 5.2)).looking_at(STAGE + Vec3::new(0.0, 0.88, 0.0), Vec3::Y),
            layer.clone(),
        ))
        .id();
    // Behind the character, the part of the Armory background this picture covers, so the
    // picture is opaque: the weapon's additive muzzle flash then shows as brightly as in
    // the original (on a transparent picture it would be lost).
    let d = 12.0;
    let h = 2.0 * d * (fov / 2.0).tan() * 1.02;
    let w = h * SIZE.x as f32 / SIZE.y as f32;
    let (u0, v0, v1) = (0.75, 60.0 / 720.0, 660.0 / 720.0);
    commands.spawn((
        Name::new("armory preview background"),
        Mesh3d(meshes.add(Rectangle::new(w, h))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(assets.load("ui/armory/textures/armory_bg.png")),
            unlit: true,
            uv_transform: Affine2::from_scale_angle_translation(Vec2::new(1.0 - u0, v1 - v0), 0.0, Vec2::new(u0, v0)),
            ..default()
        })),
        Transform::from_xyz(0.0, 0.0, -d),
        ChildOf(cam),
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
    data: Res<crate::DataDir>,
) {
    if want.held.as_ref().map(|h| &h.0) == want.wanted.as_ref() {
        return;
    }
    let Ok(ch) = chars.single() else { return };
    // wait until the character is loaded
    let Some(point) = children.iter_descendants(ch).find(|d| names.get(*d).is_ok_and(|n| n.as_str() == "Weapon_point")) else {
        return;
    };
    if let Some((_, old)) = want.held.take() {
        commands.entity(old).despawn();
    }
    let Some(name) = want.wanted.clone() else { return };
    // CharacterInterfaceV2 puts the gun point at (0.04, 0, -0.52) (0 across for
    // exoskeletons; glTF x is mirrored)
    commands.entity(point).insert(Transform::from_xyz(-0.04, 0.0, -0.52)
        .with_rotation(Quat::from_xyzw(0.5, -0.5, -0.5, 0.5))
        .with_scale(Vec3::splat(0.6299208)));
    let info: WeaponInfo = std::fs::read_to_string(data.0.join(format!("weapons/{name}/info.json")))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let sound = info.sounds.get("shoot").map(|rel| assets.load(format!("weapons/{name}/{rel}")));
    let interval = info.animations.get("Shoot").copied().filter(|v| *v > 0.05).unwrap_or(0.3).max(0.12);
    let w = commands
        .spawn((
            Name::new(format!("preview {name}")),
            HeldWeapon {
                cleaned: false,
                sound,
                interval,
                profile: std::fs::read_to_string(data.0.join(format!("weapons/{name}/profile.json")))
                    .ok()
                    .and_then(|t| serde_json::from_str(&t).ok()),
                tracks: vec![],
                age: 0.0,
            },
            crate::particles::AssetDir(format!("weapons/{name}")),
            RenderLayers::layer(LAYER),
            WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(format!("weapons/{name}/weapon.gltf")))),
            // the original parents the weapon prefab to the character's Weapon_point as it
            // is, and moves the character's arms onto the prefab's arm points (snap_arms)
            Transform::IDENTITY,
            ChildOf(point),
        ))
        .id();
    want.held = Some((name, w));
}

/// Hide the weapon file's muzzle effects until it fires.
fn hide_weapon_extras(
    mut commands: Commands,
    mut weapons: Query<(Entity, &mut HeldWeapon)>,
    children: Query<&Children>,
    names: Query<&Name>,
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
        }
        h.cleaned = true;
    }
}

fn shoot(
    mut commands: Commands,
    time: Res<Time>,
    mut st: ResMut<PreviewShoot>,
    weapons: Query<(Entity, &HeldWeapon)>,
    mut flashes: MessageWriter<crate::particles::Burst>,
) {
    let dt = time.delta_secs();
    let firing = st.time > 0.0;
    st.time = (st.time - dt).max(0.0);
    let target = if firing { 1.0 } else { 0.0 };
    st.aim += (target - st.aim) * (dt * 10.0).min(1.0);
    let Ok((we, w)) = weapons.single() else { return };
    if firing && st.aim > 0.9 {
        st.next_shot -= dt;
        if st.next_shot <= 0.0 {
            st.next_shot = w.interval;
            // the weapon's own muzzle flash particles (particles.rs)
            flashes.write(crate::particles::Burst(we));
            crate::sound::play(&mut commands, w.sound.as_ref(), None, 0.6);
        }
    }
}

/// Drag the character with the mouse to turn it (original ItemPreviewInArmoryDrag).
fn drag_spin(
    buttons: Res<ButtonInput<MouseButton>>,
    mut motion: MessageReader<MouseMotion>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cams: Query<&Camera, With<PreviewCamera>>,
    mut chars: Query<&mut Transform, With<PreviewChar>>,
    mut dragging: Local<bool>,
) {
    let dx: f32 = motion.read().map(|m| m.delta.x).sum();
    if !cams.iter().any(|c| c.is_active) {
        *dragging = false;
        return;
    }
    let Ok(win) = windows.single() else { return };
    if buttons.just_pressed(MouseButton::Left) {
        // only drags that start on the character's picture (right quarter of the screen,
        // above the stats panel)
        *dragging = win.cursor_position().is_some_and(|p| {
            p.x > win.width() * 0.75 && p.x < win.width() * 0.95 && p.y > win.height() * 0.1 && p.y < win.height() * 0.55
        });
    }
    if !buttons.pressed(MouseButton::Left) {
        *dragging = false;
    }
    if *dragging && dx != 0.0 {
        for mut t in &mut chars {
            t.rotate_y(dx * 0.01);
        }
    }
}

/// The weapon prefab brings its own arms (Arms_Mesh, posed by its profile animation); the
/// character's arms are hidden (CharacterInterfaceV2).
fn hide_char_arms(chars: Query<Entity, With<PreviewChar>>, children: Query<&Children>, names: Query<&Name>, mut vis: Query<&mut Visibility>) {
    let Ok(ch) = chars.single() else { return };
    for d in children.iter_descendants(ch) {
        if names.get(d).is_ok_and(|n| n.as_str() == "Arm_left" || n.as_str() == "Arm_right")
            && let Ok(mut v) = vis.get_mut(d)
        {
            *v = Visibility::Hidden;
        }
    }
}

/// Play the weapon's profile animation (its Armory pose, looping) on the prefab's arms and
/// gun bones.
fn play_profile(
    time: Res<Time>,
    mut weapons: Query<(Entity, &mut HeldWeapon)>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut tf: Query<&mut Transform>,
) {
    for (w, mut h) in &mut weapons {
        let Some(p) = h.profile.clone() else { continue };
        if h.tracks.is_empty() {
            // paths start below the weapon's "*_Inner" object
            let all: Vec<Entity> = children.iter_descendants(w).collect();
            let inner = all.iter().copied().find(|e| names.get(*e).is_ok_and(|n| n.as_str().to_lowercase().ends_with("_inner")));
            let Some(inner) = inner else { continue };
            for (k, t) in p.tracks.iter().enumerate() {
                let mut node = inner;
                let mut ok = true;
                for part in t.path.split('/').filter(|x| !x.is_empty()) {
                    let hit = children
                        .get(node)
                        .into_iter()
                        .flatten()
                        .copied()
                        .find(|c| names.get(*c).is_ok_and(|n| n.as_str().eq_ignore_ascii_case(part)));
                    match hit {
                        Some(c) => node = c,
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    h.tracks.push((node, k));
                }
            }
            if h.tracks.is_empty() {
                h.profile = None;
                continue;
            }
        }
        h.age += time.delta_secs();
        let t = if p.length > 0.0 { h.age % p.length } else { 0.0 };
        let f = t * p.fps;
        let (i, a) = (f.floor() as usize, f.fract());
        for &(e, k) in &h.tracks {
            let tr = &p.tracks[k];
            let Ok(mut x) = tf.get_mut(e) else { continue };
            let pick = |n: usize| (i.min(n.saturating_sub(1)), (i + 1).min(n.saturating_sub(1)));
            if !tr.rotation.is_empty() {
                let (i0, i1) = pick(tr.rotation.len());
                x.rotation = Quat::from_array(tr.rotation[i0]).slerp(Quat::from_array(tr.rotation[i1]), a);
            }
            if !tr.translation.is_empty() {
                let (i0, i1) = pick(tr.translation.len());
                x.translation = Vec3::from(tr.translation[i0]).lerp(Vec3::from(tr.translation[i1]), a);
            }
            if !tr.scale.is_empty() {
                let (i0, i1) = pick(tr.scale.len());
                x.scale = Vec3::from(tr.scale[i0]).lerp(Vec3::from(tr.scale[i1]), a);
            }
        }
    }
}
