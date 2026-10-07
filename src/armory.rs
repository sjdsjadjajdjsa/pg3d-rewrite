//! The Armory (original ShopNGUI screen, exported as ui/armory): the Weapon section with
//! its six category tabs, a grid of the weapons we have, the stats panel of the selected
//! weapon and equipping. The equipped weapon of each category is saved
//! (~/.config/pixel-gun-3d-rs/loadout.json) and used by the next match (weapons.rs).
//! The other sections (Wear, Gadgets, ...) are shown but not open yet.

use std::collections::HashMap;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    ngui::{NguiClick, NguiScreen},
    DataDir,
};

pub struct ArmoryPlugin;

impl Plugin for ArmoryPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((crate::armory_preview::ArmoryPreviewPlugin, crate::particles::ParticlesPlugin))
            .add_systems(Update, ((setup, clicks, refresh).chain(), preview_on_when_open));
    }
}

/// Category tabs along the top, in categoryNabor order (1 primary .. 6 heavy).
const TABS: [&str; 6] = ["PrimaryCategory", "BackupCategory", "MeleeCategory", "SpecilCategory", "SniperCategory", "PremiumCategory"];

/// Grid layout in 1_FirstGrid's space. OBSERVED (1280x720 screenshots, 0.9375 px per
/// virtual pixel): 4 columns 204 px apart, rows 180 px apart, cells 194x178 px.
const COLUMNS: usize = 4;
const PITCH: Vec2 = Vec2::new(217.3, 192.0);
/// Centre of the first cell. OBSERVED: (212, 199) px in the screenshots.
const FIRST: Vec2 = Vec2::new(-327.0, 144.7);
const CELL_SCALE: f32 = 1.2;
/// Most weapons one category can show (cells made once and reused).
const MAX_CELLS: usize = 12;

/// The weapon chosen in each category (categoryNabor -> weapon name), saved between runs.
#[derive(Resource, Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct SavedLoadout {
    pub equipped: HashMap<i32, String>,
}

impl SavedLoadout {
    fn path() -> std::path::PathBuf {
        crate::settings::Settings::config_dir().join("loadout.json")
    }
    pub fn load() -> Self {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }
    fn save(&self) {
        let p = Self::path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&p, serde_json::to_string_pretty(self).unwrap_or_default()) {
            warn!("could not save the loadout to {}: {e}", p.display());
        }
    }
}

/// What the Armory shows about one weapon (original WeaponSounds fields).
#[derive(Deserialize, Clone, Default)]
#[serde(default)]
#[allow(non_snake_case)]
struct Item {
    categoryNabor: i32,
    localizeWeaponKey: String,
    rarity: i32,
    fireRateShop: i32,
    ammoInClip: i32,
    isMelee: i32,
    speedModifier: f32,
    dpses: Vec<f32>,
}

/// State of the Armory screen.
#[derive(Component)]
struct Armory {
    /// weapon name -> data, only weapons that were exported
    items: Vec<(String, Item)>,
    /// cell root nodes
    cells: Vec<usize>,
    /// weapon shown in each cell
    shown: Vec<String>,
    category: i32,
    selected: Option<String>,
    loadout: SavedLoadout,
    dirty: bool,
}

/// The weapon picked in a category: the saved one, or the default (weapons.rs LOADOUT).
pub fn equipped_in(loadout: &SavedLoadout, category: i32) -> Option<&str> {
    loadout.equipped.get(&category).map(String::as_str).or_else(|| {
        crate::weapons::DEFAULT_LOADOUT.get((category - 1).max(0) as usize).copied()
    })
}

fn setup(
    mut commands: Commands,
    mut screens: Query<(Entity, &mut NguiScreen), Added<NguiScreen>>,
    data: Res<DataDir>,
    assets: Res<AssetServer>,
    preview: Option<Res<crate::armory_preview::PreviewImage>>,
) {
    for (e, mut s) in &mut screens {
        if !s.nodes.first().is_some_and(|n| n.name == "ShopNGUI") {
            continue;
        }
        // run-time parts of the original that are not part of the Weapon section
        for path in [
            "Camera/Center/Panel",
            "ArmoryView/ArmorNode",
            "ArmoryView/BtnTry",
            "Table/0_CratesInStorage",
            "Table/2_CratesNotInStorage",
            // Wear options ("Show Mask", "Show only available")
            "Camera/Bottom/Container",
            // the scroll bar: our lists fit without scrolling
            "ArmoryView/Slider",
            "Properties (OFF)/btn0/EggButtons_ShopNGUI",
            "Properties (OFF)/btn1",
            "Properties (OFF)/btn2",
            "Properties (OFF)/ReturnEveryDay",
        ] {
            if let Some(i) = s.find_path(path) {
                s.set_active(i, false);
            }
        }
        for path in [
            "ContainerGrid/GridCategoriesWeapons",
            "Camera/Right (OFF)",
            "ClipPropertiesPanel/PanelProperties (OFF)",
            // the button row under the stats (Equip / Equipped)
            "ArmoryView/Properties (OFF)",
            // the "+" next to the gems (OBSERVED)
            "ObjectForCenter/BankButton",
        ] {
            if let Some(i) = s.find_path(path) {
                s.set_active(i, true);
            }
        }
        for name in TABS.into_iter().chain(["BtnShoot"]) {
            if let Some(i) = s.find(name) {
                s.make_button(i);
            }
        }
        // the Equip row sits under the right edge of the stats panel (OBSERVED). btn0 is
        // anchored to btn2, which the game moves; we place it directly instead
        if let Some(i) = s.find_path("Properties (OFF)/btn0") {
            if let Some(w) = s.nodes[i].widget.as_mut() {
                w.anchors.clear();
            }
            s.set_pos(i, Vec2::new(900.0, -307.0));
        }
        // the bank bar is centred by game code (BankShopView) on the buttons it shows.
        // OBSERVED: coin icon at x 1020 px
        if let Some(i) = s.find_path("BankShopViewPrefab(Clone)/ObjectForCenter") {
            s.set_pos(i, Vec2::new(-72.7, 0.0));
        }
        // left column: Weapon is the open section
        if let Some(col) = s.find("GridCategory") {
            let buttons: Vec<usize> = s.subtree(col).into_iter().filter(|&b| s.nodes[b].parent == Some(col)).collect();
            for b in buttons {
                let on = s.nodes[b].name == "BtnCategory1";
                if let Some(p) = s.find_in(b, "PressedState") {
                    s.set_active(p, on);
                }
            }
        }
        if let Some(i) = s.find_path("ExpProgressMiniView/Deafault/icon_Star/Label_Lvl") {
            s.set_text(i, "1");
        }
        if let Some(i) = s.find_path("ExpProgressMiniView/MaxLevel") {
            s.set_active(i, false);
        }
        // the cells: the grafted ArmoryCell is the template
        let mut cells = vec![];
        if let Some(t) = s.find("ArmoryCell(Clone)") {
            s.set_active(t, false);
            for _ in 0..MAX_CELLS {
                let c = s.clone_subtree(t);
                s.make_button(c);
                s.nodes[c].scale = [CELL_SCALE, CELL_SCALE];
                for (name, on) in [("Blink", false), ("SeceltionIndicator", false), ("Level_Label_newbalance", true)] {
                    if let Some(i) = s.find_in(c, name) {
                        s.set_active(i, on);
                    }
                }
                // the cell's white backing: NGUI blends in gamma space, where 43 % white
                // lightens far less than in our linear blending (OBSERVED cell colours)
                if let Some(i) = s.find_in(c, "Bg") {
                    s.set_color(i, [1.0, 1.0, 1.0, 0.2]);
                }
                cells.push(c);
            }
        }
        // weapons that were exported, in weapons.json order
        let dir = data.0.join("weapons");
        let all: HashMap<String, Item> = std::fs::read_to_string(dir.join("weapons.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let mut items: Vec<(String, Item)> =
            all.into_iter().filter(|(n, _)| dir.join(n).join("weapon.gltf").exists()).collect();
        items.sort_by_key(|(n, i)| (i.rarity, n.trim_start_matches("Weapon").parse::<u32>().unwrap_or(u32::MAX)));
        for (n, _) in &items {
            let _ = assets.load::<Image>(format!("weapons/{n}/icon.png"));
        }
        commands.entity(e).insert(Armory {
            items,
            cells,
            shown: vec![],
            category: 1,
            selected: None,
            loadout: SavedLoadout::load(),
            dirty: true,
        });
        // the character holding the selected weapon, behind the stats panel
        if let Some(p) = &preview {
            commands.entity(e).with_child((
                Name::new("armory character"),
                ImageNode::new(p.0.clone()),
                Node {
                    position_type: PositionType::Absolute,
                    left: vw(75.0),
                    top: vh(60.0 / 7.2),
                    width: vw(25.0),
                    height: vh(600.0 / 7.2),
                    ..default()
                },
                GlobalZIndex(-500),
                Pickable::IGNORE,
            ));
        }
        // the purple background (scripts/make_armory_bg.py)
        commands.entity(e).with_child((
            Name::new("armory background"),
            ImageNode::new(assets.load("ui/armory/textures/armory_bg.png")),
            Node { position_type: PositionType::Absolute, left: px(0.0), top: px(0.0), width: vw(100.0), height: vh(100.0), ..default() },
            GlobalZIndex(-1000),
            Pickable::IGNORE,
        ));
    }
}

fn clicks(
    mut clicks: MessageReader<NguiClick>,
    mut screens: Query<(&NguiScreen, &mut Armory)>,
    mut shoot: ResMut<crate::armory_preview::PreviewShoot>,
) {
    for c in clicks.read() {
        let Ok((s, mut a)) = screens.get_mut(c.screen) else { continue };
        if let Some(k) = TABS.iter().position(|t| *t == c.name) {
            a.category = k as i32 + 1;
            a.selected = None;
            a.dirty = true;
        } else if let Some(k) = a.cells.iter().position(|&n| n == c.node) {
            // a weapon cell selects it (stats, preview); "Equip" then equips it (OBSERVED)
            let Some(name) = a.shown.get(k).cloned() else { continue };
            a.selected = Some(name);
            a.dirty = true;
        } else if c.name == "BtnShoot" {
            shoot.start();
        } else if c.name == "BtnEquip" {
            let Some(name) = a.selected.clone() else { continue };
            let cat = a.category;
            a.loadout.equipped.insert(cat, name.clone());
            a.loadout.save();
            info!("armory: equipped {name} in category {cat}");
            a.dirty = true;
        }
        let _ = s;
    }
}

fn refresh(
    mut screens: Query<(&mut NguiScreen, &mut Armory)>,
    assets: Res<AssetServer>,
    mut preview: ResMut<crate::armory_preview::PreviewWeapon>,
) {
    for (mut s, mut a) in &mut screens {
        if !a.dirty {
            continue;
        }
        a.dirty = false;
        let cat = a.category;
        // tabs: the open one is drawn pressed; each shows the icon of the weapon equipped in
        // that category instead of the grey silhouette (OBSERVED)
        for (k, name) in TABS.iter().enumerate() {
            let Some(t) = s.find(name) else { continue };
            let tab_cat = k as i32 + 1;
            if let Some(p) = s.find_in(t, "Pressed") {
                s.set_active(p, tab_cat == cat);
            }
            let weapon = equipped_in(&a.loadout, tab_cat)
                .filter(|w| a.items.iter().any(|(n, _)| n == w))
                .map(str::to_string);
            let kids: Vec<usize> = s.subtree(t).into_iter().filter(|&j| s.nodes[j].parent == Some(t)).collect();
            for j in kids {
                let n = s.nodes[j].name.clone();
                if n == "Texture" || n == "PrimaryIcon" {
                    if let Some(w) = &weapon {
                        s.set_texture(j, assets.load(format!("weapons/{w}/icon.png")));
                    }
                    s.set_active(j, weapon.is_some());
                } else if n == "Texture (1)" || n == "PrimaryIcon (1)" {
                    s.set_active(j, weapon.is_none());
                }
            }
        }
        let list: Vec<(String, Item)> = a.items.iter().filter(|(_, i)| i.categoryNabor == cat).cloned().collect();
        let equipped = equipped_in(&a.loadout, cat).map(str::to_string);
        let selected = a.selected.clone().or(equipped.clone()).or_else(|| list.first().map(|(n, _)| n.clone()));
        a.shown = list.iter().map(|(n, _)| n.clone()).collect();
        for (k, &c) in a.cells.iter().enumerate() {
            let Some((name, _)) = list.get(k) else {
                s.set_active(c, false);
                continue;
            };
            s.set_active(c, true);
            let (col, row) = ((k % COLUMNS) as f32, (k / COLUMNS) as f32);
            s.set_pos(c, FIRST + Vec2::new(col * PITCH.x, -row * PITCH.y));
            if let Some(i) = s.find_in(c, "ItemTexture") {
                s.set_texture(i, assets.load(format!("weapons/{name}/icon.png")));
            }
            if let Some(i) = s.find_in(c, "Level_Label_newbalance") {
                s.set_text(i, "Lvl. 1");
            }
            let rarity = list[k].1.rarity;
            // band behind "Lvl." in the rarity's colour (OBSERVED: light blue for Common)
            if let Some(i) = s.find_in(c, "BGrare") {
                s.set_color(i, rarity_band(rarity));
            }
            // owned items get a blue "bought" tint (BoughtIndicatorBg, off in the prefab);
            // the equipped one stays green. OBSERVED colour: about [50, 83, 137] in the cell
            if let Some(i) = s.find_in(c, "BoughtIndicatorBg") {
                let on = equipped.as_deref() != Some(name);
                s.set_color(i, [0.07, 0.31, 0.6, if on { 0.5 } else { 0.0 }]);
            }
            if let Some(i) = s.find_in(c, "EQUIPPED") {
                s.set_active(i, equipped.as_deref() == Some(name));
            }
            if let Some(i) = s.find_in(c, "SeceltionIndicator") {
                s.set_active(i, selected.as_deref() == Some(name));
            }
        }
        preview.wanted = selected.clone();
        // Equip for a weapon that is not equipped, the dark "Equipped" box for the one that is
        let is_equipped = selected.is_some() && selected == equipped;
        for (name, on) in [("BtnEquip", selected.is_some() && !is_equipped), ("Equipped", is_equipped)] {
            if let Some(i) = s.find_path(&format!("btn0/{name}")) {
                s.set_active(i, on);
            }
        }
        // stats panel of the selected weapon
        let item = selected.as_ref().and_then(|n| list.iter().find(|(m, _)| m == n)).map(|(_, i)| i.clone());
        stats_panel(&mut s, item.as_ref());
    }
}

/// Draw the character preview only while the Armory is on screen.
fn preview_on_when_open(
    screens: Query<&InheritedVisibility, With<Armory>>,
    mut cams: Query<&mut Camera, With<crate::armory_preview::PreviewCamera>>,
) {
    let open = screens.iter().any(|v| v.get());
    for mut c in &mut cams {
        if c.is_active != open {
            c.is_active = open;
        }
    }
}

/// Colour of the level band of a cell by rarity.
fn rarity_band(r: i32) -> [f32; 4] {
    match r {
        1 => [0.36, 0.89, 0.36, 1.0],
        2 => [0.22, 0.78, 1.0, 1.0],
        3 => [0.78, 0.42, 1.0, 1.0],
        4 => [1.0, 0.82, 0.0, 1.0],
        5 => [1.0, 0.31, 0.43, 1.0],
        _ => [0.37, 0.76, 1.0, 1.0],
    }
}

fn rarity_name(r: i32) -> &'static str {
    ["Common", "Uncommon", "Rare", "Epic", "Legendary", "Mythical"].get(r.max(0) as usize).copied().unwrap_or("Common")
}

/// Colour codes of the rarity names (OBSERVED: "Common" white, "Rare" light blue).
fn rarity_color(r: i32) -> &'static str {
    ["ffffff", "5fe35f", "37c8ff", "c86bff", "ffd200", "ff4f6e"].get(r.max(0) as usize).copied().unwrap_or("ffffff")
}

fn stats_panel(s: &mut NguiScreen, item: Option<&Item>) {
    let guns = s.find("Guns_Properties");
    let melee = s.find("Melee_Properties");
    let Some(item) = item else {
        for p in [guns, melee].into_iter().flatten() {
            s.set_active(p, false);
        }
        return;
    };
    let is_melee = item.isMelee != 0;
    if let Some(p) = guns {
        s.set_active(p, !is_melee);
    }
    if let Some(p) = melee {
        s.set_active(p, is_melee);
    }
    let Some(panel) = (if is_melee { melee } else { guns }) else { return };
    let name = s.string(&item.localizeWeaponKey);
    // our approximations of the shop numbers (the live game takes them from its servers):
    // efficiency = damage per second tier 0, mobility 50 at normal speed
    let eff = item.dpses.first().copied().unwrap_or(0.0).round().max(1.0) as i32;
    let mobility = (50.0 + (item.speedModifier - 1.0) * 300.0).round().clamp(0.0, 100.0) as i32;
    for i in s.subtree(panel) {
        if matches!(s.nodes[i].name.as_str(), "ArrowUp" | "ArrowDown") {
            s.set_active(i, false);
        }
    }
    let rarity = format!("[{}]{}[-]", rarity_color(item.rarity), rarity_name(item.rarity));
    let set = |s: &mut NguiScreen, path: &str, text: String| {
        let found = s.subtree(panel).into_iter().find(|&j| s.path(j).ends_with(path));
        if let Some(i) = found {
            for j in s.subtree(i) {
                if s.nodes[j].label.is_some() {
                    s.set_text(j, text.clone());
                }
            }
        }
    };
    if is_melee {
        set(s, "Melee_Properties/GunName", name);
        set(s, "WeaponEfficiency/Count", eff.to_string());
        set(s, "Fire_Rate/FireRateMeleeLab", item.fireRateShop.to_string());
        set(s, "Mobility/CapacityLabel", mobility.to_string());
        set(s, "Rarity/Label", rarity);
    } else {
        set(s, "Guns_Properties/GunName", name);
        set(s, "WeaponEfficiency/Count", eff.to_string());
        set(s, "Fire_Rate/FireRateLabel", item.fireRateShop.to_string());
        set(s, "Capacity/CapacityLabel", item.ammoInClip.to_string());
        set(s, "Mobility/MobilityLabel", mobility.to_string());
        set(s, "Rariry/Label", rarity);
    }
}
