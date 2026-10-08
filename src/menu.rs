//! Start menu: the original game's lobby screen (re-drawn from the exported NGUI data, see
//! ngui.rs). "To Battle" / "Campaign" open a map picker; buttons that need the game's
//! online services show a short note instead. Without an exported lobby we fall back to a
//! plain map list.
//!
//! The menu is its own small Bevy app. Picking a map starts a fresh copy of this program
//! with the map name (`pixel-gun-3d-rs <map>`) and closes the menu; F10 in game does the
//! reverse. Restarting keeps the game code simple: every match starts from a clean world.

use bevy::{app::AppExit, prelude::*};

use serde::Deserialize;

use crate::DataDir;

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(Color::srgb(0.08, 0.1, 0.14)))
            .insert_resource(crate::map::CurrentMap(std::env::var("PG3D_LOBBY").unwrap_or_else(|_| "lobby".into())))
            .init_resource::<crate::map::MapInfo>()
            .add_plugins((crate::ngui::NguiPlugin, crate::lighting::LightingPlugin, crate::settings_ui::SettingsUiPlugin))
            .add_systems(PreStartup, lobby_scene)
            .add_systems(Startup, build_menu)
            .add_systems(Update, (character_idle, test_click))
            .add_systems(Update, (button_colors, button_clicks, lobby_clicks, fade_toast, setup_lobby, lobby_wallet, lobby_avatar, event_button_gradient));
    }
}

#[derive(Component)]
struct MapButton(String);

#[derive(Component)]
struct QuitButton;

const NORMAL: Color = Color::srgb(0.18, 0.22, 0.3);
const HOVER: Color = Color::srgb(0.28, 0.36, 0.5);

/// Maps that have been exported: <data>/maps/<name>/map.gltf
pub fn exported_maps(data: &std::path::Path) -> Vec<String> {
    let mut maps: Vec<String> = std::fs::read_dir(data.join("maps"))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("map.gltf").exists())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|m| m != "lobby")
        .collect();
    maps.sort();
    maps
}

#[derive(Component)]
struct MapPicker {
    sandbox: bool,
    /// Team Fight (PG3D_MODE=teamfight) instead of the zombie campaign
    team: bool,
}

#[derive(Component)]
struct CloseButton;

#[derive(Component)]
struct Toast(f32);

#[derive(Deserialize, Default)]
struct LobbyCam {
    position: Vec3,
    forward: Vec3,
    up: Vec3,
    fov: f32,
}

#[derive(Deserialize, Default)]
struct LobbyInfo {
    #[serde(default)]
    cameras: Vec<LobbyCam>,
}

#[derive(Component)]
struct LobbyCharacter {
    graph: Handle<AnimationGraph>,
    idle: AnimationNodeIndex,
    attached: bool,
}

/// The 3D lobby behind the menu: the lobby scene (level17) with the current season's
/// decoration, the player character playing its idle animation, and the lobby music.
/// Falls back to a plain background if the lobby was not exported.
fn lobby_scene(
    settings: Res<crate::settings::Settings>,
    mut commands: Commands,
    data: Res<DataDir>,
    assets: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    // PG3D_LOBBY=lobby_<season> (testing): another exported season lobby
    let lobby = std::env::var("PG3D_LOBBY").unwrap_or_else(|_| "lobby".into());
    let dir = data.0.join(format!("maps/{lobby}"));
    let info_text = std::fs::read_to_string(dir.join("info.json")).unwrap_or_default();
    let cams: LobbyInfo = serde_json::from_str(&info_text).unwrap_or_default();
    if let Ok(map_info) = serde_json::from_str::<crate::map::MapInfo>(&info_text) {
        commands.insert_resource(map_info);
    }
    let cam = cams.cameras.first();
    let transform = cam
        .map(|c| Transform::from_translation(c.position).looking_to(c.forward, c.up))
        .unwrap_or_else(|| Transform::from_xyz(0.0, 2.0, 6.0).looking_at(Vec3::Y, Vec3::Y));
    let fov = cam.map_or(60.0, |c| c.fov);
    commands.spawn((Camera3d::default(), Projection::Perspective(PerspectiveProjection { fov: fov.to_radians(), ..default() }), transform));
    if !dir.join("map.gltf").exists() {
        return;
    }
    commands.spawn(WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(format!("maps/{lobby}/map.gltf")))));
    let char_path = "characters/default/character.gltf";
    if data.0.join(char_path).exists() {
        let (graph, idle) = AnimationGraph::from_clip(assets.load(GltfAssetLabel::Animation(0).from_asset(char_path)));
        commands.spawn((
            Name::new("lobby character"),
            LobbyCharacter { graph: graphs.add(graph), idle, attached: false },
            WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(char_path))),
            Transform::default(),
        ));
    }
    if dir.join("sounds/music.wav").exists() {
        commands.spawn((
            AudioPlayer::new(assets.load(format!("maps/{lobby}/sounds/music.wav"))),
            PlaybackSettings::LOOP.with_volume(bevy::audio::Volume::Linear(settings.music_volume())),
            crate::settings_ui::Music,
        ));
    }
}

/// Put the character where the lobby's character holder is and start its idle loop.
fn character_idle(
    mut commands: Commands,
    mut chars: Query<(Entity, &mut LobbyCharacter)>,
    names: Query<(Entity, &Name, &GlobalTransform)>,
    children: Query<&Children>,
    mut players: Query<&mut AnimationPlayer>,
) {
    for (e, mut c) in &mut chars {
        if c.attached {
            continue;
        }
        let Some((holder, _, gt)) = names.iter().find(|(_, n, _)| n.as_str() == "MainMenu_Pers_MainMenu") else { continue };
        let Some(player) = children.iter_descendants(e).find(|d| players.contains(*d)) else { continue };
        commands.entity(e).insert(gt.compute_transform());
        let _ = holder;
        if let Ok(mut p) = players.get_mut(player) {
            p.play(c.idle).repeat();
        }
        commands.entity(player).insert(AnimationGraphHandle(c.graph.clone()));
        c.attached = true;
    }
}

/// The two original screens: the lobby and the game mode hub ("To Battle").
#[derive(Resource, Default)]
struct Screens {
    lobby: Option<Entity>,
    modes: Option<Entity>,
    settings: Option<Entity>,
    armory: Option<Entity>,
}

fn build_menu(mut commands: Commands, data: Res<DataDir>, assets: Res<AssetServer>) {
    let lobby = crate::ngui::spawn_screen(&mut commands, &assets, &data, "lobby");
    let modes = crate::ngui::spawn_screen(&mut commands, &assets, &data, "modes");
    let settings = crate::ngui::spawn_screen(&mut commands, &assets, &data, "settings");
    let armory = crate::ngui::spawn_screen(&mut commands, &assets, &data, "armory");
    // PG3D_ARMORY=1 (tests): start in the Armory
    let start_armory = std::env::var("PG3D_ARMORY").is_ok();
    for e in [modes, settings].into_iter().chain([armory].into_iter().filter(|_| !start_armory)).flatten() {
        commands.entity(e).insert(Visibility::Hidden);
    }
    if lobby.is_none() {
        warn!("no exported lobby UI (run export_ngui.py level17 MainMenuNGUI lobby); using plain menu");
        spawn_map_picker(&mut commands, &data, &assets, false, false, false);
    }
    if start_armory {
        show(&mut commands, lobby, false);
    }
    commands.insert_resource(Screens { lobby, modes, settings, armory });
}

fn show(commands: &mut Commands, e: Option<Entity>, on: bool) {
    if let Some(e) = e {
        commands.entity(e).insert(if on { Visibility::Inherited } else { Visibility::Hidden });
    }
}

/// The original fills in and switches parts of the lobby at run time (player data from
/// the server, button press states, events). Put it into a sensible offline state.
fn setup_lobby(
    mut screens: Query<&mut crate::ngui::NguiScreen, Added<crate::ngui::NguiScreen>>,
    assets: Res<AssetServer>,
    data: Res<DataDir>,
    settings: Res<crate::settings::Settings>,
    controls: Res<crate::controls::Controls>,
) {
    for mut s in &mut screens {
        if s.nodes.first().is_some_and(|n| n.name == "SettingsWindow") {
            crate::settings_ui::setup_settings(&mut s, &settings, &controls);
            continue;
        }
        if s.nodes.first().is_some_and(|n| n.name == "AllModesView") {
            setup_modes(&mut s, &assets, &data);
            continue;
        }
        let hide_names = [
            "Pressed",         // button pressed look; NGUI swaps it in while you hold the button
            "State_Locked",    // "unlocks at level X" overlays
            "state_Queue",     // event download queue
            "icon_loading",
        ];
        let hide_paths = [
            // a downloaded event's button; the Event Center button sits in the same spot
            "RightAnchor/TemplateEventLobbyButtonLoader",
            // tournaments and the special-offer button need the online services
            "BotBtnsLayout_Online/BtnCompetition",
            "SettingsBtnsLayout/BtnSpecial",
            "BtnBattle_A/Normal/NotifRewardForModes",
            "panel_Boosters/Layout/FirstPurchaseGiftLobbyButton",
            "RightAnchor/ConfrontationEvent_LobbyButton",
            "LeftAnchor/LobbyTopBtnsLayout/OfferwallButton2",
            // offer banners (Soldier's Choice etc.) come from the server; offline the list
            // is an empty frame
            "LeftAnchor/UniversalMiniBannerListUI",
        ];
        // left column buttons react through ButtonHandler/OnClickAction instead of an
        // NGUI UIButton, so they have no click area of their own
        for name in ["BtnArmory", "BtnStoreLobby_New", "BtnAghanim"] {
            if let Some(i) = s.find(name) {
                s.make_button(i);
            }
        }
        // PlatformDependentActive switches the Exit button on for desktop builds
        if let Some(i) = s.find_path("LobbyTopBtnsLayout/BtnExit") {
            s.set_active(i, true);
        }
        for i in 0..s.nodes.len() {
            let name = s.nodes[i].name.clone();
            let path = s.path(i);
            if hide_names.contains(&name.as_str()) || hide_paths.iter().any(|p| path.ends_with(p)) {
                s.set_active(i, false);
            }
        }
        // the saved level and XP (progress.rs), as "80 / 500 XP" like the original
        let progress = crate::progress::Progress::load();
        let needed = crate::progress::Progress::needed(&data.0, progress.level);
        let level = progress.level.to_string();
        let exp = match needed {
            Some(n) => format!("{} / {n} [14ff05]XP", progress.xp),
            None => format!("{} [14ff05]XP", progress.xp),
        };
        let fill = needed.map_or(1.0, |n| (progress.xp as f32 / n.max(1) as f32).clamp(0.0, 1.0));
        let texts = [
            ("Profile/btn_Progress/Default/Label_PlayerName", "Player"),
            ("Label_Lvl", level.as_str()),
            ("Label_ExpAmount", exp.as_str()),
        ];
        for (suffix, text) in texts {
            let hits: Vec<usize> = (0..s.nodes.len()).filter(|&i| s.path(i).ends_with(suffix)).collect();
            for i in hits {
                s.set_text(i, text);
            }
        }
        // WidgetLayout closes the gap the hidden Tournament button leaves
        if let Some(i) = s.find_path("BotBtnsLayout_Online/BtnClan") {
            s.set_pos(i, Vec2::new(-200.0, 0.0));
        }
        if let Some(i) = s.find_path("panel_Boosters/Layout/DiscordButton") {
            s.set_pos(i, Vec2::new(0.0, -120.0)); // OBSERVED: same 60 step as the two above
        }
        // Pixel Pass: no season running offline, so the "Season over!" look of the original
        // (State_OnSeasonEnd, crown instead of the next reward), level 1, 0 of 150 points
        if let Some(pp) = s.find("PixelPassLobbyButton") {
            for (name, on) in [("State_OnSeasonEnd(OFF)", true), ("Last_Cell_Reward", false), ("crown_icon", true), ("Golden_Shield", false)] {
                if let Some(i) = s.find_in(pp, name) {
                    s.set_active(i, on);
                }
            }
            // each label has shadow copies below it ("Level_Label (1)", "Progress_Label (2)")
            for i in s.subtree(pp) {
                let name = s.nodes[i].name.clone();
                if name.starts_with("Level_Label") {
                    s.set_text(i, "1");
                } else if name.starts_with("Progress_Label") {
                    s.set_text(i, "0/150");
                } else if name == "notifications" {
                    s.set_active(i, false);
                }
            }
            if let Some(i) = s.find_in(pp, "ProgressBarCont").and_then(|c| s.find_in(c, "fill")) {
                s.set_fill(i, 0.0);
            }
        }
        // XP bar (the yellow "addition" part animates newly won XP: same as the fill here)
        let bars: Vec<usize> = (0..s.nodes.len())
            .filter(|&i| ["progbar_fill", "progbar_addition"].contains(&s.nodes[i].name.as_str()) && s.path(i).contains("PlayerExp_UI"))
            .collect();
        for i in bars {
            s.set_fill(i, fill);
        }
    }
}

/// Event Center button: Unity mixes the purple "gradi" overlay with the cyan button in
/// gamma space, Bevy in linear light, where it comes out far weaker (1280x720 lobby shot:
/// green 174 vs 123 halfway along). For a dark channel the gamma mix equals a linear mix
/// with alpha 1-(1-a)^2.2, so the gradient's alpha is remapped that way (colour alpha folded
/// in), and the red lowered (0.54 -> 0.30) so the purple doesn't turn lavender.
fn event_button_gradient(mut screens: Query<&mut crate::ngui::NguiScreen>, mut images: ResMut<Assets<Image>>, mut done: Local<bool>) {
    if *done {
        return;
    }
    for mut s in &mut screens {
        let Some(i) = s.find_path("EventCenterButton/Holder/ToEventButton/Bg_normal/gradi") else { continue };
        let Some(h) = s.texture_handle(i) else { continue };
        let Some(src) = images.get(&h) else { return };
        *done = true;
        let a0 = s.nodes[i].widget.as_ref().map_or(1.0, |w| w.color[3]);
        let mut img = src.clone();
        if let Some(data) = img.data.as_mut() {
            for px in data.chunks_exact_mut(4) {
                let a = px[3] as f32 / 255.0 * a0;
                px[3] = ((1.0 - (1.0 - a).powf(2.2)) * 255.0).round() as u8;
            }
        }
        s.set_texture(i, images.add(img));
        s.set_color(i, [0.30, 0.0, 1.0, 1.0]);
        return;
    }
}

/// The profile picture (PlayerExp_UI/.../tex_Avatar): the original shows the skin's face.
/// Cut the 8x8 face (x 8-16, y 8-16 of the 64x32 skin) out of the character's texture.
fn lobby_avatar(
    mut screens: Query<&mut crate::ngui::NguiScreen>,
    assets: Res<AssetServer>,
    data: Res<DataDir>,
    mut images: ResMut<Assets<Image>>,
    mut skin: Local<Option<Handle<Image>>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let path = "characters/default/textures/0000_player.png";
    if !data.0.join(path).exists() {
        *done = true;
        return;
    }
    let h = skin.get_or_insert_with(|| assets.load(path)).clone();
    let Some(img) = images.get(&h) else { return };
    let Some(target) = screens.iter_mut().find_map(|s| s.find_path("holder_ProfileWithAvatar/tex_Avatar").map(|i| (s, i))) else {
        return;
    };
    *done = true;
    let (w, bytes) = (img.width() as usize, img.data.clone().unwrap_or_default());
    if bytes.len() < w * 16 * 4 {
        return;
    }
    let mut face = Vec::with_capacity(8 * 8 * 4);
    for y in 8..16 {
        face.extend_from_slice(&bytes[(y * w + 8) * 4..(y * w + 16) * 4]);
    }
    let mut out = Image::new(
        bevy::render::render_resource::Extent3d { width: 8, height: 8, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        face,
        img.texture_descriptor.format,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    out.sampler = bevy::image::ImageSampler::nearest();
    let (mut s, i) = target;
    s.set_texture(i, images.add(out));
}

/// PG3D_MENU_CLICK=BtnBattle_A,GMH_GameMode_Cell_New_Big (automated tests): press these
/// buttons one after another, two seconds apart.
pub fn test_click(
    time: Res<Time<Real>>,
    screens: Query<(Entity, &crate::ngui::NguiScreen, &InheritedVisibility)>,
    mut out: MessageWriter<crate::ngui::NguiClick>,
    mut done: Local<usize>,
) {
    let Ok(list) = std::env::var("PG3D_MENU_CLICK") else { return };
    let names: Vec<&str> = list.split(',').collect();
    if *done >= names.len() || time.elapsed_secs() < 2.0 * (*done + 1) as f32 {
        return;
    }
    let name = names[*done];
    for (e, s, vis) in &screens {
        if !vis.get() {
            continue;
        }
        let node = (0..s.nodes.len()).find(|&i| s.nodes[i].name == name && s.nodes[i].button.is_some() && s.visible(i));
        if let Some(node) = node.or_else(|| s.nodes.iter().position(|n| n.name == name)) {
            info!("test click {name}");
            out.write(crate::ngui::NguiClick { screen: e, node, name: name.to_string() });
            *done += 1;
            return;
        }
    }
}

/// Mode hub: the original fills its cards from server data. Offline we have three modes:
/// Campaign (zombies), Sandbox (walk around without enemies) and Team Fight (team_match.rs).
fn setup_modes(s: &mut crate::ngui::NguiScreen, assets: &AssetServer, data: &DataDir) {
    // card pictures: loading screens of two exported maps
    let maps = exported_maps(&data.0);
    let pics: Vec<String> = maps.iter().filter(|m| data.0.join(format!("previews/{m}.png")).exists()).cloned().collect();
    let pic = |n: usize| pics.get(n % pics.len().max(1)).map(|m| assets.load(format!("previews/{m}.png")));
    let cells = [
        ("Column_01/GMH_GameMode_Cell_New_Big", Some(("Campaign", "Fight waves of zombies on any map"))),
        ("Column_02/GMH_GameMode_Cell_New", Some(("Sandbox", "Explore the maps - no enemies"))),
        ("Column_02/GMH_GameMode_Cell_New_2", Some(("Team Fight", "Your team against the zombies - most kills wins"))),
        ("Column_03", None),
        ("Column_04", None),
    ];
    let hide_in_cells = [
        "TournamentComponent",
        "CompetitionComponent",
        "LockedByXp",
        "LockedByNotAvailable",
        "Tag_holder",
        "Pressed",
        "Bundles_GMH_GameMode",
        "TapReward",
        "TapRewardAnimation",
        "GMHLabelNotification",
    ];
    for i in 0..s.nodes.len() {
        let path = s.path(i);
        let name = s.nodes[i].name.clone();
        for (cell, content) in &cells {
            if !path.ends_with(cell) {
                continue;
            }
            match content {
                None => s.set_active(i, false),
                Some((title, desc)) => {
                    let kids: Vec<usize> = (i + 1..s.nodes.len()).filter(|&j| s.path(j).starts_with(&format!("{path}/"))).collect();
                    for j in kids {
                        let n = s.nodes[j].name.clone();
                        if n == "tex_MapPreview" {
                            // fill the card (its clipping panel) with a map picture
                            let parent = s.nodes[j].parent.unwrap_or(j);
                            if let Some(p) = s.nodes[parent].panel.as_ref().filter(|p| p.clipping != 0) {
                                let size = Vec2::new(p.clip[2], p.clip[3]);
                                s.set_size(j, size);
                            }
                            if let Some(h) = pic(i) {
                                s.set_texture(j, h);
                            }
                        } else if n.starts_with("Label_Title") {
                            s.set_text(j, *title);
                        } else if n == "Label_Description" {
                            s.set_text(j, *desc);
                        } else if hide_in_cells.contains(&n.as_str()) {
                            s.set_active(j, false);
                        }
                    }
                }
            }
        }
        // party / private match / shop need servers
        if ["Button_Party", "Button_Private", "ShopButton", "GMHRecommendations_panel", "TemplateEventInfoHolder", "ButtonHintHolder", "GMHModeReward"]
            .contains(&name.as_str())
        {
            s.set_active(i, false);
        }
    }
}

/// What the original lobby buttons do here.
fn lobby_clicks(
    mut commands: Commands,
    mut clicks: MessageReader<crate::ngui::NguiClick>,
    data: Res<DataDir>,
    assets: Res<AssetServer>,
    screens: Res<Screens>,
    pickers: Query<(), With<MapPicker>>,
    mut exit: MessageWriter<AppExit>,
) {
    for c in clicks.read() {
        let n = c.name.as_str();
        if n == "BtnExit" {
            exit.write(AppExit::Success);
        } else if n == "BtnArmory" && screens.armory.is_some() {
            show(&mut commands, screens.lobby, false);
            show(&mut commands, screens.armory, true);
        } else if n == "Back_Button" && Some(c.screen) == screens.armory {
            show(&mut commands, screens.armory, false);
            show(&mut commands, screens.lobby, true);
        } else if Some(c.screen) == screens.armory {
            // handled in armory.rs
        } else if n.starts_with("BtnBattle") && screens.modes.is_some() {
            show(&mut commands, screens.lobby, false);
            show(&mut commands, screens.modes, true);
        } else if n == "btn_back" {
            show(&mut commands, screens.modes, false);
            show(&mut commands, screens.lobby, true);
        } else if n == "BtnSettings" && screens.settings.is_some() {
            show(&mut commands, screens.lobby, false);
            show(&mut commands, screens.settings, true);
        } else if n == "BtnBack" {
            show(&mut commands, screens.settings, false);
            show(&mut commands, screens.lobby, true);
        } else if n.starts_with("BtnBattle") || n == "BtnLobby_Campaign" || n == "GMH_GameMode_Cell_New_Big" || n == "Button_Battle" {
            if pickers.is_empty() {
                spawn_map_picker(&mut commands, &data, &assets, true, false, false);
            }
        } else if n == "GMH_GameMode_Cell_New" {
            if pickers.is_empty() {
                spawn_map_picker(&mut commands, &data, &assets, true, true, false);
            }
        } else if n == "GMH_GameMode_Cell_New_2" {
            if pickers.is_empty() {
                spawn_map_picker(&mut commands, &data, &assets, true, false, true);
            }
        } else if Some(c.screen) == screens.settings {
            // handled in settings_ui.rs
        } else {
            commands.spawn((
                Toast(2.5),
                Text::new(format!("{n}: needs the game's online services - not available offline")),
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
    }
}

fn fade_toast(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut Toast)>) {
    for (e, mut t) in &mut q {
        t.0 -= time.delta_secs();
        if t.0 <= 0.0 {
            commands.entity(e).despawn();
        }
    }
}

fn spawn_map_picker(commands: &mut Commands, data: &DataDir, assets: &AssetServer, overlay: bool, sandbox: bool, team: bool) {
    let maps = exported_maps(&data.0);
    commands
        .spawn((
            MapPicker { sandbox, team },
            GlobalZIndex(50_000),
            BackgroundColor(if overlay { Color::srgba(0.0, 0.0, 0.0, 0.8) } else { Color::NONE }),
            Node {
            width: percent(100.0),
            height: percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: px(10.0),
            position_type: PositionType::Absolute,
            ..default()
        }))
        .with_children(|root| {
            root.spawn((Text::new("PIXEL GUN 3D"), TextFont { font_size: FontSize::Px(56.0), ..default() }));
            root.spawn((
                Text::new(if team {
                    "Team Fight - pick a map"
                } else if sandbox {
                    "Sandbox - pick a map"
                } else {
                    "Campaign - pick a map"
                }),
                TextFont { font_size: FontSize::Px(22.0), ..default() },
                TextColor(Color::srgb(0.7, 0.75, 0.85)),
            ));
            if maps.is_empty() {
                root.spawn(Text::new(
                    "No maps exported yet. Run scripts/export_all.sh in ../pixel-gun-3d-re",
                ));
            }
            // map buttons in a wrapping grid
            root.spawn(Node {
                flex_wrap: FlexWrap::Wrap,
                justify_content: JustifyContent::Center,
                max_width: px(900.0),
                column_gap: px(10.0),
                row_gap: px(10.0),
                margin: UiRect::top(px(20.0)),
                ..default()
            })
            .with_children(|grid| {
                for m in &maps {
                    // the game's own loading picture of the map, if exported
                    let preview = data.0.join(format!("previews/{m}.png"));
                    let mut card = grid.spawn((
                        Node {
                            width: px(200.0),
                            height: px(140.0),
                            flex_direction: FlexDirection::Column,
                            justify_content: JustifyContent::FlexEnd,
                            ..default()
                        },
                        BackgroundColor(NORMAL),
                        Button,
                        MapButton(m.clone()),
                    ));
                    if preview.exists() {
                        card.insert(ImageNode::new(assets.load(format!("previews/{m}.png"))));
                    }
                    card.with_child((
                        Text::new(m.replace('_', " ")),
                        TextFont { font_size: FontSize::Px(18.0), ..default() },
                        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
                        Node { padding: UiRect::all(px(4.0)), ..default() },
                    ));
                }
            });
            if overlay {
                root.spawn((Node { margin: UiRect::top(px(30.0)), ..button_node() }, BackgroundColor(NORMAL), Button, CloseButton))
                    .with_child((Text::new("Back"), TextFont { font_size: FontSize::Px(20.0), ..default() }));
            } else {
                root.spawn((Node { margin: UiRect::top(px(30.0)), ..button_node() }, BackgroundColor(NORMAL), Button, QuitButton))
                    .with_child((Text::new("Quit"), TextFont { font_size: FontSize::Px(20.0), ..default() }));
            }
            root.spawn((
                Text::new("In game: WASD move, Space jump, mouse aim/fire, right mouse zoom, R reload,\n1-9 weapons, V fly camera, F10 back to this menu"),
                TextFont { font_size: FontSize::Px(16.0), ..default() },
                TextColor(Color::srgb(0.6, 0.65, 0.75)),
                Node { margin: UiRect::top(px(30.0)), ..default() },
            ));
        });
}

fn button_node() -> Node {
    Node {
        width: px(200.0),
        height: px(48.0),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        ..default()
    }
}

fn button_colors(
    mut q: Query<(&Interaction, &mut BackgroundColor), (Changed<Interaction>, Or<(With<MapButton>, With<QuitButton>, With<CloseButton>)>)>,
) {
    for (i, mut bg) in &mut q {
        bg.0 = if *i == Interaction::None { NORMAL } else { HOVER };
    }
}

fn button_clicks(
    maps: Query<(&Interaction, &MapButton), Changed<Interaction>>,
    quit: Query<&Interaction, (Changed<Interaction>, With<QuitButton>)>,
    close: Query<&Interaction, (Changed<Interaction>, With<CloseButton>)>,
    pickers: Query<(Entity, &MapPicker)>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    if close.iter().any(|i| *i == Interaction::Pressed) {
        for (e, _) in &pickers {
            commands.entity(e).despawn();
        }
    }
    let sandbox = pickers.iter().next().is_some_and(|(_, p)| p.sandbox);
    let team = pickers.iter().next().is_some_and(|(_, p)| p.team);
    for (i, m) in &maps {
        if *i == Interaction::Pressed {
            relaunch_mode(Some(&m.0), sandbox, team.then_some("teamfight"));
            exit.write(AppExit::Success);
        }
    }
    if quit.iter().any(|i| *i == Interaction::Pressed) {
        exit.write(AppExit::Success);
    }
}

/// Start a new copy of this program (with a map, or without one = menu).
pub fn relaunch(map: Option<&str>) {
    relaunch_mode(map, false, None);
}

/// `sandbox`: start the map without enemies. Keeps the current game mode (PG3D_MODE), so
/// "play again" stays in Team Fight.
pub fn relaunch_with(map: Option<&str>, sandbox: bool) {
    relaunch_mode(map, sandbox, std::env::var("PG3D_MODE").ok().as_deref());
}

/// `mode`: PG3D_MODE of the new copy (None: the zombie campaign).
pub fn relaunch_mode(map: Option<&str>, sandbox: bool, mode: Option<&str>) {
    let Ok(exe) = std::env::current_exe() else { return };
    let mut cmd = std::process::Command::new(exe);
    match mode {
        Some(m) => cmd.env("PG3D_MODE", m),
        None => cmd.env_remove("PG3D_MODE"),
    };
    if sandbox {
        cmd.env("PG3D_NOBOTS", "1");
    } else {
        cmd.env_remove("PG3D_NOBOTS");
    }
    if let Some(m) = map {
        cmd.arg(m);
    }
    if let Err(e) = cmd.spawn() {
        error!("could not start {:?}: {e}", map);
    }
}

/// In game: F10 goes back to the menu.
pub fn back_to_menu(keys: Res<ButtonInput<KeyCode>>, mut exit: MessageWriter<AppExit>) {
    if keys.just_pressed(KeyCode::F10) {
        relaunch(None);
        exit.write(AppExit::Success);
    }
}

/// Coins and gems in the lobby's top-right panel (CoinsShopButton/CurrencyPanel: two
/// ItemCurrencyUI cells, coins on the left, gems on the right).
fn lobby_wallet(
    wallet: Res<crate::wallet::Wallet>,
    assets: Res<AssetServer>,
    mut screens: Query<&mut crate::ngui::NguiScreen>,
    mut done: Local<Option<crate::wallet::Wallet>>,
) {
    if *done == Some(*wallet) {
        return;
    }
    for mut s in &mut screens {
        // the Armory's coin bar (BankShopView), numbers grouped like "5 079"
        if s.nodes.first().is_some_and(|n| n.name == "ShopNGUI") {
            for (name, v) in [("CoinsLabel", wallet.coins), ("GemLabel", wallet.gems)] {
                if let Some(i) = s.find(name) {
                    s.set_text(i, crate::results::group(v.clamp(0, i32::MAX as i64) as i32));
                }
            }
            continue;
        }
        let Some(panel) = s.find("CurrencyPanelLayout") else { continue };
        *done = Some(*wallet);
        for (cell, v, icon) in [("Currency (1)", wallet.coins, "coins_118d"), ("Currency", wallet.gems, "gems_118a")] {
            let Some(c) = s.find_in(panel, cell).filter(|&c| s.nodes[c].parent == Some(panel)) else { continue };
            if let Some(t) = s.find_in(c, "Texture") {
                s.set_texture(t, assets.load(format!("ui/lobby/textures/{icon}.png")));
            }
            if let Some(l) = s.find_in(c, "Label") {
                // no thousands spaces here, unlike the match HUD (OBSERVED: "4376" "4797")
                s.set_text(l, v.max(0).to_string());
            }
        }
    }
}
