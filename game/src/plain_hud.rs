//! In-match HUD for game builds whose NGUI screens can't be exported (Unity 6 builds
//! encrypt the script layouts, see ../pixel-gun-3d-re/README.md): health, ammo, weapon
//! slots and the Arena box (score, wave, zombies left) drawn with plain Bevy UI, placed like
//! the original. Only spawned when <data>/ui/ingame is missing (ingame_ui.rs draws the
//! original otherwise).

#[allow(unused_imports)]
use crate::scene::{FixedUpdate, PostStartup, PostUpdate, PreStartup, Startup, Update};
use bevy::prelude::*;

use crate::{
    DataDir,
    bots::{PLAYER_MAX_HEALTH, PlayerStats, Wave},
    ingame_ui::HudHidden,
    weapons::Loadout,
};

pub struct PlainHudPlugin;

impl crate::scene::ScenePlugin for PlainHudPlugin {
    fn build(&self, app: &mut crate::scene::SceneBuilder) {
        app.add_systems(Startup, spawn).add_systems(Update, update);
    }
}

#[derive(Component)]
struct PlainHud;
#[derive(Component)]
struct HealthFill;
#[derive(Component)]
struct HealthLabel;
#[derive(Component)]
struct AmmoLabel;
#[derive(Component)]
struct Slot(usize);
#[derive(Component)]
struct ArenaBox;
#[derive(Component)]
struct ArenaLabel;

const PANEL: Color = Color::srgba(0.0, 0.0, 0.0, 0.45);
const SLOT: Color = Color::srgba(0.0, 0.0, 0.0, 0.45);
const SLOT_ON: Color = Color::srgba(0.2, 0.75, 0.2, 0.85);

fn font(size: f32) -> TextFont {
    TextFont { font_size: FontSize::Px(size), ..default() }
}

fn spawn(mut commands: Commands, data: Res<DataDir>) {
    if data.0.join("ui/ingame/ui.json").exists() {
        return;
    }
    let corner = |left: bool| Node {
        position_type: PositionType::Absolute,
        bottom: px(20.0),
        left: if left { px(20.0) } else { auto() },
        right: if left { auto() } else { px(20.0) },
        padding: UiRect::axes(px(14.0), px(10.0)),
        column_gap: px(12.0),
        align_items: AlignItems::Center,
        ..default()
    };
    commands
        .spawn((PlainHud, Node { width: percent(100.0), height: percent(100.0), position_type: PositionType::Absolute, ..default() }, Pickable::IGNORE))
        .with_children(|root| {
            root.spawn((corner(true), BackgroundColor(PANEL))).with_children(|p| {
                p.spawn((Text::new("+"), font(30.0), TextColor(Color::srgb(1.0, 0.3, 0.3))));
                p.spawn((Node { width: px(220.0), height: px(18.0), ..default() }, BackgroundColor(Color::srgba(0.3, 0.05, 0.05, 0.8))))
                    .with_child((HealthFill, Node { width: percent(100.0), height: percent(100.0), ..default() }, BackgroundColor(Color::srgb(0.85, 0.15, 0.15))));
                p.spawn((HealthLabel, Text::new("100"), font(26.0)));
            });
            root.spawn((corner(false), BackgroundColor(PANEL))).with_child((AmmoLabel, Text::new(""), font(30.0)));
            root.spawn(Node {
                position_type: PositionType::Absolute,
                bottom: px(20.0),
                width: percent(100.0),
                justify_content: JustifyContent::Center,
                column_gap: px(6.0),
                ..default()
            })
            .with_children(|row| {
                for i in 0..9 {
                    row.spawn((Slot(i), Node { width: px(40.0), height: px(40.0), justify_content: JustifyContent::Center, align_items: AlignItems::Center, display: Display::None, ..default() }, BackgroundColor(SLOT)))
                        .with_child((Text::new((i + 1).to_string()), font(20.0)));
                }
            });
            root.spawn((
                ArenaBox,
                Node {
                    position_type: PositionType::Absolute,
                    top: px(16.0),
                    width: percent(100.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
            ))
            .with_children(|b| {
                b.spawn((Node { padding: UiRect::axes(px(18.0), px(8.0)), ..default() }, BackgroundColor(PANEL)))
                    .with_child((ArenaLabel, Text::new(""), font(24.0)));
            });
        });
}

#[allow(clippy::too_many_arguments)]
fn update(
    loadout: Res<Loadout>,
    stats: Res<PlayerStats>,
    wave: Option<Res<Wave>>,
    over: Res<crate::results::MatchOver>,
    hidden: Res<HudHidden>,
    mut root: Query<&mut Visibility, With<PlainHud>>,
    mut fill: Query<&mut Node, (With<HealthFill>, Without<Slot>)>,
    mut texts: ParamSet<(Query<&mut Text, With<HealthLabel>>, Query<&mut Text, With<AmmoLabel>>, Query<&mut Text, With<ArenaLabel>>)>,
    mut slots: Query<(&Slot, &mut Node, &mut BackgroundColor), Without<HealthFill>>,
    mut arena: Query<&mut Visibility, (With<ArenaBox>, Without<PlainHud>)>,
) {
    let dead = stats.health <= 0.0 || over.0 || hidden.0;
    for mut v in &mut root {
        v.set_if_neq(if dead { Visibility::Hidden } else { Visibility::Inherited });
    }
    let k = (stats.health / PLAYER_MAX_HEALTH).clamp(0.0, 1.0);
    for mut n in &mut fill {
        let w = percent(k * 100.0);
        if n.width != w {
            n.width = w;
        }
    }
    set(&mut texts.p0(), format!("{:.0}%", k * 100.0));
    let ammo = loadout
        .weapons
        .get(loadout.current)
        .filter(|w| w.def.isMelee == 0)
        .map_or(String::new(), |w| if loadout.reloading > 0.0 { "Reloading".into() } else { format!("{} / {}", w.clip, w.reserve) });
    set(&mut texts.p1(), ammo);
    for (s, mut n, mut bg) in &mut slots {
        let display = if s.0 < loadout.weapons.len() { Display::Flex } else { Display::None };
        if n.display != display {
            n.display = display;
        }
        let c = if s.0 == loadout.current { SLOT_ON } else { SLOT };
        if bg.0 != c {
            bg.0 = c;
        }
    }
    let waves = std::env::var("PG3D_NOBOTS").is_err() && !crate::team_match::team_mode();
    for mut v in &mut arena {
        v.set_if_neq(if waves { Visibility::Inherited } else { Visibility::Hidden });
    }
    if let Some(w) = wave.filter(|_| waves) {
        let left = if w.number == 0 { crate::bots::WAVE_SIZE } else { w.left };
        set(&mut texts.p2(), format!("Score {}    Wave {}    Zombies {left}", crate::results::group(stats.score), w.number.max(1)));
    }
}

fn set(q: &mut Query<&mut Text, impl bevy::ecs::query::QueryFilter>, text: String) {
    for mut t in q.iter_mut() {
        if t.0 != text {
            t.0 = text.clone();
        }
    }
}
