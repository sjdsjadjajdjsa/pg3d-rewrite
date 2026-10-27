//! First start: no exported game data yet. Finds the player's own Pixel Gun 3D install (or
//! lets them pick / drop its folder) and runs the exporter next door
//! (../exporter/scripts/setup.py), showing its progress. `cargo run -- --setup`
//! opens this screen again, e.g. after a game update.

#[allow(unused_imports)]
use crate::scene::{FixedUpdate, PostStartup, PostUpdate, PreStartup, Startup, Update};
use bevy::{prelude::*, window::FileDragAndDrop};
use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};

/// The exporter project (PG3D_RE overrides it).
pub fn exporter_dir() -> PathBuf {
    std::env::var_os("PG3D_RE").map_or_else(|| PathBuf::from("../exporter"), PathBuf::from)
}

/// Whether there is anything to play: at least one exported map.
pub fn has_data(data: &Path) -> bool {
    !crate::menu::exported_maps(data).is_empty()
}

pub struct SetupPlugin;

impl crate::scene::ScenePlugin for SetupPlugin {
    fn build(&self, app: &mut crate::scene::SceneBuilder) {
        app.insert_resource(ClearColor(Color::srgb(0.08, 0.1, 0.14)))
            .insert_resource(Job::default())
            .init_resource::<StopImportOnExit>()
            .add_systems(Startup, (spawn, search))
            .add_systems(Update, (dropped_folders, clicks, button_colors, refresh, auto_import));
    }
}

#[derive(Clone, Default, PartialEq)]
enum Phase {
    #[default]
    Searching,
    NoPython,
    NotFound(String),
    Found(PathBuf),
    Importing { step: usize, steps: usize, what: String },
    Finished { ok: bool, summary: String },
}

#[derive(Default)]
struct Shared {
    phase: Phase,
    /// export steps that failed
    failed: Vec<String>,
    /// the running setup.py (its own process group on Unix)
    pid: Option<u32>,
}

/// State shared with the worker threads (search, folder dialog, exporter).
#[derive(Resource, Default, Clone)]
struct Job(Arc<Mutex<Shared>>);

impl Job {
    fn set(&self, phase: Phase) {
        if let Ok(mut s) = self.0.lock() {
            s.phase = phase;
        }
    }
    fn phase(&self) -> Phase {
        self.0.lock().map(|s| s.phase.clone()).unwrap_or_default()
    }
    fn busy(&self) -> bool {
        matches!(self.phase(), Phase::Searching | Phase::Importing { .. })
    }
}

#[derive(Component)]
struct Status;
#[derive(Component)]
struct Detail;
#[derive(Component)]
struct Failures;
#[derive(Component)]
struct Bar;
#[derive(Component)]
struct BarFill;
#[derive(Component, Clone, Copy, PartialEq)]
enum Action {
    Import,
    Play,
    Choose,
    Search,
}

const NORMAL: Color = Color::srgb(0.18, 0.22, 0.3);
const HOVER: Color = Color::srgb(0.28, 0.36, 0.5);
const MUTED: Color = Color::srgb(0.7, 0.75, 0.85);

/// Closing the window stops a running import (setup.py and the exporters it started).
#[derive(Resource)]
struct StopImportOnExit(Job);

impl FromWorld for StopImportOnExit {
    fn from_world(world: &mut World) -> Self {
        Self(world.resource::<Job>().clone())
    }
}

impl Drop for StopImportOnExit {
    fn drop(&mut self) {
        let Some(pid) = self.0.0.lock().ok().and_then(|mut s| s.pid.take()) else { return };
        let _ = if cfg!(windows) {
            Command::new("taskkill").args(["/T", "/F", "/PID", &pid.to_string()]).output()
        } else {
            Command::new("kill").args(["-TERM", "--", &format!("-{pid}")]).output()
        };
    }
}

/// Python 3 on this computer: the Windows launcher first, then the usual names.
fn python() -> Option<Vec<&'static str>> {
    let candidates: &[&[&str]] = if cfg!(windows) { &[&["py", "-3"], &["python"], &["python3"]] } else { &[&["python3"], &["python"]] };
    candidates.iter().find_map(|c| {
        let out = Command::new(c[0]).args(&c[1..]).arg("--version").output().ok()?;
        let version = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
        (out.status.success() && version.contains("Python 3")).then(|| c.to_vec())
    })
}

fn setup_command(py: &[&str]) -> Command {
    let mut c = Command::new(py[0]);
    c.args(&py[1..]).arg("scripts/setup.py").current_dir(exporter_dir()).env("PYTHONIOENCODING", "utf-8");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0); // the exporters it starts can be stopped together
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console window pops up
    }
    c
}

/// Asks setup.py where the game is (`folder`: check this one instead of searching).
fn find(job: Job, folder: Option<PathBuf>) {
    job.set(Phase::Searching);
    std::thread::spawn(move || {
        let Some(py) = python() else { return job.set(Phase::NoPython) };
        let mut c = setup_command(&py);
        c.arg("--find");
        if let Some(f) = &folder {
            c.arg(f);
        }
        let found = c.output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
        job.set(match (found, folder) {
            (Some(p), _) if !p.is_empty() => Phase::Found(PathBuf::from(p)),
            (_, Some(f)) => Phase::NotFound(format!("{} is not a Pixel Gun 3D install.", f.display())),
            _ => Phase::NotFound(String::new()),
        });
    });
}

fn import(job: Job, game: PathBuf) {
    if let Ok(mut s) = job.0.lock() {
        s.phase = Phase::Importing { step: 0, steps: 0, what: "starting".into() };
        s.failed.clear();
    }
    std::thread::spawn(move || {
        let Some(py) = python() else { return job.set(Phase::NoPython) };
        let child = setup_command(&py).arg("--game").arg(&game).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => return job.set(Phase::Finished { ok: false, summary: format!("could not start Python: {e}") }),
        };
        if let Ok(mut s) = job.0.lock() {
            s.pid = Some(child.id());
        }
        // the last line on stderr says why setup.py stopped
        let stderr = child.stderr.take().map(|err| {
            std::thread::spawn(move || BufReader::new(err).lines().map_while(Result::ok).filter(|l| !l.trim().is_empty()).last().unwrap_or_default())
        });
        let mut summary = String::new();
        if let Some(out) = child.stdout.take() {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                on_line(&job, &line, &mut summary);
            }
        }
        let ok = child.wait().is_ok_and(|s| s.success());
        if let Ok(mut s) = job.0.lock() {
            s.pid = None;
        }
        let err = stderr.and_then(|t| t.join().ok()).unwrap_or_default();
        job.set(Phase::Finished { ok, summary: if ok || err.is_empty() { summary } else { err.trim().to_string() } });
    });
}

/// One line of setup.py / export_all.py output: "[setup] what", "[3/40] what",
/// "      FAILED  why", "done: ...".
fn on_line(job: &Job, line: &str, summary: &mut String) {
    let Ok(mut s) = job.0.lock() else { return };
    let t = line.trim();
    if let Some((tag, what)) = t.strip_prefix('[').and_then(|r| r.split_once("] ")) {
        let step = tag.split_once('/').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)));
        if let Some((step, steps)) = step {
            s.phase = Phase::Importing { step, steps, what: what.to_string() };
            return;
        }
        if tag == "setup" {
            s.phase = Phase::Importing { step: 0, steps: 0, what: what.to_string() };
            return;
        }
    }
    if t.starts_with("done:") {
        *summary = t.to_string();
    } else if t.starts_with("FAILED")
        && let Phase::Importing { what, .. } = &s.phase
    {
        let what = what.clone();
        s.failed.push(what);
    }
}

fn search(job: Res<Job>) {
    find(job.clone(), None);
}

fn spawn(mut commands: Commands) {
    commands.spawn(Camera2d);
    let text = |size: f32, color: Color| {
        (TextFont { font_size: FontSize::Px(size), ..default() }, TextColor(color), TextLayout::justify(Justify::Center))
    };
    commands
        .spawn(Node {
            width: percent(100.0),
            height: percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: px(14.0),
            padding: UiRect::all(px(24.0)),
            ..default()
        })
        .with_children(|root| {
            root.spawn((Text::new("PIXEL GUN 3D"), text(56.0, Color::WHITE)));
            root.spawn((Text::new("Import the maps, weapons and zombies from your own copy of the game"), text(20.0, MUTED)));
            root.spawn((Status, Text::new(""), text(24.0, Color::WHITE), Node { margin: UiRect::top(px(24.0)), ..default() }));
            root.spawn((Detail, Text::new(""), text(17.0, MUTED), Node { max_width: px(760.0), ..default() }));
            root.spawn((Bar, Node { width: px(480.0), height: px(10.0), ..default() }, BackgroundColor(NORMAL), Visibility::Hidden))
                .with_child((BarFill, Node { width: percent(0.0), height: percent(100.0), ..default() }, BackgroundColor(Color::srgb(0.35, 0.8, 0.35))));
            root.spawn(Node { column_gap: px(12.0), margin: UiRect::top(px(16.0)), ..default() }).with_children(|row| {
                for (action, label) in [(Action::Import, "Import"), (Action::Play, "Play"), (Action::Choose, "Choose folder..."), (Action::Search, "Search again")] {
                    row.spawn((action, Button, Node { padding: UiRect::axes(px(22.0), px(12.0)), display: Display::None, ..default() }, BackgroundColor(NORMAL)))
                        .with_child((Text::new(label), TextFont { font_size: FontSize::Px(20.0), ..default() }));
                }
            });
            root.spawn((Failures, Text::new(""), text(15.0, Color::srgb(0.95, 0.7, 0.55)), Node { max_width: px(900.0), margin: UiRect::top(px(16.0)), ..default() }));
        });
}

/// Show the job's state: texts, progress bar and the buttons that make sense now.
fn refresh(
    job: Res<Job>,
    mut texts: Query<(&mut Text, Has<Status>, Has<Detail>), Or<(With<Status>, With<Detail>, With<Failures>)>>,
    mut bar: Query<&mut Visibility, With<Bar>>,
    mut fill: Query<&mut Node, (With<BarFill>, Without<Action>)>,
    mut buttons: Query<(&Action, &mut Node), Without<BarFill>>,
) {
    let (phase, failed) = match job.0.lock() {
        Ok(s) => (s.phase.clone(), s.failed.clone()),
        Err(_) => return,
    };
    let steam = "Install \"Pixel Gun 3D: PC Edition\" from Steam (it is free), then press Search again. \
                 Or choose (or drop on this window) the folder with \"Pixel Gun 3D.exe\" in it.";
    let (status, detail, progress, show): (String, String, Option<f32>, &[Action]) = match &phase {
        Phase::Searching => ("Looking for Pixel Gun 3D...".into(), String::new(), None, &[]),
        Phase::NoPython => (
            "Python 3 is needed for the import".into(),
            if cfg!(windows) {
                "Install it from python.org (tick \"Add python.exe to PATH\"), then press Search again.".into()
            } else {
                "Install python3 with your package manager, then press Search again.".into()
            },
            None,
            &[Action::Search],
        ),
        Phase::NotFound(why) => ("Pixel Gun 3D not found".into(), format!("{why} {steam}").trim().to_string(), None, &[Action::Choose, Action::Search]),
        Phase::Found(p) => (
            "Found Pixel Gun 3D".into(),
            format!("{}\nImport only reads the game's files and takes a few minutes.", p.display()),
            None,
            &[Action::Import, Action::Choose],
        ),
        Phase::Importing { step, steps, what } => (
            if *steps > 0 { format!("Importing... {step} / {steps}") } else { "Importing...".into() },
            what.clone(),
            Some(if *steps > 0 { step.saturating_sub(1) as f32 / *steps as f32 } else { 0.0 }),
            &[],
        ),
        Phase::Finished { ok: true, summary } => ("Import finished".into(), summary.clone(), Some(1.0), &[Action::Play, Action::Choose]),
        Phase::Finished { ok: false, summary } => ("Import failed".into(), summary.clone(), None, &[Action::Choose, Action::Search]),
    };
    let failures = if failed.is_empty() {
        String::new()
    } else {
        format!("Not readable in this game version (the rewrite uses its own simpler versions): {}", failed.join(", "))
    };
    for (mut t, is_status, is_detail) in &mut texts {
        let want = if is_status { &status } else if is_detail { &detail } else { &failures };
        if t.0 != *want {
            t.0 = want.clone();
        }
    }
    let visible = if progress.is_some() { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in &mut bar {
        v.set_if_neq(visible);
    }
    for mut n in &mut fill {
        let width = percent(progress.unwrap_or(0.0) * 100.0);
        if n.width != width {
            n.width = width;
        }
    }
    for (a, mut n) in &mut buttons {
        let display = if show.contains(a) { Display::Flex } else { Display::None };
        if n.display != display {
            n.display = display;
        }
    }
}

fn clicks(job: Res<Job>, q: Query<(&Interaction, &Action), Changed<Interaction>>) {
    for (i, a) in &q {
        if *i != Interaction::Pressed || job.busy() {
            continue;
        }
        match a {
            Action::Search => find(job.clone(), None),
            Action::Import => {
                if let Phase::Found(p) = job.phase() {
                    import(job.clone(), p);
                }
            }
            Action::Play => crate::scene::request(crate::scene::Scene::Menu),
            Action::Choose => {
                let job = job.clone();
                job.set(Phase::Searching);
                std::thread::spawn(move || {
                    let folder = rfd::FileDialog::new().set_title("The Pixel Gun 3D folder").pick_folder();
                    find(job, folder);
                });
            }
        }
    }
}

/// PG3D_SETUP_AUTO=1 (tests): press Import once the game is found, Play when it is done.
fn auto_import(job: Res<Job>, mut started: Local<bool>) {
    if std::env::var("PG3D_SETUP_AUTO").is_err() {
        return;
    }
    match job.phase() {
        Phase::Found(p) if !*started => {
            *started = true;
            import(job.clone(), p);
        }
        Phase::Finished { ok: true, .. } => crate::scene::request(crate::scene::Scene::Menu),
        _ => {}
    }
}

/// A folder (or the game's .exe) dropped on the window.
fn dropped_folders(job: Res<Job>, mut drops: MessageReader<FileDragAndDrop>) {
    for d in drops.read() {
        if let FileDragAndDrop::DroppedFile { path_buf, .. } = d
            && !job.busy()
        {
            find(job.clone(), Some(path_buf.clone()));
        }
    }
}

fn button_colors(mut q: Query<(&Interaction, &mut BackgroundColor), (Changed<Interaction>, With<Action>)>) {
    for (i, mut bg) in &mut q {
        bg.0 = if *i == Interaction::None { NORMAL } else { HOVER };
    }
}
