//! F12 saves a screenshot to screenshots/.
//! PG3D_AUTOSHOT=file.png takes one screenshot after the map has loaded, then quits
//! (used for automated checks).

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

fn auto_shot(mut commands: Commands, frames: Res<FrameCount>, mut exit: MessageWriter<AppExit>) {
    let Ok(path) = std::env::var("PG3D_AUTOSHOT") else { return };
    match frames.0 {
        300 => {
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
        }
        360 => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}
