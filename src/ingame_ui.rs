//! The original in-match HUD (InGameGUI prefab, exported with
//! `export_ngui.py resources.assets InGameGUI ingame`), drawn by ngui.rs. We use its PC
//! weapon bar (WeaponToggles/Panel_Tap: six category slots with key hints, ammo and the
//! green "selected" frame), the health bar and the pause button in the top-left corner.
//! Everything for touch screens, multiplayer, pets, chat and events is switched off.
//! The top-centre box (score · wave · zombies left) is the Arena mode's own part,
//! InGameGUIPart_Arena (`export_ngui.py resources.assets InGameGUIPart_Arena arenapart`).
//! Slot pictures: <data>/weapons/<Weapon>/icon.png from export_weapon_icons.py (the game's
//! "<Tag>_icon1_big"), else the bar's generic category icon.

use bevy::prelude::*;

use crate::{
    DataDir,
    bots::{PLAYER_MAX_HEALTH, PlayerStats},
    ngui::NguiScreen,
    weapons::Loadout,
};

pub struct InGameUiPlugin;

impl Plugin for InGameUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn)
            .add_systems(Update, (setup, place_corners, update, update_arena_box, wave_messages, reload_ring, key_tips).chain());
    }
}

#[derive(Resource)]
struct Hud(Option<Entity>);

#[derive(Resource)]
struct ArenaBox(Option<Entity>);

/// Node indices we update every frame.
#[derive(Component, Default)]
struct HudNodes {
    /// per slot: toggle, ammo label, selected frame, picture
    slots: Vec<(usize, Option<usize>, Option<usize>, Option<usize>)>,
    icons_for: Vec<String>,
    health_label: Option<usize>,
    health_fill: Option<usize>,
    top_left: Option<usize>,
    /// BottomRightAnchor (pinned to the corner like TopLeftAnchor) with the PC key tips
    bottom_right: Option<usize>,
    /// TipsPanel key hints: (Texture_Tip node, action)
    tips: Vec<(usize, crate::controls::Action)>,
    tips_for: Vec<String>,
    /// InGameNotifications labels: "Wave is over! Pick up the prize!", "New wave!", and
    /// Wave1_And_Counter (the 5-4-3-2-1 countdown between waves)
    wave_done: Option<usize>,
    new_wave: Option<usize>,
    counter: Option<usize>,
    /// "Reloading" label and the ring that fills while reloading (CenterAnchor SCOPE/ReloadBar)
    reload_label: Option<usize>,
    reload_bar: Option<usize>,
    reload_fill: Option<usize>,
    last: String,
}

/// Size of the PC weapon bar relative to the prefab (see `setup`).
const PC_BAR_SCALE: f32 = 1.1;

/// Slot order of the original bar (keys 1-6 by default).
const SLOTS: [&str; 6] = ["PrimaryToggle", "BackupToggle", "MelleToggle", "SpecialToggle", "SniperlToggle", "PremiumToggle"];

fn spawn(mut commands: Commands, assets: Res<AssetServer>, data: Res<DataDir>) {
    let e = crate::ngui::spawn_screen(&mut commands, &assets, &data, "ingame");
    if e.is_none() {
        warn!("no exported in-match HUD (run export_ngui.py resources.assets InGameGUI ingame)");
    }
    commands.insert_resource(Hud(e));
    let b = if std::env::var("PG3D_NOBOTS").is_ok() {
        None
    } else {
        crate::ngui::spawn_screen(&mut commands, &assets, &data, "arenapart")
    };
    commands.insert_resource(ArenaBox(b));
}

/// Score, wave and zombies left in the Arena box at the top.
fn update_arena_box(
    stats: Res<PlayerStats>,
    wave: Res<crate::bots::Wave>,
    over: Res<crate::results::MatchOver>,
    arena: Res<ArenaBox>,
    mut screens: Query<&mut NguiScreen>,
    mut commands: Commands,
    mut last: Local<String>,
) {
    let Some(e) = arena.0 else { return };
    let Ok(mut s) = screens.get_mut(e) else { return };
    let dead = stats.health <= 0.0 || over.0;
    let key = format!("{} {} {} {dead}", stats.score, wave.number, wave.left);
    if *last == key {
        return;
    }
    *last = key;
    commands.entity(e).insert(if dead { Visibility::Hidden } else { Visibility::Inherited });
    for (name, text) in [
        ("TimeSurvivalLabel", stats.score.to_string()),
        ("WaveSurvivalLabel", format!("Wave {}", wave.number.max(1))),
        ("ScoreSurvivalLabel", wave.left.to_string()),
    ] {
        if let Some(i) = s.find(name) {
            s.set_text(i, text);
        }
    }
}

fn child(s: &NguiScreen, parent: usize, name: &str) -> Option<usize> {
    (parent + 1..s.nodes.len()).find(|&j| s.nodes[j].parent == Some(parent) && s.nodes[j].name == name)
}

fn setup(mut commands: Commands, mut screens: Query<(Entity, &mut NguiScreen), Added<NguiScreen>>) {
    for (e, mut s) in &mut screens {
        if s.nodes.first().is_none_or(|n| n.name != "InGameGUI") {
            continue;
        }
        let mut nodes = HudNodes::default();
        let Some(container) = s.find("InterfaceContainer") else { continue };
        let Some(interface) = s.find("InGameInterface") else { continue };
        let Some(camera) = s.nodes[interface].parent else { continue };
        let joystick = s.find("JoystikPanel");
        // only the in-game interface, and in it only the weapon bar and the top-left corner
        for i in 0..s.nodes.len() {
            let p = s.nodes[i].parent;
            let name = s.nodes[i].name.as_str();
            if (p == Some(camera) && i != interface && Some(i) != joystick)
                || (p == Some(interface) && i != container)
                || (p == Some(container) && !["WeaponToggles", "TopLeftAnchor", "InGameNotifications", "CenterAnchor SCOPE"].contains(&name))
            {
                s.set_active(i, false);
            }
        }
        if let Some(t) = s.find("InGameNotifications") {
            for i in s.subtree(t) {
                if s.nodes[i].parent == Some(t) {
                    s.set_active(i, false);
                }
            }
            nodes.wave_done = s.find_in(t, "WaveDone");
            nodes.new_wave = s.find_in(t, "NewWave");
            nodes.counter = s.find_in(t, "Wave1_And_Counter");
            nodes.reload_label = s.find_in(t, "ReloadLabel");
        }
        if let Some(t) = s.find("CenterAnchor SCOPE") {
            // only the reload ring for now (our own crosshair is drawn by weapons.rs)
            for i in s.subtree(t) {
                if s.nodes[i].parent == Some(t) {
                    s.set_active(i, false);
                }
            }
            nodes.reload_bar = s.find_in(t, "ReloadBar");
            nodes.reload_fill = nodes.reload_bar.and_then(|b| s.find_in(b, "Loading"));
        }
        // bottom-right: only TipsPanel, the PC hints for reload / shoot / aim with their keys
        if let Some(b) = s.find("BottomRightAnchor") {
            nodes.bottom_right = Some(b);
            let keep: Vec<usize> = std::iter::successors(Some(b), |&j| s.nodes[j].parent).collect();
            for j in joystick.map(|j| s.subtree(j)).unwrap_or_default() {
                let p = s.nodes[j].parent;
                if p.is_some_and(|p| keep.contains(&p) && p != b) && !keep.contains(&j) {
                    s.set_active(j, false);
                }
                if p == Some(b) && s.nodes[j].name != "TipsPanel" {
                    s.set_active(j, false);
                }
            }
            use crate::controls::Action;
            for (name, action) in [("Reload Tip", Action::Reload), ("Attack Tip", Action::Fire), ("Aim Tip", Action::Aim)] {
                if let Some(t) = s.find_in(b, name).and_then(|t| s.find_in(t, "Texture_Tip")) {
                    nodes.tips.push((t, action));
                }
            }
        }
        if let Some(t) = s.find("TopLeftAnchor") {
            nodes.top_left = Some(t);
            for i in s.subtree(t) {
                if s.nodes[i].parent == Some(t) && !["Pause Button", "ProgressBarHealth"].contains(&s.nodes[i].name.as_str()) {
                    s.set_active(i, false);
                }
            }
            if let Some(h) = s.find_in(t, "ProgressBarHealth") {
                nodes.health_label = s.find_in(h, "Label");
                nodes.health_fill = s.find_in(h, "ProgressArmor");
            }
        }
        if let Some(bar) = s.find("WeaponToggles") {
            // On PC the slot bar is 10 % bigger than the prefab's AnchorPanel (544x194): the
            // slots are anchored to it, so they grow with it (MEASURED from 1920x1080
            // screenshots of the original: selected frame 131x136 px instead of 118x124;
            // the code that resizes it hasn't been found yet)
            if let Some(a) = s.find_in(bar, "AnchorPanel") {
                s.set_size(a, Vec2::new(544.0, 194.0) * PC_BAR_SCALE);
            }
            for name in ["Panel_Tap_Dater", "TutorTargetAnchor"] {
                if let Some(i) = child(&s, bar, name) {
                    s.set_active(i, false);
                }
            }
            if let Some(tap) = child(&s, bar, "Panel_Tap") {
                s.set_active(tap, true);
                for i in s.subtree(tap) {
                    if ["ModulesIcons", "WrongModuleLabel", "GamepadTipRB", "GamepadTipLB", "BackgroundBlockCoop", "EndlessAmmo_Layout", "EndlessAmmo_Layout "]
                        .contains(&s.nodes[i].name.as_str())
                    {
                        s.set_active(i, false);
                    }
                }
                for name in SLOTS {
                    let Some(t) = child(&s, tap, name) else { continue };
                    nodes.slots.push((t, child(&s, t, "Label"), child(&s, t, "Checkmark"), child(&s, t, "ShopIcon")));
                    // the key hint above the slot: GamepadTipView shows the bound key's kb_*
                    // picture at its own size (64x64, or 164x64 for keys like L Ctrl), not the
                    // prefab's 128x64 (MEASURED: "2" box 45 px wide at 1920x1080, "L Ctrl" 115)
                    if let Some(tip) = child(&s, t, "GamepadTip").and_then(|g| child(&s, g, "Texture_Tip")) {
                        s.set_size(tip, Vec2::new(64.0, 64.0));
                    }
                }
            }
        }
        commands.entity(e).insert((nodes, GlobalZIndex(0)));
    }
}

/// The original pins the corner containers to the screen edges with UIAnchor scripts
/// (their saved positions are meaningless); do the same.
fn place_corners(window: Single<&Window, Changed<Window>>, mut screens: Query<(&mut NguiScreen, &HudNodes)>) {
    for (mut s, n) in &mut screens {
        let vh = 768.0;
        let vw = vh * window.width() / window.height().max(1.0);
        if let Some(t) = n.top_left {
            s.set_pos(t, Vec2::new(-vw / 2.0, vh / 2.0));
        }
        if let Some(t) = n.bottom_right {
            s.set_pos(t, Vec2::new(vw / 2.0, -vh / 2.0));
        }
    }
}

fn update(
    loadout: Res<Loadout>,
    stats: Res<PlayerStats>,
    over: Res<crate::results::MatchOver>,
    hud: Res<Hud>,
    mut screens: Query<(&mut NguiScreen, &mut HudNodes)>,
    mut commands: Commands,
    assets: Res<AssetServer>,
    data: Res<DataDir>,
    time: Res<Time>,
    mut shown: Local<Option<f32>>,
) {
    let Some(e) = hud.0 else { return };
    let Ok((mut s, mut n)) = screens.get_mut(e) else { return };
    let names: Vec<String> = loadout.weapons.iter().map(|w| w.name.clone()).collect();
    if names != n.icons_for {
        for (slot, name) in names.iter().enumerate() {
            let Some(&(_, _, _, Some(icon))) = n.slots.get(slot) else { continue };
            let rel = format!("weapons/{name}/icon.png");
            if data.0.join(&rel).exists() {
                s.set_texture(icon, assets.load(rel));
            }
        }
        n.icons_for = names;
    }
    let dead = stats.health <= 0.0 || over.0;
    // the bar slides to the new value instead of jumping (OBSERVED: ~120-150 %/s in clips
    // d_rec4 and d_rec8)
    let target = (stats.health / PLAYER_MAX_HEALTH).clamp(0.0, 1.0);
    let cur = shown.unwrap_or(target);
    let step = 1.3 * time.delta_secs();
    let k = cur + (target - cur).clamp(-step, step);
    *shown = Some(k);
    let ammo: Vec<String> = loadout
        .weapons
        .iter()
        .map(|w| if w.def.isMelee != 0 { String::new() } else { format!("{}/{}", w.clip, w.reserve) })
        .collect();
    let key = format!("{dead} {k} {} {ammo:?}", loadout.current);
    if key == n.last {
        return;
    }
    n.last = key;
    commands.entity(e).insert(if dead { Visibility::Hidden } else { Visibility::Inherited });
    if let Some(i) = n.health_label {
        s.set_text(i, format!("{:.0}%", k * 100.0));
    }
    if let Some(i) = n.health_fill {
        let h = s.nodes[i].widget.as_ref().map_or(20.0, |w| w.h);
        let w = s.nodes[i].widget.as_ref().map_or(106.0, |w| w.w);
        s.set_size(i, Vec2::new((w * k).max(0.0), h));
    }
    for (slot, &(toggle, label, selected, _)) in n.slots.clone().iter().enumerate() {
        let used = slot < loadout.weapons.len();
        s.set_active(toggle, used);
        if let (Some(l), Some(text)) = (label, ammo.get(slot)) {
            s.set_active(l, !text.is_empty());
            s.set_text(l, text.clone());
        }
        if let Some(c) = selected {
            s.set_active(c, slot == loadout.current);
        }
    }
}

/// Between Arena waves (timings OBSERVED in reference clip d_rec7): "Wave is over! Pick up
/// the prize!" for the first 5 s of the break, then a 5-4-3-2-1 countdown, then "New wave!"
/// for about a second once the next wave has started.
fn wave_messages(
    wave: Res<crate::bots::Wave>,
    stats: Res<PlayerStats>,
    hud: Res<Hud>,
    mut screens: Query<(&mut NguiScreen, &mut HudNodes)>,
    mut last: Local<String>,
) {
    let Some(e) = hud.0 else { return };
    let Ok((mut s, n)) = screens.get_mut(e) else { return };
    let between = wave.number > 0 && wave.left == 0 && wave.break_left > 0.0 && stats.health > 0.0;
    let elapsed = crate::bots::WAVE_BREAK - wave.break_left;
    let done = between && elapsed < 5.0;
    let count = if between && !done { wave.break_left.ceil().clamp(1.0, 5.0) as u32 } else { 0 };
    let new = wave.number > 1 && wave.break_left <= 0.0 && wave.age < 1.25 && stats.health > 0.0;
    let key = format!("{done} {count} {new}");
    if *last == key {
        return;
    }
    *last = key;
    if let Some(i) = n.wave_done {
        s.set_active(i, done);
    }
    if let Some(i) = n.new_wave {
        s.set_active(i, new);
    }
    if let Some(i) = n.counter {
        s.set_active(i, count > 0);
        s.set_text(i, count.to_string());
    }
}

/// Steps of the reload ring's fill (pictures made once from the ring texture).
const RING_STEPS: usize = 48;

/// While reloading: the original "Reloading" label under the crosshair and a ring around it
/// whose bright part grows anticlockwise from the top (OBSERVED in clip d_rec4) over a
/// faint full ring (ReloadBar/Loading over ReloadBar/LoadingBg, same texture).
#[allow(clippy::too_many_arguments)]
fn reload_ring(
    loadout: Res<Loadout>,
    stats: Res<PlayerStats>,
    hud: Res<Hud>,
    mut screens: Query<(&mut NguiScreen, &HudNodes)>,
    assets: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    mut src: Local<Option<Handle<Image>>>,
    mut steps: Local<Vec<Handle<Image>>>,
    mut last: Local<Option<usize>>,
) {
    let Some(e) = hud.0 else { return };
    let Ok((mut s, n)) = screens.get_mut(e) else { return };
    if steps.is_empty() {
        let h = src.get_or_insert_with(|| assets.load("ui/ingame/textures/loading_kruzhok_11d9.png")).clone();
        let Some(img) = images.get(&h) else { return };
        let (w, ht) = (img.width() as usize, img.height() as usize);
        let Some(data) = img.data.clone() else { return };
        let fmt = img.texture_descriptor.format;
        let sampler = img.sampler.clone();
        for k in 0..=RING_STEPS {
            let f = k as f32 / RING_STEPS as f32;
            let mut d = data.clone();
            for y in 0..ht {
                for x in 0..w {
                    let (dx, dy) = (x as f32 + 0.5 - w as f32 / 2.0, y as f32 + 0.5 - ht as f32 / 2.0);
                    // angle anticlockwise from the top, 0..1
                    let a = ((-dx).atan2(-dy) / std::f32::consts::TAU).rem_euclid(1.0);
                    if a > f {
                        d[(y * w + x) * 4 + 3] = 0;
                    }
                }
            }
            let mut im = Image::new(
                bevy::render::render_resource::Extent3d { width: w as u32, height: ht as u32, depth_or_array_layers: 1 },
                bevy::render::render_resource::TextureDimension::D2,
                d,
                fmt,
                bevy::asset::RenderAssetUsages::RENDER_WORLD,
            );
            im.sampler = sampler.clone();
            steps.push(images.add(im));
        }
    }
    let reload = loadout.weapons.get(loadout.current).map_or(1.0, |w| w.reload_time.max(0.01));
    let on = loadout.reloading > 0.0 && stats.health > 0.0;
    let k = on.then(|| (((1.0 - loadout.reloading / reload) * RING_STEPS as f32) as usize).min(RING_STEPS));
    if *last == k {
        return;
    }
    *last = k;
    for i in [n.reload_label, n.reload_bar].into_iter().flatten() {
        s.set_active(i, on);
    }
    if let (Some(i), Some(k)) = (n.reload_fill, k) {
        s.set_texture(i, steps[k].clone());
    }
}

/// The key pictures (kb_*.png, export_keys.py) of the current bindings on the PC key tips,
/// at the picture's own size like GamepadTipView (64x64, 164x64 for wide keys).
fn key_tips(
    controls: Res<crate::controls::Controls>,
    hud: Res<Hud>,
    mut screens: Query<(&mut NguiScreen, &mut HudNodes)>,
    assets: Res<AssetServer>,
    images: Res<Assets<Image>>,
    mut pending: Local<Vec<(usize, Handle<Image>)>>,
) {
    let Some(e) = hud.0 else { return };
    let Ok((mut s, mut n)) = screens.get_mut(e) else { return };
    let names: Vec<String> = n.tips.iter().map(|&(_, a)| controls.get(a).unity_key_name()).collect();
    if names != n.tips_for {
        for (&(node, _), name) in n.tips.iter().zip(&names) {
            let h: Handle<Image> = assets.load(format!("ui/keys/kb_{name}.png"));
            s.set_texture(node, h.clone());
            pending.push((node, h));
        }
        n.tips_for = names;
    }
    pending.retain(|(node, h)| {
        let Some(im) = images.get(h) else { return true };
        s.set_size(*node, im.size_f32());
        false
    });
}
