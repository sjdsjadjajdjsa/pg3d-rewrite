//! First-person camera. Two modes, toggled with V:
//!   Walk: camera sits at the player's eyes (see player.rs)
//!   Fly:  free debug camera, WASD + Space/Ctrl, Shift = fast
//! Click the window to capture the mouse, Esc to release.

use std::f32::consts::FRAC_PI_2;

use bevy::{
    input::mouse::AccumulatedMouseMotion,
    prelude::*,
    window::{CursorGrabMode, CursorOptions},
};

use crate::player::{EYE_HEIGHT, Player};

pub struct CameraPlugin;

#[derive(Resource, PartialEq, Eq, Clone, Copy, Debug)]
pub enum CameraMode {
    Walk,
    Fly,
}

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        // PG3D_CAM (used for automated screenshots) starts in fly mode.
        let mode = if std::env::var("PG3D_CAM").is_ok() { CameraMode::Fly } else { CameraMode::Walk };
        app.insert_resource(mode)
            .add_systems(Startup, spawn_camera)
            .add_systems(
                Update,
                (
                    grab_cursor,
                    toggle_mode,
                    look,
                    fly.run_if(resource_equals(CameraMode::Fly)),
                    follow_player.run_if(resource_equals(CameraMode::Walk)),
                )
                    .chain(),
            );
    }
}

#[derive(Component)]
pub struct PlayerCamera {
    /// fly-mode speed, metres per second
    pub fly_speed: f32,
    /// radians per pixel of mouse movement
    pub sensitivity: f32,
}

fn spawn_camera(mut commands: Commands) {
    // PG3D_CAM="x,y,z,tx,ty,tz" overrides the start position (x,y,z) and look target (t*).
    let start = std::env::var("PG3D_CAM")
        .ok()
        .and_then(|s| s.split(',').map(|v| v.trim().parse::<f32>().ok()).collect::<Option<Vec<_>>>())
        .filter(|v| v.len() == 6)
        .map(|v| Transform::from_xyz(v[0], v[1], v[2]).looking_at(Vec3::new(v[3], v[4], v[5]), Vec3::Y))
        .unwrap_or_default();
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection { fov: 70f32.to_radians(), ..default() }),
        PlayerCamera { fly_speed: 10.0, sensitivity: 0.002 },
        start,
    ));
}

fn grab_cursor(
    mut cursor: Single<&mut CursorOptions>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    if mouse.just_pressed(MouseButton::Left) {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
    if keys.just_pressed(KeyCode::Escape) {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
}

fn toggle_mode(keys: Res<ButtonInput<KeyCode>>, mut mode: ResMut<CameraMode>) {
    if keys.just_pressed(KeyCode::KeyV) {
        *mode = match *mode {
            CameraMode::Walk => CameraMode::Fly,
            CameraMode::Fly => CameraMode::Walk,
        };
        info!("camera mode: {:?}", *mode);
    }
}

fn look(
    motion: Res<AccumulatedMouseMotion>,
    cursor: Single<&CursorOptions>,
    camera: Single<(&mut Transform, &PlayerCamera)>,
    mut player: Single<&mut Player>,
) {
    let (mut transform, cam) = camera.into_inner();
    if cursor.grab_mode != CursorGrabMode::None && motion.delta != Vec2::ZERO {
        let (yaw, pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
        let yaw = yaw - motion.delta.x * cam.sensitivity;
        let pitch = (pitch - motion.delta.y * cam.sensitivity).clamp(-FRAC_PI_2 + 0.01, FRAC_PI_2 - 0.01);
        transform.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
    }
    player.yaw = transform.rotation.to_euler(EulerRot::YXZ).0;
}

fn fly(keys: Res<ButtonInput<KeyCode>>, time: Res<Time>, camera: Single<(&mut Transform, &PlayerCamera)>) {
    let (mut transform, cam) = camera.into_inner();
    let mut dir = Vec3::ZERO;
    let forward = transform.forward().as_vec3();
    let right = transform.right().as_vec3();
    if keys.pressed(KeyCode::KeyW) { dir += forward }
    if keys.pressed(KeyCode::KeyS) { dir -= forward }
    if keys.pressed(KeyCode::KeyD) { dir += right }
    if keys.pressed(KeyCode::KeyA) { dir -= right }
    if keys.any_pressed([KeyCode::Space, KeyCode::KeyE]) { dir += Vec3::Y }
    if keys.any_pressed([KeyCode::ControlLeft, KeyCode::KeyQ]) { dir -= Vec3::Y }
    let fast = if keys.pressed(KeyCode::ShiftLeft) { 4.0 } else { 1.0 };
    transform.translation += dir.normalize_or_zero() * cam.fly_speed * fast * time.delta_secs();
}

fn follow_player(
    player: Single<&Transform, (With<Player>, Without<PlayerCamera>)>,
    mut camera: Single<&mut Transform, With<PlayerCamera>>,
) {
    camera.translation = player.translation + Vec3::Y * EYE_HEIGHT;
}
