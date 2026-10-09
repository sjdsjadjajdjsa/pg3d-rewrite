//! The original settings window (exported NGUI screen), shared by the lobby menu and the
//! in-match pause screen. Three of its tabs work:
//!   * Settings (sliders icon, TabSetting): first-person camera sensitivity, field of view
//!     (60-90, SettingsTabSetting) and "Invert cam".
//!   * Multimedia: Music and Sound FX. Changes apply live and are saved.
//!   * Key binding (keyboard icon, TabRebind = the PC version's KeyMappingView): one row per
//!     action; click a row, press a key or mouse button. A key already in use asks first
//!     (ChangeButtonWindow_Used) and then swaps the two. "Default Settings" resets all.
//! The other tabs (touch controls, graphics, ...) are hidden, as are options we don't have.

#[allow(unused_imports)]
use crate::scene::{FixedUpdate, PostStartup, PostUpdate, PreStartup, Startup, Update};
use bevy::{diagnostic::FrameCount, prelude::*};

use crate::{
    controls::{Action, Bind, Controls},
    ngui::{NguiClick, NguiScreen},
};

pub struct SettingsUiPlugin;

impl crate::scene::ScenePlugin for SettingsUiPlugin {
    fn build(&self, app: &mut crate::scene::SceneBuilder) {
        app.init_resource::<Rebinding>()
            .add_systems(Update, (settings_sliders, tab_clicks, toggle_clicks, anti_aliasing_clicks, rebind_clicks, capture_key, binding_icons, scroll_rows).chain());
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

/// One line of the key binding list: an action we let you rebind, or one the original shows
/// locked (lock icon, "cantChangeOnKeyboard" in KeyMappingViewData) with its fixed key picture.
#[derive(Clone, Copy)]
enum Row {
    Action(Action),
    Fixed(&'static str, &'static str),
}

/// The General tab of the original (scripts/dump_keymapping.py), with what our game has:
/// no Move Slowly, module/gadget, chart, booster, chat, emoji or graffiti keys. Free camera
/// is ours.
const ROWS: [Row; 18] = [
    Row::Action(Action::Slot1),
    Row::Action(Action::Slot2),
    Row::Action(Action::Slot3),
    Row::Action(Action::Slot4),
    Row::Action(Action::Slot5),
    Row::Action(Action::Slot6),
    Row::Action(Action::Forward),
    Row::Action(Action::Back),
    Row::Action(Action::Left),
    Row::Action(Action::Right),
    Row::Fixed("View", "MouseMove"),
    Row::Action(Action::Fire),
    Row::Action(Action::Aim),
    Row::Fixed("Switch Weapon", "MouseWheel"),
    Row::Action(Action::Jump),
    Row::Action(Action::Reload),
    Row::Fixed("Pause", "Escape"),
    Row::Action(Action::FreeCamera),
];

impl Row {
    fn label(self) -> &'static str {
        match self {
            Row::Action(a) => a.label(),
            Row::Fixed(l, _) => l,
        }
    }
}

const TABS: [&str; 4] = ["TabSetting", "TabMultimedia", "TabGraphics", "TabRebind"];

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
                    s.set_text(l, "General");
                }
            }
        }
    }
    let Some(row) = s.find_in(page, "KeyActionView") else { return };
    let mut rows = vec![row];
    for _ in 1..ROWS.len() {
        rows.push(s.clone_subtree(row));
    }
    for &r in &rows {
        if let Some(i) = s.find_in(r, "ButtonIconScaler") {
            s.set_active(i, false);
        }
        if let Some(i) = s.find_in(r, "ButtonLabel") {
            s.set_active(i, true);
        }
    }
    // PG3D_LIST_SCROLL=n (automated screenshots): start the list scrolled down n rows
    let first = std::env::var("PG3D_LIST_SCROLL").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    place_rows(s, first);
    show_bindings(s, controls);
}

/// Rows that fit the list's clip area (ScrollView: 466 high, rows 48).
const VISIBLE_ROWS: usize = 9;

/// Show rows `first..first + VISIBLE_ROWS` stacked from the top of the list. (We don't clip
/// NGUI panels, so the list scrolls a whole row at a time.)
fn place_rows(s: &mut NguiScreen, first: usize) {
    let rows = rows(s);
    let Some(&top) = rows.first() else { return };
    let h = s.nodes[top].widget.as_ref().map_or(48.0, |w| w.h);
    let (x, y0) = (s.nodes[top].pos[0], s.nodes[top].pos[1]);
    for (k, &r) in rows.iter().enumerate() {
        let on = k >= first && k < first + VISIBLE_ROWS;
        s.set_active(r, on);
        s.set_pos(r, Vec2::new(x, y0 - h * (k as f32 - first as f32)));
    }
}

/// Mouse wheel over the settings window while the key binding tab is open: scroll the list.
fn scroll_rows(
    scroll: Res<bevy::input::mouse::AccumulatedMouseScroll>,
    mut screens: Query<(Entity, &mut NguiScreen, &InheritedVisibility)>,
    mut first: Local<std::collections::HashMap<Entity, usize>>,
) {
    if scroll.delta.y == 0.0 {
        return;
    }
    for (e, mut s, vis) in &mut screens {
        let Some(page) = rebind_page(&s) else { continue };
        if !vis.get() || !s.visible(page) {
            continue;
        }
        let max = rows(&s).len().saturating_sub(VISIBLE_ROWS);
        let f = first.entry(e).or_default();
        let next = if scroll.delta.y > 0.0 { f.saturating_sub(1) } else { (*f + 1).min(max) };
        if next != *f {
            *f = next;
            place_rows(&mut s, next);
        }
    }
}

/// The bound keys as the game's key pictures (kb_<Unity key name>.png, export_keys.py) at
/// their own size, as KeyActionView shows them; keys without a picture keep the text label.
fn binding_icons(
    controls: Res<Controls>,
    data: Res<crate::DataDir>,
    assets: Res<AssetServer>,
    mut screens: Query<(Entity, &mut NguiScreen)>,
    mut shown: Local<std::collections::HashMap<Entity, Controls>>,
) {
    for (e, mut s) in &mut screens {
        if rebind_page(&s).is_none() || shown.get(&e) == Some(&*controls) {
            continue;
        }
        shown.insert(e, controls.clone());
        for (r, row) in rows(&s).into_iter().zip(ROWS) {
            let name = match row {
                Row::Action(a) => controls.get(a).unity_key_name(),
                Row::Fixed(_, k) => k.to_string(),
            };
            let fixed = matches!(row, Row::Fixed(..));
            for (n, on) in [("state_CanChange", !fixed), ("state_CantChange", fixed)] {
                if let Some(i) = s.find_in(r, n) {
                    s.set_active(i, on);
                }
            }
            let rel = format!("ui/keys/kb_{name}.png");
            let size = png_size(&data.0.join(&rel));
            let (Some(scaler), Some(icon)) = (s.find_in(r, "ButtonIconScaler"), s.find_in(r, "ButtonIcon")) else { continue };
            s.set_active(scaler, size.is_some());
            if let Some(i) = s.find_in(r, "ButtonLabel") {
                s.set_active(i, size.is_none());
            }
            if let Some(size) = size {
                s.set_texture(icon, assets.load(rel));
                s.set_size(icon, size);
            }
        }
    }
}

/// Width and height from a PNG file's header.
fn png_size(path: &std::path::Path) -> Option<Vec2> {
    use std::io::Read;
    let mut b = [0u8; 24];
    std::fs::File::open(path).ok()?.read_exact(&mut b).ok()?;
    if &b[1..4] != b"PNG" {
        return None;
    }
    let w = u32::from_be_bytes([b[16], b[17], b[18], b[19]]);
    let h = u32::from_be_bytes([b[20], b[21], b[22], b[23]]);
    Some(Vec2::new(w as f32, h as f32))
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
    for (r, row) in rows(s).into_iter().zip(ROWS) {
        if let Some(i) = s.find_in(r, "ActionLabel") {
            s.set_text(i, row.label());
        }
        if let (Some(i), Row::Action(a)) = (s.find_in(r, "ButtonLabel"), row) {
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
                let Some(Row::Action(a)) = ROWS.get(k).copied() else { continue };
                state.waiting = Some((c.screen, a, frame.0));
                info!("key binding: waiting for a key for {a:?}");
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
            "TabControls" => s.set_active(i, false),
            "TabSetting" | "TabMultimedia" | "TabGraphics" | "TabRebind" => {
                let k = match s.nodes[i].name.as_str() {
                    "TabSetting" => 1.0,
                    "TabMultimedia" => 2.0,
                    "TabGraphics" => 3.0,
                    _ => 4.0,
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
    setup_graphics(s, settings);
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

/// The Graphics tab (the PC version's SettingsGraphics): Anti-Aliasing, Motion Blur, Bloom,
/// V-Sync on the left, Chromatic Aberration and (ours) Ambient Occlusion on the right.
/// Resolution, frame rate and screen mode are left out.
fn setup_graphics(s: &mut NguiScreen, settings: &crate::settings::Settings) {
    let Some(page) = (0..s.nodes.len()).find(|&i| s.nodes[i].name == "TabGraphics" && !is_tab_button(s, i)) else { return };
    for name in ["ToggleGroup_Resolution", "ToggleGroup_FPS", "Toggle_ScreenMode", "PopupList_Cont"] {
        if let Some(i) = s.find_in(page, name) {
            s.set_active(i, false);
        }
    }
    for (k, name) in ["ToggleGroup_AntiAlising", "Toggle_Blur", "Toggle_Bloom", "Toggle_VSync"].iter().enumerate() {
        if let Some(i) = s.find_in(page, name) {
            s.set_pos(i, Vec2::new(0.0, -100.0 * k as f32));
        }
    }
    if let Some(ab) = s.find_in(page, "Toggle_Aberration") {
        s.set_pos(ab, Vec2::new(0.0, 0.0));
        let ao = s.clone_subtree(ab);
        s.nodes[ao].name = "Toggle_AmbientOcclusion".into();
        s.set_pos(ao, Vec2::new(0.0, -100.0));
        if let Some(l) = s.find_in(ao, "AberrationCheckBox_Label") {
            s.set_text(l, "Ambient Occlusion");
        }
    }
    if let Some(g) = s.find_in(page, "ToggleGroup_AntiAlising") {
        for n in ["Classic", "Modern", "Ambient"] {
            if let Some(i) = s.find_in(g, n) {
                s.set_active(i, false);
            }
        }
        for n in ["PrevAudioButton", "NextAudioButton"] {
            if let Some(i) = s.find_in(g, n) {
                s.make_button(i);
            }
        }
        if let Some(l) = s.find_in(g, "MusicName_Label") {
            s.set_text(l, settings.anti_aliasing.label());
        }
    }
    show_graphics_checks(s, settings);
}

fn show_graphics_checks(s: &mut NguiScreen, settings: &crate::settings::Settings) {
    for (cont, on) in [
        ("Toggle_Blur", settings.motion_blur),
        ("Toggle_Bloom", settings.bloom),
        ("Toggle_VSync", settings.vsync),
        ("Toggle_Aberration", settings.chromatic_aberration),
        ("Toggle_AmbientOcclusion", settings.ambient_occlusion),
    ] {
        if let Some(c) = s.find(cont).and_then(|c| s.find_in(c, "CheckBox")) {
            s.set_active(c, true);
            if let Some(k) = s.find_in(c, "check") {
                s.set_active(k, on);
            }
        }
    }
}

/// The Graphics tab's anti-aliasing arrows: cycle Off / MSAA / FXAA / SMAA / TAA.
fn anti_aliasing_clicks(
    mut clicks: MessageReader<NguiClick>,
    mut screens: Query<&mut NguiScreen>,
    mut settings: ResMut<crate::settings::Settings>,
) {
    use crate::settings::AntiAliasing;
    for c in clicks.read() {
        let step: i32 = match c.name.as_str() {
            "PrevAudioButton" => -1,
            "NextAudioButton" => 1,
            _ => continue,
        };
        let Ok(mut s) = screens.get_mut(c.screen) else { continue };
        let path = s.path(c.node);
        if !path.contains("ToggleGroup_AntiAlising") {
            continue;
        }
        let all = AntiAliasing::ALL;
        let i = all.iter().position(|a| *a == settings.anti_aliasing).unwrap_or(0) as i32;
        settings.anti_aliasing = all[(i + step).rem_euclid(all.len() as i32) as usize];
        settings.save();
        if let Some(g) = (0..s.nodes.len()).find(|&j| s.nodes[j].name == "ToggleGroup_AntiAlising")
            && let Some(l) = s.find_in(g, "MusicName_Label")
        {
            s.set_text(l, settings.anti_aliasing.label());
        }
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
        } else if path.contains("Toggle_Bloom") {
            settings.bloom = !settings.bloom;
            settings.bloom
        } else if path.contains("Toggle_Blur") {
            settings.motion_blur = !settings.motion_blur;
            settings.motion_blur
        } else if path.contains("Toggle_VSync") {
            settings.vsync = !settings.vsync;
            settings.vsync
        } else if path.contains("Toggle_Aberration") {
            settings.chromatic_aberration = !settings.chromatic_aberration;
            settings.chromatic_aberration
        } else if path.contains("Toggle_AmbientOcclusion") {
            settings.ambient_occlusion = !settings.ambient_occlusion;
            settings.ambient_occlusion
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

