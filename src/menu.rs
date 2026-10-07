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
            .insert_resource(crate::map::CurrentMap("lobby".into()))
            .init_resource::<crate::map::MapInfo>()
            .add_plugins((crate::ngui::NguiPlugin, crate::lighting::LightingPlugin))
            .add_systems(PreStartup, lobby_scene)
            .add_systems(Startup, build_menu)
            .add_systems(Update, (character_idle, test_click))
            .add_systems(Update, (button_colors, button_clicks, lobby_clicks, fade_toast, setup_lobby));
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
        .collect();
    maps.sort();
    maps
}

#[derive(Component)]
struct MapPicker;

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
    mut commands: Commands,
    data: Res<DataDir>,
    assets: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    let dir = data.0.join("maps/lobby");
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
    commands.spawn(WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset("maps/lobby/map.gltf"))));
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
            AudioPlayer::new(assets.load("maps/lobby/sounds/music.wav")),
            PlaybackSettings::LOOP.with_volume(bevy::audio::Volume::Linear(0.5)),
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

fn build_menu(mut commands: Commands, data: Res<DataDir>, assets: Res<AssetServer>) {
    if crate::ngui::spawn_screen(&mut commands, &assets, &data, "lobby").is_none() {
        warn!("no exported lobby UI (run export_ngui.py level17 MainMenuNGUI lobby); using plain menu");
        spawn_map_picker(&mut commands, &data, false);
    }
}

/// The original fills in and switches parts of the lobby at run time (player data from
/// the server, button press states, events). Put it into a sensible offline state.
fn setup_lobby(mut screens: Query<&mut crate::ngui::NguiScreen, Added<crate::ngui::NguiScreen>>) {
    for mut s in &mut screens {
        let hide_names = [
            "Pressed",         // button pressed look; NGUI swaps it in while you hold the button
            "State_Locked",    // "unlocks at level X" overlays
            "state_Queue",     // event download queue
            "icon_loading",
        ];
        let hide_paths = [
            "RightAnchor/EventCenterButton",
            "RightAnchor/ConfrontationEvent_LobbyButton",
            "LeftAnchor/LobbyTopBtnsLayout/OfferwallButton2",
        ];
        for i in 0..s.nodes.len() {
            let name = s.nodes[i].name.clone();
            let path = s.path(i);
            if hide_names.contains(&name.as_str()) || hide_paths.iter().any(|p| path.ends_with(p)) {
                s.set_active(i, false);
            }
        }
        let texts = [
            ("Profile/btn_Progress/Default/Label_PlayerName", "Player"),
            ("Label_Lvl", "1"),
            ("Label_ExpAmount", "0 / 100 [14ff05]XP"),
        ];
        for (suffix, text) in texts {
            let hits: Vec<usize> = (0..s.nodes.len()).filter(|&i| s.path(i).ends_with(suffix)).collect();
            for i in hits {
                s.set_text(i, text);
            }
        }
    }
}

/// PG3D_MENU_CLICK=BtnBattle_A (automated tests): press that lobby button after 2 seconds.
fn test_click(
    time: Res<Time>,
    screens: Query<(Entity, &crate::ngui::NguiScreen)>,
    mut out: MessageWriter<crate::ngui::NguiClick>,
    mut done: Local<bool>,
) {
    let Ok(name) = std::env::var("PG3D_MENU_CLICK") else { return };
    if *done || time.elapsed_secs() < 2.0 {
        return;
    }
    for (e, s) in &screens {
        if let Some(node) = s.nodes.iter().position(|n| n.name == name) {
            out.write(crate::ngui::NguiClick { screen: e, node, name: name.clone() });
            *done = true;
        }
    }
}

/// What the original lobby buttons do here.
fn lobby_clicks(
    mut commands: Commands,
    mut clicks: MessageReader<crate::ngui::NguiClick>,
    data: Res<DataDir>,
    pickers: Query<(), With<MapPicker>>,
) {
    for c in clicks.read() {
        let n = c.name.as_str();
        if n.starts_with("BtnBattle") || n == "BtnLobby_Campaign" {
            if pickers.is_empty() {
                spawn_map_picker(&mut commands, &data, true);
            }
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

fn spawn_map_picker(commands: &mut Commands, data: &DataDir, overlay: bool) {
    let maps = exported_maps(&data.0);
    commands
        .spawn((
            MapPicker,
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
                Text::new("Rust rewrite - pick a map"),
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
                    grid.spawn((button_node(), BackgroundColor(NORMAL), Button, MapButton(m.clone())))
                        .with_child((Text::new(m.replace('_', " ")), TextFont { font_size: FontSize::Px(20.0), ..default() }));
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
    pickers: Query<Entity, With<MapPicker>>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    if close.iter().any(|i| *i == Interaction::Pressed) {
        for e in &pickers {
            commands.entity(e).despawn();
        }
    }
    for (i, m) in &maps {
        if *i == Interaction::Pressed {
            relaunch(Some(&m.0));
            exit.write(AppExit::Success);
        }
    }
    if quit.iter().any(|i| *i == Interaction::Pressed) {
        exit.write(AppExit::Success);
    }
}

/// Start a new copy of this program (with a map, or without one = menu).
pub fn relaunch(map: Option<&str>) {
    let Ok(exe) = std::env::current_exe() else { return };
    let mut cmd = std::process::Command::new(exe);
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
