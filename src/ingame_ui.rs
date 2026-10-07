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
            .add_systems(Update, (setup, place_corners, update, update_arena_box).chain());
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
    last: String,
}

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
        // only the in-game interface, and in it only the weapon bar and the top-left corner
        for i in 0..s.nodes.len() {
            let p = s.nodes[i].parent;
            let name = s.nodes[i].name.as_str();
            if (p == Some(camera) && i != interface)
                || (p == Some(interface) && i != container)
                || (p == Some(container) && name != "WeaponToggles" && name != "TopLeftAnchor")
            {
                s.set_active(i, false);
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
        let Some(t) = n.top_left else { continue };
        let vh = 768.0;
        let vw = vh * window.width() / window.height().max(1.0);
        s.set_pos(t, Vec2::new(-vw / 2.0, vh / 2.0));
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
    let k = (stats.health / PLAYER_MAX_HEALTH).clamp(0.0, 1.0);
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
