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
            .add_systems(Update, ((setup, clicks, refresh).chain(), preview_on_when_open, fade_toasts));
    }
}

/// Category tabs along the top, in categoryNabor order (1 primary .. 6 heavy).
const TABS: [&str; 6] = ["PrimaryCategory", "BackupCategory", "MeleeCategory", "SpecilCategory", "SniperCategory", "PremiumCategory"];

/// Sections in the left column: (button, its category tabs along the top).
const SECTIONS: [(&str, &str); 7] = [
    ("BtnCategory1", "GridCategoriesWeapons"),
    ("BtnCategory3", "GridCategoriesWear"),
    ("BtnCategory6", "GridCategoriesGadgets"),
    ("BtnCategory_Misc", "GridCategoriesMisc"),
    ("BtnCategory4", "GridCategoriesBattle"),
    ("BtnCategory_Pets", "GridCategoriesPets"),
    ("BtnCategory8", "GridCategoriesCraft"),
];

/// Grid layout in 1_FirstGrid's space. OBSERVED (1280x720 screenshots, 0.9375 px per
/// virtual pixel): 4 columns 204 px apart, rows 180 px apart, cells 194x178 px.
const COLUMNS: usize = 4;
const PITCH: Vec2 = Vec2::new(217.3, 192.0);
/// Centre of the first cell. OBSERVED: (212, 199) px in the screenshots.
const FIRST: Vec2 = Vec2::new(-327.0, 144.7);
const CELL_SCALE: f32 = 1.2;
/// Most weapons one category can show (cells made once and reused).
const MAX_CELLS: usize = 16;

/// The weapon chosen in each category (categoryNabor -> weapon name), saved between runs.
#[derive(Resource, Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct SavedLoadout {
    pub equipped: HashMap<i32, String>,
    /// weapons bought in the Armory (the starter weapons are always owned)
    pub owned: Vec<String>,
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
    /// effect lines under the stats ("Automatic", "Zoom", ...)
    InShopEffects: Vec<i32>,
    /// from the game's balance tables (balance.json, filled in at load)
    #[serde(skip)]
    price: i64,
    #[serde(skip)]
    gems: bool,
    #[serde(skip)]
    level: i32,
    #[serde(skip)]
    shop: Option<ShopStats>,
}

/// The Armory's numbers for a weapon (BalanceWeapons: SR fire rate, A capacity,
/// SM mobility).
#[derive(Deserialize, Clone, Copy, Default)]
#[serde(default)]
#[allow(non_snake_case)]
struct ShopStats {
    SR: Option<f32>,
    A: Option<f32>,
    SM: Option<f32>,
}

/// Per weapon number: unlock level, grade, price coefficient, currency (BalanceWeaponsEconomic).
#[derive(Deserialize, Clone, Copy, Default)]
#[serde(default)]
#[allow(non_snake_case)]
struct Economy {
    L: Option<i32>,
    G: Option<i32>,
    PC: Option<f32>,
    BC: Option<i32>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Balance {
    /// "RC" = rare grade, coins; ... 65 levels each (BalanceEconomic.WeaponPrices)
    prices: HashMap<String, Vec<f32>>,
    economy: HashMap<String, Economy>,
    shop: HashMap<String, ShopStats>,
}

impl Balance {
    /// The original's price (ItemRecord.get_PriceItemRecord, GameAssembly 0x835640):
    /// the price table of the weapon's grade (common and uncommon use the rare row) at the
    /// weapon's level, times its coefficient, rounded to 5, at least 5. The level is the
    /// weapon's unlock level here: the live game raises it with the player's level (a
    /// level 65 account sees "Lvl. 55" and RC[54] = 530 coins, as in the screenshots).
    fn price(&self, number: &str) -> Option<(i64, bool, i32)> {
        let e = self.economy.get(number)?;
        let level = e.L.unwrap_or(1).clamp(1, 65);
        let grade = match e.G.unwrap_or(0) {
            3 => "E",
            4 => "L",
            5 => "M",
            _ => "R",
        };
        let gems = e.BC == Some(1);
        let row = self.prices.get(&format!("{grade}{}", if gems { "G" } else { "C" }))?;
        let base = *row.get((level - 1) as usize).or(row.last())?;
        let p = ((base * e.PC.unwrap_or(1.0) / 5.0).round() * 5.0).max(5.0) as i64;
        Some((p, gems, level))
    }
}

impl Item {
    /// our approximation of the shop's Efficiency number (the live game gets it from its
    /// servers): damage per second at tier 0
    fn efficiency(&self) -> i32 {
        self.dpses.first().copied().unwrap_or(0.0).round().max(1.0) as i32
    }
}

/// InShopEffects value -> (translation key, stat icon). Values matched against the
/// screenshots (Machine Gun 0 "Automatic", Pixel Gun 1 "Single shots", Shotgun 5,
/// Combat Knife 39 "Melee", sniper rifles 11 "Zoom", Wizard Wand / Apocalypse 10
/// "Area damage", Apocalypse 2 "Rockets").
fn shop_effect(v: i32) -> Option<(&'static str, &'static str)> {
    Some(match v {
        0 => ("Key_1391", "shop_stats_auto"),
        1 => ("Key_1392", "shop_stats_sngl"),
        2 => ("Key_1394", "shop_stats_rkt"),
        5 => ("Key_1390", "shop_stats_shtgn"),
        10 => ("Key_0403", "shop_stats_area_dmg"),
        11 => ("Key_0404", "shop_stats_zoom"),
        39 => ("Key_0441", "shop_stats_melee"),
        _ => return None,
    })
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
    /// open section (SECTIONS index; 0 = Weapon) and, outside Weapon, its open tab
    section: usize,
    tab: usize,
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
        // Outside Weapon nothing is equipped yet: the game would put the equipped item's
        // icon on each tab, else the tab's empty-slot icon ("... (1)", EmptyIcon,
        // *_Empty). The prefab's sample icons (rifles, hats) go.
        for (_, grid) in &SECTIONS[1..] {
            for t in section_tabs(&s, grid) {
                let kids: Vec<usize> = s.subtree(t).into_iter().filter(|&j| s.nodes[j].parent == Some(t)).collect();
                let tab = s.nodes[t].name.clone();
                for j in kids {
                    let n = s.nodes[j].name.clone();
                    if matches!(n.as_str(), "Texture" | "PrimaryIcon" | "icon_Graffiti") {
                        // the Skins tab shows the character's face
                        s.set_active(j, tab == "SkinsCategory");
                    }
                }
            }
        }
        // the Craft row's first slot belongs to a tab the game leaves out; its tabs start
        // at the left like the others (OBSERVED)
        if let Some(i) = s.find("GridCategoriesCraft") {
            s.set_pos(i, Vec2::new(-88.0, 0.0));
        }
        // the Skins tab's face is drawn the size of the other tabs' icons (OBSERVED)
        if let Some(i) = s.find_path("SkinsCategory/PrimaryIcon") {
            s.set_size(i, Vec2::new(64.0, 64.0));
            s.set_pos(i, Vec2::new(0.0, 6.0));
        }
        // the Nest: no egg is hatching, so no "Egg is ready" / timer text over it
        for name in ["EggIsReadyLabel", "TimerUI"] {
            if let Some(i) = s.find_path(&format!("ArsenalNestButton/Holder/{name}")) {
                s.set_active(i, false);
            }
        }
        // the left column's sections and every section's tabs can be clicked
        for (btn, grid) in SECTIONS {
            if let Some(i) = s.find(btn) {
                s.make_button(i);
            }
            for t in section_tabs(&s, grid) {
                s.make_button(t);
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
        let balance: Balance = std::fs::read_to_string(dir.join("balance.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        for (n, i) in &mut items {
            let num = n.trim_start_matches("Weapon");
            let (p, g, l) = balance.price(num).unwrap_or((500, false, 1));
            (i.price, i.gems, i.level) = (p, g, l);
            i.shop = balance.shop.get(num).copied();
        }
        for (n, _) in &items {
            let _ = assets.load::<Image>(format!("weapons/{n}/icon.png"));
        }
        commands.entity(e).insert(Armory {
            items,
            cells,
            shown: vec![],
            category: 1,
            section: 0,
            tab: 0,
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
    mut commands: Commands,
    mut clicks: MessageReader<NguiClick>,
    mut screens: Query<(&NguiScreen, &mut Armory)>,
    mut shoot: ResMut<crate::armory_preview::PreviewShoot>,
    mut wallet: ResMut<crate::wallet::Wallet>,
) {
    for c in clicks.read() {
        let Ok((s, mut a)) = screens.get_mut(c.screen) else { continue };
        if let Some(k) = SECTIONS.iter().position(|(b, _)| *b == c.name) {
            a.section = k;
            a.tab = 0;
            a.dirty = true;
        } else if let Some(k) = section_tabs(s, SECTIONS[a.section].1).iter().position(|&t| t == c.node).filter(|_| a.section != 0) {
            a.tab = k;
            a.dirty = true;
        } else if let Some(k) = TABS.iter().position(|t| *t == c.name) {
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
        } else if c.name == "BtnBuy" {
            let Some(name) = a.selected.clone() else { continue };
            let Some((price, gems)) = a.items.iter().find(|(n, _)| *n == name).map(|(_, i)| (price(i), i.gems)) else { continue };
            if a.loadout.owns(&name) {
                continue;
            }
            let (have, what) = if gems { (wallet.gems, "gems") } else { (wallet.coins, "coins") };
            if have < price {
                info!("armory: {name} costs {price} {what}, only {have}");
                toast(&mut commands, format!("Not enough {what}: {price} needed, you have {have}"));
                continue;
            }
            if gems {
                wallet.add_gems(-price);
            } else {
                wallet.add_coins(-price);
            }
            // bought weapons are equipped straight away (OBSERVED in clip h_rec1)
            let cat = a.category;
            a.loadout.owned.push(name.clone());
            a.loadout.equipped.insert(cat, name.clone());
            a.loadout.save();
            info!("armory: bought {name} for {price} and equipped it");
            a.dirty = true;
        } else if c.name == "BtnEquip" {
            let Some(name) = a.selected.clone() else { continue };
            if !a.loadout.owns(&name) {
                continue;
            }
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
        // left column: the open section is drawn pressed, its tabs shown along the top
        for (k, (btn, grid)) in SECTIONS.iter().enumerate() {
            if let Some(b) = s.find(btn)
                && let Some(p) = s.find_in(b, "PressedState")
            {
                s.set_active(p, k == a.section);
            }
            if let Some(g) = s.find(grid) {
                s.set_active(g, k == a.section);
            }
        }
        if let Some(i) = s.find_path("ArmoryView/NoOffers") {
            s.set_active(i, a.section != 0);
        }
        // the stats panel only for weapons
        if let Some(i) = s.find_path("Camera/Right (OFF)") {
            s.set_active(i, a.section == 0);
        }
        if a.section != 0 {
            // sections we have no items for yet: their tabs, an empty grid and "No Items!"
            for (k, t) in section_tabs(&s, SECTIONS[a.section].1).into_iter().enumerate() {
                if let Some(p) = s.find_in(t, "Pressed") {
                    s.set_active(p, k == a.tab);
                }
            }
            for &c in &a.cells {
                s.set_active(c, false);
            }
            for name in ["Guns_Properties", "Melee_Properties"] {
                if let Some(i) = s.find(name) {
                    s.set_active(i, false);
                }
            }
            for path in ["PanelProperties (OFF)/Special", "Properties (OFF)/btn0/BtnEquip"] {
                if let Some(i) = s.find_path(path) {
                    s.set_active(i, false);
                }
            }
            continue;
        }
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
            let owned = a.loadout.owns(name);
            // owned weapons are at level 1 (no upgrades yet); locked ones show the level
            // they come at
            if let Some(i) = s.find_in(c, "Level_Label_newbalance") {
                s.set_text(i, format!("Lvl. {}", if owned { 1 } else { list[k].1.level }));
            }
            // locked weapons show their price next to a coin (OBSERVED "Lvl. 55  530")
            if let Some(i) = s.find_in(c, "GunPrice") {
                s.set_active(i, !owned);
                s.set_text(i, crate::results::group(price(&list[k].1) as i32));
                let gems = list[k].1.gems;
                for (icon, on) in [("UpgradeArrow", false), ("Coin", !gems), ("Gem", gems)] {
                    if let Some(j) = s.find_in(i, icon) {
                        s.set_active(j, on);
                    }
                }
            }
            let rarity = list[k].1.rarity;
            // band behind "Lvl." in the rarity's colour (OBSERVED: light blue for Common)
            if let Some(i) = s.find_in(c, "BGrare") {
                s.set_color(i, rarity_band(rarity));
            }
            // owned items get a blue "bought" tint (BoughtIndicatorBg, off in the prefab);
            // the equipped one stays green. OBSERVED colour: about [50, 83, 137] in the cell
            if let Some(i) = s.find_in(c, "BoughtIndicatorBg") {
                let on = owned && equipped.as_deref() != Some(name);
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
        let sel_owned = selected.as_ref().is_some_and(|n| a.loadout.owns(n));
        // nothing there for the equipped weapon (OBSERVED)
        for (name, on) in [("BtnEquip", sel_owned && !is_equipped), ("Equipped", false)] {
            if let Some(i) = s.find_path(&format!("btn0/{name}")) {
                s.set_active(i, on);
            }
        }
        // a locked weapon: its price and "Buy" in the same place
        let sel_price = selected.as_ref().and_then(|n| list.iter().find(|(m, _)| m == n)).map(|(_, i)| price(i));
        let sel_gems = selected.as_ref().and_then(|n| list.iter().find(|(m, _)| m == n)).is_some_and(|(_, i)| i.gems);
        if let Some(b2) = s.find_path("Properties (OFF)/btn2") {
            s.set_active(b2, selected.is_some() && !sel_owned);
            let kids: Vec<usize> = s.subtree(b2).into_iter().filter(|&j| s.nodes[j].parent == Some(b2)).collect();
            for j in kids {
                let n = s.nodes[j].name.clone();
                s.set_active(j, n == "BtnBuy" || n == "PriceContainer");
            }
            if let Some(pc) = s.find_in(b2, "PriceContainer") {
                if let Some(inner) = s.subtree(pc).into_iter().find(|&j| s.nodes[j].parent == Some(pc)) {
                    s.set_active(inner, true);
                }
                if let Some(l) = s.find_in(pc, "PriceInfoLabel") {
                    s.set_text(l, crate::results::group(sel_price.unwrap_or(0) as i32));
                }
                if let Some(g) = s.find_in(pc, "Gem") {
                    s.set_active(g, sel_gems);
                }
                if let Some(g) = s.find_in(pc, "Gold") {
                    s.set_active(g, !sel_gems);
                }
            }
        }
        // stats panel of the selected weapon
        let item = selected.as_ref().and_then(|n| list.iter().find(|(m, _)| m == n)).map(|(_, i)| i.clone());
        let equipped_item = equipped.as_ref().and_then(|n| list.iter().find(|(m, _)| m == n)).map(|(_, i)| i.clone());
        stats_panel(&mut s, item.as_ref(), equipped_item.as_ref());
    }
}

impl SavedLoadout {
    /// Starter weapons (the default loadout) and bought ones.
    pub fn owns(&self, weapon: &str) -> bool {
        crate::weapons::DEFAULT_LOADOUT.contains(&weapon) || self.owned.iter().any(|w| w == weapon)
    }
}

fn price(item: &Item) -> i64 {
    item.price
}

/// A short message at the bottom of the screen.
#[derive(Component)]
struct Toast(f32);

fn toast(commands: &mut Commands, text: String) {
    commands.spawn((
        Toast(2.5),
        Text::new(text),
        TextFont { font_size: FontSize::Px(20.0), ..default() },
        TextColor(Color::WHITE),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
        Node {
            position_type: PositionType::Absolute,
            bottom: px(40.0),
            left: percent(25.0),
            width: percent(50.0),
            padding: UiRect::all(px(10.0)),
            justify_content: JustifyContent::Center,
            ..default()
        },
        GlobalZIndex(100_000),
    ));
}

fn fade_toasts(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut Toast)>) {
    for (e, mut t) in &mut q {
        t.0 -= time.delta_secs();
        if t.0 <= 0.0 {
            commands.entity(e).despawn();
        }
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
/// The tab buttons of a section's top row (direct children of its grid).
fn section_tabs(s: &NguiScreen, grid: &str) -> Vec<usize> {
    let Some(g) = s.find(grid) else { return vec![] };
    s.subtree(g).into_iter().filter(|&j| s.nodes[j].parent == Some(g)).collect()
}

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

fn stats_panel(s: &mut NguiScreen, item: Option<&Item>, equipped: Option<&Item>) {
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
    let eff = item.efficiency();
    // the game's own shop numbers (BalanceWeapons), else our approximations
    let shop = item.shop.unwrap_or_default();
    let mobility = shop.SM.map_or_else(|| (50.0 + (item.speedModifier - 1.0) * 300.0).round().clamp(0.0, 100.0), |v| v.round()) as i32;
    let fire_rate = shop.SR.map_or(item.fireRateShop, |v| v.round() as i32);
    let capacity = shop.A.map_or(item.ammoInClip, |v| v.round() as i32);
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
        set(s, "Fire_Rate/FireRateMeleeLab", fire_rate.to_string());
        set(s, "Mobility/CapacityLabel", mobility.to_string());
        set(s, "Rarity/Label", rarity);
    } else {
        set(s, "Guns_Properties/GunName", name);
        set(s, "WeaponEfficiency/Count", eff.to_string());
        set(s, "Fire_Rate/FireRateLabel", fire_rate.to_string());
        set(s, "Capacity/CapacityLabel", capacity.to_string());
        set(s, "Mobility/MobilityLabel", mobility.to_string());
        set(s, "Rariry/Label", rarity);
    }
    // Efficiency compared with the equipped weapon: yellow when it is that one or as
    // good, red when weaker, green when stronger (OBSERVED: equipped yellow, weaker red)
    let col = match equipped.map(|e| eff.cmp(&e.efficiency())) {
        Some(std::cmp::Ordering::Less) => [1.0, 0.25, 0.2, 1.0],
        Some(std::cmp::Ordering::Greater) => [0.3, 1.0, 0.2, 1.0],
        _ => [1.0, 0.93, 0.0, 1.0],
    };
    if let Some(we) = s.subtree(panel).into_iter().find(|&j| s.nodes[j].name == "WeaponEfficiency") {
        for j in s.subtree(we) {
            if matches!(s.nodes[j].name.as_str(), "Label" | "Count" | "Icon") {
                s.set_color(j, col);
            }
        }
    }
    effects(s, item);
}

/// The row under the stats with the weapon's effects ("Automatic", "Zoom", ...), at most
/// three: two side by side, a third centred below (the prefab's layout).
fn effects(s: &mut NguiScreen, item: &Item) {
    let Some(special) = s.find_path("PanelProperties (OFF)/Special") else { return };
    s.set_active(special, true);
    if let Some(i) = s.find_in(special, "DescriptionGadget") {
        s.set_active(i, false);
    }
    let fx: Vec<(&str, &str)> = item.InShopEffects.iter().filter_map(|v| shop_effect(*v)).take(3).collect();
    let slots = ["EffectsLabel1", "EffectsLabel3", "EffectsLabel5", "EffectsLabel2", "EffectsLabel4"];
    let pos: &[Vec2] = match fx.len() {
        1 => &[Vec2::new(21.0, 0.0)],
        2 => &[Vec2::new(-45.0, 0.0), Vec2::new(87.0, 0.0)],
        _ => &[Vec2::new(-45.0, 13.0), Vec2::new(87.0, 13.0), Vec2::new(21.0, -12.0)],
    };
    for (k, slot) in slots.iter().enumerate() {
        let Some(l) = s.find_in(special, slot) else { continue };
        let Some((key, icon)) = fx.get(k) else {
            s.set_active(l, false);
            continue;
        };
        s.set_active(l, true);
        let text = s.string(key);
        s.set_text(l, text);
        if let Some(p) = pos.get(k) {
            s.set_pos(l, *p);
        }
        if let Some(sp) = s.subtree(l).into_iter().find(|&j| j != l && s.nodes[j].sprite.is_some()) {
            s.set_sprite(sp, icon);
        }
    }
}
