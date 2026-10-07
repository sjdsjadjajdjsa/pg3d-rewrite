//! End of a Campaign match: when the player dies the camera drops to the floor and tips
//! over while the screen flashes red and fades to black (DEATH_TIME), then the original
//! survival results screen (LevelComplete scene, "YOU ARE ZOMBIFIED!") shows waves
//! survived, monsters killed, score and best score. Retry restarts the map, Menu / Quit go
//! back to the lobby.
//!
//! UI: <data>/ui/results from `export_ngui.py level6 "UI Root (2D)" results`.

use bevy::{
    app::AppExit,
    prelude::*,
    window::{CursorGrabMode, CursorOptions},
};

use crate::{
    DataDir,
    bots::{PlayerStats, Wave},
    map::CurrentMap,
    ngui::{NguiClick, NguiScreen},
};

pub struct ResultsPlugin;

/// True once the match has ended (the player stays dead, the results screen is up).
#[derive(Resource, Default)]
pub struct MatchOver(pub bool);

#[derive(Resource)]
struct ResultsScreen(Option<Entity>);

/// Seconds from death to the results screen.
const DEATH_TIME: f32 = 2.0;
/// How far the camera falls (eye height 1.7 m above the feet -> 0.3 m).
const DEATH_DROP: f32 = 1.4;
/// Sideways tilt of the fallen camera.
const DEATH_ROLL_DEG: f32 = 75.0;

/// The death sequence in progress.
#[derive(Resource, Default)]
struct Death {
    /// seconds since death, None while alive
    t: Option<f32>,
    /// camera rotation at the moment of death
    base: Quat,
    overlay: Option<Entity>,
}

#[derive(Component)]
struct DeathOverlay;

impl Plugin for ResultsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MatchOver>()
            .init_resource::<Death>()
            .add_systems(Startup, spawn_screen)
            .add_systems(Update, (test_die, setup, dying, end_match, clicks).chain())
            // after the camera systems (Update) have placed the camera for this frame
            .add_systems(PostUpdate, death_camera.before(bevy::transform::TransformSystems::Propagate));
    }
}

/// PG3D_DIE=seconds (automated tests): the player dies at that time.
fn test_die(time: Res<Time>, mut stats: ResMut<PlayerStats>, mut done: Local<bool>) {
    let Some(at) = std::env::var("PG3D_DIE").ok().and_then(|v| v.parse::<f32>().ok()) else { return };
    if !*done && time.elapsed_secs() > at {
        *done = true;
        stats.health = 0.0;
    }
}

fn campaign() -> bool {
    std::env::var("PG3D_NOBOTS").is_err()
}

/// Starts and runs the death sequence: red flash fading to black, HUD and weapon hidden.
#[allow(clippy::too_many_arguments)]
fn dying(
    mut commands: Commands,
    time: Res<Time>,
    stats: Res<PlayerStats>,
    over: Res<MatchOver>,
    mut death: ResMut<Death>,
    camera: Single<&Transform, With<crate::camera::PlayerCamera>>,
    mut cursor: Single<&mut CursorOptions>,
    hud: Query<Entity, (With<Node>, Without<ChildOf>, Without<NguiScreen>, Without<DeathOverlay>)>,
    view_models: Query<Entity, With<crate::weapons::ViewModel>>,
    mut overlays: Query<&mut BackgroundColor, With<DeathOverlay>>,
) {
    if over.0 || stats.health > 0.0 || !campaign() {
        return;
    }
    let Some(t) = death.t else {
        info!("player died: death camera");
        death.t = Some(0.0);
        death.base = camera.rotation;
        death.overlay = Some(
            commands
                .spawn((
                    DeathOverlay,
                    Node { position_type: PositionType::Absolute, width: percent(100.0), height: percent(100.0), ..default() },
                    BackgroundColor(Color::srgba(0.8, 0.0, 0.0, 0.5)),
                    GlobalZIndex(40_000),
                    Pickable::IGNORE,
                ))
                .id(),
        );
        for h in &hud {
            commands.entity(h).insert(Visibility::Hidden);
        }
        for v in &view_models {
            commands.entity(v).insert(Visibility::Hidden);
        }
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
        return;
    };
    let t = t + time.delta_secs();
    death.t = Some(t);
    // red flash -> dark red -> black
    let k = (t / DEATH_TIME).clamp(0.0, 1.0);
    let red = 0.8 * (1.0 - k);
    let alpha = 0.5 + 0.5 * k * k;
    for mut bg in &mut overlays {
        bg.0 = Color::srgba(red, 0.0, 0.0, alpha);
    }
}

/// The fallen camera: drops to the floor (fast, then slower) and tips onto its side.
fn death_camera(death: Res<Death>, mut camera: Single<&mut Transform, With<crate::camera::PlayerCamera>>) {
    let Some(t) = death.t else { return };
    let k = (t / (DEATH_TIME * 0.5)).clamp(0.0, 1.0);
    let ease = 1.0 - (1.0 - k) * (1.0 - k);
    camera.translation.y -= DEATH_DROP * ease;
    let (yaw, pitch, _) = death.base.to_euler(EulerRot::YXZ);
    // look slightly up at whoever got us, rolled sideways
    let pitch = pitch + (0.25 - pitch) * ease * 0.5;
    camera.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, DEATH_ROLL_DEG.to_radians() * ease);
}

fn spawn_screen(mut commands: Commands, assets: Res<AssetServer>, data: Res<DataDir>) {
    let e = crate::ngui::spawn_screen(&mut commands, &assets, &data, "results");
    if let Some(e) = e {
        commands.entity(e).insert(Visibility::Hidden);
    }
    commands.insert_resource(ResultsScreen(e));
}

/// Put the screen into its "survival game over" state (the game does this at run time).
fn setup(mut screens: Query<&mut NguiScreen, Added<NguiScreen>>) {
    for mut s in &mut screens {
        if s.nodes.first().is_none_or(|n| n.name != "UI Root (2D)") {
            continue;
        }
        for i in 0..s.nodes.len() {
            let name = s.nodes[i].name.clone();
            let parent = s.nodes[i].parent.map(|p| s.nodes[p].name.clone()).unwrap_or_default();
            match name.as_str() {
                "BackgroundSurvivalTexture" | "GameOverSprite" => s.set_active(i, true),
                "NextImageButton" | "MiniGameButton" | "CampaingWindowNGUI" | "ArenaFinishNGUI" | "Facebook" | "Twitter"
                | "ShareButton" | "ShareTogle" => s.set_active(i, false),
                // the campaign star objectives also live under GameOverSprite
                _ if parent == "GameOverSprite" && name != "LabelZombified" && name != "BGFooter" => {
                    s.set_active(i, false)
                }
                _ => {}
            }
        }
    }
}

fn best_score_path() -> std::path::PathBuf {
    crate::settings::Settings::config_dir().join("best_scores.json")
}

/// Best score per map, stored next to the settings.
fn update_best(map: &str, score: i32) -> i32 {
    let p = best_score_path();
    let mut all: std::collections::HashMap<String, i32> =
        std::fs::read_to_string(&p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let best = all.get(map).copied().unwrap_or(0).max(score);
    all.insert(map.to_string(), best);
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&p, serde_json::to_string_pretty(&all).unwrap_or_default());
    best
}

#[allow(clippy::too_many_arguments)]
fn end_match(
    mut death: ResMut<Death>,
    stats: Res<PlayerStats>,
    wave: Res<Wave>,
    map: Res<CurrentMap>,
    mut over: ResMut<MatchOver>,
    screen: Res<ResultsScreen>,
    mut screens: Query<&mut NguiScreen>,
    mut time: ResMut<Time<Virtual>>,
    mut cursor: Single<&mut CursorOptions>,
    mut commands: Commands,
    hud: Query<Entity, (With<Node>, Without<ChildOf>, Without<NguiScreen>, Without<DeathOverlay>)>,
) {
    if over.0 || stats.health > 0.0 || !campaign() || death.t.is_none_or(|t| t < DEATH_TIME) {
        return;
    }
    let Some(e) = screen.0 else { return };
    over.0 = true;
    if let Some(o) = death.overlay.take() {
        commands.entity(o).despawn();
    }
    let survived = wave.number.saturating_sub(1);
    let best = update_best(&map.0, stats.score);
    info!("match over: waves survived {survived}, kills {}, score {}, best {best}", stats.kills, stats.score);
    if let Ok(mut s) = screens.get_mut(e) {
        let values = [
            ("WavesVal", survived.to_string()),
            ("MonstersVal", stats.kills.to_string()),
            ("ScoreVal", stats.score.to_string()),
            ("BEST SCORE VAL", best.to_string()),
        ];
        for (name, v) in values {
            if let Some(i) = s.nodes.iter().position(|n| n.name == name) {
                s.set_text(i, v);
            }
        }
    }
    commands.entity(e).insert(Visibility::Inherited);
    // the in-game HUD (crosshair, health, ammo) is not part of the results screen
    for h in &hud {
        commands.entity(h).insert(Visibility::Hidden);
    }
    time.pause();
    cursor.grab_mode = CursorGrabMode::None;
    cursor.visible = true;
}

fn clicks(
    mut clicks: MessageReader<NguiClick>,
    over: Res<MatchOver>,
    map: Res<CurrentMap>,
    mut exit: MessageWriter<AppExit>,
) {
    if !over.0 {
        return;
    }
    for c in clicks.read() {
        match c.name.as_str() {
            "RetryImageButton" => {
                crate::menu::relaunch_with(Some(&map.0), false);
                exit.write(AppExit::Success);
            }
            "MenuImageButton" | "QuitButton" => {
                crate::menu::relaunch(None);
                exit.write(AppExit::Success);
            }
            _ => {}
        }
    }
}
