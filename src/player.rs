//! First-person walking, modelled on the original FirstPersonControlSharp +
//! Unity CharacterController setup of the "Player" prefab.
//!
//! Numbers marked RECOVERED were read from the game's Player prefab
//! (resources.assets) or project settings. Numbers marked GUESS are not
//! recovered yet: the original reads them at run time from
//! PlayerCoreSettingsStorager "dynamic settings" (probably server config).

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::map::{Layer, MapMarkers, Trigger};

// --- RECOVERED: CharacterController on the Player prefab -------------------------
/// Total capsule height, metres.
pub const HEIGHT: f32 = 1.8;
/// Capsule radius, metres.
pub const RADIUS: f32 = 0.4;
/// Capsule centre relative to the player's origin (Unity: center.y = -0.1).
pub const CENTER_Y: f32 = -0.1;
/// Steepest walkable slope, degrees (Unity slopeLimit).
pub const SLOPE_LIMIT_DEG: f32 = 80.0;
/// Highest ledge we walk up without jumping (Unity stepOffset).
pub const STEP_OFFSET: f32 = 0.6;

// --- RECOVERED: physics settings + FirstPersonControlSharp fields ----------------
/// Physics.gravity (globalgamemanagers PhysicsManager).
pub const GRAVITY: f32 = 9.81;
/// FirstPersonControlSharp.velocityDownFallMultiplier: gravity is stronger when falling.
pub const FALL_MULTIPLIER: f32 = 1.2;
/// FirstPersonControlSharp.maxNegativeVelocityYPerFrame: terminal fall speed.
pub const MAX_FALL_SPEED: f32 = 17.0;
/// FirstPersonControlSharp.maxPositiveVelocityYPerFrame.
pub const MAX_RISE_SPEED: f32 = 20.0;
/// FirstPersonControlSharp.jumpUpMultiplier.
pub const JUMP_UP_MULTIPLIER: f32 = 1.04;

// --- GUESS: not recovered yet ------------------------------------------------------
/// Walking speed, m/s.
pub const WALK_SPEED: f32 = 7.0;
/// Jump take-off speed before JUMP_UP_MULTIPLIER (≈1.3 m jump).
pub const JUMP_SPEED: f32 = 5.0;
/// Eye height above the player's origin.
pub const EYE_HEIGHT: f32 = 0.7;

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_player)
            .add_systems(FixedUpdate, (read_input, move_player).chain())
            .add_systems(Update, (kill_zones, trace));
    }
}

#[derive(Component)]
pub struct Player {
    pub velocity: Vec3,
    pub grounded: bool,
    /// Where we respawn after falling into a kill zone.
    pub spawn: Vec3,
    /// Camera yaw (radians), set by the camera system; WASD is relative to it.
    pub yaw: f32,
    jump_requested: bool,
}

fn spawn_player(mut commands: Commands, markers: Res<MapMarkers>) {
    let spawn = markers
        .0
        .iter()
        .find(|m| {
            let n = m.name.to_lowercase();
            (n.contains("player") || n.contains("spawn")) && !n.contains("enemy") && !n.contains("bonus")
        })
        .map(|m| m.position + Vec3::Y * 1.0)
        .unwrap_or(Vec3::new(0.0, 5.0, 0.0));
    // PG3D_SPAWN="x,y,z" (testing) puts the player somewhere else
    let spawn = std::env::var("PG3D_SPAWN")
        .ok()
        .and_then(|s| s.split(',').map(|v| v.trim().parse::<f32>().ok()).collect::<Option<Vec<_>>>())
        .filter(|v| v.len() == 3)
        .map_or(spawn, |v| Vec3::new(v[0], v[1], v[2]));
    info!("player spawn at {spawn}");

    let cylinder = HEIGHT - 2.0 * RADIUS;
    commands
        .spawn((
            Name::new("player"),
            Player { velocity: Vec3::ZERO, grounded: false, spawn, yaw: 0.0, jump_requested: false },
            Transform::from_translation(spawn),
            RigidBody::Kinematic,
            CustomPositionIntegration,
            TransformInterpolation,
            CollidingEntities::default(),
        ))
        .with_children(|c| {
            c.spawn((
                Collider::capsule(RADIUS, cylinder),
                Transform::from_xyz(0.0, CENTER_Y, 0.0),
                CollisionLayers::new(Layer::Player, LayerMask::ALL),
            ));
        });
}

/// Jump presses are read every frame (Update would miss none, FixedUpdate might),
/// so we latch them here and consume them in move_player.
fn read_input(keys: Res<ButtonInput<KeyCode>>, mut player: Single<&mut Player>) {
    if keys.pressed(KeyCode::Space) {
        player.jump_requested = true;
    }
}

fn move_player(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    camera_mode: Res<crate::camera::CameraMode>,
    player: Single<(Entity, &mut Transform, &mut Player, &Children)>,
    colliders: Query<&Collider>,
    move_and_slide: MoveAndSlide,
    spatial: SpatialQuery,
) {
    let (entity, mut transform, mut p, children) = player.into_inner();
    let Some((collider_entity, collider)) =
        children.iter().find_map(|c| colliders.get(c).ok().map(|col| (c, col)))
    else {
        return;
    };
    let dt = time.delta_secs();

    // --- wish direction from WASD, relative to where the camera looks (yaw only)
    let mut input = Vec2::ZERO;
    if std::env::var("PG3D_AUTOWALK").is_ok() {
        // automated test: walk forward, jump every 2 seconds
        input.y = 1.0;
        if time.elapsed_secs() % 2.0 < dt {
            p.jump_requested = true;
        }
    }
    if *camera_mode == crate::camera::CameraMode::Walk {
        if keys.pressed(KeyCode::KeyW) { input.y += 1.0 }
        if keys.pressed(KeyCode::KeyS) { input.y -= 1.0 }
        if keys.pressed(KeyCode::KeyD) { input.x += 1.0 }
        if keys.pressed(KeyCode::KeyA) { input.x -= 1.0 }
    }
    let rot = Quat::from_rotation_y(p.yaw);
    let wish = rot * Vec3::new(input.x, 0.0, -input.y).normalize_or_zero() * WALK_SPEED;
    p.velocity.x = wish.x;
    p.velocity.z = wish.z;

    // --- gravity / jumping
    if p.grounded {
        if std::mem::take(&mut p.jump_requested) && *camera_mode == crate::camera::CameraMode::Walk {
            p.velocity.y = JUMP_SPEED * JUMP_UP_MULTIPLIER;
            p.grounded = false;
        } else {
            p.velocity.y = -2.0; // keep pressed onto slopes / steps down
        }
    } else {
        p.jump_requested = false;
        let g = if p.velocity.y < 0.0 { GRAVITY * FALL_MULTIPLIER } else { GRAVITY };
        p.velocity.y = (p.velocity.y - g * dt).clamp(-MAX_FALL_SPEED, MAX_RISE_SPEED);
    }

    // --- move and slide along whatever we hit
    let filter = SpatialQueryFilter::from_mask(Layer::World).with_excluded_entities([entity, collider_entity]);
    let min_ground_normal_y = SLOPE_LIMIT_DEG.to_radians().cos();
    // while moving up (jumping) we can't be standing on anything
    let rising = p.velocity.y > 0.0;
    let start = transform.translation + Vec3::Y * CENTER_Y;
    let slide = |from: Vec3, velocity: Vec3, grounded: &mut bool| {
        move_and_slide.move_and_slide(
            collider,
            from,
            Quat::IDENTITY,
            velocity,
            time.delta(),
            &MoveAndSlideConfig::default(),
            // collide with the level only: not with ourselves, not with trigger volumes
            &filter,
            |hit| {
                if !rising && hit.normal.y >= min_ground_normal_y {
                    *grounded = true;
                }
                MoveAndSlideHitResponse::Accept
            },
        )
    };
    let mut grounded = false;
    let out = slide(start, p.velocity, &mut grounded);
    let mut end = out.position;
    let mut projected = out.projected_velocity;

    // --- step up small ledges (Unity CharacterController.stepOffset):
    // if a wall stopped us while walking on the ground, try the same move from
    // STEP_OFFSET higher, then drop back down onto whatever is there.
    let wanted = Vec2::new(p.velocity.x, p.velocity.z) * dt;
    let got = Vec2::new(end.x - start.x, end.z - start.z);
    if p.grounded && !rising && wanted.length() > 1e-4 && got.length() < wanted.length() * 0.9 {
        let up = spatial
            .cast_shape(collider, start, Quat::IDENTITY, Dir3::Y, &ShapeCastConfig::from_max_distance(STEP_OFFSET), &filter)
            .map_or(STEP_OFFSET, |h| h.distance);
        let mut g = false;
        let high = slide(start + Vec3::Y * up, Vec3::new(p.velocity.x, 0.0, p.velocity.z), &mut g).position;
        let down = spatial.cast_shape(
            collider,
            high,
            Quat::IDENTITY,
            Dir3::NEG_Y,
            &ShapeCastConfig::from_max_distance(up + 0.05),
            &filter,
        );
        if let Some(hit) = down {
            let landed = high - Vec3::Y * hit.distance;
            let got_high = Vec2::new(landed.x - start.x, landed.z - start.z);
            if hit.normal1.y.abs() >= min_ground_normal_y && got_high.length() > got.length() + 1e-3 {
                end = landed;
                projected = Vec3::new(p.velocity.x, 0.0, p.velocity.z);
                grounded = true;
            }
        }
    }

    transform.translation = end - Vec3::Y * CENTER_Y;
    if !grounded || p.velocity.y > 0.0 {
        // keep vertical speed from the slide result (e.g. stop when hitting a ceiling)
        p.velocity.y = projected.y;
    }
    p.grounded = grounded;
}

/// PG3D_TRACE=1 logs the player's position twice a second.
fn trace(time: Res<Time>, player: Single<(&Transform, &Player)>, mut last: Local<f32>) {
    if std::env::var("PG3D_TRACE").is_err() || time.elapsed_secs() - *last < 0.5 {
        return;
    }
    *last = time.elapsed_secs();
    let (t, p) = *player;
    info!("player pos={:.2} vel={:.2} grounded={}", t.translation, p.velocity, p.grounded);
}

/// Original levels have "DeadCollider" trigger boxes under the map: touching one kills
/// you. Here we just respawn.
fn kill_zones(
    player: Single<(&mut Transform, &mut Player, &Children)>,
    colliding: Query<&CollidingEntities>,
    triggers: Query<&Trigger>,
) {
    let (mut transform, mut p, children) = player.into_inner();
    let touching_dead = children.iter().filter_map(|c| colliding.get(c).ok()).flat_map(|c| c.iter()).any(
        |e| triggers.get(*e).is_ok_and(|t| t.name.to_lowercase().contains("dead")),
    );
    if touching_dead || transform.translation.y < -200.0 {
        transform.translation = p.spawn;
        p.velocity = Vec3::ZERO;
    }
}
