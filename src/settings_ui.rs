//! The original settings window (exported NGUI screen), shared by the lobby menu and the
//! in-match pause screen. Three of its tabs work:
//!   * Settings (sliders icon, TabSetting): first-person camera sensitivity, field of view
//!     (60-90, SettingsTabSetting) and "Invert cam".
//!   * Multimedia: Music and Sound FX. Changes apply live and are saved.
//!   * Key binding (keyboard icon, TabRebind = the PC version's KeyMappingView): one row per
//!     action; click a row, press a key or mouse button. A key already in use asks first
//!     (ChangeButtonWindow_Used) and then swaps the two. "Default Settings" resets all.
//! The other tabs (touch controls, graphics, ...) are hidden, as are options we don't have.

use bevy::{diagnostic::FrameCount, prelude::*};

use crate::{
    controls::{Action, Bind, Controls},
    ngui::{NguiClick, NguiScreen},
};

pub struct SettingsUiPlugin;

impl Plugin for SettingsUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Rebinding>()
            .add_systems(Update, (settings_sliders, tab_clicks, toggle_clicks, rebind_clicks, capture_key).chain());
    }
}

/// Key binding in progress.
#[derive(Resource, Default)]
pub struct Rebinding {
    /// screen, action waiting for a key, frame the wait started
    waiting: Option<(Entity, Action, u32)>,
    /// key pressed that another action already uses: (screen, action, key, other action)
    conflict: Option<(Entity, Action, Bind, Action)>,
    /// frame the last popup closed (its Esc must not also close the pause screen)
    closed_frame: u32,
}

impl Rebinding {
    /// True while a key binding popup is open (or closed this very frame).
    pub fn busy(&self, frame: &FrameCount) -> bool {
        self.waiting.is_some() || self.conflict.is_some() || self.closed_frame == frame.0
    }
}

const TABS: [&str; 3] = ["TabSetting", "TabMultimedia", "TabRebind"];

/// Field of view range of the original's slider (SettingsTabSetting: UISliderCont 60..90).
const FOV_RANGE: (f32, f32) = (60.0, 90.0);

/// Tab buttons sit under ".../TabsTable/"; the tab pages are nodes with the same name elsewhere.
fn is_tab_button(s: &NguiScreen, i: usize) -> bool {
    s.nodes[i].parent.is_some_and(|p| s.nodes[p].name == "TabsTable")
}

fn select_tab(s: &mut NguiScreen, tab: &str) {
    for i in 0..s.nodes.len() {
        let name = s.nodes[i].name.clone();
        if !TABS.contains(&name.as_str()) {
            continue;
        }
        if is_tab_button(s, i) {
            if let Some(sel) = s.find_in(i, "SelectedBG (OFF)") {
                s.set_active(sel, name == tab);
            }
        } else if s.nodes[i].parent.is_some_and(|p| s.nodes[p].name != "TabsTable") {
            s.set_active(i, name == tab);
        }
    }
}

/// The key binding page (the gamepad tab has a second copy of the same widgets).
fn rebind_page(s: &NguiScreen) -> Option<usize> {
    (0..s.nodes.len()).find(|&i| s.nodes[i].name == "TabRebind" && !is_tab_button(s, i))
}

/// Builds the key binding page: one copy of the KeyActionView row per action.
fn setup_rebind(s: &mut NguiScreen, controls: &Controls) {
    let Some(page) = rebind_page(s) else { return };
    for name in ["ScrollBar", "KeyMappingNewButtonApplyerView (OFF)", "Default_Settings_window (OFF)"] {
        if let Some(i) = s.find_in(page, name) {
            s.set_active(i, false);
        }
    }
    // the page's single category tab
    if let Some(t) = s.find_in(page, "KeyMappingTabView") {
        for (n, on) in [("bg_selected", true), ("bg_not_selected", false)] {
            if let Some(i) = s.find_in(t, n) {
                s.set_active(i, on);
                if let Some(l) = s.find_in(i, "NameLabel") {
                    s.set_text(l, "Keyboard and mouse");
                }
            }
        }
    }
    let Some(row) = s.find_in(page, "KeyActionView") else { return };
    let mut rows = vec![row];
    for _ in 1..Action::ALL.len() {
        rows.push(s.clone_subtree(row));
    }
    let h = s.nodes[row].widget.as_ref().map_or(48.0, |w| w.h);
    for (k, &r) in rows.iter().enumerate() {
        s.set_pos(r, Vec2::new(s.nodes[row].pos[0], s.nodes[row].pos[1] - h * k as f32));
        if let Some(i) = s.find_in(r, "ButtonIconScaler") {
            s.set_active(i, false);
        }
        if let Some(i) = s.find_in(r, "ButtonLabel") {
            s.set_active(i, true);
        }
    }
    show_bindings(s, controls);
}

fn rows(s: &NguiScreen) -> Vec<usize> {
    let Some(page) = rebind_page(s) else { return vec![] };
    s.subtree(page).into_iter().filter(|&i| s.nodes[i].name == "KeyActionView").collect()
}

/// Node `name` on the key binding page.
fn find(s: &NguiScreen, name: &str) -> Option<usize> {
    s.find_in(rebind_page(s)?, name)
}

fn show_bindings(s: &mut NguiScreen, controls: &Controls) {
    for (r, a) in rows(s).into_iter().zip(Action::ALL) {
        if let Some(i) = s.find_in(r, "ActionLabel") {
            s.set_text(i, a.label());
        }
        if let Some(i) = s.find_in(r, "ButtonLabel") {
            s.set_text(i, controls.get(a).label());
        }
    }
}

fn popup(s: &mut NguiScreen, window: Option<&str>) {
    let names = ["KeyMappingNewButtonApplyerView (OFF)", "ChangeButtonWindow_Normal", "ChangeButtonWindow_Used"];
    for n in names {
        if let Some(i) = find(s, n) {
            let on = match window {
                Some(w) => n == names[0] || n == w,
                None => false,
            };
            s.set_active(i, on);
        }
    }
}

fn tab_clicks(mut clicks: MessageReader<NguiClick>, mut screens: Query<&mut NguiScreen>) {
    for c in clicks.read() {
        if !TABS.contains(&c.name.as_str()) {
            continue;
        }
        let Ok(mut s) = screens.get_mut(c.screen) else { continue };
        if is_tab_button(&s, c.node) {
            select_tab(&mut s, &c.name);
        }
    }
}

fn rebind_clicks(
    mut clicks: MessageReader<NguiClick>,
    mut screens: Query<&mut NguiScreen>,
    mut state: ResMut<Rebinding>,
    mut controls: ResMut<Controls>,
    frame: Res<FrameCount>,
) {
    for c in clicks.read() {
        let Ok(mut s) = screens.get_mut(c.screen) else { continue };
        let path = s.path(c.node);
        match c.name.as_str() {
            "KeyActionView" if state.waiting.is_none() && state.conflict.is_none() => {
                let Some(k) = rows(&s).iter().position(|&r| r == c.node) else { continue };
                state.waiting = Some((c.screen, Action::ALL[k], frame.0));
                info!("key binding: waiting for a key for {:?}", Action::ALL[k]);
                popup(&mut s, Some("ChangeButtonWindow_Normal"));
            }
            "btn_apply" | "btn_cancel" if path.contains("ChangeButtonWindow_Used") => {
                if let (Some((_, action, bind, other)), true) = (state.conflict, c.name == "btn_apply") {
                    let old = controls.get(action);
                    controls.0.insert(other, old);
                    controls.0.insert(action, bind);
                    controls.save();
                    info!("key binding: {action:?} = {bind:?}, {other:?} = {old:?}");
                }
                state.conflict = None;
                state.closed_frame = frame.0;
                popup(&mut s, None);
            }
            "btn_Default_Settings_active" => {
                if let Some(i) = find(&s, "Default_Settings_window (OFF)") {
                    s.set_active(i, true);
                }
            }
            "btn_apply" | "btn_cancel" if path.contains("Default_Settings_window") => {
                if c.name == "btn_apply" {
                    *controls = Controls::default();
                    controls.save();
                    info!("key bindings reset to defaults");
                }
                if let Some(i) = find(&s, "Default_Settings_window (OFF)") {
                    s.set_active(i, false);
                }
            }
            _ => continue,
        }
        let controls = controls.clone();
        for mut s in &mut screens {
            show_bindings(&mut s, &controls);
        }
    }
}

/// While the "press a key" popup is open: the next key or mouse button press is the new
/// binding, Esc cancels.
fn capture_key(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut screens: Query<&mut NguiScreen>,
    mut state: ResMut<Rebinding>,
    mut controls: ResMut<Controls>,
    frame: Res<FrameCount>,
) {
    let Some((screen, action, started)) = state.waiting else { return };
    // the click that opened the popup is not the new key
    if frame.0 == started {
        return;
    }
    let Ok(mut s) = screens.get_mut(screen) else { return };
    if keys.just_pressed(KeyCode::Escape) {
        state.waiting = None;
        state.closed_frame = frame.0;
        popup(&mut s, None);
        return;
    }
    // PG3D_REBIND_KEY=KeyF (automated tests): pretend that key was pressed a second later
    let fake = std::env::var("PG3D_REBIND_KEY").ok().filter(|_| frame.0 > started + 60).and_then(|k| Bind::parse(&k));
    let Some(bind) = Bind::just_pressed(&keys, &mouse).or(fake) else { return };
    state.waiting = None;
    if let Some(other) = controls.user_of(bind, action) {
        state.conflict = Some((screen, action, bind, other));
        if let Some(w) = find(&s, "ChangeButtonWindow_Used") {
            if let Some(i) = s.find_in(w, "Label_category") {
                s.set_text(i, format!("{}:", bind.label()));
            }
            if let Some(i) = s.find_in(w, "Label_action") {
                s.set_text(i, other.label());
            }
        }
        popup(&mut s, Some("ChangeButtonWindow_Used"));
        return;
    }
    controls.0.insert(action, bind);
    controls.save();
    info!("key binding: {action:?} = {bind:?}");
    state.closed_frame = frame.0;
    popup(&mut s, None);
    let controls = controls.clone();
    for mut s in &mut screens {
        show_bindings(&mut s, &controls);
    }
}

/// Marks background music players whose volume follows the Music slider.
#[derive(Component)]
pub struct Music;

/// Settings window: we use the Multimedia tab. Music and Sound FX are the original
/// sliders; the brightness slider becomes mouse sensitivity. Phone-only options are hidden.
pub fn setup_settings(s: &mut crate::ngui::NguiScreen, settings: &crate::settings::Settings, controls: &Controls) {
    setup_rebind(s, controls);
    // touch-screen tabs: hide their buttons and close the gaps
    for i in 0..s.nodes.len() {
        if !is_tab_button(s, i) {
            continue;
        }
        match s.nodes[i].name.as_str() {
            "TabControls" | "TabGraphics" => s.set_active(i, false),
            "TabSetting" | "TabMultimedia" | "TabRebind" => {
                let k = match s.nodes[i].name.as_str() {
                    "TabSetting" => 1.0,
                    "TabMultimedia" => 2.0,
                    _ => 3.0,
                };
                s.set_pos(i, Vec2::new(94.0 * k, s.nodes[i].pos[1]));
                s.make_button(i);
            }
            _ => {}
        }
    }
    // the first tab; PG3D_SETTINGS_TAB=TabMultimedia|TabRebind (automated screenshots)
    let tab = std::env::var("PG3D_SETTINGS_TAB").ok().filter(|t| TABS.contains(&t.as_str()));
    select_tab(s, tab.as_deref().unwrap_or("TabSetting"));
    let hide = [
        "3DTouchCont",
        "CheckBoxCont_4",
        "CheckBoxCont_5",
        "LobbyMusicToggleGroup",
        "FPSToggleCont",
        "ChatCont",
        "NotificaionsCont",
        "GS_StreamerCont",
        "CheckBox",
        "BrightnessCont",
        // Settings tab: what we don't have (third-person camera, acceleration, phone/BR options)
        "ThirdPerson_settings",
        "Acceleration_param",
        "CheckBoxCont_1",
        "CheckBoxCont_2",
        "CheckBoxCont_3",
        "Btns_Toggle",
    ];
    for i in 0..s.nodes.len() {
        let name = s.nodes[i].name.clone();
        if hide.contains(&name.as_str()) {
            s.set_active(i, false);
        }
        // switched on by the game once the window has opened
        if name == "BtnBack" || (name == "Header_Label" && s.nodes[i].parent == Some(0)) {
            s.set_active(i, true);
        }
    }
    for (cont, on) in [("InvertCamCont", settings.invert_y), ("MusicCont", settings.music_on), ("SoundFXCont", settings.sfx_on)] {
        if let Some(c) = s.find(cont).and_then(|c| s.find_in(c, "CheckBox")) {
            s.set_active(c, true);
            if let Some(k) = s.find_in(c, "check") {
                s.set_active(k, on);
            }
        }
    }
    for (cont, value) in [("MusicCont", settings.music), ("SoundFXCont", settings.sfx)] {
        let Some(c) = s.nodes.iter().position(|n| n.name == cont) else { continue };
        let prefix = s.path(c);
        let kids: Vec<usize> = (c..s.nodes.len()).filter(|&j| s.path(j).starts_with(&prefix)).collect();
        for j in kids {
            let n = s.nodes[j].name.clone();
            if s.nodes[j].slider.is_some() {
                s.set_slider(j, value);
            } else if n == "Amound_Label" {
                s.set_text(j, slider_text(cont, value));
            }
        }
    }
    let fov = (settings.fov - FOV_RANGE.0) / (FOV_RANGE.1 - FOV_RANGE.0);
    for (cont, value) in [("FirstPerson_settings", settings.sensitivity), ("FOV", fov)] {
        let Some(c) = s.find(cont) else { continue };
        if let Some(j) = s.find_in(c, "Slider") {
            s.set_slider(j, value);
        }
        if let Some(j) = s.find_in(c, "Amound_Label") {
            s.set_text(j, slider_text(cont, value));
        }
    }
}

/// UISliderCont's value label: a percentage without a sign ("33", "50"), or for the FOV
/// the value itself ("80.0").
fn slider_text(cont: &str, value: f32) -> String {
    if cont == "FOV" {
        format!("{:.1}", FOV_RANGE.0 + value.clamp(0.0, 1.0) * (FOV_RANGE.1 - FOV_RANGE.0))
    } else {
        format!("{:.0}", value * 100.0)
    }
}

/// Music and sound effect volumes after the mute check boxes.
fn apply_volumes(settings: &crate::settings::Settings, music: &mut Query<&mut AudioSink, With<Music>>) {
    crate::sound::set_sfx_volume(settings.sfx_volume());
    for mut sink in music {
        sink.set_volume(bevy::audio::Volume::Linear(settings.music_volume()));
    }
}

/// Check boxes: "Invert cam", and Music / Sound FX on-off.
fn toggle_clicks(
    mut clicks: MessageReader<NguiClick>,
    mut screens: Query<&mut NguiScreen>,
    mut settings: ResMut<crate::settings::Settings>,
    mut music: Query<&mut AudioSink, With<Music>>,
) {
    for c in clicks.read() {
        if c.name != "CheckBox" {
            continue;
        }
        let Ok(mut s) = screens.get_mut(c.screen) else { continue };
        let path = s.path(c.node);
        let on = if path.contains("InvertCamCont") {
            settings.invert_y = !settings.invert_y;
            settings.invert_y
        } else if path.contains("MusicCont") {
            settings.music_on = !settings.music_on;
            settings.music_on
        } else if path.contains("SoundFXCont") {
            settings.sfx_on = !settings.sfx_on;
            settings.sfx_on
        } else {
            continue;
        };
        settings.save();
        apply_volumes(&settings, &mut music);
        if let Some(k) = s.find_in(c.node, "check") {
            s.set_active(k, on);
        }
    }
}

/// Slider moved in the settings window: apply and save.
fn settings_sliders(
    mut moved: MessageReader<crate::ngui::NguiSlider>,
    mut screens: Query<&mut crate::ngui::NguiScreen>,
    mut settings: ResMut<crate::settings::Settings>,
    music: Query<&mut AudioSink, With<Music>>,
    mut cameras: Query<&mut crate::camera::PlayerCamera>,
) {
    let mut changed = false;
    let mut music = music;
    for m in moved.read() {
        let Ok(mut s) = screens.get_mut(m.screen) else { continue };
        let path = s.path(m.node);
        let (field, cont) = if path.contains("MusicCont") {
            (&mut settings.music, "MusicCont")
        } else if path.contains("SoundFXCont") {
            (&mut settings.sfx, "SoundFXCont")
        } else if path.contains("FirstPerson_settings/Slider") {
            (&mut settings.sensitivity, "FirstPerson_settings")
        } else if path.contains("/FOV/") {
            (&mut settings.fov, "FOV")
        } else {
            continue;
        };
        *field = if cont == "FOV" { FOV_RANGE.0 + m.value * (FOV_RANGE.1 - FOV_RANGE.0) } else { m.value };
        changed = true;
        let label = (0..s.nodes.len()).find(|&j| s.nodes[j].name == "Amound_Label" && s.path(j).contains(&format!("/{cont}/")));
        if let Some(j) = label {
            s.set_text(j, slider_text(cont, m.value));
        }
    }
    if changed {
        apply_volumes(&settings, &mut music);
        for mut c in &mut cameras {
            c.sensitivity = settings.mouse_sensitivity();
        }
        settings.save();
    }
}

