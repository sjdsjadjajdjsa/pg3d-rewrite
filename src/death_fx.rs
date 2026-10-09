//! The angel: when anyone dies, a white see-through ghost of the blocky body with little
//! wings (Player_Dead_Angel, material Player_Dead at 60 % alpha) rises out of where they
//! fell, flapping its arms, 4.3 m in its one 4.8 s clip ("Take 001"), then is gone.
//! Exported by export_death.py into characters/dead_angel/.

#[allow(unused_imports)]
use crate::scene::{FixedUpdate, PostStartup, PostUpdate, PreStartup, Startup, Update};
use bevy::prelude::*;

use crate::DataDir;

pub struct DeathFxPlugin;

impl crate::scene::ScenePlugin for DeathFxPlugin {
    fn build(&self, app: &mut crate::scene::SceneBuilder) {
        app.add_systems(Startup, load).add_systems(Update, (spawn, start, age).chain());
    }
}

const PATH: &str = "characters/dead_angel/angel.gltf";
/// "Take 001"'s length
const LIFE: f32 = 4.8;

/// Someone died here (their feet) facing `yaw`.
#[derive(Message, Clone, Copy)]
pub struct SpawnAngel {
    pub at: Vec3,
    pub yaw: f32,
}

#[derive(Resource)]
struct AngelAssets {
    scene: Handle<WorldAsset>,
    graph: Handle<AnimationGraph>,
    clip: AnimationNodeIndex,
}

#[derive(Component)]
struct Angel {
    age: f32,
    playing: bool,
}

fn load(mut commands: Commands, data: Res<DataDir>, assets: Res<AssetServer>, mut graphs: ResMut<Assets<AnimationGraph>>) {
    if !data.0.join(PATH).exists() {
        warn!("no {PATH} (run export_death.py): no angels");
        return;
    }
    let (graph, clip) = AnimationGraph::from_clip(assets.load(GltfAssetLabel::Animation(0).from_asset(PATH)));
    commands.insert_resource(AngelAssets { scene: assets.load(GltfAssetLabel::Scene(0).from_asset(PATH)), graph: graphs.add(graph), clip });
}

fn spawn(mut commands: Commands, mut msgs: MessageReader<SpawnAngel>, angel: Option<Res<AngelAssets>>) {
    let Some(a) = angel else {
        msgs.clear();
        return;
    };
    for m in msgs.read() {
        commands.spawn((
            Name::new("angel"),
            Angel { age: 0.0, playing: false },
            WorldAssetRoot(a.scene.clone()),
            Transform::from_translation(m.at).with_rotation(Quat::from_rotation_y(m.yaw)),
        ));
    }
}

/// Once its scene is in: play the clip once.
fn start(
    mut commands: Commands,
    a: Option<Res<AngelAssets>>,
    mut angels: Query<(Entity, &mut Angel)>,
    children: Query<&Children>,
    mut players: Query<&mut AnimationPlayer>,
) {
    let Some(a) = a else { return };
    for (e, mut angel) in &mut angels {
        if angel.playing {
            continue;
        }
        let Some(p) = children.iter_descendants(e).find(|d| players.contains(*d)) else { continue };
        if let Ok(mut player) = players.get_mut(p) {
            player.play(a.clip);
        }
        commands.entity(p).insert(AnimationGraphHandle(a.graph.clone()));
        angel.playing = true;
    }
}

fn age(
    mut commands: Commands,
    time: Res<Time>,
    mut angels: Query<(Entity, &mut Angel, &GlobalTransform)>,
    children: Query<&Children>,
    names: Query<(&Name, &GlobalTransform)>,
) {
    for (e, mut a, gt) in &mut angels {
        let before = a.age;
        a.age += time.delta_secs();
        if before < 2.0 && a.age >= 2.0 {
            let body = children.iter_descendants(e).find_map(|d| names.get(d).ok().filter(|(n, _)| n.as_str() == "PLAYER_Body"));
            debug!("angel at {:.1}: body {:?} after 2 s (playing {})", gt.translation(), body.map(|b| b.1.translation()), a.playing);
        }
        if a.age >= LIFE {
            commands.entity(e).despawn();
        }
    }
}
