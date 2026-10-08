//! The original's mode and map picker (GameModeHub's ModeMapSelector, exported into the
//! "modes" screen by grafting the prefabs ModeSelector_default, MapSelector_grid and
//! ModeInfo_default): a header with the mode's name, a row of mode tabs (GMHModeTab), a
//! grid of map cells (GMHMapTab / MapCellSmall) that scrolls sideways, the mode's
//! description, and Back / To Battle! in the shared bottom bar.
//!
//! The original fills all of it from server data; here the modes are the ones this remake
//! has and the maps are the exported ones (their loading pictures on the cells).

use bevy::{input::mouse::MouseWheel, prelude::*};

use crate::{DataDir, ngui::NguiScreen};

/// One playable mode.
pub struct Mode {
    pub name: &'static str,
    pub description: &'static str,
    /// mode icon from the game (resources.assets z_mode_*, export_mode_icons.py)
    pub icon: &'static str,
    /// PG3D_MODE of the match (None: the zombie campaign)
    pub env: Option<&'static str>,
    pub sandbox: bool,
}

pub const MODES: [Mode; 3] = [
    Mode {
        name: "Team Fight",
        description: "Two teams fight against each other. Team with the most kills wins",
        icon: "z_mode_teamf",
        env: Some("teamfight"),
        sandbox: false,
    },
    Mode {
        name: "Campaign",
        description: "Fight waves of zombies. Survive as long as you can",
        icon: "z_mode_wws",
        env: None,
        sandbox: false,
    },
    Mode {
        name: "Sandbox",
        description: "Explore the maps freely - no enemies",
        icon: "z_mode_frp",
        env: None,
        sandbox: true,
    },
];

/// What the picker shows.
#[derive(Resource, Default)]
pub struct ModeSelect {
    pub open: bool,
    pub mode: usize,
    /// selected map (None: Random Map)
    pub map: Option<String>,
    built: bool,
    /// tab click areas (node) -> mode
    tabs: Vec<(usize, usize)>,
    /// map cells: (cell node, map; None = Random Map)
    cells: Vec<(usize, Option<String>)>,
    /// map cells' x before scrolling
    cell_x: Vec<f32>,
    scroll: f32,
    max_scroll: f32,
}

/// Cell spacing of the map grid (UIGrid: 228 x 170 cells; OBSERVED gaps in the original's
/// screenshot, mode_teamfight_maps.png).
const CELL: Vec2 = Vec2::new(246.0, 186.0);
const ROWS: usize = 2;
/// Cells move this far left of the prefab's grid (virtual units).
const TAB_W: f32 = 180.0;
const LEFT_SHIFT: f32 = -165.0;

/// Maps kept in the game files but not offered in the picker (nor by Random Map): the
/// second Deadly Arena (arena2; arena stays) and the Training ground. Still playable with
/// `cargo run -- <map>`.
pub const HIDDEN: [&str; 2] = ["arena2", "new_start_map3"];

/// The map's picture: the game's own loading art (export_previews.py). Maps without one
/// aren't offered.
pub fn preview(data: &DataDir, map: &str) -> Option<String> {
    if HIDDEN.contains(&map) {
        return None;
    }
    let p = format!("previews/{map}.png");
    data.0.join(&p).exists().then_some(p)
}

/// The map's name as the game's map picker shows it: SceneInfo.keyTranslateShortName
/// (export_scene_info.py -> maps/scenes.json), else the file name tidied up.
pub fn pretty(map: &str) -> String {
    #[derive(serde::Deserialize)]
    struct Scene {
        short: String,
    }
    static NAMES: std::sync::OnceLock<std::collections::HashMap<String, Scene>> = std::sync::OnceLock::new();
    let names = NAMES.get_or_init(|| {
        let dir = std::env::var("PG3D_DATA").unwrap_or_else(|_| crate::DEFAULT_DATA.to_string());
        std::fs::read_to_string(std::path::Path::new(&dir).join("maps/scenes.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    });
    if let Some(s) = names.get(map).filter(|s| !s.short.is_empty()) {
        return s.short.clone();
    }
    map.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Build the tabs and cells once (clones of the prefab templates).
fn build(s: &mut NguiScreen, st: &mut ModeSelect, assets: &AssetServer, data: &DataDir, maps: &[String]) {
    st.built = true;
    let icon = |name: &str| assets.load(format!("ui/modes/modeart/{name}.png"));
    // mode tabs along the Layout (150 wide each)
    if let Some(tab) = s.find_path("ModeSelector_default(Clone)/Layout/ModeTab") {
        for k in 0..MODES.len() {
            let t = if k == 0 { tab } else { s.clone_subtree(tab) };
            s.set_pos(t, Vec2::new(k as f32 * TAB_W, 0.0));
            if let Some(h) = s.find_in(t, "Holder") {
                // wider than the prefab's 150 so names fit on one line, as in the
                // original's 16:9 layout (OBSERVED: ~180 virtual units per tab)
                s.set_size(h, Vec2::new(TAB_W, 82.0));
                s.set_pos(h, Vec2::new(TAB_W / 2.0, 0.0));
                s.make_button(h);
                st.tabs.push((h, k));
            }
            // the tab's name (and the yellow copies shown when it is selected), widened so
            // the original's one-line look fits ("Team Fight")
            for i in s.subtree(t) {
                let n = s.nodes[i].name.clone();
                if n == "Name" || n.starts_with("Name_selected") {
                    s.set_text(i, MODES[k].name);
                }
            }
            // Container/Icon (XPLockedUI has an "Icon" too)
            let icon_node = s.subtree(t).into_iter().find(|&i| {
                s.nodes[i].name == "Icon" && s.nodes[i].parent.is_some_and(|p| s.nodes[p].name == "Container")
            });
            if let Some(i) = icon_node {
                s.set_texture(i, icon(MODES[k].icon));
            }
            for hide in ["XPLockedUI", "Tag_holder", "State_Locked"] {
                if let Some(i) = s.find_in(t, hide) {
                    s.set_active(i, false);
                }
            }
        }
    }
    // map cells: Random Map, then every exported map with a loading picture
    let Some(cell) = s.find_path("MapSelector_grid(Clone)/Container/Scroll View/Grid/MapCellSmall") else { return };

    let mut list: Vec<Option<String>> = vec![None];
    list.extend(maps.iter().filter(|m| preview(data, m).is_some()).cloned().map(Some));
    for (k, m) in list.iter().enumerate() {
        let c = if k == 0 { cell } else { s.clone_subtree(cell) };
        let (col, row) = (k / ROWS, k % ROWS);
        // the original's grid starts near the left edge of a 16:9 screen (OBSERVED in
        // mode_teamfight_maps.png); the prefab's Grid (x -503) is laid out for 4:3
        let x = LEFT_SHIFT + col as f32 * CELL.x;
        s.set_pos(c, Vec2::new(x, -(row as f32) * CELL.y));
        st.cell_x.push(x);
        s.make_button(c);
        for hide in ["TimerUI", "ItemHolder", "TegSpecific", "disable-if-locked", "GMHLabelNotification", "XPLockedUI"] {
            if let Some(i) = s.find_in(c, hide) {
                s.set_active(i, false);
            }
        }
        let name = m.as_deref().map(pretty).unwrap_or_else(|| s.string("Key_2463"));
        for i in s.subtree(c) {
            if s.nodes[i].name == "NameLabel" {
                s.set_text(i, name.clone());
            }
        }
        if let Some(p) = s.find_in(c, "tex_MapPreview") {
            let pic = match m {
                Some(m) => assets.load(preview(data, m).unwrap_or_default()),
                // the original's Random Map cell shows a die
                None => icon("z_mode_random"),
            };
            s.set_texture(p, pic);
        }
        st.cells.push((c, m.clone()));
    }
    let cols = list.len().div_ceil(ROWS) as f32;
    st.max_scroll = (cols * CELL.x - 1340.0).max(0.0);
}

/// Show the picker for `mode` (or the hub again with `open == false`).
pub fn show(s: &mut NguiScreen, st: &mut ModeSelect, assets: &AssetServer, data: &DataDir, maps: &[String], open: bool, mode: usize) {
    if open && !st.built {
        build(s, st, assets, data, maps);
    }
    st.open = open;
    st.mode = mode.min(MODES.len() - 1);
    if let Some(i) = s.find("ModeMapSelector (OFF)") {
        s.set_active(i, open);
    }
    if let Some(i) = s.find("CategorySelector_New") {
        s.set_active(i, !open);
    }
    refresh(s, st);
}

/// Selected tab / cell looks, header and description.
fn refresh(s: &mut NguiScreen, st: &ModeSelect) {
    let m = &MODES[st.mode];
    if let Some(v) = s.find_path("MapSelector_grid(Clone)/SelectedModeView") {
        for i in s.subtree(v) {
            if s.nodes[i].name.starts_with("ModeNameLabel") || s.nodes[i].name.starts_with("Label_Header") {
                s.set_text(i, m.name);
            }
        }
    }
    if let Some(i) = s.find_path("ModeInfo_default(Clone)/ModeDescription_Holder/Label_ModeDescription") {
        s.set_text(i, m.description);
    }
    for &(h, k) in &st.tabs {
        let sel = k == st.mode;
        for (name, on) in [("bg_selected", sel), ("bg_normal", !sel), ("Name", !sel)] {
            if let Some(i) = s.find_in(h, name) {
                s.set_active(i, on);
            }
        }
    }
    for (c, map) in &st.cells {
        let sel = *map == st.map;
        for (name, on) in [("SelectedState", sel), ("NotSelectedState", !sel)] {
            if let Some(i) = s.find_in(*c, name) {
                s.set_active(i, on);
            }
        }
        // the selected cell's name label lives in SelectedState: keep it filled
        if let Some(sel_state) = s.find_in(*c, "SelectedState") {
            let name = s.subtree(*c).into_iter().find(|&i| s.nodes[i].name == "NameLabel" && !s.subtree(sel_state).contains(&i)).map(|i| s.text(i));
            if let Some(name) = name {
                for i in s.subtree(sel_state) {
                    if s.nodes[i].name.contains("Label") {
                        s.set_text(i, name.clone());
                    }
                }
            }
        }
    }
}

/// A click in the picker: a tab or a map cell. True if it was one.
pub fn click(s: &mut NguiScreen, st: &mut ModeSelect, node: usize) -> bool {
    if !st.open {
        return false;
    }
    if let Some(&(_, k)) = st.tabs.iter().find(|(h, _)| *h == node) {
        st.mode = k;
        refresh(s, st);
        return true;
    }
    if let Some((_, m)) = st.cells.iter().find(|(c, _)| *c == node).cloned() {
        st.map = m;
        refresh(s, st);
        return true;
    }
    false
}

/// Scroll the map grid sideways with the mouse wheel (the original drags it).
pub fn scroll(
    mut wheel: MessageReader<MouseWheel>,
    mut st: ResMut<ModeSelect>,
    mut screens: Query<&mut NguiScreen>,
) {
    let d: f32 = wheel.read().map(|w| w.y + w.x).sum();
    if !st.open || d == 0.0 {
        return;
    }
    st.scroll = (st.scroll - d * 120.0).clamp(0.0, st.max_scroll);
    for mut s in &mut screens {
        if s.nodes.first().is_some_and(|n| n.name == "AllModesView") {
            for (&(c, _), &x) in st.cells.iter().zip(&st.cell_x) {
                let y = s.nodes[c].pos[1];
                let y = s.pos_override.get(&c).map_or(y, |p| p.y);
                s.set_pos(c, Vec2::new(x - st.scroll, y));
            }
        }
    }
}
