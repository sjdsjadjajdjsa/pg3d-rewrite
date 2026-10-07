//! End of a Campaign match: when the player dies, the original survival results screen
//! (LevelComplete scene, "YOU ARE ZOMBIFIED!") shows waves survived, monsters killed,
//! score and best score. Retry restarts the map, Menu / Quit go back to the lobby.
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

impl Plugin for ResultsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MatchOver>()
            .add_systems(Startup, spawn_screen)
            .add_systems(Update, (setup, end_match, clicks));
    }
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
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| ".".into());
    base.join("pixel-gun-3d-rs/best_scores.json")
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
    stats: Res<PlayerStats>,
    wave: Res<Wave>,
    map: Res<CurrentMap>,
    mut over: ResMut<MatchOver>,
    screen: Res<ResultsScreen>,
    mut screens: Query<&mut NguiScreen>,
    mut time: ResMut<Time<Virtual>>,
    mut cursor: Single<&mut CursorOptions>,
    mut commands: Commands,
    hud: Query<Entity, (With<Node>, Without<ChildOf>, Without<NguiScreen>)>,
) {
    if over.0 || stats.health > 0.0 || std::env::var("PG3D_NOBOTS").is_ok() {
        return;
    }
    let Some(e) = screen.0 else { return };
    over.0 = true;
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
