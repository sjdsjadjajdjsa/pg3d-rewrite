//! Start menu: lists the maps that have been exported and starts the game on a click.
//!
//! The menu is its own small Bevy app. Picking a map starts a fresh copy of this program
//! with the map name (`pixel-gun-3d-rs <map>`) and closes the menu; F10 in game does the
//! reverse. Restarting keeps the game code simple: every match starts from a clean world.

use bevy::{app::AppExit, prelude::*};

use crate::DataDir;

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(Color::srgb(0.08, 0.1, 0.14)))
            .add_systems(Startup, build_menu)
            .add_systems(Update, (button_colors, button_clicks));
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

fn build_menu(mut commands: Commands, data: Res<DataDir>) {
    commands.spawn(Camera2d);
    let maps = exported_maps(&data.0);
    commands
        .spawn(Node {
            width: percent(100.0),
            height: percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: px(10.0),
            ..default()
        })
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
            root.spawn((Node { margin: UiRect::top(px(30.0)), ..button_node() }, BackgroundColor(NORMAL), Button, QuitButton))
                .with_child((Text::new("Quit"), TextFont { font_size: FontSize::Px(20.0), ..default() }));
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

fn button_colors(mut q: Query<(&Interaction, &mut BackgroundColor), Changed<Interaction>>) {
    for (i, mut bg) in &mut q {
        bg.0 = if *i == Interaction::None { NORMAL } else { HOVER };
    }
}

fn button_clicks(
    maps: Query<(&Interaction, &MapButton), Changed<Interaction>>,
    quit: Query<&Interaction, (Changed<Interaction>, With<QuitButton>)>,
    mut exit: MessageWriter<AppExit>,
) {
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
