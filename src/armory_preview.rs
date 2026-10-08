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
    /// "Shoot" and "Reload" sounds (weapon info.json)
    sound: Option<Handle<AudioSource>>,
    reload_sound: Option<Handle<AudioSource>>,
    /// rounds in a clip (0: melee, never reloads)
    clip_size: i32,
    /// "Profile" (the Armory pose, export_profiles.py) and the first-person clips Idle,
    /// Shoot, Shoot1.., Reload (export_weapon_clips.py); tracks resolved once loaded
    clips: std::collections::HashMap<String, Profile>,
    bound: std::collections::HashMap<String, Vec<(Entity, usize)>>,
    /// time in the looping profile pose
    age: f32,
    phase: Phase,
    /// pretend ammo and how many reloads so far (ShopShootingController)
    ammo: i32,
    reloads: i32,
    /// next barrel for Shoot1/Shoot2 weapons
    barrel: u32,
}

/// ShopShootingController's states, simplified.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    /// looping the profile pose
    Profile,
    /// cross-fading from the profile pose to Idle (startIdleCrossFadeTimeout)
    ToIdle(f32),
    /// playing this Shoot clip, seconds in
    Shot(&'static str, f32),
    /// playing Reload, seconds in
    Reload(f32),
    /// Idle, cross-fading back to the profile pose (endIdleCrossFadeTimeout)
    ToProfile(f32),
}

/// ShopShootingController.startIdleCrossFadeTimeout / endIdleCrossFadeTimeout (serialized
/// values of the Armory's instance)
const START_FADE: f32 = 0.05;
const END_FADE: f32 = 0.3;

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

/// The Armory's Shoot button (ShopShootButtonController): fires while held down.
#[derive(Resource, Default)]
pub struct PreviewShoot {
    pub held: bool,
}

impl PreviewShoot {
    pub fn start(&mut self) {
        self.held = true;
    }
}

#[derive(Deserialize, Default)]
struct WeaponInfo {
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
    let reload_sound = info.sounds.get("reload").map(|rel| assets.load(format!("weapons/{name}/{rel}")));
    let read = |f: &str| std::fs::read_to_string(data.0.join(format!("weapons/{name}/{f}"))).ok();
    let mut clips: std::collections::HashMap<String, Profile> =
        read("clips.json").and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    if let Some(p) = read("profile.json").and_then(|t| serde_json::from_str(&t).ok()) {
        clips.insert("Profile".into(), p);
    }
    let clip_size = weapon_clip_size(&data.0, &name);
    let w = commands
        .spawn((
            Name::new(format!("preview {name}")),
            HeldWeapon {
                cleaned: false,
                sound,
                reload_sound,
                clip_size,
                clips,
                bound: Default::default(),
                age: 0.0,
                phase: Phase::Profile,
                ammo: clip_size,
                reloads: 0,
                barrel: 1,
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

/// ShopShootingController.Update, simplified: hold Shoot and the weapon's arms cross-fade
/// from the profile pose to the first-person Idle, play "Shoot" for every shot (muzzle
/// flash and sound with it), "Reload" when the pretend clip runs low (the first two times
/// after a third of a clip, then when empty), and on release go to Idle and fade back to
/// the profile pose.
fn shoot(
    mut commands: Commands,
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut st: ResMut<PreviewShoot>,
    mut weapons: Query<(Entity, &mut HeldWeapon)>,
    mut flashes: MessageWriter<crate::particles::Burst>,
) {
    // PG3D_HOLD_SHOOT=1 (automated screenshots): keep the button held
    if !mouse.pressed(MouseButton::Left) && std::env::var("PG3D_HOLD_SHOOT").is_err() {
        st.held = false;
    }
    if std::env::var("PG3D_HOLD_SHOOT").is_ok() {
        st.held = true;
    }
    let dt = time.delta_secs();
    let Ok((we, mut w)) = weapons.single_mut() else { return };
    let len = |w: &HeldWeapon, c: &str| w.clips.get(c).map_or(0.0, |p| p.length);
    let has_fp = w.clips.contains_key("Idle");
    w.phase = match w.phase {
        Phase::Profile if st.held => Phase::ToIdle(0.0),
        Phase::Profile => Phase::Profile,
        Phase::ToIdle(t) if t + dt < START_FADE => Phase::ToIdle(t + dt),
        Phase::ToIdle(_) => Phase::Shot("", f32::MAX),
        Phase::Shot(c, t) if t + dt < len(&w, c).max(0.1) => Phase::Shot(c, t + dt),
        Phase::Shot(..) => {
            let low = if w.reloads < 2 { w.clip_size - w.clip_size / 3 } else { 0 };
            if w.clip_size > 0 && w.ammo <= low {
                crate::sound::play(&mut commands, w.reload_sound.as_ref(), None, 0.6);
                Phase::Reload(0.0)
            } else if st.held {
                // Shoot, or Shoot1 / Shoot2 in turn for two-barrelled weapons
                let clip = if w.clips.contains_key("Shoot") || !w.clips.contains_key("Shoot1") {
                    "Shoot"
                } else {
                    let c = if w.barrel == 1 { "Shoot1" } else { "Shoot2" };
                    w.barrel = if w.barrel == 1 && w.clips.contains_key("Shoot2") { 2 } else { 1 };
                    c
                };
                w.ammo -= 1;
                flashes.write(crate::particles::Burst(we));
                crate::sound::play(&mut commands, w.sound.as_ref(), None, 0.6);
                Phase::Shot(clip, 0.0)
            } else {
                Phase::ToProfile(0.0)
            }
        }
        Phase::Reload(t) if t + dt < len(&w, "Reload").max(0.3) => Phase::Reload(t + dt),
        Phase::Reload(_) => {
            w.ammo = w.clip_size;
            w.reloads += 1;
            if st.held { Phase::Shot("", f32::MAX) } else { Phase::ToProfile(0.0) }
        }
        Phase::ToProfile(t) if st.held => {
            let _ = t;
            Phase::ToIdle(START_FADE)
        }
        Phase::ToProfile(t) if t + dt < END_FADE => Phase::ToProfile(t + dt),
        Phase::ToProfile(_) => Phase::Profile,
    };
    // weapons without first-person clips only flash
    if !has_fp && matches!(w.phase, Phase::ToIdle(_)) {
        w.phase = Phase::Shot("", f32::MAX);
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

/// A bone's rotation, translation and scale in one pose (None: the clip doesn't key it).
type BonePose = (Option<Quat>, Option<Vec3>, Option<Vec3>);

/// Bone transforms of one clip at time `t` (looping or held at the end).
fn sample(p: &Profile, bound: &[(Entity, usize)], t: f32, looped: bool) -> Vec<(Entity, BonePose)> {
    let t = if p.length <= 0.0 {
        0.0
    } else if looped {
        t % p.length
    } else {
        t.clamp(0.0, p.length)
    };
    let f = t * p.fps;
    let (i, a) = (f.floor() as usize, f.fract());
    let pick = |n: usize| (i.min(n.saturating_sub(1)), (i + 1).min(n.saturating_sub(1)));
    bound
        .iter()
        .map(|&(e, k)| {
            let tr = &p.tracks[k];
            let rot = (!tr.rotation.is_empty()).then(|| {
                let (i0, i1) = pick(tr.rotation.len());
                Quat::from_array(tr.rotation[i0]).slerp(Quat::from_array(tr.rotation[i1]), a)
            });
            let pos = (!tr.translation.is_empty()).then(|| {
                let (i0, i1) = pick(tr.translation.len());
                Vec3::from(tr.translation[i0]).lerp(Vec3::from(tr.translation[i1]), a)
            });
            let sc = (!tr.scale.is_empty()).then(|| {
                let (i0, i1) = pick(tr.scale.len());
                Vec3::from(tr.scale[i0]).lerp(Vec3::from(tr.scale[i1]), a)
            });
            (e, (rot, pos, sc))
        })
        .collect()
}

/// Rounds in the weapon's clip (weapons.json ammoInClip; 0 for melee).
fn weapon_clip_size(data: &std::path::Path, name: &str) -> i32 {
    #[derive(Deserialize)]
    #[allow(non_snake_case)]
    struct W {
        #[serde(default)]
        ammoInClip: i32,
        #[serde(default)]
        isMelee: i32,
    }
    static ALL: std::sync::OnceLock<std::collections::HashMap<String, W>> = std::sync::OnceLock::new();
    let all = ALL.get_or_init(|| {
        std::fs::read_to_string(data.join("weapons/weapons.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    });
    all.get(name).map_or(0, |w| if w.isMelee != 0 { 0 } else { w.ammoInClip.max(0) })
}

/// Pose the weapon's arms and gun bones: the profile clip (the Armory pose, looping) or,
/// while shooting, the first-person clips, cross-fading between them (Unity
/// Animation.CrossFade: a linear blend over the fade time).
fn play_profile(
    time: Res<Time>,
    mut weapons: Query<(Entity, &mut HeldWeapon)>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut tf: Query<&mut Transform>,
) {
    for (w, mut h) in &mut weapons {
        if h.bound.is_empty() && !h.clips.is_empty() {
            // paths start below the weapon's "*_Inner" object
            let all: Vec<Entity> = children.iter_descendants(w).collect();
            let inner = all.iter().copied().find(|e| names.get(*e).is_ok_and(|n| n.as_str().to_lowercase().ends_with("_inner")));
            let Some(inner) = inner else { continue };
            let resolve = |path: &str| {
                let mut node = inner;
                for part in path.split('/').filter(|x| !x.is_empty()) {
                    node = children
                        .get(node)
                        .into_iter()
                        .flatten()
                        .copied()
                        .find(|c| names.get(*c).is_ok_and(|n| n.as_str().eq_ignore_ascii_case(part)))?;
                }
                Some(node)
            };
            let bound: std::collections::HashMap<String, Vec<(Entity, usize)>> = h
                .clips
                .iter()
                .map(|(name, p)| {
                    let v = p.tracks.iter().enumerate().filter_map(|(k, t)| resolve(&t.path).map(|e| (e, k))).collect();
                    (name.clone(), v)
                })
                .collect();
            h.bound = bound;
        }
        if !h.bound.get("Profile").is_some_and(|b| !b.is_empty()) {
            continue;
        }
        let dt = time.delta_secs();
        // the profile pose keeps its place while shooting (normalizedTime restored)
        if matches!(h.phase, Phase::Profile | Phase::ToProfile(_)) {
            h.age += dt;
        }
        let clip = |name: &str, t: f32, looped: bool| match (h.clips.get(name), h.bound.get(name)) {
            (Some(p), Some(b)) => sample(p, b, t, looped),
            _ => vec![],
        };
        let profile = clip("Profile", h.age, true);
        let idle = clip("Idle", 0.0, true);
        let (a, b, k) = match h.phase {
            Phase::Profile => (profile, vec![], 0.0),
            Phase::ToIdle(t) => (profile, idle, (t / START_FADE).clamp(0.0, 1.0)),
            Phase::Shot(c, t) => {
                let shot = if c.is_empty() || t == f32::MAX { idle } else { clip(c, t, false) };
                (shot, vec![], 0.0)
            }
            Phase::Reload(t) => (clip("Reload", t, false), vec![], 0.0),
            Phase::ToProfile(t) => (idle, profile, (t / END_FADE).clamp(0.0, 1.0)),
        };
        let mut pose: std::collections::HashMap<Entity, BonePose> = a.into_iter().collect();
        for (e, (r, p, s)) in b {
            let x = pose.entry(e).or_insert((None, None, None));
            x.0 = match (x.0, r) {
                (Some(q0), Some(q1)) => Some(q0.slerp(q1, k)),
                (q, None) => q,
                (None, q) => q,
            };
            x.1 = match (x.1, p) {
                (Some(v0), Some(v1)) => Some(v0.lerp(v1, k)),
                (v, None) => v,
                (None, v) => v,
            };
            x.2 = match (x.2, s) {
                (Some(v0), Some(v1)) => Some(v0.lerp(v1, k)),
                (v, None) => v,
                (None, v) => v,
            };
        }
        for (e, (r, p, s)) in pose {
            let Ok(mut x) = tf.get_mut(e) else { continue };
            if let Some(r) = r {
                x.rotation = r;
            }
            if let Some(p) = p {
                x.translation = p;
            }
            if let Some(s) = s {
                x.scale = s;
            }
        }
    }
}
