//! The game's own mouse cursor: a small ring (GamepadCursor prefab, texture
//! GamepadCursor.png, 40x40 UITexture) drawn instead of the system arrow whenever the
//! cursor is free (menus, pause, results). Hidden while the mouse looks around in a match.

use bevy::{
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};

pub struct CursorPlugin;

impl Plugin for CursorPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn).add_systems(PostUpdate, follow);
    }
}

/// Picture size at a 720 px tall window. OBSERVED: the ring is 34 px across in 1280x720
/// screenshots and fills 240 of the texture's 256 pixels.
const SIZE_720: f32 = 34.0 * 256.0 / 240.0;

#[derive(Component)]
struct GameCursor;

fn spawn(mut commands: Commands, assets: Res<AssetServer>) {
    commands.spawn((
        Name::new("game cursor"),
        GameCursor,
        ImageNode::new(assets.load("ui/cursor/textures/GamepadCursor_1a20.png")),
        Node { position_type: PositionType::Absolute, ..default() },
        GlobalZIndex(i32::MAX),
        Pickable::IGNORE,
        Visibility::Hidden,
    ));
}

fn follow(
    mut windows: Query<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
    mut cursor: Query<(&mut Node, &mut Visibility), With<GameCursor>>,
) {
    let Ok((window, mut options)) = windows.single_mut() else { return };
    let Ok((mut node, mut vis)) = cursor.single_mut() else { return };
    let free = options.grab_mode == CursorGrabMode::None;
    if free && options.visible {
        options.visible = false;
    }
    let pos = window.cursor_position().filter(|_| free);
    let want = if pos.is_some() { Visibility::Inherited } else { Visibility::Hidden };
    if *vis != want {
        *vis = want;
    }
    if let Some(p) = pos {
        let size = SIZE_720 * window.height() / 720.0;
        node.left = px(p.x - size / 2.0);
        node.top = px(p.y - size / 2.0);
        node.width = px(size);
        node.height = px(size);
    }
}
