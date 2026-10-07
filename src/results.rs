//! End of an Arena / Campaign match, as in the original (reference clips in
//! ../pixel-gun-3d-re/private/reference/): at zero health the game cuts straight to a
//! "Loading" screen over the blurred coliseum (LOADING_TIME), then the Results table
//! (waves survived, monsters killed, score, best score) with a green "Again" button and
//! the yellow back arrow. No death camera, no "You are zombified!" banner.
//!
//! UI: <data>/ui/results from `export_ngui.py level6 "UI Root (2D)" results`. That scene
//! holds several end screens; we switch on what the current game shows (ArenaResults,
//! BackgroundSurvivalTexture, MiniGameButton = "Again", QuitButton).

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

/// Seconds the "Loading" screen shows before the table (OBSERVED: about 0.5-1 s).
const LOADING_TIME: f32 = 0.8;

/// Seconds since death while the loading screen is up.
#[derive(Resource, Default)]
struct Loading(Option<f32>);

#[derive(Component)]
struct LoadingLabel;

/// Parts of the screen that appear only after "Loading".
const TABLE_PARTS: [&str; 3] = ["ArenaResults", "BottomLine", "BottomLeftCorner"];

impl Plugin for ResultsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MatchOver>()
            .init_resource::<Loading>()
            .add_systems(Startup, spawn_screen)
            .add_systems(Update, (test_die, setup, death, end_match, clicks).chain());
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

fn spawn_screen(mut commands: Commands, assets: Res<AssetServer>, data: Res<DataDir>) {
    let e = crate::ngui::spawn_screen(&mut commands, &assets, &data, "results");
    if let Some(e) = e {
        commands.entity(e).insert(Visibility::Hidden);
    }
    commands.insert_resource(ResultsScreen(e));
}

/// Put the screen into the state the current game shows after an Arena match.
fn setup(mut screens: Query<&mut NguiScreen, Added<NguiScreen>>) {
    for mut s in &mut screens {
        if s.nodes.first().is_none_or(|n| n.name != "UI Root (2D)") {
            continue;
        }
        for i in 0..s.nodes.len() {
            match s.nodes[i].name.as_str() {
                "BackgroundSurvivalTexture" | "MiniGameButton" => s.set_active(i, true),
                "GameOverSprite" | "NextImageButton" | "MenuImageButton" | "RetryImageButton" | "MiniGame_NextMatchPrice"
                | "CampaingWindowNGUI" | "ArenaFinishNGUI" | "Facebook" | "Twitter" | "ShareButton" | "ShareTogle" => {
                    s.set_active(i, false)
                }
                _ => {}
            }
        }
    }
}

fn show_table(s: &mut NguiScreen, on: bool) {
    for i in 0..s.nodes.len() {
        if TABLE_PARTS.contains(&s.nodes[i].name.as_str()) {
            s.set_active(i, on);
        }
    }
}

/// At zero health: hide the HUD and weapon, release the mouse and show "Loading".
#[allow(clippy::too_many_arguments)]
fn death(
    mut commands: Commands,
    time: Res<Time<Real>>,
    stats: Res<PlayerStats>,
    over: Res<MatchOver>,
    mut loading: ResMut<Loading>,
    screen: Res<ResultsScreen>,
    mut screens: Query<&mut NguiScreen>,
    mut cursor: Single<&mut CursorOptions>,
    hud: Query<Entity, (With<Node>, Without<ChildOf>, Without<NguiScreen>, Without<LoadingLabel>)>,
    view_models: Query<Entity, With<crate::weapons::ViewModel>>,
    mut vtime: ResMut<Time<Virtual>>,
) {
    if over.0 || stats.health > 0.0 || !campaign() {
        return;
    }
    let Some(e) = screen.0 else { return };
    if let Some(t) = loading.0.as_mut() {
        *t += time.delta_secs();
        return;
    }
    info!("player died: loading screen");
    loading.0 = Some(0.0);
    if let Ok(mut s) = screens.get_mut(e) {
        show_table(&mut s, false);
    }
    commands.entity(e).insert(Visibility::Inherited);
    commands.spawn((
        LoadingLabel,
        Node {
            position_type: PositionType::Absolute,
            width: percent(100.0),
            top: percent(52.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        GlobalZIndex(200_000),
        children![(
            Text::new("Loading"),
            TextFont { font_size: FontSize::Px(26.0), ..default() },
            TextColor(Color::srgb(1.0, 0.85, 0.2)),
            TextShadow { offset: Vec2::splat(2.0), color: Color::BLACK },
        )],
    ));
    for h in &hud {
        commands.entity(h).insert(Visibility::Hidden);
    }
    for v in &view_models {
        commands.entity(v).insert(Visibility::Hidden);
    }
    vtime.pause();
    cursor.grab_mode = CursorGrabMode::None;
    cursor.visible = true;
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
    mut commands: Commands,
    loading: Res<Loading>,
    stats: Res<PlayerStats>,
    wave: Res<Wave>,
    map: Res<CurrentMap>,
    mut over: ResMut<MatchOver>,
    screen: Res<ResultsScreen>,
    mut screens: Query<&mut NguiScreen>,
    labels: Query<Entity, With<LoadingLabel>>,
) {
    if over.0 || loading.0.is_none_or(|t| t < LOADING_TIME) {
        return;
    }
    let Some(e) = screen.0 else { return };
    over.0 = true;
    for l in &labels {
        commands.entity(l).despawn();
    }
    let survived = wave.number.saturating_sub(1);
    let best = update_best(&map.0, stats.score);
    info!("match over: waves survived {survived}, kills {}, score {}, best {best}", stats.kills, stats.score);
    if let Ok(mut s) = screens.get_mut(e) {
        show_table(&mut s, true);
        let values = [
            ("WavesVal", survived.to_string()),
            ("MonstersVal", stats.kills.to_string()),
            ("ScoreVal", group(stats.score)),
            ("BEST SCORE VAL", group(best)),
        ];
        for (name, v) in values {
            if let Some(i) = s.nodes.iter().position(|n| n.name == name) {
                s.set_text(i, v);
            }
        }
    }
}

/// 18080 -> "18 080", as the original writes scores.
pub fn group(n: i32) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
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
            "MiniGameButton" | "RetryImageButton" => {
                crate::menu::relaunch_with(Some(&map.0), false);
                exit.write(AppExit::Success);
            }
            "QuitButton" | "MenuImageButton" => {
                crate::menu::relaunch(None);
                exit.write(AppExit::Success);
            }
            _ => {}
        }
    }
}
