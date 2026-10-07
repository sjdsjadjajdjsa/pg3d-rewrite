//! F12 saves a screenshot to screenshots/.
//! PG3D_AUTOSHOT=file.png takes one screenshot after the map has loaded, then quits
//! (used for automated checks). PG3D_AUTOSHOT_TIME=seconds picks a time instead.

use bevy::{
    app::AppExit,
    diagnostic::FrameCount,
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
};

pub struct ScreenshotPlugin;

impl Plugin for ScreenshotPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (manual_shot, auto_shot));
    }
}

fn manual_shot(mut commands: Commands, keys: Res<ButtonInput<KeyCode>>, mut n: Local<u32>) {
    if keys.just_pressed(KeyCode::F12) {
        let _ = std::fs::create_dir_all("screenshots");
        let path = format!("screenshots/shot-{}.png", *n);
        *n += 1;
        info!("saving {path}");
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    }
}

fn auto_shot(
    mut commands: Commands,
    frames: Res<FrameCount>,
    time: Res<Time<Real>>,
    mut exit: MessageWriter<AppExit>,
    mut taken: Local<Option<u32>>,
) {
    let Ok(path) = std::env::var("PG3D_AUTOSHOT") else { return };
    // PG3D_AUTOSHOT_TIME=seconds: shoot at that (real) time instead of a frame number
    if let Some(at) = std::env::var("PG3D_AUTOSHOT_TIME").ok().and_then(|v| v.parse::<f32>().ok()) {
        match *taken {
            None if time.elapsed_secs() >= at => {
                commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
                *taken = Some(frames.0);
            }
            Some(f) if frames.0 == f + 60 => {
                exit.write(AppExit::Success);
            }
            _ => {}
        }
        return;
    }
    // PG3D_AUTOSHOT_FRAME picks the frame (default 300); quit 60 frames later
    let at: u32 = std::env::var("PG3D_AUTOSHOT_FRAME").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
    match frames.0 {
        f if f == at => {
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
        }
        f if f == at + 60 => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}
