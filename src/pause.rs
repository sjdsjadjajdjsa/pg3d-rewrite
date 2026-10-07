//! In-match pause screen (Esc): the original pause panel (SettingsPanel), which is the
//! settings window plus a Resume button. The game world stops while it is open.
//! Resume / the back arrow / Esc continue; F10 still leaves to the menu.

use bevy::{
    prelude::*,
    window::{CursorGrabMode, CursorOptions},
};

use crate::{DataDir, ngui::NguiScreen};

pub struct PausePlugin;

/// True while the pause screen is open.
#[derive(Resource, Default)]
pub struct Paused(pub bool);

#[derive(Resource)]
struct PauseScreen(Option<Entity>);

impl Plugin for PausePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Paused>()
            .add_systems(Startup, spawn_pause_screen)
            .add_systems(Update, (setup, toggle, clicks, crate::settings_ui::settings_sliders));
    }
}

fn spawn_pause_screen(mut commands: Commands, assets: Res<AssetServer>, data: Res<DataDir>) {
    let e = crate::ngui::spawn_screen(&mut commands, &assets, &data, "pause");
    if let Some(e) = e {
        commands.entity(e).insert(Visibility::Hidden);
    }
    commands.insert_resource(PauseScreen(e));
}

fn setup(mut screens: Query<&mut NguiScreen, Added<NguiScreen>>, settings: Res<crate::settings::Settings>) {
    for mut s in &mut screens {
        crate::settings_ui::setup_settings(&mut s, &settings);
        // the window's own tab bar and its "Settings" header are drawn by the pause panel
        for i in 0..s.nodes.len() {
            if s.nodes[i].name == "GamepadSettingsButton" {
                s.set_active(i, false);
            }
        }
    }
}

fn set_paused(
    on: bool,
    paused: &mut Paused,
    time: &mut Time<Virtual>,
    cursor: &mut CursorOptions,
    commands: &mut Commands,
    screen: &PauseScreen,
) {
    paused.0 = on;
    if on {
        time.pause();
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    } else {
        time.unpause();
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
    if let Some(e) = screen.0 {
        commands.entity(e).insert(if on { Visibility::Inherited } else { Visibility::Hidden });
    }
}

fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mut paused: ResMut<Paused>,
    mut time: ResMut<Time<Virtual>>,
    mut cursor: Single<&mut CursorOptions>,
    mut commands: Commands,
    screen: Res<PauseScreen>,
    over: Res<crate::results::MatchOver>,
) {
    // PG3D_PAUSE=1 (automated tests): open the pause screen after 2 seconds
    let test = std::env::var("PG3D_PAUSE").is_ok() && !paused.0 && time.elapsed_secs() > 2.0;
    if over.0 {
        return; // the results screen is up
    }
    if keys.just_pressed(KeyCode::Escape) || test {
        let on = !paused.0;
        set_paused(on, &mut paused, &mut time, &mut cursor, &mut commands, &screen);
    }
}

fn clicks(
    mut clicks: MessageReader<crate::ngui::NguiClick>,
    mut paused: ResMut<Paused>,
    mut time: ResMut<Time<Virtual>>,
    mut cursor: Single<&mut CursorOptions>,
    mut commands: Commands,
    screen: Res<PauseScreen>,
) {
    for c in clicks.read() {
        match c.name.as_str() {
            "BtnResume" | "BtnBack" => {
                set_paused(false, &mut paused, &mut time, &mut cursor, &mut commands, &screen);
            }
            _ => {}
        }
    }
}
