//! Pixel Gun 3D rewrite in Rust + Bevy.
//!
//! Run:  cargo run -- arena          (map name = folder in <data>/maps/)
//! Data: assets are NOT part of this repo. They are exported from your own copy of the
//!       game by ../pixel-gun-3d-re/scripts/export_map.py. Override the location with
//!       PG3D_DATA=/path/to/export.
//!
//! Controls: click = capture mouse, Esc = release, WASD move, Space jump,
//!           left mouse = fire, R = reload, 1-6 / wheel = switch weapon,
//!           V = toggle walk / fly camera, F12 = screenshot.

mod camera;
mod lighting;
mod map;
mod player;
mod screenshot;
mod weapons;

use avian3d::prelude::*;
use bevy::prelude::*;

/// Default location of the exported game data, relative to this project folder.
const DEFAULT_DATA: &str = "../pixel-gun-3d-re/private/export";

/// Where the exported data lives on disk (for files we read directly, like JSON).
#[derive(Resource)]
pub struct DataDir(pub std::path::PathBuf);

fn main() {
    let map_name = std::env::args().nth(1).unwrap_or_else(|| "arena".to_string());
    let data_dir = std::env::var("PG3D_DATA").unwrap_or_else(|_| DEFAULT_DATA.to_string());

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    // Bevy loads every asset path relative to this folder.
                    file_path: data_dir.clone(),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: format!("Pixel Gun 3D (Rust) - {map_name}"),
                        ..default()
                    }),
                    ..default()
                }),
        )
        .add_plugins(PhysicsPlugins::default())
        .insert_resource(DataDir(data_dir.into()))
        .insert_resource(map::CurrentMap(map_name))
        .add_plugins((
            map::MapPlugin,
            lighting::LightingPlugin,
            player::PlayerPlugin,
            camera::CameraPlugin,
            screenshot::ScreenshotPlugin,
            weapons::WeaponsPlugin,
        ))
        .run();
}
