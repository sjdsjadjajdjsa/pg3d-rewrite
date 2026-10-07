//! Pixel Gun 3D rewrite in Rust + Bevy.
//!
//! Run:  cargo run                   (start menu)
//!       cargo run -- arena          (straight into a map = folder in <data>/maps/)
//! Data: assets are NOT part of this repo. They are exported from your own copy of the
//!       game by ../pixel-gun-3d-re/scripts/export_map.py. Override the location with
//!       PG3D_DATA=/path/to/export.
//!
//! Controls: click = capture mouse, Esc = pause, WASD move, Space jump,
//!           left mouse = fire, R = reload, 1-6 / wheel = switch weapon,
//!           V = toggle walk / fly camera, F12 = screenshot, F10 = back to menu.

mod bots;
mod camera;
mod lighting;
mod map;
mod menu;
mod ngui;
mod navmesh;
mod particles;
mod pickups;
mod player;
mod projectiles;
mod screenshot;
mod settings;
mod settings_ui;
mod pause;
mod sound;
mod weapons;

use avian3d::prelude::*;
use bevy::prelude::*;

/// Default location of the exported game data, relative to this project folder.
const DEFAULT_DATA: &str = "../pixel-gun-3d-re/private/export";

/// Where the exported data lives on disk (for files we read directly, like JSON).
#[derive(Resource)]
pub struct DataDir(pub std::path::PathBuf);

/// Automated test runs (PG3D_AUTOSHOT set): the window opens unfocused at the bottom of
/// the window stack and all real keyboard/mouse input is ignored, so tests never react to
/// (or steal) what you are typing elsewhere.
pub fn automated() -> bool {
    std::env::var("PG3D_AUTOSHOT").is_ok()
}

fn ignore_real_input(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut motion: ResMut<bevy::input::mouse::AccumulatedMouseMotion>,
    mut scroll: ResMut<bevy::input::mouse::AccumulatedMouseScroll>,
) {
    keys.reset_all();
    mouse.reset_all();
    motion.delta = Vec2::ZERO;
    scroll.delta = Vec2::ZERO;
}

fn main() {
    sound::set_sfx_volume(settings::Settings::load().sfx);
    let data_dir = std::env::var("PG3D_DATA").unwrap_or_else(|_| DEFAULT_DATA.to_string());
    let Some(map_name) = std::env::args().nth(1) else {
        // no map given: show the start menu
        App::new()
            .add_plugins(
                DefaultPlugins
                    .set(AssetPlugin { file_path: data_dir.clone(), ..default() })
                    .set(WindowPlugin {
                        primary_window: Some(Window {
                            title: "Pixel Gun 3D (Rust)".into(),
                            focused: !automated(),
                            ..default()
                        }),
                        ..default()
                    }),
            )
            .insert_resource(DataDir(data_dir.into()))
            .insert_resource(settings::Settings::load())
            .add_plugins((menu::MenuPlugin, screenshot::ScreenshotPlugin))
            .run();
        return;
    };

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
                        focused: !automated(),
                        window_level: if automated() {
                            bevy::window::WindowLevel::AlwaysOnBottom
                        } else {
                            bevy::window::WindowLevel::Normal
                        },
                        ..default()
                    }),
                    ..default()
                }),
        )
        .add_plugins(PhysicsPlugins::default())
        .insert_resource(DataDir(data_dir.into()))
            .insert_resource(settings::Settings::load())
        .insert_resource(map::CurrentMap(map_name))
        .add_plugins((
            map::MapPlugin,
            navmesh::NavMeshPlugin,
            particles::ParticlesPlugin,
            lighting::LightingPlugin,
            player::PlayerPlugin,
            camera::CameraPlugin,
            screenshot::ScreenshotPlugin,
            sound::SoundPlugin,
            weapons::WeaponsPlugin,
            bots::BotsPlugin,
            projectiles::ProjectilesPlugin,
            pickups::PickupsPlugin,
            crate::ngui::NguiPlugin,
            pause::PausePlugin,
        ))
        .add_systems(Update, menu::back_to_menu)
        .add_systems(
            PreUpdate,
            ignore_real_input.after(bevy::input::InputSystems).run_if(|| automated()),
        )
        .run();
}
