//! In-match HUD, laid out like the original (reference clips in
//! ../pixel-gun-3d-re/private/reference/, Arena mini-game):
//!   * (health bar and weapon bar: the original sprites, see ingame_ui.rs)
//!   * (top centre box with score / wave / zombies left: ingame_ui.rs, original sprites)
//!   * damage: red screen edges that get stronger as health drops and flash on every
//!     hit, plus a red arrow around the crosshair pointing at whoever hit you
//! The original draws these with NGUI sprites from its in-match atlas; we draw them with
//! plain Bevy UI shapes (no icons yet).

use bevy::{asset::RenderAssetUsages, prelude::*, render::render_resource::{Extent3d, TextureDimension, TextureFormat}};

use crate::bots::{PLAYER_MAX_HEALTH, PlayerHit, PlayerStats};

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_hud).add_systems(Update, (damage_flash, hit_arrows, heal_flash));
    }
}

#[derive(Component)]
struct Vignette;

/// Green screen tint when health goes up (OBSERVED in clip d_rec8: ~0.5 s, strongest at
/// the edges, after picking up a health bonus).
#[derive(Component)]
struct HealTint;
const HEAL_FADE: f32 = 0.5;

/// Seconds the hit flash and arrows take to fade.
const HIT_FADE: f32 = 1.0;

#[derive(Component)]
struct HitArrow {
    from: Vec3,
    age: f32,
}

#[derive(Resource)]
struct HudImages {
    arrow: Handle<Image>,
}

#[derive(Resource, Default)]
struct Flash(f32);

fn rgba_image(w: u32, h: u32, f: impl Fn(f32, f32) -> f32) -> Image {
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let a = f((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32).clamp(0.0, 1.0);
            data.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
        }
    }
    Image::new(
        Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn spawn_hud(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.init_resource::<Flash>();
    // a curved chevron pointing up
    let arrow = images.add(rgba_image(96, 48, |u, v| {
        let x = (u - 0.5) * 2.0;
        let y = 1.0 - v;
        let curve = 0.15 + 0.55 * (1.0 - x * x);
        let thick = 0.22 * (1.0 - x.abs()).max(0.0);
        if (y - curve).abs() < thick { 1.0 } else { 0.0 }
    }));
    commands.insert_resource(HudImages { arrow });

    commands.spawn((
        Vignette,
        vignette(0.0),
        Node { position_type: PositionType::Absolute, width: percent(100.0), height: percent(100.0), ..default() },
        GlobalZIndex(-1),
        Pickable::IGNORE,
    ));
    commands.spawn((
        HealTint,
        heal_tint(0.0),
        Node { position_type: PositionType::Absolute, width: percent(100.0), height: percent(100.0), ..default() },
        GlobalZIndex(-1),
        Pickable::IGNORE,
    ));

}

/// Red edges: a steady part from lost health plus a flash on each hit.
fn damage_flash(
    time: Res<Time>,
    stats: Res<PlayerStats>,
    mut hits: MessageReader<PlayerHit>,
    mut flash: ResMut<Flash>,
    mut edges: Single<&mut BackgroundGradient, With<Vignette>>,
    mut commands: Commands,
    images: Res<HudImages>,
) {
    for h in hits.read() {
        flash.0 = 1.0;
        commands.spawn((
            HitArrow { from: h.from, age: 0.0 },
            ImageNode::new(images.arrow.clone()).with_color(Color::srgba(0.85, 0.0, 0.0, 0.9)),
            Node { position_type: PositionType::Absolute, width: px(96.0), height: px(48.0), ..default() },
            GlobalZIndex(5),
            Pickable::IGNORE,
        ));
    }
    flash.0 = (flash.0 - time.delta_secs() / HIT_FADE).max(0.0);
    let lost = 1.0 - (stats.health / PLAYER_MAX_HEALTH).clamp(0.0, 1.0);
    let alpha = (lost * 0.55 + flash.0 * 0.35).min(0.8);
    **edges = vignette(alpha);
}

/// Transparent in the middle, red at the edges (a CSS-style radial gradient).
fn vignette(alpha: f32) -> BackgroundGradient {
    let red = |a: f32| Color::srgba(0.85, 0.0, 0.0, a);
    BackgroundGradient::from(RadialGradient::new(
        UiPosition::CENTER,
        RadialGradientShape::FarthestCorner,
        vec![ColorStop::new(red(0.0), percent(60.0)), ColorStop::new(red(alpha * 0.4), percent(82.0)), ColorStop::new(red(alpha), percent(100.0))],
    ))
}

/// Arrows sit on a circle around the crosshair, on the side the hit came from
/// (top = in front of you), and fade out.
fn hit_arrows(
    mut commands: Commands,
    time: Res<Time>,
    window: Single<&Window>,
    camera: Single<&GlobalTransform, With<crate::camera::PlayerCamera>>,
    mut arrows: Query<(Entity, &mut HitArrow, &mut Node, &mut UiTransform, &mut ImageNode)>,
) {
    let centre = Vec2::new(window.width(), window.height()) / 2.0;
    let fwd = camera.forward().as_vec3().with_y(0.0).normalize_or_zero();
    let right = camera.right().as_vec3().with_y(0.0).normalize_or_zero();
    for (e, mut a, mut node, mut tf, mut img) in &mut arrows {
        a.age += time.delta_secs();
        if a.age >= HIT_FADE {
            commands.entity(e).despawn();
            continue;
        }
        let to = (a.from - camera.translation()).with_y(0.0);
        // clockwise angle from straight ahead
        let ang = to.dot(right).atan2(to.dot(fwd));
        let r = 70.0;
        let pos = centre + Vec2::new(ang.sin(), -ang.cos()) * r;
        node.left = px(pos.x - 48.0);
        node.top = px(pos.y - 24.0);
        tf.rotation = Rot2::radians(ang);
        img.color.set_alpha(0.9 * (1.0 - a.age / HIT_FADE));
    }
}

fn heal_flash(
    time: Res<Time>,
    stats: Res<PlayerStats>,
    mut prev: Local<Option<f32>>,
    mut left: Local<f32>,
    mut tint: Single<&mut BackgroundGradient, With<HealTint>>,
) {
    if prev.is_some_and(|p| stats.health > p + 0.01) {
        *left = HEAL_FADE;
    }
    *prev = Some(stats.health);
    if *left <= 0.0 {
        return;
    }
    *left = (*left - time.delta_secs()).max(0.0);
    // rises quickly, then fades
    let t = 1.0 - *left / HEAL_FADE;
    let a = if t < 0.2 { t / 0.2 } else { (1.0 - t) / 0.8 };
    **tint = heal_tint(a * 0.55);
}

fn heal_tint(alpha: f32) -> BackgroundGradient {
    let green = |a: f32| Color::srgba(0.1, 0.8, 0.1, a);
    BackgroundGradient::from(RadialGradient::new(
        UiPosition::CENTER,
        RadialGradientShape::FarthestCorner,
        vec![ColorStop::new(green(alpha * 0.25), percent(0.0)), ColorStop::new(green(alpha), percent(100.0))],
    ))
}
