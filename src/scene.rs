//! One window for the whole game: the menu and every match are "scenes" of a single App.
//!
//! Each plugin of a scene adds its systems to the scene's own schedules (the labels below,
//! which shadow Bevy's Startup / Update / ... in the plugin files). Switching scenes
//! despawns everything the old scene spawned, throws its schedules away (so every
//! `Local` starts fresh, like a newly started program), installs the new scene's plugins,
//! runs its PreStartup / Startup / PostStartup once, and from then on its Update,
//! FixedUpdate and PostUpdate run inside Bevy's.

use std::sync::Mutex;

use bevy::{
    ecs::schedule::{ScheduleConfigs, ScheduleLabel},
    ecs::system::ScheduleSystem,
    prelude::*,
    window::{Monitor, PrimaryWindow},
};

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct PreStartup;
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Startup;
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct PostStartup;
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Update;
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct FixedUpdate;
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct PostUpdate;

/// A plugin of a scene (built again every time its scene starts).
pub trait ScenePlugin {
    fn build(&self, app: &mut SceneBuilder);
}

macro_rules! tuple_plugin {
    ($($p:ident),+) => {
        #[allow(non_snake_case)]
        impl<$($p: ScenePlugin),+> ScenePlugin for ($($p,)+) {
            fn build(&self, app: &mut SceneBuilder) {
                let ($($p,)+) = self;
                $($p.build(app);)+
            }
        }
    };
}
tuple_plugin!(A);
tuple_plugin!(A, B);
tuple_plugin!(A, B, C);
tuple_plugin!(A, B, C, D);
tuple_plugin!(A, B, C, D, E);

/// What scene plugins build into: the App's World, with App-like methods.
pub struct SceneBuilder<'w> {
    pub world: &'w mut World,
}

impl SceneBuilder<'_> {
    pub fn add_systems<M>(&mut self, label: impl ScheduleLabel, systems: impl IntoScheduleConfigs<ScheduleSystem, M>) -> &mut Self {
        self.world.resource_mut::<Schedules>().add_systems(label, systems);
        self
    }
    pub fn insert_resource<R: Resource>(&mut self, r: R) -> &mut Self {
        self.world.insert_resource(r);
        self
    }
    /// A fresh value every time the scene starts.
    pub fn init_resource<R: Resource + FromWorld>(&mut self) -> &mut Self {
        self.world.remove_resource::<R>();
        self.world.init_resource::<R>();
        self
    }
    pub fn add_plugins(&mut self, p: impl ScenePlugin) -> &mut Self {
        p.build(self);
        self
    }
}

/// Which scene to show.
#[derive(Clone, Debug, PartialEq)]
pub enum Scene {
    /// importing the game's files (setup.rs)
    Setup,
    Menu,
    /// a match on this map; mode: PG3D_MODE ("teamfight"), sandbox: no enemies
    Game { map: String, mode: Option<String>, sandbox: bool },
}

static REQUEST: Mutex<Option<Scene>> = Mutex::new(None);

/// Switch to `scene` at the end of this frame.
pub fn request(scene: Scene) {
    if let Ok(mut r) = REQUEST.lock() {
        *r = Some(scene);
    }
}

/// The scenes' plugin lists (main.rs).
#[derive(Resource)]
pub struct SceneSets {
    pub setup: fn(&mut SceneBuilder),
    pub menu: fn(&mut SceneBuilder),
    pub game: fn(&mut SceneBuilder),
}


pub fn switch_scene(world: &mut World) {
    let Some(scene) = REQUEST.lock().ok().and_then(|mut r| r.take()) else { return };
    info!("scene: {scene:?}");
    // the old scene's entities (top-level ones; children go with them)
    let mut q = world.query_filtered::<Entity, Without<ChildOf>>();
    let all: Vec<Entity> = q.iter(world).collect();
    // what exists before the first scene belongs to the App (windows, observers, ...)
    if !world.contains_resource::<AppEntities>() {
        world.insert_resource(AppEntities(all.iter().copied().collect()));
    }
    let app_entities = world.resource::<AppEntities>().0.clone();
    let all: Vec<Entity> = all.into_iter().filter(|e| !app_entities.contains(e)).collect();
    // entities that live for the whole App
    let keep = |w: &World, e: Entity| {
        let Ok(r) = w.get_entity(e) else { return true };
        r.contains::<bevy::ecs::resource::IsResource>()
            || r.contains::<Window>()
            || r.contains::<Monitor>()
            || r.contains::<Observer>()
            || r.contains::<bevy::picking::pointer::PointerId>()
            || r.contains::<Gamepad>()
    };
    let old: Vec<Entity> = all.into_iter().filter(|&e| !keep(world, e)).collect();
    for e in old {
        if let Ok(ec) = world.get_entity_mut(e) {
            ec.despawn();
        }
    }
    // and its systems
    {
        let mut s = world.resource_mut::<Schedules>();
        for l in [
            PreStartup.intern(),
            Startup.intern(),
            PostStartup.intern(),
            Update.intern(),
            FixedUpdate.intern(),
            PostUpdate.intern(),
        ] {
            s.remove(l);
        }
    }
    world.resource_mut::<Time<Virtual>>().unpause();
    // what the scene's plugins read from the environment (as for a freshly started copy)
    let title = match &scene {
        Scene::Setup => "Pixel Gun 3D (Rust) - import".to_string(),
        Scene::Menu => {
            unsafe {
                std::env::remove_var("PG3D_MODE");
                std::env::remove_var("PG3D_NOBOTS");
            }
            "Pixel Gun 3D (Rust)".to_string()
        }
        Scene::Game { map, mode, sandbox } => {
            unsafe {
                match mode {
                    Some(m) => std::env::set_var("PG3D_MODE", m),
                    None => std::env::remove_var("PG3D_MODE"),
                }
                if *sandbox {
                    std::env::set_var("PG3D_NOBOTS", "1");
                } else {
                    std::env::remove_var("PG3D_NOBOTS");
                }
            }
            world.insert_resource(crate::map::CurrentMap(map.clone()));
            format!("Pixel Gun 3D (Rust) - {map}")
        }
    };
    let mut wq = world.query_filtered::<&mut Window, With<PrimaryWindow>>();
    if let Ok(mut w) = wq.single_mut(world) {
        w.title = title;
    }
    // a new scene starts with a free, visible mouse (a match grabs it on the first click)
    let mut cq = world.query_filtered::<&mut bevy::window::CursorOptions, With<PrimaryWindow>>();
    if let Ok(mut c) = cq.single_mut(world) {
        c.grab_mode = bevy::window::CursorGrabMode::None;
        c.visible = true;
    }
    let sets = world.resource::<SceneSets>();
    let build = match scene {
        Scene::Setup => sets.setup,
        Scene::Menu => sets.menu,
        Scene::Game { .. } => sets.game,
    };
    {
        let mut b = SceneBuilder { world };
        build(&mut b);
    }
    for l in [PreStartup.intern(), Startup.intern(), PostStartup.intern()] {
        let _ = world.try_run_schedule(l);
    }
    world.insert_resource(CurrentScene(scene));
}

/// Entities made before any scene (kept for the App's whole life).
#[derive(Resource)]
struct AppEntities(std::collections::HashSet<Entity>);

/// The scene that is showing.
#[derive(Resource, Clone)]
#[allow(dead_code)]
pub struct CurrentScene(pub Scene);

pub fn run_update(world: &mut World) {
    let _ = world.try_run_schedule(Update);
}
pub fn run_fixed_update(world: &mut World) {
    let _ = world.try_run_schedule(FixedUpdate);
}
pub fn run_post_update(world: &mut World) {
    let _ = world.try_run_schedule(PostUpdate);
}

/// The App-wide part: scene runners and switching.
pub struct ScenesPlugin;

impl Plugin for ScenesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(bevy::app::Update, run_update)
            .add_systems(bevy::app::FixedUpdate, run_fixed_update)
            .add_systems(bevy::app::PostUpdate, run_post_update.after(TransformSystems::Propagate))
            .add_systems(bevy::app::Last, switch_scene);
    }
}

#[allow(dead_code)]
fn _assert(_: ScheduleConfigs<ScheduleSystem>) {}
