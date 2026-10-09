//! Game feel: gun kick and explosion shake on the camera, the original's hit marker
//! around the crosshair (AimCrosshairController), a white flash on the bot you hit, sparks
//! where bullets land, and a jolt of colour fringing when you get hit.

#[allow(unused_imports)]
use crate::scene::{FixedUpdate, PostStartup, PostUpdate, PreStartup, Startup, Update};
use bevy::{post_process::effect_stack::ChromaticAberration, prelude::*};

use crate::bots::{BotDamage, PlayerHit};
use crate::camera::PlayerCamera;

pub struct FeelPlugin;

impl crate::scene::ScenePlugin for FeelPlugin {
    fn build(&self, app: &mut crate::scene::SceneBuilder) {
        app.init_resource::<Kick>()
            .add_systems(Startup, spawn_hitmarker)
            .add_systems(
                Update,
                (
                    (shot_kick, explosion_shake, apply_kick).chain().after(crate::camera::CameraSet),
                    show_hitmarks,
                    bot_flash,
                    sparks,
                    hurt_fringe,
                ),
            );
    }
}

/// "You fired": camera kick in degrees (weapons.rs).
#[derive(Message)]
pub struct ShotKick(pub f32);

/// "Your shot hit": the hit marker (white hit, yellow headshot, red kill).
#[derive(Message, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HitMark {
    Hit,
    Head,
    Kill,
}

/// "A bullet hit the world here" (weapons.rs).
#[derive(Message)]
pub struct ShotImpact {
    pub point: Vec3,
    pub normal: Vec3,
}

// ---------------------------------------------------------------- camera kick / shake

/// Recoil (pitch, radians: where the kick wants the view, and how much of it is applied) and
/// shake (0..1 trauma).
#[derive(Resource, Default)]
struct Kick {
    target: f32,
    applied: f32,
    trauma: f32,
    shake_applied: Vec2,
    t: f32,
}

/// Seconds the view takes to settle back after a shot (GUESS, by feel).
const KICK_RETURN: f32 = 6.0;

fn shot_kick(mut shots: MessageReader<ShotKick>, mut kick: ResMut<Kick>) {
    for s in shots.read() {
        if std::env::var("PG3D_TRACE_FEEL").is_ok() {
            info!("feel: kick {:.2} deg", s.0);
        }
        kick.target = (kick.target + s.0.to_radians()).min(8f32.to_radians());
    }
}

/// Explosions shake the view, more the closer they are (gone at 25 m).
fn explosion_shake(
    added: Query<&Transform, Added<crate::projectiles::Explosion>>,
    camera: Single<&GlobalTransform, With<PlayerCamera>>,
    mut kick: ResMut<Kick>,
) {
    for t in &added {
        let d = t.translation.distance(camera.translation());
        kick.trauma = (kick.trauma + (1.0 - d / 25.0).max(0.0) * 0.9).min(1.0);
    }
}

/// Applies the change of kick and shake since last frame on top of the mouse look (so the
/// view always comes back exactly to where you aimed).
fn apply_kick(time: Res<Time>, mut kick: ResMut<Kick>, mut camera: Single<&mut Transform, With<PlayerCamera>>) {
    let dt = time.delta_secs();
    let k = &mut *kick;
    k.t += dt;
    // up fast, then the target itself drifts back to zero
    let want = k.target;
    let new = k.applied + (want - k.applied) * (1.0 - (-dt * 40.0).exp());
    k.target -= k.target * (1.0 - (-dt * KICK_RETURN).exp());
    let dp = new - k.applied;
    k.applied = new;
    k.trauma = (k.trauma - dt * 1.6).max(0.0);
    let s = k.trauma * k.trauma * 2.2f32.to_radians();
    let shake = Vec2::new((k.t * 37.0).sin() + (k.t * 23.0).sin() * 0.5, (k.t * 41.0).cos() + (k.t * 29.0).sin() * 0.5) * s;
    let ds = shake - k.shake_applied;
    k.shake_applied = shake;
    if dp == 0.0 && ds == Vec2::ZERO {
        return;
    }
    let (yaw, pitch, roll) = camera.rotation.to_euler(EulerRot::YXZ);
    let lim = std::f32::consts::FRAC_PI_2 - 0.01;
    camera.rotation = Quat::from_euler(EulerRot::YXZ, yaw + ds.x, (pitch + dp + ds.y).clamp(-lim, lim), roll);
}

// ---------------------------------------------------------------- hit marker

/// AimCrosshairController: aimHitSprites, four 28 px corners 25 px out from the centre
/// (one texture, NGUI-flipped per corner), coloured _aimHitColor (white), _aimKillColor
/// (red) or _aimHeadShotColor (yellow, with the "_hs" texture), faded by aimHitTweenAlpha
/// in 0.35 s.
const MARK_FADE: f32 = 0.35;
const MARK_OFFSET: f32 = 25.0;
const MARK_SIZE: f32 = 28.0;

#[derive(Component)]
struct MarkCorner;

#[derive(Resource)]
struct Mark {
    hit: Handle<Image>,
    head: Handle<Image>,
    left: f32,
    kind: HitMark,
}

fn spawn_hitmarker(mut commands: Commands, assets: Res<AssetServer>) {
    let hit: Handle<Image> = assets.load("ui/hitmarker/hitmarkerTEST1.png");
    let head: Handle<Image> = assets.load("ui/hitmarker/hitmarkerTEST_hs.png");
    // (x, y) of the corner: the texture is the bottom-left one; flip for the others
    for (sx, sy) in [(-1.0f32, 1.0f32), (-1.0, -1.0), (1.0, -1.0), (1.0, 1.0)] {
        let mut img = ImageNode::new(hit.clone()).with_color(Color::NONE);
        img.flip_x = sx > 0.0;
        img.flip_y = sy < 0.0;
        commands.spawn((
            MarkCorner,
            img,
            Node {
                position_type: PositionType::Absolute,
                left: percent(50.0),
                top: percent(50.0),
                width: px(MARK_SIZE),
                height: px(MARK_SIZE),
                margin: UiRect {
                    left: px(sx * MARK_OFFSET - MARK_SIZE / 2.0),
                    top: px(sy * MARK_OFFSET - MARK_SIZE / 2.0),
                    ..default()
                },
                ..default()
            },
            GlobalZIndex(6),
            Pickable::IGNORE,
        ));
    }
    commands.insert_resource(Mark { hit, head, left: 0.0, kind: HitMark::Hit });
}

fn show_hitmarks(
    time: Res<Time>,
    mut marks: MessageReader<HitMark>,
    mut mark: ResMut<Mark>,
    mut corners: Query<&mut ImageNode, With<MarkCorner>>,
) {
    let mut best: Option<HitMark> = None;
    for m in marks.read() {
        best = best.max(Some(*m));
    }
    if let Some(m) = best {
        if std::env::var("PG3D_TRACE_FEEL").is_ok() {
            info!("feel: hit mark {}", match m { HitMark::Hit => "hit", HitMark::Head => "head", HitMark::Kill => "kill" });
        }
        // a kill or headshot isn't overwritten by a plain hit in the same moment
        if mark.left <= MARK_FADE * 0.5 || m >= mark.kind {
            mark.kind = m;
        }
        mark.left = MARK_FADE;
    }
    if mark.left <= 0.0 && best.is_none() {
        return;
    }
    mark.left = (mark.left - time.delta_secs()).max(0.0);
    let a = mark.left / MARK_FADE;
    let (tex, color) = match mark.kind {
        HitMark::Hit => (mark.hit.clone(), Color::srgba(1.0, 1.0, 1.0, a)),
        HitMark::Head => (mark.head.clone(), Color::srgba(1.0, 0.776, 0.0, a)),
        HitMark::Kill => (mark.hit.clone(), Color::srgba(1.0, 0.0, 0.0, a)),
    };
    for mut img in &mut corners {
        img.image = tex.clone();
        img.color = color;
    }
}

// ---------------------------------------------------------------- bot flash

/// The bot's own copies of its materials (made on its first hit) and the flash left.
#[derive(Component)]
struct Flash {
    mats: Vec<(Handle<StandardMaterial>, StandardMaterial)>,
    left: f32,
}

const FLASH_TIME: f32 = 0.12;

fn bot_flash(
    mut commands: Commands,
    time: Res<Time>,
    mut hits: MessageReader<BotDamage>,
    children: Query<&Children>,
    meshes: Query<&MeshMaterial3d<StandardMaterial>>,
    mut flashes: Query<&mut Flash>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for h in hits.read() {
        if let Ok(mut f) = flashes.get_mut(h.bot) {
            f.left = FLASH_TIME;
            continue;
        }
        let mut mats = vec![];
        for d in children.iter_descendants(h.bot) {
            let Ok(m) = meshes.get(d) else { continue };
            let Some(orig) = materials.get(&m.0).cloned() else { continue };
            let own = materials.add(orig.clone());
            commands.entity(d).insert(MeshMaterial3d(own.clone()));
            mats.push((own, orig));
        }
        if let Ok(mut ec) = commands.get_entity(h.bot) {
            ec.try_insert(Flash { mats, left: FLASH_TIME });
        }
    }
    let dt = time.delta_secs();
    for mut f in &mut flashes {
        if f.left <= 0.0 && f.left > -1.0 {
            continue;
        }
        f.left = (f.left - dt).max(-2.0);
        let k = (f.left / FLASH_TIME).clamp(0.0, 1.0);
        for (own, orig) in &f.mats {
            let Some(mut m) = materials.get_mut(own) else { continue };
            if orig.unlit {
                m.base_color = orig.base_color.mix(&Color::WHITE, k * 0.55);
            } else {
                m.emissive = orig.emissive + LinearRgba::rgb(1.0, 1.0, 1.0) * (k * 0.6);
            }
        }
        if f.left <= 0.0 {
            f.left = -2.0;
        }
    }
}

// ---------------------------------------------------------------- sparks

#[derive(Component)]
struct Spark {
    vel: Vec3,
    age: f32,
    life: f32,
}

#[derive(Resource)]
struct SparkAssets(Handle<Mesh>, Handle<StandardMaterial>);

fn sparks(
    mut commands: Commands,
    time: Res<Time>,
    mut impacts: MessageReader<ShotImpact>,
    mut q: Query<(Entity, &mut Transform, &mut Spark)>,
    assets: Option<Res<SparkAssets>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut n: Local<u32>,
) {
    let (mesh, mat) = match assets {
        Some(a) => (a.0.clone(), a.1.clone()),
        None => {
            let a = SparkAssets(
                meshes.add(Cuboid::new(0.05, 0.05, 0.05)),
                // brighter than white: glows with bloom
                materials.add(StandardMaterial { base_color: Color::srgb(1.0, 0.8, 0.35), emissive: LinearRgba::rgb(6.0, 3.6, 1.0), unlit: true, ..default() }),
            );
            let out = (a.0.clone(), a.1.clone());
            commands.insert_resource(a);
            out
        }
    };
    for i in impacts.read() {
        if std::env::var("PG3D_TRACE_FEEL").is_ok() {
            info!("feel: sparks at {:.1}", i.point);
        }
        for _ in 0..5 {
            *n = n.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let r = |s: u32| ((n.wrapping_mul(s) >> 8) % 1000) as f32 / 1000.0 - 0.5;
            let dir = (i.normal * 1.2 + Vec3::new(r(3), r(7), r(11)) * 1.6).normalize_or_zero();
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(mat.clone()),
                Transform::from_translation(i.point + i.normal * 0.02),
                Spark { vel: dir * (2.0 + (r(13) + 0.5) * 3.0), age: 0.0, life: 0.18 + (r(17) + 0.5) * 0.15 },
            ));
        }
    }
    let dt = time.delta_secs();
    for (e, mut t, mut s) in &mut q {
        s.age += dt;
        if s.age >= s.life {
            commands.entity(e).despawn();
            continue;
        }
        s.vel.y -= 9.8 * dt;
        t.translation += s.vel * dt;
        t.scale = Vec3::splat(1.0 - s.age / s.life);
    }
}

// ---------------------------------------------------------------- hurt

/// A jolt of colour fringing when you get hit, on top of the map's own (graphics.rs).
fn hurt_fringe(
    time: Res<Time>,
    mut hits: MessageReader<PlayerHit>,
    mut cams: Query<&mut ChromaticAberration>,
    mut jolt: Local<f32>,
    mut base: Local<Option<f32>>,
) {
    if hits.read().count() > 0 {
        *jolt = 1.0;
    }
    if *jolt <= 0.0 {
        return;
    }
    *jolt = (*jolt - time.delta_secs() / 0.35).max(0.0);
    for mut c in &mut cams {
        let b = *base.get_or_insert(c.intensity);
        c.intensity = b + 0.02 * *jolt;
    }
    if *jolt <= 0.0 {
        *base = None;
    }
}
