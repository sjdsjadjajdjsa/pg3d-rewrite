//! A small runtime for Unity's particle systems: muzzle flashes, shells, smoke, explosions.
//!
//! The exporter (export_map.py `particle_settings`) tags glTF nodes that had a ParticleSystem
//! with `{"particle_texture": "...png", "particle": {...}}`: emission (bursts and a rate over
//! the system's duration, looping or not), start values as [min, max] ranges, the emitter
//! shape, the over-lifetime modules sampled at 9 points (size, colour, velocity, texture-
//! sheet frame), simulation space and gravity, and how the renderer draws (billboard or
//! stretched along the velocity; additive, alpha or multiply blending).
//!
//! Guns: FlashFire (export: `extras.flash_fire`) switches its gun-flash object on for
//! `timeFireAction` seconds after every shot; while it is on, the looping systems under it
//! keep playing (so holding fire keeps the flash going) and when it goes off their
//! particles vanish with it. Systems outside a gun flash (shells, explosions) play once per
//! [`Burst`]. Map systems with Play On Awake loop from the start.

#[allow(unused_imports)]
use crate::scene::{FixedUpdate, PostStartup, PostUpdate, PreStartup, Startup, Update};
use bevy::{
    camera::visibility::RenderLayers,
    gltf::GltfExtras,
    math::Affine2,
    prelude::*,
};
use serde::Deserialize;

pub struct ParticlesPlugin;

impl crate::scene::ScenePlugin for ParticlesPlugin {
    fn build(&self, app: &mut crate::scene::SceneBuilder) {
        app.add_systems(Update, (attach_emitters, shots, flash_timers, emit, update_particles).chain())
            .add_systems(Update, trace_vis);
    }
}

/// Put on a prefab root: asset folder its glTF lives in (texture paths are relative to it).
#[derive(Component)]
pub struct AssetDir(pub String);

/// "This prefab fired" (a gun shot, an explosion): gun flashes switch on, other particle
/// systems under it play once.
#[derive(Message)]
pub struct Burst(pub Entity);

/// On a two-gun weapon's root: which gun fired (0 first, 1 second); only that gun's flash
/// (the k-th FlashFire under the weapon) goes off.
#[derive(Component, Clone, Copy)]
pub struct Barrel(pub usize);

/// Most particles alive at once (all systems together).
const MAX_PARTICLES: usize = 600;

#[derive(Deserialize, Clone)]
pub struct BurstDef {
    time: f32,
    min: f32,
    max: f32,
    #[serde(default = "one")]
    cycles: u32,
    #[serde(default)]
    interval: f32,
}

fn one() -> u32 {
    1
}

#[derive(Deserialize, Clone, Default)]
pub struct Shape {
    #[serde(rename = "type")]
    kind: i32,
    #[serde(default)]
    angle: f32,
    #[serde(default)]
    radius: f32,
}

#[derive(Deserialize, Clone, Default)]
pub struct Velocity {
    x: [f32; 2],
    y: [f32; 2],
    z: [f32; 2],
    #[serde(default)]
    world: bool,
}

#[derive(Deserialize, Clone)]
pub struct Render {
    #[serde(default)]
    mode: i32,
    #[serde(default = "two")]
    length_scale: f32,
    #[serde(default)]
    velocity_scale: f32,
    #[serde(default)]
    blend: String,
}

fn two() -> f32 {
    2.0
}

#[derive(Deserialize, Clone)]
pub struct ParticleSettings {
    pub life: f32,
    pub size: f32,
    pub color: [f32; 4],
    pub tiles: [u32; 2],
    pub row: u32,
    pub single_row: bool,
    /// 2: the full export (older exports only have the values above)
    #[serde(default)]
    v: u32,
    #[serde(default = "point_two")]
    duration: f32,
    #[serde(default)]
    looping: bool,
    #[serde(default)]
    play_on_awake: bool,
    #[serde(default)]
    world: bool,
    #[serde(default)]
    life_range: Option<[f32; 2]>,
    #[serde(default)]
    speed_range: [f32; 2],
    #[serde(default)]
    size_range: Option<[f32; 2]>,
    #[serde(default)]
    rotation_range: [f32; 2],
    #[serde(default)]
    spin_range: [f32; 2],
    #[serde(default)]
    gravity: f32,
    #[serde(default)]
    color_range: Option<[[f32; 4]; 2]>,
    #[serde(default)]
    rate: f32,
    #[serde(default)]
    bursts: Option<Vec<BurstDef>>,
    #[serde(default)]
    shape: Option<Shape>,
    #[serde(default)]
    size_curve: Option<Vec<f32>>,
    #[serde(default)]
    color_curve: Option<Vec<[f32; 4]>>,
    #[serde(default)]
    velocity: Option<Velocity>,
    #[serde(default)]
    frames: Option<Vec<f32>>,
    #[serde(default)]
    start_frame: [f32; 2],
    #[serde(default = "one_f")]
    uv_cycles: f32,
    #[serde(default)]
    render: Option<Render>,
}

fn point_two() -> f32 {
    0.2
}

fn one_f() -> f32 {
    1.0
}

/// Linear read of a curve sampled at evenly spaced points of 0..1.
fn curve(points: &[f32], t: f32) -> f32 {
    if points.is_empty() {
        return 1.0;
    }
    let f = t.clamp(0.0, 1.0) * (points.len() - 1) as f32;
    let i = (f.floor() as usize).min(points.len() - 1);
    let j = (i + 1).min(points.len() - 1);
    points[i] + (points[j] - points[i]) * f.fract()
}

fn curve4(points: &[[f32; 4]], t: f32) -> Vec4 {
    if points.is_empty() {
        return Vec4::ONE;
    }
    let f = t.clamp(0.0, 1.0) * (points.len() - 1) as f32;
    let i = (f.floor() as usize).min(points.len() - 1);
    let j = (i + 1).min(points.len() - 1);
    Vec4::from(points[i]).lerp(Vec4::from(points[j]), f.fract())
}

#[derive(Deserialize)]
struct Extras {
    particle_texture: String,
    particle: ParticleSettings,
}

#[derive(Deserialize)]
struct FlashExtras {
    flash_fire: FlashDef,
}

#[derive(Deserialize)]
struct FlashDef {
    time: f32,
    object: Option<String>,
}

/// A FlashFire: its gun-flash object is on for `time` seconds after each shot.
#[derive(Component)]
pub struct FlashFire {
    time: f32,
    object: Option<String>,
    target: Option<Entity>,
    left: f32,
}

#[derive(Component)]
pub struct ParticleEmitter {
    texture: Handle<Image>,
    s: ParticleSettings,
    /// seconds into the current cycle while playing
    clock: Option<f32>,
    /// keep looping (inside an active gun flash, or a looping map system)
    looping: bool,
    rate_acc: f32,
    /// bursts already fired this cycle (index, repeats done)
    fired: Vec<u32>,
    rng: u32,
    /// the gun flash this emitter belongs to (its particles vanish when it goes off)
    flash: Option<Entity>,
}

impl ParticleEmitter {
    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng % 100_000) as f32 / 100_000.0
    }
    fn range(&mut self, r: [f32; 2]) -> f32 {
        let k = self.rand();
        r[0] + (r[1] - r[0]) * k
    }
    fn play(&mut self, looping: bool) {
        self.clock = Some(0.0);
        self.looping = looping;
        self.rate_acc = 0.0;
        self.fired = vec![0; self.s.bursts.as_ref().map_or(0, |b| b.len())];
    }
}

#[derive(Component)]
struct Particle {
    age: f32,
    life: f32,
    s: ParticleSettings,
    material: Handle<StandardMaterial>,
    /// emitter (local simulation: `pos`/`vel` are in its space) or None (world space)
    local_to: Option<Entity>,
    pos: Vec3,
    vel: Vec3,
    /// velocity-over-lifetime (constant per particle), same space as `vel` unless `extra_world`
    extra: Vec3,
    extra_world: bool,
    size: f32,
    rot: f32,
    spin: f32,
    color: Vec4,
    start_frame: f32,
    flash: Option<Entity>,
}

#[derive(Resource)]
struct Quad(Handle<Mesh>);

fn attach_emitters(
    mut commands: Commands,
    added: Query<(Entity, &GltfExtras), Added<GltfExtras>>,
    parents: Query<&ChildOf>,
    dirs: Query<&AssetDir>,
    assets: Res<AssetServer>,
    mut seed: Local<u32>,
) {
    for (e, extras) in &added {
        if let Ok(f) = serde_json::from_str::<FlashExtras>(&extras.value) {
            commands.entity(e).insert(FlashFire { time: f.flash_fire.time.max(0.02), object: f.flash_fire.object, target: None, left: 0.0 });
        }
        let Ok(x) = serde_json::from_str::<Extras>(&extras.value) else { continue };
        let Some(dir) = parents.iter_ancestors(e).find_map(|a| dirs.get(a).ok()) else { continue };
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        // map systems that play by themselves (smoke, fire, sparkles)
        let auto = dir.0.starts_with("maps/") && x.particle.v >= 2 && x.particle.play_on_awake && x.particle.looping;
        let mut em = ParticleEmitter {
            texture: assets.load(format!("{}/{}", dir.0, x.particle_texture)),
            s: x.particle,
            clock: None,
            looping: false,
            rate_acc: 0.0,
            fired: vec![],
            rng: *seed | 1,
            flash: None,

        };
        if auto {
            em.play(true);
        }
        commands.entity(e).insert(em);
    }
}

/// A shot: gun flashes switch on (or stay on), other emitters under the prefab play once.
fn shots(
    mut bursts: MessageReader<Burst>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut flashes: Query<&mut FlashFire>,
    mut emitters: Query<&mut ParticleEmitter>,
    barrels: Query<&Barrel>,
) {
    for Burst(root) in bursts.read() {
        let all: Vec<Entity> = children.iter_descendants(*root).collect();
        let mut in_flash: Vec<Entity> = vec![];
        let barrel = barrels.get(*root).ok().map(|b| b.0);
        let flash_list: Vec<Entity> = all.iter().copied().filter(|e| flashes.contains(*e)).collect();
        for &f in &all {
            if let Some(k) = barrel
                && flash_list.len() >= 2
                && flash_list.get(k % flash_list.len()) != Some(&f)
            {
                continue;
            }
            let Ok(mut ff) = flashes.get_mut(f) else { continue };
            // gunFlashObj, or (when empty) the FlashFire's own object
            let target = ff.target.or_else(|| {
                ff.object.as_ref().and_then(|n| all.iter().copied().find(|d| names.get(*d).is_ok_and(|x| x.as_str() == n))).or(Some(f))
            });
            ff.target = target;
            let Some(t) = target else { continue };
            let starting = ff.left <= 0.0;
            ff.left = ff.time;
            let under: Vec<Entity> = std::iter::once(t).chain(children.iter_descendants(t)).collect();
            for e in &under {
                if let Ok(mut em) = emitters.get_mut(*e) {
                    em.flash = Some(f);
                    // switching the object on restarts its systems; while on they keep looping
                    if starting || em.clock.is_none() {
                        let looping = em.s.looping && em.s.v >= 2;
                        em.play(looping);
                    }
                }
            }
            in_flash.extend(under);
        }
        for e in all {
            if in_flash.contains(&e) {
                continue;
            }
            if let Ok(mut em) = emitters.get_mut(e) {
                em.play(false);
            }
        }
    }
}

/// FlashFire.Update: when its time is up the flash object goes off (its particles with it).
fn flash_timers(
    mut commands: Commands,
    time: Res<Time>,
    mut flashes: Query<(Entity, &mut FlashFire)>,
    mut emitters: Query<&mut ParticleEmitter>,
    particles: Query<(Entity, &Particle)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (f, mut ff) in &mut flashes {
        if ff.left <= 0.0 {
            continue;
        }
        ff.left -= time.delta_secs();
        if ff.left > 0.0 {
            continue;
        }
        for mut em in &mut emitters {
            if em.flash == Some(f) {
                em.clock = None;
            }
        }
        for (e, p) in &particles {
            if p.flash == Some(f) {
                materials.remove(&p.material);
                commands.entity(e).despawn();
            }
        }
    }
}

/// Emitters that are playing: bursts at their times, the rate over the cycle.
#[allow(clippy::too_many_arguments)]
fn emit(
    mut commands: Commands,
    time: Res<Time>,
    mut emitters: Query<(Entity, &mut ParticleEmitter, &GlobalTransform)>,
    parents: Query<&ChildOf>,
    layers: Query<&RenderLayers>,
    alive: Query<(), With<Particle>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    quad: Option<Res<Quad>>,
) {
    let quad = match quad {
        Some(q) => q.0.clone(),
        None => {
            let h = meshes.add(Rectangle::new(1.0, 1.0));
            commands.insert_resource(Quad(h.clone()));
            h
        }
    };
    let mut count = alive.iter().count();
    let dt = time.delta_secs();
    for (e, mut em, gt) in &mut emitters {
        let Some(clock) = em.clock else { continue };
        let step = dt * em.s.sim_speed();
        let now = clock + step;
        let mut n = 0u32;
        if em.s.v < 2 {
            // older exports: one particle per shot
            n = 1;
            em.clock = None;
        } else {
            let bursts = em.s.bursts.clone().unwrap_or_default();
            for (i, b) in bursts.iter().enumerate() {
                let done = em.fired.get(i).copied().unwrap_or(0);
                let next = b.time + done as f32 * b.interval.max(0.01);
                if done < b.cycles.max(1) && next <= now && next >= clock - 1e-4 {
                    let k = em.range([b.min, b.max]).round().max(0.0) as u32;
                    n += k;
                    if let Some(x) = em.fired.get_mut(i) {
                        *x += 1;
                    }
                }
            }
            em.rate_acc += em.s.rate * step;
            let whole = em.rate_acc.floor();
            n += whole as u32;
            em.rate_acc -= whole;
            if now >= em.s.duration {
                if em.looping {
                    em.play(true);
                    em.clock = Some(now - em.s.duration);
                } else {
                    em.clock = None;
                }
            } else {
                em.clock = Some(now);
            }
        }
        if n == 0 {
            continue;
        }
        // draw on the same render layer as the prefab (the view model has its own camera)
        let layer = std::iter::once(e).chain(parents.iter_ancestors(e)).find_map(|a| layers.get(a).ok()).cloned().unwrap_or_default();
        let (_, rot, _) = gt.to_scale_rotation_translation();
        if std::env::var("PG3D_TRACE_FX").is_ok() {
            info!("fx emit {n} at {:.2} layer {:?} flash={:?} blend={:?}", gt.translation(), layer, em.flash, em.s.render.as_ref().map(|r| r.blend.clone()));
        }
        for _ in 0..n {
            if count >= MAX_PARTICLES {
                break;
            }
            count += 1;
            let p = new_particle(&mut em, e, gt, rot);
            let blend = em.s.render.as_ref().map_or("add", |r| r.blend.as_str());
            let material = materials.add(StandardMaterial {
                base_color: Color::NONE,
                base_color_texture: Some(em.texture.clone()),
                unlit: true,
                alpha_mode: match blend {
                    "alpha" => AlphaMode::Blend,
                    "mul" => AlphaMode::Multiply,
                    _ => AlphaMode::Add,
                },
                cull_mode: None,
                fog_enabled: false,
                ..default()
            });
            let world = p.local_to.map_or(p.pos, |_| gt.transform_point(p.pos));
            commands.spawn((
                Particle { material: material.clone(), ..p },
                Mesh3d(quad.clone()),
                MeshMaterial3d(material),
                Transform::from_translation(world).with_scale(Vec3::ZERO),
                layer.clone(),
            ));
        }
    }
}

impl ParticleSettings {
    fn sim_speed(&self) -> f32 {
        1.0
    }
}

/// A new particle from the emitter's shape and start values. Unity's x axis is mirrored in
/// the export, so local x components are negated.
fn new_particle(em: &mut ParticleEmitter, e: Entity, gt: &GlobalTransform, rot: Quat) -> Particle {
    let s = em.s.clone();
    let life = em.range(s.life_range.unwrap_or([s.life, s.life])).max(0.01);
    let speed = em.range(s.speed_range);
    let size = em.range(s.size_range.unwrap_or([s.size, s.size]));
    let k = em.rand();
    let color = s.color_range.map_or(Vec4::from(s.color), |[a, b]| Vec4::from(a).lerp(Vec4::from(b), k));
    let (mut pos, dir) = match s.shape.as_ref() {
        None => (Vec3::ZERO, Vec3::Z),
        Some(sh) => {
            let a = em.rand() * std::f32::consts::TAU;
            let r = sh.radius * em.rand().sqrt();
            match sh.kind {
                // sphere / hemisphere (shells too): outwards
                0..=3 => {
                    let z = em.rand() * 2.0 - 1.0;
                    let z = if sh.kind >= 2 { z.abs() } else { z };
                    let t = em.rand() * std::f32::consts::TAU;
                    let d = Vec3::new((1.0 - z * z).sqrt() * t.cos(), (1.0 - z * z).sqrt() * t.sin(), z);
                    (d * if sh.kind % 2 == 1 { sh.radius } else { r }, d)
                }
                // cones: from a disc, tilted out by the cone angle at its rim
                4 | 7 | 8 | 9 => {
                    let p = Vec3::new(r * a.cos(), r * a.sin(), 0.0);
                    let tilt = sh.angle.to_radians().tan() * if sh.radius > 0.0 { r / sh.radius } else { 1.0 };
                    (p, (Vec3::Z + Vec3::new(a.cos(), a.sin(), 0.0) * tilt).normalize())
                }
                // circle: in the XY plane, outwards
                10 | 11 => {
                    let d = Vec3::new(a.cos(), a.sin(), 0.0);
                    (d * if sh.kind == 11 { sh.radius } else { r }, d)
                }
                // edge: along x, upwards
                12 => (Vec3::new((em.rand() * 2.0 - 1.0) * sh.radius, 0.0, 0.0), Vec3::Y),
                // box and the rest: forwards
                _ => (Vec3::ZERO, Vec3::Z),
            }
        }
    };
    pos.x = -pos.x;
    let dir = Vec3::new(-dir.x, dir.y, dir.z);
    let mut vel = dir * speed;
    let (extra, extra_world) = match s.velocity.clone() {
        Some(v) => {
            let x = em.range(v.x);
            let y = em.range(v.y);
            let z = em.range(v.z);
            (if v.world { Vec3::new(x, y, z) } else { Vec3::new(-x, y, z) }, v.world)
        }
        None => (Vec3::ZERO, false),
    };
    let local_to = (!s.world).then_some(e);
    if s.world {
        pos = gt.transform_point(pos);
        vel = rot * vel;
    }
    let rot0 = em.range(s.rotation_range);
    let spin = em.range(s.spin_range);
    let start_frame = em.range(s.start_frame);
    Particle {
        age: 0.0,
        life,
        material: Handle::default(),
        local_to,
        pos,
        vel,
        extra,
        extra_world,
        size,
        rot: rot0,
        spin,
        color,
        start_frame,
        flash: em.flash,
        s,
    }
}

/// Cameras particles turn to face: the match camera, or the Armory preview's.
type FacingCamera = Or<(With<crate::camera::PlayerCamera>, With<crate::armory_preview::PreviewCamera>)>;

#[allow(clippy::type_complexity)]
fn update_particles(
    mut commands: Commands,
    time: Res<Time>,
    cameras: Query<&GlobalTransform, FacingCamera>,
    globals: Query<&GlobalTransform, Without<Particle>>,
    mut q: Query<(Entity, &mut Particle, &mut Transform)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // the match camera, or the Armory preview's in the menu
    let Some(camera) = cameras.iter().next() else { return };
    let cam = camera.compute_transform();
    let dt = time.delta_secs();
    for (e, mut p, mut t) in &mut q {
        // PG3D_FX_HOLD=k (testing): particles stop at that point of their life
        let hold = std::env::var("PG3D_FX_HOLD").ok().and_then(|v| v.parse::<f32>().ok());
        p.age += if hold.is_some() { 0.0 } else { dt };
        let k = hold.unwrap_or(p.age / p.life);
        let emitter = p.local_to.and_then(|x| globals.get(x).ok());
        if k >= 1.0 || (p.local_to.is_some() && emitter.is_none()) {
            materials.remove(&p.material);
            commands.entity(e).despawn();
            continue;
        }
        // move (gravity pulls down in the world)
        let g = Vec3::NEG_Y * 9.81 * p.s.gravity * dt;
        match emitter {
            Some(em) => {
                let inv = em.to_scale_rotation_translation().1.inverse();
                p.vel += inv * g;
            }
            None => p.vel += g,
        }
        let mut step = p.vel;
        if !p.extra_world || emitter.is_none() {
            step += p.extra;
        }
        p.pos += step * dt;
        if p.extra_world && let Some(em) = emitter {
            let inv = em.to_scale_rotation_translation().1.inverse();
            let d = inv * p.extra * dt;
            p.pos += d;
        }
        p.rot += p.spin * dt;
        let (world, world_vel) = match emitter {
            Some(em) => (em.transform_point(p.pos), em.to_scale_rotation_translation().1 * (p.vel + p.extra)),
            None => (p.pos, p.vel + p.extra),
        };
        let size = p.size * p.s.size_curve.as_deref().map_or(1.0, |c| curve(c, k));
        let render = p.s.render.clone();
        let mode = render.as_ref().map_or(0, |r| r.mode);
        t.translation = world;
        if mode == 1 && world_vel.length() > 1e-3 {
            // stretched along the movement, turned to face the camera
            let r = render.unwrap();
            let axis = world_vel.normalize();
            let to_cam = (cam.translation - world).normalize_or_zero();
            let normal = (to_cam - axis * to_cam.dot(axis)).normalize_or_zero();
            let up = normal.cross(axis).normalize_or_zero();
            if normal != Vec3::ZERO {
                t.rotation = Quat::from_mat3(&Mat3::from_cols(axis, up, normal));
            }
            let len = size * r.length_scale + world_vel.length() * r.velocity_scale;
            t.scale = Vec3::new(len, size, 1.0);
        } else {
            t.rotation = cam.rotation * Quat::from_rotation_z(-p.rot);
            t.scale = Vec3::splat(size);
        }
        if std::env::var("PG3D_TRACE_FX").is_ok() {
            info!("fx particle k={k:.2} at {:.2} scale {:.3} cam {:.2} col {:.2}", t.translation, t.scale, cam.translation, p.color * p.s.color_curve.as_deref().map_or(Vec4::ONE, |cc| curve4(cc, k)));
        }
        // colour over lifetime (older exports: fade out)
        let mut c = p.color * p.s.color_curve.as_deref().map_or(Vec4::ONE, |cc| curve4(cc, k));
        if p.s.v < 2 {
            c.w *= 1.0 - k;
        }
        // the texture-sheet frame for this point in the particle's life
        let [tx, ty] = p.s.tiles;
        let count = if p.s.single_row { tx } else { tx * ty }.max(1);
        let norm = match p.s.frames.as_deref() {
            Some(fr) => curve(fr, k),
            None => k,
        };
        let f = (((norm * p.s.uv_cycles) * count as f32 + p.start_frame) as u32) % count;
        let (col, row) = if p.s.single_row { (f, p.s.row) } else { (f % tx, f / tx) };
        if let Some(mut m) = materials.get_mut(&p.material) {
            let size = Vec2::new(1.0 / tx as f32, 1.0 / ty as f32);
            m.uv_transform = Affine2::from_scale_angle_translation(size, 0.0, Vec2::new(col as f32, row as f32) * size);
            m.base_color = Color::srgba(c.x, c.y, c.z, c.w.clamp(0.0, 1.0));
        }
    }
}

/// PG3D_TRACE_FX=1: how many particles exist and are drawn, once a second.
#[allow(clippy::type_complexity)]
fn trace_vis(time: Res<Time>, q: Query<(&ViewVisibility, &InheritedVisibility, &GlobalTransform, &MeshMaterial3d<StandardMaterial>), With<Particle>>, mats: Res<Assets<StandardMaterial>>) {
    if std::env::var("PG3D_TRACE_FX").is_err() || (time.elapsed_secs() % 1.0) >= time.delta_secs() {
        return;
    }
    let n = q.iter().count();
    let drawn = q.iter().filter(|x| x.0.get()).count();
    let inh = q.iter().filter(|x| x.1.get()).count();
    let s = q.iter().next().map(|x| format!("{:.2} {:?}", x.2.compute_transform().scale, mats.get(&x.3.0).map(|m| (m.base_color, m.alpha_mode))));
    info!("fx vis: {n} particles, {inh} inherited, {drawn} drawn; {s:?}");
}
