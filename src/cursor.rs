//! The game's own mouse cursor: a small ring (GamepadCursor prefab, texture
//! GamepadCursor.png) instead of the system arrow whenever the cursor is free (menus,
//! pause, results). It is a real system cursor, so it moves without any delay; the ring
//! picture is scaled to the window height once and again when the window is resized.

use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    window::{CursorIcon, CustomCursor, CustomCursorImage, PrimaryWindow},
};

pub struct CursorPlugin;

impl Plugin for CursorPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load).add_systems(Update, (apply, log_fps));
    }
}

/// Picture size at a 720 px tall window. OBSERVED: the ring is 34 px across in 1280x720
/// screenshots and fills 240 of the texture's 256 pixels.
const SIZE_720: f32 = 34.0 * 256.0 / 240.0;

#[derive(Resource)]
struct RingSource(Handle<Image>);

fn load(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(RingSource(assets.load("ui/cursor/textures/GamepadCursor_1a20.png")));
}

/// Box-filter `src` (RGBA8) down to `n` x `n`.
fn shrink(src: &Image, n: u32) -> Option<Image> {
    let data = src.data.as_ref()?;
    let (w, h) = (src.width(), src.height());
    if data.len() < (w * h * 4) as usize || n == 0 {
        return None;
    }
    let mut out = vec![0u8; (n * n * 4) as usize];
    for y in 0..n {
        for x in 0..n {
            let (x0, x1) = (x * w / n, ((x + 1) * w / n).max(x * w / n + 1));
            let (y0, y1) = (y * h / n, ((y + 1) * h / n).max(y * h / n + 1));
            let mut acc = [0u32; 4];
            let mut cnt = 0u32;
            for sy in y0..y1.min(h) {
                for sx in x0..x1.min(w) {
                    let i = ((sy * w + sx) * 4) as usize;
                    let a = data[i + 3] as u32;
                    // premultiply so transparent texels don't darken the edge
                    for c in 0..3 {
                        acc[c] += data[i + c] as u32 * a;
                    }
                    acc[3] += a;
                    cnt += 1;
                }
            }
            let o = ((y * n + x) * 4) as usize;
            for c in 0..3 {
                out[o + c] = acc[c].checked_div(acc[3]).unwrap_or(0) as u8;
            }
            out[o + 3] = (acc[3] / cnt.max(1)) as u8;
        }
    }
    Some(Image::new(
        Extent3d { width: n, height: n, depth_or_array_layers: 1 },
        TextureDimension::D2,
        out,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::all(),
    ))
}

fn apply(
    mut commands: Commands,
    source: Option<Res<RingSource>>,
    mut images: ResMut<Assets<Image>>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    mut current: Local<u32>,
) {
    let Some(source) = source else { return };
    if std::env::var("PG3D_PLAIN_CURSOR").is_ok() {
        return;
    }
    let Ok((e, window)) = windows.single() else { return };
    let n = ((SIZE_720 * window.height() / 720.0).round() as u32).clamp(16, 128);
    if *current == n {
        return;
    }
    let Some(src) = images.get(&source.0) else { return };
    let Some(img) = shrink(src, n) else { return };
    *current = n;
    let handle = images.add(img);
    commands.entity(e).insert(CursorIcon::Custom(CustomCursor::Image(CustomCursorImage {
        handle,
        hotspot: (n as u16 / 2, n as u16 / 2),
        ..default()
    })));
}

/// PG3D_FPS=1: log frames per second every two seconds (finding slow screens).
fn log_fps(time: Res<Time<Real>>, mut acc: Local<(f32, u32)>) {
    if std::env::var("PG3D_FPS").is_err() {
        return;
    }
    acc.0 += time.delta_secs();
    acc.1 += 1;
    if acc.0 >= 2.0 {
        info!("fps {:.0} (frame {:.1} ms)", acc.1 as f32 / acc.0, acc.0 * 1000.0 / acc.1 as f32);
        *acc = (0.0, 0);
    }
}
