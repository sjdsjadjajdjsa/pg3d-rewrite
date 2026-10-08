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

mod armory;
mod armory_preview;
mod beams;
mod bots;
mod camera;
mod controls;
mod cursor;
mod explosives;
mod hud;
mod ingame_ui;
mod lighting;
mod map;
mod menu;
mod mirrored_skins;
mod ngui;
mod navmesh;
mod particles;
mod pickups;
mod player;
mod projectiles;
mod progress;
mod results;
mod team_match;
mod screenshot;
mod settings;
mod settings_ui;
mod pause;
mod sound;
mod wallet;
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
    sound::set_sfx_volume(settings::Settings::load().sfx_volume());
    let data_dir = std::env::var("PG3D_DATA").unwrap_or_else(|_| DEFAULT_DATA.to_string());
    let Some(map_name) = std::env::args().nth(1) else {
        // no map given: show the start menu
        App::new()
            // a command on an entity that is already gone (e.g. a zombie despawned the same
            // frame) logs a warning instead of stopping the game
            .set_error_handler(bevy::ecs::error::warn)
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
            .insert_resource(controls::Controls::load())
            .insert_resource(wallet::Wallet::load())
            .add_plugins((menu::MenuPlugin, screenshot::ScreenshotPlugin, cursor::CursorPlugin, armory::ArmoryPlugin, mirrored_skins::MirroredSkinsPlugin))
            .run();
        return;
    };

    App::new()
        .set_error_handler(bevy::ecs::error::warn)
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
                        // PG3D_RES=1920x1080 (testing): exact window size, to compare with
                        // screenshots of the original
                        resolution: std::env::var("PG3D_RES")
                            .ok()
                            .and_then(|r| {
                                let (w, h) = r.split_once('x')?;
                                Some(bevy::window::WindowResolution::new(w.parse().ok()?, h.parse().ok()?).with_scale_factor_override(1.0))
                            })
                            .unwrap_or_default(),
                        decorations: std::env::var("PG3D_RES").is_err(),
                        // a desktop panel keeps a normal window shorter than the screen
                        mode: if std::env::var("PG3D_RES").is_ok_and(|r| r == "1920x1080") {
                            bevy::window::WindowMode::BorderlessFullscreen(bevy::window::MonitorSelection::Index(1))
                        } else {
                            bevy::window::WindowMode::Windowed
                        },
                        ..default()
                    }),
                    ..default()
                }),
        )
        .add_plugins(PhysicsPlugins::default())
        // PG3D_PHYSDEBUG=1: draw every collider (hit boxes, map)
        .add_plugins(PhysicsDebugPlugin)
        .insert_gizmo_config(PhysicsGizmos::default(), GizmoConfig { enabled: std::env::var("PG3D_PHYSDEBUG").is_ok(), ..default() })
        .insert_resource(DataDir(data_dir.into()))
        .insert_resource(settings::Settings::load())
        .insert_resource(controls::Controls::load())
        .insert_resource(wallet::Wallet::load())
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
            results::ResultsPlugin,
        ))
        .add_plugins((team_match::TeamMatchPlugin, mirrored_skins::MirroredSkinsPlugin))
        .add_plugins((hud::HudPlugin, ingame_ui::InGameUiPlugin, explosives::ExplosivesPlugin, cursor::CursorPlugin, beams::BeamsPlugin))
        .add_systems(Update, (menu::back_to_menu, menu::test_click))
        .add_systems(
            PreUpdate,
            ignore_real_input.after(bevy::input::InputSystems).run_if(|| automated()),
        )
        .run();
}
