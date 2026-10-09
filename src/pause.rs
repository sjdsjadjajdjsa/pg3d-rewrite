//! In-match pause (Esc or the pause button): the original Menu (InGameGUI/PausePanel)
//! over the darkened match, with Resume, Settings and Quit (back to the lobby). Settings
//! opens the original pause settings window (SettingsPanel); its back arrow and Esc return
//! to the Menu. The game world stops while either is open. F10 still leaves to the lobby.

#[allow(unused_imports)]
use crate::scene::{FixedUpdate, PostStartup, PostUpdate, PreStartup, Startup, Update};
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

impl crate::scene::ScenePlugin for PausePlugin {
    fn build(&self, app: &mut crate::scene::SceneBuilder) {
        app.init_resource::<Paused>()
            .add_systems(Startup, spawn_pause_screen)
            .add_plugins(crate::settings_ui::SettingsUiPlugin)
            .add_systems(Update, (setup, toggle, hide_while_paused).chain());
    }
}

fn spawn_pause_screen(mut commands: Commands, assets: Res<AssetServer>, data: Res<DataDir>) {
    let e = crate::ngui::spawn_screen(&mut commands, &assets, &data, "pause");
    if let Some(e) = e {
        commands.entity(e).insert(Visibility::Hidden);
    }
    commands.insert_resource(PauseScreen(e));
}

fn setup(
    mut screens: Query<&mut NguiScreen, Added<NguiScreen>>,
    settings: Res<crate::settings::Settings>,
    controls: Res<crate::controls::Controls>,
) {
    for mut s in &mut screens {
        match s.nodes.first().map(|n| n.name.as_str()) {
            Some("SettingsPanel") => {
                crate::settings_ui::setup_settings(&mut s, &settings, &controls);
                // the window's own tab bar and its "Settings" header are drawn by the pause panel
                for i in 0..s.nodes.len() {
                    if s.nodes[i].name == "GamepadSettingsButton" {
                        s.set_active(i, false);
                    }
                }
            }
            Some("InGameGUI") => setup_menu(&mut s),
            _ => {}
        }
    }
}

/// The in-match "Menu" (InGameGUI/PausePanel, PauseGUIController): Resume, Settings, Quit.
/// Its other parts (private match rules, event tips, passes, duel warning) stay off, and so
/// does Armory, which we don't have; UIGrid stacks the three buttons 80 apart, centred.
fn setup_menu(s: &mut NguiScreen) {
    let Some(p) = s.find("PausePanel") else { return };
    for i in s.subtree(p) {
        if s.nodes[i].parent == Some(p) && !["darken", "ButtonsWidget"].contains(&s.nodes[i].name.as_str()) {
            s.set_active(i, false);
        }
    }
    let Some(grid) = s.find_in(p, "Grid") else { return };
    let mut y = 80.0;
    for name in ["BtnResume", "BtnTasks", "BtnArsenal", "BtnSettings", "BtnQuit", "BtnFinish"] {
        let Some(b) = s.find_in(grid, name) else { continue };
        let on = ["BtnResume", "BtnSettings", "BtnQuit"].contains(&name);
        s.set_active(b, on);
        if on {
            s.set_pos(b, Vec2::new(s.nodes[b].pos[0], y));
            y -= 80.0;
        }
    }
}

/// What the pause screen shows: the Menu, or the settings window opened from it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Playing,
    Menu,
    Settings,
}

#[allow(clippy::too_many_arguments)]
fn show(
    view: View,
    paused: &mut Paused,
    time: &mut Time<Virtual>,
    cursor: &mut CursorOptions,
    commands: &mut Commands,
    screen: &PauseScreen,
    screens: &mut Query<&mut NguiScreen>,
    current: &mut View,
) {
    *current = view;
    paused.0 = view != View::Playing;
    if paused.0 {
        time.pause();
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    } else {
        time.unpause();
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
    if let Some(e) = screen.0 {
        commands.entity(e).insert(if view == View::Settings { Visibility::Inherited } else { Visibility::Hidden });
    }
    for mut s in screens.iter_mut() {
        if s.nodes.first().is_some_and(|n| n.name == "InGameGUI") {
            if let Some(p) = s.find("PausePanel") {
                s.set_active(p, view == View::Menu);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mut paused: ResMut<Paused>,
    mut time: ResMut<Time<Virtual>>,
    mut cursor: Single<&mut CursorOptions>,
    mut commands: Commands,
    screen: Res<PauseScreen>,
    over: Res<crate::results::MatchOver>,
    rebinding: Res<crate::settings_ui::Rebinding>,
    frame: Res<bevy::diagnostic::FrameCount>,
    mut screens: Query<&mut NguiScreen>,
    mut current: Local<Option<View>>,
    mut clicks: MessageReader<crate::ngui::NguiClick>,

    mut tested: Local<bool>,
) {
    let cur = current.get_or_insert(View::Playing);
    if over.0 || rebinding.busy(&frame) {
        clicks.clear();
        return; // the results screen is up / Esc belongs to the key binding popup
    }
    // PG3D_PAUSE=1|settings (automated tests): open the Menu (or settings) after 2 seconds
    let test = std::env::var("PG3D_PAUSE").ok().filter(|_| !*tested && time.elapsed_secs() > 2.0);
    *tested |= test.is_some();
    let mut next = None;
    if let Some(t) = test {
        next = Some(if t == "settings" { View::Settings } else { View::Menu });
    }
    if keys.just_pressed(KeyCode::Escape) {
        next = Some(match *cur {
            View::Playing => View::Menu,
            View::Menu => View::Playing,
            View::Settings => View::Menu,
        });
    }
    for c in clicks.read() {
        let in_settings = screen.0 == Some(c.screen);
        match c.name.as_str() {
            "Pause Button" if *cur == View::Playing => next = Some(View::Menu),
            "BtnResume" | "BtnBack" if in_settings => next = Some(View::Menu),
            "BtnResume" => next = Some(View::Playing),
            "BtnSettings" => next = Some(View::Settings),
            "BtnQuit" => {
                crate::menu::relaunch(None);
            }
            _ => {}
        }
    }
    if let Some(v) = next.filter(|v| v != cur) {
        show(v, &mut paused, &mut time, &mut cursor, &mut commands, &screen, &mut screens, cur);
    }
}

/// While paused the original shows only the top of the HUD (health, Arena box, coins) under
/// the darkening: the weapon bar, key tips, notices, crosshair and gun are hidden.
fn hide_while_paused(
    paused: Res<Paused>,
    mut screens: Query<&mut NguiScreen>,
    mut guns: Query<&mut Visibility, (With<crate::weapons::ViewModel>, Without<crate::weapons::Crosshair>)>,
    mut crosshair: Query<&mut Visibility, With<crate::weapons::Crosshair>>,
) {
    if !paused.is_changed() {
        return;
    }
    let on = !paused.0;
    for mut s in &mut screens {
        if s.nodes.first().is_none_or(|n| n.name != "InGameGUI") {
            continue;
        }
        for name in ["WeaponToggles", "BottomRightAnchor", "GadgetsSFX", "CenterAnchor SCOPE", "InGameNotifications"] {
            if let Some(i) = s.find(name) {
                s.set_active(i, on);
            }
        }
    }
    // the gun itself is hidden (changing its camera's layers or switching the camera off
    // can crash Bevy's shadow setup when it comes back)
    for mut v in &mut guns {
        *v = if on { Visibility::Inherited } else { Visibility::Hidden };
    }
    for mut v in &mut crosshair {
        *v = if on { Visibility::Inherited } else { Visibility::Hidden };
    }
}
