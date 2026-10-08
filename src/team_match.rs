//! Team Fight rules and its match ending, as in the original (reference clips
//! i_rec1_match.mp4 / i_rec2_match.mp4, frames in private/reference/shots6):
//!
//! * Tab holds "List of players": the two-team table over the dimmed game.
//! * When the clock runs out: a big 3, 2, 1 in the last seconds, then the Defeat/Victory
//!   banner with its shield, then the same table titled "Defeat"/"Victory" with the reward
//!   panel (XP and coins counting up) and a green OK that leads back to the menu.
//!
//! The rewards are the game's own (BalanceRewards.TeamFight: XP and coins by place).
//! Until there are other players, the rival team is the zombies: its score is how often they
//! killed you. On for `PG3D_MODE=teamfight`.
//!
//! UI: <data>/ui/table from `export_ngui.py resources.assets NetworkStartTableNGUI table`.

use bevy::{
    app::AppExit,
    prelude::*,
    window::{CursorGrabMode, CursorOptions},
};
use serde::Deserialize;

use crate::{
    DataDir,
    bots::PlayerStats,
    ngui::{NguiClick, NguiScreen},
};

pub struct TeamMatchPlugin;

/// Seconds of a Team Fight match (GUESS: the clip shows about 4-5 minutes).
const MATCH_TIME: f32 = 300.0;
/// The countdown shows in the last seconds (OBSERVED: 3, 2, 1 at one second each).
const COUNTDOWN: f32 = 3.0;
/// Seconds the Defeat/Victory banner shows before the table (OBSERVED: about 2 s).
const BANNER_TIME: f32 = 2.0;
/// Seconds the rewards take to count up (OBSERVED: about 1.5 s).
const COUNT_TIME: f32 = 1.5;

pub fn team_mode() -> bool {
    std::env::var("PG3D_MODE").is_ok_and(|m| m.eq_ignore_ascii_case("teamfight"))
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Phase {
    #[default]
    Playing,
    /// banner up for this many seconds
    Banner(f32),
    /// table and rewards up for this many seconds
    Table(f32),
}

#[derive(Resource, Default)]
pub struct TeamMatch {
    pub time_left: f32,
    /// times the player was killed: the rival team's score
    pub deaths: i32,
    was_dead: bool,
    pub phase: Phase,
    /// XP and coins of this match
    reward: (i64, i64),
    paid: bool,
}

#[derive(Resource)]
struct TableScreen(Option<Entity>);

/// The team scores and match clock at the top (InGameGUIPart_TeamFight, <data>/ui/teampart).
#[derive(Resource)]
struct ScoreBox(Option<Entity>);

#[derive(Component)]
struct CountdownLabel;

#[derive(Component)]
struct CountdownRoot;

#[derive(Deserialize, Default)]
struct ModeReward {
    #[serde(default, rename = "XP")]
    xp: Vec<i64>,
    #[serde(default, rename = "C")]
    coins: Vec<i64>,
}

#[derive(Deserialize, Default)]
struct Rewards {
    #[serde(default)]
    rewards: std::collections::HashMap<String, ModeReward>,
}

impl Plugin for TeamMatchPlugin {
    fn build(&self, app: &mut App) {
        if !team_mode() {
            return;
        }
        app.insert_resource(TeamMatch {
            time_left: std::env::var("PG3D_MATCH_TIME").ok().and_then(|v| v.parse().ok()).unwrap_or(MATCH_TIME),
            ..default()
        })
        .add_systems(Startup, spawn_screen)
        .add_systems(Update, (setup, clock, score_box, countdown, table_key, ending, clicks).chain());
    }
}

fn spawn_screen(mut commands: Commands, assets: Res<AssetServer>, data: Res<DataDir>) {
    let e = crate::ngui::spawn_screen(&mut commands, &assets, &data, "table");
    if let Some(e) = e {
        commands.entity(e).insert(Visibility::Hidden);
    }
    commands.insert_resource(TableScreen(e));
    let b = crate::ngui::spawn_screen(&mut commands, &assets, &data, "teampart");
    if b.is_none() {
        warn!("no exported Team Fight HUD (run export_ngui.py resources.assets InGameGUIPart_TeamFight teampart)");
    }
    commands.insert_resource(ScoreBox(b));
}

/// A direct child of `parent` by name (find_in searches the whole subtree).
fn child(s: &NguiScreen, parent: usize, name: &str) -> Option<usize> {
    (0..s.nodes.len()).find(|&i| s.nodes[i].parent == Some(parent) && s.nodes[i].name == name)
}

fn is_table(s: &NguiScreen) -> bool {
    s.nodes.first().is_some_and(|n| n.name == "NetworkStartTableNGUI")
}

/// What the table screen shows.
#[derive(Clone, Copy, PartialEq)]
enum View {
    /// Tab: "List of players"
    List,
    /// the Defeat / Victory banner
    Banner(Outcome),
    /// the final table with the rewards
    End(Outcome),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Outcome {
    Victory,
    Defeat,
    Draw,
}

/// Switch the parts of NetworkStartTableNGUI on and off for `view`.
fn layout(s: &mut NguiScreen, view: View) {
    let Some(cam) = s.find("Camera") else { return };
    let tops: Vec<usize> = (0..s.nodes.len()).filter(|&i| s.nodes[i].parent == Some(cam)).collect();
    for i in tops {
        let on = match s.nodes[i].name.as_str() {
            "TableRanks" => !matches!(view, View::Banner(_)),
            "FinishedInterface" => matches!(view, View::Banner(_)),
            "EndInterface" => matches!(view, View::End(_)),
            _ => false,
        };
        s.set_active(i, on);
    }
    // only the team table
    if let Some(p) = s.find("RanksPanel") {
        let kids: Vec<usize> = (0..s.nodes.len()).filter(|&i| s.nodes[i].parent == Some(p)).collect();
        for i in kids {
            let on = s.nodes[i].name == "FonTableTeam";
            s.set_active(i, on);
        }
    }
    let title = match view {
        View::List => s.string("Key_0306"),
        View::End(Outcome::Victory) | View::Banner(Outcome::Victory) => s.string("Key_1115"),
        View::End(Outcome::Defeat) | View::Banner(Outcome::Defeat) => s.string("Key_1116"),
        _ => s.string("Key_9676"),
    };
    if let Some(t) = s.find("LabelsAboveTable").and_then(|l| s.find_in(l, "TitleLabel")) {
        s.set_active(t, true);
        // UpHead (and its outline copies) is the big yellow title; Label a second line
        for i in s.subtree(t) {
            if s.nodes[i].name.starts_with("UpHead") {
                s.set_text(i, title.clone());
            }
        }
        if let Some(l) = s.find_in(t, "Label") {
            s.set_active(l, false);
        }
    }
    // the banner: the shield with the big yellow word over it (UpHead, as the table title)
    if let (View::Banner(o), Some(f)) = (view, s.find("FinishedInterface")) {
        for (name, which) in [("ShieldWin", Outcome::Victory), ("ShieldDefeat", Outcome::Defeat), ("ShieldDraw", Outcome::Draw)] {
            if let Some(i) = s.find_in(f, name) {
                s.set_active(i, o == which);
            }
        }
        for name in ["Win", "Defeat", "Draw"] {
            for i in s.subtree(f) {
                if s.nodes[i].name == name {
                    s.set_active(i, false);
                }
            }
        }
        for name in ["WinLose", "WinLoseSingle", "ContainerShields"] {
            if let Some(i) = s.find_in(f, name) {
                s.set_active(i, false);
            }
        }
        if let Some(u) = s.find_in(f, "UpHead") {
            s.set_active(u, true);
            for i in s.subtree(u) {
                if s.nodes[i].name.starts_with("UpHead") {
                    s.set_text(i, title.clone());
                }
            }
        }
    }
    if let (View::End(_), Some(end)) = (view, s.find("EndInterface")) {
        // the reward panel: XP and coins only (no chests, gems, clans, trophies, sharing)
        let kids: Vec<usize> = (0..s.nodes.len()).filter(|&i| s.nodes[i].parent == Some(end)).collect();
        for i in kids {
            let on = matches!(s.nodes[i].name.as_str(), "BottomCenter" | "Reward");
            s.set_active(i, on);
        }
        if let Some(r) = s.find_in(end, "Reward") {
            for (name, on) in [("Vip20", false), ("ExpBooster", false), ("GemsHarvester", false), ("PotentialReward", false)] {
                if let Some(i) = s.find_in(r, name) {
                    s.set_active(i, on);
                }
            }
            // the frame fades in at run time (colours start transparent): a cyan edge
            // round a dark blue panel, the tab label in white (OBSERVED in end_reward_ok.png)
            if let Some(l) = child(s, r, "Label") {
                s.set_color(l, [1.0, 1.0, 1.0, 1.0]);
                let t = s.string("Key_1989");
                s.set_text(l, t);
            }
            for (name, c) in [
                ("FrameReward", [0.2, 0.75, 1.0, 0.6]),
                ("FrameRewardGlow", [0.2, 0.75, 1.0, 0.35]),
                ("FrameRewardBG", [0.06, 0.16, 0.42, 0.9]),
                ("FrameRewardBG (1)", [0.25, 0.75, 0.95, 1.0]),
            ] {
                if let Some(i) = s.find_in(r, name) {
                    s.set_color(i, c);
                }
            }
        }
        if let Some(g) = s.find_in(end, "RewardGrid") {
            let kids: Vec<usize> = (0..s.nodes.len()).filter(|&i| s.nodes[i].parent == Some(g)).collect();
            for i in kids {
                let on = matches!(s.nodes[i].name.as_str(), "02RewardExp" | "04RewardGold");
                s.set_active(i, on);
            }
        }
        if let Some(i) = s.find_in(end, "ShareToggle(off)") {
            s.set_active(i, false);
        }
    }
}

/// One line of the table: nick, score, kills, is it you.
type Row<'a> = (&'a str, i32, i32, bool);

/// Top-level UI nodes (the HUD and other screens).
type UiRoots<'w, 's, F> = Query<'w, 's, Entity, (With<Node>, Without<ChildOf>, F)>;

/// Fill the two teams: you on the blue side, the zombies on the red.
fn fill_table(s: &mut NguiScreen, stats: &PlayerStats, m: &TeamMatch) {
    let your = s.string("Key_1771");
    let rival = s.string("Key_1772");
    // other modes' tables have parts with the same names: look inside the team table
    let Some(team) = s.find("FonTableTeam") else { return };
    for (table, name, total) in [("TableBlue", your, stats.kills), ("TableRed", rival, m.deaths)] {
        let Some(t) = s.find_in(team, table) else { continue };
        if let Some(i) = s.find_in(t, "TeamLabel") {
            s.set_text(i, name);
        }
        for i in s.subtree(t) {
            if s.nodes[i].name.starts_with("TotalScore") {
                s.set_text(i, total.to_string());
            }
        }
    }
    // the original's table has no level column
    for t in ["TableBlue", "TableRed"] {
        let Some(t) = s.find_in(team, t) else { continue };
        for i in s.subtree(t) {
            if s.nodes[i].name == "LvlLabel" || s.nodes[i].label.as_ref().is_some_and(|l| l.key == "Key_1073") {
                s.set_active(i, false);
            }
        }
    }
    let rows: [(usize, Option<Row>); 2] = [
        (0, Some(("Player", stats.score, stats.kills, true))),
        (6, (m.deaths > 0).then_some(("Zombies", m.deaths * 100, m.deaths, false))),
    ];
    for k in 0..12 {
        let Some(b) = s.find_in(team, &format!("TeamFight_PlayerButton{k}")) else { continue };
        let row = rows.iter().find(|(n, _)| *n == k).and_then(|(_, r)| *r);
        s.set_active(b, row.is_some());
        let Some((nick, score, kills, mine)) = row else { continue };
        for (label, v) in [
            ("Nick_Label", nick.to_string()),
            ("Score_Label", crate::results::group(score)),
            ("CountKill_Label", kills.to_string()),
            ("Place_Label", "1".to_string()),
        ] {
            if let Some(i) = s.find_in(b, label) {
                s.set_text(i, v);
            }
        }
        // your own row has no add-friend / report buttons
        for (n, on) in [("Rank_Label", false), ("Check", !mine), ("btn_Report", !mine), ("LikeUI_Container", !mine), ("bg", !mine)] {
            if let Some(i) = child(s, b, n) {
                s.set_active(i, on);
            }
        }
        if let Some(i) = s.find_in(b, "bg_IsMine") {
            s.set_active(i, mine);
        }
    }
}

fn setup(mut screens: Query<&mut NguiScreen, Added<NguiScreen>>) {
    for mut s in &mut screens {
        if is_table(&s) {
            layout(&mut s, View::List);
        }
    }
}

/// The match clock; count the player's deaths for the rival team.
fn clock(time: Res<Time>, stats: Res<PlayerStats>, mut m: ResMut<TeamMatch>) {
    let dead = stats.health <= 0.0;
    if dead && !m.was_dead {
        m.deaths += 1;
    }
    m.was_dead = dead;
    if m.phase == Phase::Playing {
        m.time_left = (m.time_left - time.delta_secs()).max(0.0);
    }
}

/// "m:ss" like TimeTeambattleLabel.
fn clock_text(secs: f32) -> String {
    let t = secs.ceil() as i32;
    format!("{}:{:02}", t / 60, t % 60)
}

/// Fill the box at the top: your team's kills on the left (the blue side, framed as yours),
/// the rival's on the right, the clock between them; the side that leads gets the crown and a
/// yellow score.
/// The prefab's labels are named the other way round (ScoreTeamRedLabel sits on the left).
fn score_box(
    m: Res<TeamMatch>,
    stats: Res<PlayerStats>,
    hidden: Res<crate::ingame_ui::HudHidden>,
    sb: Res<ScoreBox>,
    mut screens: Query<&mut NguiScreen>,
    mut commands: Commands,
    mut last: Local<String>,
) {
    let Some(e) = sb.0 else { return };
    let Ok(mut s) = screens.get_mut(e) else { return };
    let show = m.phase == Phase::Playing && !hidden.0;
    let (mut mine, mut rival) = (stats.kills, m.deaths);
    // PG3D_TEAM_SCORE="17|34" (automated screenshots): fixed scores, to compare with the clips
    if let Some((a, b)) = std::env::var("PG3D_TEAM_SCORE").ok().and_then(|v| {
        let (a, b) = v.split_once('|')?;
        Some((a.parse().ok()?, b.parse().ok()?))
    }) {
        (mine, rival) = (a, b);
    }
    let time = clock_text(m.time_left);
    let key = format!("{show} {mine} {rival} {time}");
    if *last == key {
        return;
    }
    *last = key;
    commands.entity(e).insert(if show { Visibility::Inherited } else { Visibility::Hidden });
    for (name, text) in [
        ("ScoreTeamRedLabel", mine.to_string()),
        ("ScoreTeamBlueLabel", rival.to_string()),
        ("TimeTeambattleLabel", time),
    ] {
        if let Some(i) = s.find(name) {
            s.set_text(i, text);
        }
    }
    // the leading side's score turns yellow (OBSERVED: about #F7D605 in the clips)
    for (name, lead) in [("ScoreTeamRedLabel", mine > rival), ("ScoreTeamBlueLabel", rival > mine)] {
        if let Some(i) = s.find(name) {
            s.set_color(i, if lead { [0.97, 0.84, 0.02, 1.0] } else { [1.0; 4] });
        }
    }
    for (name, on) in [("WiningBlue", mine > rival), ("WiningRed", rival > mine)] {
        if let Some(i) = s.find(name) {
            s.set_active(i, on);
        }
    }
}

/// The big 3, 2, 1 before the end.
fn countdown(
    mut commands: Commands,
    m: Res<TeamMatch>,
    mut label: Query<&mut Text, With<CountdownLabel>>,
    root: Query<Entity, With<CountdownRoot>>,
) {
    let show = m.phase == Phase::Playing && m.time_left > 0.0 && m.time_left <= COUNTDOWN;
    let n = m.time_left.ceil() as i32;
    match (show, label.single_mut()) {
        (true, Ok(mut t)) => t.0 = n.to_string(),
        (true, Err(_)) if root.is_empty() => {
            commands.spawn((
                CountdownRoot,
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100.0),
                    top: percent(9.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                GlobalZIndex(150_000),
                children![(
                    CountdownLabel,
                    Text::new(n.to_string()),
                    TextFont { font_size: FontSize::Px(64.0), ..default() },
                    TextColor(Color::srgb(1.0, 0.25, 0.1)),
                    TextShadow { offset: Vec2::splat(3.0), color: Color::BLACK },
                )],
            ));
        }
        (false, _) => {
            for e in &root {
                commands.entity(e).despawn();
            }
        }
        _ => {}
    }
}

/// Tab holds the list of players.
#[allow(clippy::too_many_arguments)]
fn table_key(
    keys: Res<ButtonInput<KeyCode>>,
    m: Res<TeamMatch>,
    stats: Res<PlayerStats>,
    screen: Res<TableScreen>,
    mut screens: Query<&mut NguiScreen>,
    mut vis: Query<&mut Visibility>,
    hud: UiRoots<Without<Dim>>,
    mut commands: Commands,
    mut open: Local<bool>,
    mut hid: Local<Vec<Entity>>,
    mut hud_hidden: ResMut<crate::ingame_ui::HudHidden>,
    mut filled: Local<bool>,
) {
    if m.phase != Phase::Playing {
        return;
    }
    let Some(e) = screen.0 else { return };
    let want = keys.pressed(KeyCode::Tab) || std::env::var("PG3D_TABLE").is_ok();
    // the screen loads a frame or two after it is spawned: fill it once it's there
    if want && (!*filled || stats.is_changed()) && let Ok(mut s) = screens.get_mut(e) {
        layout(&mut s, View::List);
        fill_table(&mut s, &stats, &m);
        *filled = true;
    }
    if want == *open {
        return;
    }
    *open = want;
    if !want {
        *filled = false;
    }
    if let Ok(mut v) = vis.get_mut(e) {
        *v = if want { Visibility::Inherited } else { Visibility::Hidden };
    }
    // the in-match HUD makes way for the table, over the dimmed game
    hud_hidden.0 = want;
    if want {
        for h in &hud {
            if h != e && vis.get(h).is_ok_and(|v| *v != Visibility::Hidden) {
                hid.push(h);
                commands.entity(h).insert(Visibility::Hidden);
            }
        }
    } else {
        for h in hid.drain(..) {
            commands.entity(h).insert(Visibility::Inherited);
        }
    }
    set_dim(&mut commands, want);
}

/// Darkens the game behind the table.
#[derive(Component)]
struct Dim;

fn set_dim(commands: &mut Commands, on: bool) {
    if on {
        commands.queue(|w: &mut World| {
            let mut q = w.query_filtered::<(), With<Dim>>();
            if q.iter(w).next().is_none() {
                w.spawn((
                    Dim,
                    Node { position_type: PositionType::Absolute, width: percent(100.0), height: percent(100.0), ..default() },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
                    GlobalZIndex(90_000),
                ));
            }
        });
    } else {
        commands.queue(|w: &mut World| {
            let mut q = w.query_filtered::<Entity, With<Dim>>();
            let es: Vec<Entity> = q.iter(w).collect();
            for e in es {
                w.despawn(e);
            }
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn ending(
    time: Res<Time<Real>>,
    mut m: ResMut<TeamMatch>,
    stats: Res<PlayerStats>,
    data: Res<DataDir>,
    screen: Res<TableScreen>,
    mut screens: Query<&mut NguiScreen>,
    mut vis: Query<&mut Visibility>,
    mut wallet: ResMut<crate::wallet::Wallet>,
    mut cursor: Single<&mut CursorOptions>,
    mut vtime: ResMut<Time<Virtual>>,
    mut hidden: Local<bool>,
    mut hud_hidden: ResMut<crate::ingame_ui::HudHidden>,
    hud: UiRoots<Without<NguiScreen>>,
    view_models: Query<Entity, With<crate::weapons::ViewModel>>,
    mut commands: Commands,
) {
    let Some(e) = screen.0 else { return };
    let outcome = match stats.kills.cmp(&m.deaths) {
        std::cmp::Ordering::Greater => Outcome::Victory,
        std::cmp::Ordering::Less => Outcome::Defeat,
        std::cmp::Ordering::Equal => Outcome::Draw,
    };
    let dt = time.delta_secs();
    match m.phase {
        Phase::Playing => {
            if m.time_left > 0.0 {
                return;
            }
            info!("team fight over: {outcome:?} {} : {}", stats.kills, m.deaths);
            m.phase = Phase::Banner(0.0);
            hud_hidden.0 = true;
            vtime.pause();
            cursor.grab_mode = CursorGrabMode::None;
            cursor.visible = true;
            // place 1 on our side of the table (BalanceRewards.TeamFight XP/C by place)
            let r: Rewards = std::fs::read_to_string(data.0.join("weapons/balance.json"))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default();
            let tf = r.rewards.get("TeamFight");
            m.reward = (
                tf.and_then(|t| t.xp.first().copied()).unwrap_or(20),
                tf.and_then(|t| t.coins.first().copied()).unwrap_or(4),
            );
            if let Ok(mut s) = screens.get_mut(e) {
                layout(&mut s, View::Banner(outcome));
            }
            if let Ok(mut v) = vis.get_mut(e) {
                *v = Visibility::Inherited;
            }
        }
        Phase::Banner(t) => {
            let t = t + dt;
            m.phase = Phase::Banner(t);
            if t < BANNER_TIME {
                return;
            }
            m.phase = Phase::Table(0.0);
            if !*hidden {
                *hidden = true;
                for h in &hud {
                    if h != e {
                        commands.entity(h).insert(Visibility::Hidden);
                    }
                }
                for v in &view_models {
                    commands.entity(v).insert(Visibility::Hidden);
                }
            }
            if let Ok(mut s) = screens.get_mut(e) {
                layout(&mut s, View::End(outcome));
                fill_table(&mut s, &stats, &m);
            }
            if !m.paid {
                m.paid = true;
                wallet.add_coins(m.reward.1);
                let up = crate::progress::Progress::add_xp(&data.0, m.reward.0);
                info!("reward: {} XP, {} coins{}", m.reward.0, m.reward.1, if up { " (level up)" } else { "" });
            }
        }
        Phase::Table(t) => {
            let t = t + dt;
            m.phase = Phase::Table(t);
            let k = (t / COUNT_TIME).clamp(0.0, 1.0);
            let Ok(mut s) = screens.get_mut(e) else { return };
            for (item, prefix, total) in [("02RewardExp", "ExpLable", m.reward.0), ("04RewardGold", "MoneyLable", m.reward.1)] {
                let Some(g) = s.find(item) else { continue };
                let v = ((total as f32) * k).round() as i64;
                let text = if prefix == "ExpLable" { format!("{v} XP") } else { format!("{v} {}", s.string("Key_0275")) };
                for i in s.subtree(g) {
                    if s.nodes[i].name.starts_with(prefix) {
                        s.set_text(i, text.clone());
                    }
                }
            }
        }
    }
}

fn clicks(mut clicks: MessageReader<NguiClick>, m: Res<TeamMatch>, mut exit: MessageWriter<AppExit>) {
    for c in clicks.read() {
        if matches!(m.phase, Phase::Table(_)) && matches!(c.name.as_str(), "Ok" | "Background" | "Continue_GoToLobbyButton") {
            crate::menu::relaunch(None);
            exit.write(AppExit::Success);
        }
    }
}
