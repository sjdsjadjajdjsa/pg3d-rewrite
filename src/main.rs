//! Pixel Gun 3D rewrite in Rust + Bevy.
//!
//! Run:  cargo run                   (start menu; menu and matches share one window)
//!       cargo run -- arena          (straight into a map = folder in <data>/maps/)
//!       cargo run -- --setup        (import the game's files again; also shown on the
//!                                    first start, when nothing is exported yet;
//!                                    PG3D_SETUP_AUTO=1 imports and starts without clicks)
//! Data: assets are NOT part of this repo. They are exported from your own copy of the
//!       game by ../pixel-gun-3d-re/scripts/export_map.py. Override the location with
//!       PG3D_DATA=/path/to/export.
//!
//! Controls: click = capture mouse, Esc = pause, WASD move, Space jump,
//!           left mouse = fire, R = reload, 1-6 / wheel = switch weapon,
//!           V = toggle walk / fly camera, F12 = screenshot, F10 = back to menu.

mod feel;
mod graphics;
mod animator;
mod armory;
mod armory_preview;
mod beams;
mod bots;
mod camera;
mod controls;
mod cursor;
mod death_fx;
mod explosives;
mod hud;
mod ingame_ui;
mod lighting;
mod map;
mod menu;
mod mirrored_skins;
mod mode_select;
mod ngui;
mod navmesh;
mod particles;
mod plain_hud;
mod pickups;
mod player;
mod projectiles;
mod progress;
mod results;
mod team_match;
mod tdm_bots;
mod scene;
mod screenshot;
mod settings;
mod settings_ui;
mod setup;
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
    // a crash leaves its message and backtrace in ~/.config/pixel-gun-3d-rs/crash.log
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let dir = settings::Settings::config_dir();
        let _ = std::fs::create_dir_all(&dir);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let thread = std::thread::current().name().unwrap_or("?").to_string();
        let text = format!("--- crash at unix {stamp} (thread {thread})\n{info}\n{bt}\n");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("crash.log")) {
            use std::io::Write;
            let _ = f.write_all(text.as_bytes());
        }
        default_hook(info);
    }));
    sound::set_sfx_volume(settings::Settings::load().sfx_volume());
    // absolute: Bevy resolves a relative asset folder against the executable's folder when
    // not started by cargo, while our own file reads resolve it against the working directory
    let data_dir = std::env::var("PG3D_DATA").unwrap_or_else(|_| DEFAULT_DATA.to_string());
    let data_dir = std::path::absolute(&data_dir).map_or(data_dir, |p| p.to_string_lossy().into_owned());
    // a map name starts straight in a match, else the menu; both run in this one window
    // and switch between each other (scene.rs)
    let first = match std::env::args().nth(1) {
        Some(arg) if arg == "--setup" => scene::Scene::Setup,
        None if !setup::has_data(std::path::Path::new(&data_dir)) => scene::Scene::Setup,
        Some(map) => scene::Scene::Game {
            map,
            mode: std::env::var("PG3D_MODE").ok(),
            sandbox: std::env::var("PG3D_NOBOTS").is_ok(),
        },
        None => scene::Scene::Menu,
    };
    scene::request(first);
    let mut app = App::new();
    // automated runs sit unfocused behind other windows, where Bevy would throttle to 60 Hz:
    // keep them running flat out so frame rates measured in tests are real
    if automated() {
        app.insert_resource(bevy::winit::WinitSettings::continuous());
    }
    app.set_error_handler(bevy::ecs::error::warn)
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    // Bevy loads every asset path relative to this folder.
                    file_path: data_dir.clone(),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Pixel Gun 3D (Rust)".into(),
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
        .insert_resource(map::CurrentMap("lobby".into()))
        .insert_resource(scene::SceneSets { setup: setup_scene, menu: menu_scene, game: game_scene })
        // messages every scene shares
        .add_message::<beams::SpawnBeam>()
        .add_message::<bots::BotDamage>()
        .add_message::<bots::PlayerHit>()
        .add_message::<death_fx::SpawnAngel>()
        .add_message::<particles::Burst>()
        .add_message::<projectiles::SpawnProjectile>()
        .add_message::<feel::ShotKick>()
        .add_message::<feel::HitMark>()
        .add_message::<feel::ShotImpact>()
        .add_plugins(scene::ScenesPlugin)
        .add_systems(Startup, sound::load_game_sounds)
        .add_systems(Update, (sound::ui_click_sounds, graphics::apply).after(scene::run_update))
        .add_systems(Last, graphics::limit_fps)
        .add_systems(
            PreUpdate,
            ignore_real_input.after(bevy::input::InputSystems).run_if(automated),
        );
    ngui::register(&mut app);
    app.run();
}

/// Importing the game's files from the player's own install (first start, --setup).
fn setup_scene(app: &mut scene::SceneBuilder) {
    app.add_plugins((setup::SetupPlugin, screenshot::ScreenshotPlugin));
}

/// The start menu (lobby, Armory, Settings, mode picker).
fn menu_scene(app: &mut scene::SceneBuilder) {
    app.add_plugins((menu::MenuPlugin, screenshot::ScreenshotPlugin, cursor::CursorPlugin, armory::ArmoryPlugin, mirrored_skins::MirroredSkinsPlugin));
}

/// A match on CurrentMap.
fn game_scene(app: &mut scene::SceneBuilder) {
    app.add_plugins((
        map::MapPlugin,
        navmesh::NavMeshPlugin,
        particles::ParticlesPlugin,
        lighting::LightingPlugin,
        player::PlayerPlugin,
    ))
    .add_plugins((camera::CameraPlugin, screenshot::ScreenshotPlugin, sound::SoundPlugin, weapons::WeaponsPlugin, bots::BotsPlugin))
    .add_plugins((projectiles::ProjectilesPlugin, pickups::PickupsPlugin, ngui::NguiPlugin, pause::PausePlugin, results::ResultsPlugin))
    .add_plugins((team_match::TeamMatchPlugin, tdm_bots::TdmBotsPlugin, mirrored_skins::MirroredSkinsPlugin, death_fx::DeathFxPlugin))
    .add_plugins((hud::HudPlugin, ingame_ui::InGameUiPlugin, explosives::ExplosivesPlugin, cursor::CursorPlugin, beams::BeamsPlugin))
    .add_plugins((feel::FeelPlugin, plain_hud::PlainHudPlugin))
    .add_systems(scene::Update, (menu::back_to_menu, menu::test_click, hide_hud));
}

/// PG3D_NOHUD=1 (map pictures for the mode picker): no HUD and no gun in view.
fn hide_hud(
    mut commands: Commands,
    roots: Query<Entity, (With<Node>, Without<ChildOf>)>,
    guns: Query<Entity, With<weapons::ViewModel>>,
) {
    if std::env::var("PG3D_NOHUD").is_err() {
        return;
    }
    for e in roots.iter().chain(guns.iter()) {
        commands.entity(e).insert(Visibility::Hidden);
    }
}
