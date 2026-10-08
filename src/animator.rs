//! A small Mecanim runtime: plays an AnimatorController exported by export_player_anims.py
//! ("_controller" in anims.json: the controller's compiled state machines) the way Unity does —
//! parameters, any-state and state transitions with conditions, exit times and cross-fades,
//! 1D and 2D (freeform directional / cartesian, gradient band interpolation) blend trees,
//! state speed multipliers, and additive layers (relative to the clip's first frame).
//!
//! Bones a clip doesn't animate keep their rest pose (Write Defaults), as in the original.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use serde::Deserialize;

/// One exported clip: per bone path, samples at `fps` (glTF axes).
#[derive(Deserialize, Clone, Default)]
pub struct Clip {
    pub length: f32,
    pub fps: f32,
    pub tracks: Vec<Track>,
}

#[derive(Deserialize, Clone, Default)]
pub struct Track {
    pub path: String,
    #[serde(default)]
    pub rotation: Vec<[f32; 4]>,
    #[serde(default)]
    pub translation: Vec<[f32; 3]>,
}

impl Clip {
    /// Rotation and position of track `k` at `t` seconds (already wrapped / clamped).
    fn sample(&self, k: usize, t: f32) -> (Option<Quat>, Option<Vec3>) {
        let tr = &self.tracks[k];
        let f = (t * self.fps.max(1.0)).max(0.0);
        let (i, a) = (f.floor() as usize, f.fract());
        let pick = |n: usize| (i.min(n.saturating_sub(1)), (i + 1).min(n.saturating_sub(1)));
        let rot = (!tr.rotation.is_empty()).then(|| {
            let (i0, i1) = pick(tr.rotation.len());
            Quat::from_array(tr.rotation[i0]).slerp(Quat::from_array(tr.rotation[i1]), a)
        });
        let pos = (!tr.translation.is_empty()).then(|| {
            let (i0, i1) = pick(tr.translation.len());
            Vec3::from(tr.translation[i0]).lerp(Vec3::from(tr.translation[i1]), a)
        });
        (rot, pos)
    }
}

#[derive(Deserialize, Clone)]
pub struct Condition {
    pub param: Option<String>,
    pub mode: Option<String>,
    pub threshold: f32,
}

#[derive(Deserialize, Clone)]
pub struct Transition {
    /// None: exit to the machine's default state
    pub to: Option<String>,
    pub duration: f32,
    pub offset: f32,
    pub exit_time: f32,
    pub has_exit_time: bool,
    #[serde(default = "yes")]
    pub can_self: bool,
    pub conditions: Vec<Condition>,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize, Clone, Default)]
pub struct Node {
    #[serde(default)]
    pub clip: Option<String>,
    #[serde(default)]
    pub blend: Option<String>,
    #[serde(default)]
    pub x: Option<String>,
    #[serde(default)]
    pub y: Option<String>,
    #[serde(default)]
    pub children: Vec<usize>,
    #[serde(default)]
    pub thresholds: Vec<f32>,
    #[serde(default)]
    pub positions: Vec<[f32; 2]>,
}

#[derive(Deserialize, Clone)]
pub struct State {
    pub name: String,
    pub speed: f32,
    pub speed_param: Option<String>,
    pub r#loop: bool,
    pub tree: Vec<Node>,
    pub transitions: Vec<Transition>,
}

#[derive(Deserialize, Clone)]
pub struct Machine {
    pub default: String,
    pub any_state: Vec<Transition>,
    pub states: Vec<State>,
}

#[derive(Deserialize, Clone)]
pub struct Layer {
    pub state_machine: usize,
    pub additive: bool,
    pub weight: f32,
}

#[derive(Deserialize, Clone)]
pub struct ControllerDef {
    pub layers: Vec<Layer>,
    pub state_machines: Vec<Machine>,
}

/// A loaded controller with its clips.
pub struct Controller {
    pub def: ControllerDef,
    pub clips: HashMap<String, Clip>,
}

impl Controller {
    /// `anims.json`: clip name -> Clip, plus "_controller".
    pub fn load(path: &std::path::Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        let map: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&text).ok()?;
        let def: ControllerDef = serde_json::from_value(map.get("_controller")?.clone()).ok()?;
        let clips = map
            .into_iter()
            .filter(|(k, _)| !k.starts_with('_'))
            .filter_map(|(k, v)| serde_json::from_value(v).ok().map(|c| (k, c)))
            .collect();
        Some(Self { def, clips })
    }

    fn state_index(&self, machine: usize, name: &str) -> Option<usize> {
        self.def.state_machines[machine].states.iter().position(|s| s.name == name)
    }

    /// (clip, weight) of a state's blend tree for the current parameters.
    fn weights(&self, state: &State, params: &HashMap<String, f32>) -> Vec<(String, f32)> {
        let mut out = vec![];
        if !state.tree.is_empty() {
            self.node_weights(&state.tree, 0, 1.0, params, &mut out);
        }
        out
    }

    fn node_weights(&self, tree: &[Node], i: usize, w: f32, params: &HashMap<String, f32>, out: &mut Vec<(String, f32)>) {
        let n = &tree[i];
        if n.children.is_empty() {
            if let Some(c) = &n.clip
                && w > 1e-4
            {
                out.push((c.clone(), w));
            }
            return;
        }
        let p = |name: &Option<String>| name.as_ref().and_then(|k| params.get(k)).copied().unwrap_or(0.0);
        let ws: Vec<f32> = match n.blend.as_deref() {
            Some("1d") => blend_1d(&n.thresholds, p(&n.x)),
            Some("freeform_cartesian") => gradient_bands(&n.positions, Vec2::new(p(&n.x), p(&n.y)), false),
            Some(_) => gradient_bands(&n.positions, Vec2::new(p(&n.x), p(&n.y)), true),
            None => vec![1.0; n.children.len()],
        };
        for (k, c) in n.children.iter().enumerate() {
            let cw = ws.get(k).copied().unwrap_or(0.0) * w;
            if cw > 1e-4 {
                self.node_weights(tree, *c, cw, params, out);
            }
        }
    }

    /// Seconds one cycle of the state lasts (weighted clip lengths).
    fn state_length(&self, weights: &[(String, f32)]) -> f32 {
        let total: f32 = weights.iter().map(|(_, w)| w).sum();
        if total <= 0.0 {
            return 1.0;
        }
        (weights.iter().map(|(c, w)| self.clips.get(c).map_or(1.0, |c| c.length) * w).sum::<f32>() / total).max(1e-3)
    }
}

/// Linear blend between the two thresholds around `v`.
fn blend_1d(th: &[f32], v: f32) -> Vec<f32> {
    let mut w = vec![0.0; th.len()];
    if th.is_empty() {
        return w;
    }
    if v <= th[0] {
        w[0] = 1.0;
        return w;
    }
    for i in 0..th.len() - 1 {
        if v <= th[i + 1] {
            let a = ((v - th[i]) / (th[i + 1] - th[i]).max(1e-6)).clamp(0.0, 1.0);
            w[i] = 1.0 - a;
            w[i + 1] = a;
            return w;
        }
    }
    *w.last_mut().unwrap() = 1.0;
    w
}

/// Gradient band interpolation (Johansen; Unity's freeform 2D blend trees), in polar space
/// for the directional kind.
fn gradient_bands(points: &[[f32; 2]], p: Vec2, polar: bool) -> Vec<f32> {
    let pts: Vec<Vec2> = points.iter().map(|q| Vec2::from(*q)).collect();
    let n = pts.len();
    let ang = |a: Vec2, b: Vec2| if a.length() < 1e-5 || b.length() < 1e-5 { 0.0 } else { a.angle_to(b) };
    let mut w = vec![0.0f32; n];
    for i in 0..n {
        let mut wi = f32::MAX;
        for j in 0..n {
            if i == j {
                continue;
            }
            let (pi, pj) = (pts[i], pts[j]);
            let (vij, vip) = if polar {
                let (li, lj, lp) = (pi.length(), pj.length(), p.length());
                let avg = ((li + lj) * 0.5).max(1e-5);
                let (aij, aip) = if li < 1e-5 {
                    (0.0, ang(pj, p))
                } else if lj < 1e-5 {
                    (0.0, ang(pi, p))
                } else {
                    (ang(pi, pj), ang(pi, p))
                };
                const ANGLE_WEIGHT: f32 = 2.0;
                (Vec2::new((lj - li) / avg, aij * ANGLE_WEIGHT), Vec2::new((lp - li) / avg, aip * ANGLE_WEIGHT))
            } else {
                (pj - pi, p - pi)
            };
            let d = vij.length_squared();
            if d < 1e-9 {
                continue;
            }
            wi = wi.min((1.0 - vij.dot(vip) / d).max(0.0));
        }
        w[i] = if wi == f32::MAX { 1.0 } else { wi };
    }
    let sum: f32 = w.iter().sum();
    if sum > 1e-6 {
        w.iter_mut().for_each(|x| *x /= sum);
    } else if n > 0 {
        // outside every band: the nearest point
        let k = (0..n).min_by(|a, b| pts[*a].distance(p).total_cmp(&pts[*b].distance(p))).unwrap();
        w[k] = 1.0;
    }
    w
}

#[derive(Clone, Copy, Default)]
struct Playing {
    state: usize,
    /// normalized time (cycles)
    time: f32,
}

#[derive(Clone, Default)]
struct LayerRun {
    cur: Playing,
    prev: Option<Playing>,
    fade: f32,
    fade_len: f32,
    started: bool,
}

/// One character's Animator: parameters and where each layer is.
#[derive(Component, Default)]
pub struct Animator {
    pub params: HashMap<String, f32>,
    triggers: HashSet<String>,
    layers: Vec<LayerRun>,
    /// bone path (below the animated root) -> entity, and its rest pose
    pub bones: HashMap<String, Entity>,
    pub rest: HashMap<Entity, (Quat, Vec3)>,
}

impl Animator {
    pub fn set(&mut self, name: &str, v: f32) {
        self.params.insert(name.to_string(), v);
    }
    pub fn set_bool(&mut self, name: &str, v: bool) {
        self.set(name, if v { 1.0 } else { 0.0 });
    }
    pub fn trigger(&mut self, name: &str) {
        self.triggers.insert(name.to_string());
    }
    pub fn state_name<'a>(&self, c: &'a Controller, layer: usize) -> Option<&'a str> {
        let l = self.layers.get(layer)?;
        let m = &c.def.state_machines[c.def.layers[layer].state_machine];
        Some(m.states[l.cur.state].name.as_str())
    }

    fn holds(&self, cond: &[Condition]) -> bool {
        cond.iter().all(|c| {
            let Some(p) = &c.param else { return true };
            let v = self.params.get(p).copied().unwrap_or(0.0);
            match c.mode.as_deref() {
                Some("if") => self.triggers.contains(p) || v != 0.0,
                Some("if_not") => v == 0.0 && !self.triggers.contains(p),
                Some("greater") => v > c.threshold,
                Some("less") => v < c.threshold,
                Some("equals") => (v - c.threshold).abs() < 1e-4,
                Some("not_equal") => (v - c.threshold).abs() >= 1e-4,
                _ => true,
            }
        })
    }

    /// Advance every layer by `dt` seconds.
    pub fn update(&mut self, c: &Controller, dt: f32) {
        if self.layers.len() != c.def.layers.len() {
            self.layers = vec![LayerRun::default(); c.def.layers.len()];
        }
        for li in 0..c.def.layers.len() {
            let mi = c.def.layers[li].state_machine;
            let m = &c.def.state_machines[mi];
            let mut run = self.layers[li].clone();
            if !run.started {
                run.cur = Playing { state: c.state_index(mi, &m.default).unwrap_or(0), time: 0.0 };
                run.started = true;
            }
            // transitions: any state first, then the current state's (not while cross-fading)
            let cur = &m.states[run.cur.state];
            let mut take: Option<&Transition> = None;
            for t in &m.any_state {
                let to = t.to.as_deref().and_then(|n| c.state_index(mi, n));
                if (t.can_self || to != Some(run.cur.state)) && self.holds(&t.conditions) {
                    take = Some(t);
                    break;
                }
            }
            if take.is_none() && run.prev.is_none() {
                let len = c.state_length(&c.weights(cur, &self.params));
                let step = dt * self.speed(cur) / len;
                for t in &cur.transitions {
                    let exit_ok = !t.has_exit_time
                        || if cur.r#loop {
                            ((run.cur.time - t.exit_time).floor() < (run.cur.time + step - t.exit_time).floor())
                                || (t.exit_time > 1.0 && run.cur.time >= t.exit_time)
                        } else {
                            run.cur.time >= t.exit_time
                        };
                    if exit_ok && self.holds(&t.conditions) && (t.has_exit_time || !t.conditions.is_empty()) {
                        take = Some(t);
                        break;
                    }
                }
            }
            if let Some(t) = take {
                for cond in &t.conditions {
                    if let Some(p) = &cond.param {
                        self.triggers.remove(p);
                    }
                }
                let to = t.to.as_deref().and_then(|n| c.state_index(mi, n)).unwrap_or_else(|| c.state_index(mi, &m.default).unwrap_or(0));
                run.prev = Some(run.cur);
                run.cur = Playing { state: to, time: t.offset };
                run.fade = 0.0;
                run.fade_len = t.duration;
            }
            // time
            for p in std::iter::once(&mut run.cur).chain(run.prev.iter_mut()) {
                let s = &m.states[p.state];
                let len = c.state_length(&c.weights(s, &self.params));
                p.time += dt * self.speed(s) / len;
            }
            run.fade += dt;
            if run.prev.is_some() && run.fade >= run.fade_len {
                run.prev = None;
            }
            self.layers[li] = run;
        }
    }

    fn speed(&self, s: &State) -> f32 {
        s.speed * s.speed_param.as_ref().map_or(1.0, |p| self.params.get(p).copied().unwrap_or(0.0))
    }

    /// The pose of a state at its normalized time: bone -> (rotation, position, weight).
    fn state_pose(&self, c: &Controller, s: &State, time: f32, additive: bool, out: &mut HashMap<Entity, (Quat, Vec3, f32)>, layer_w: f32) {
        for (name, w) in c.weights(s, &self.params) {
            let Some(clip) = c.clips.get(&name) else { continue };
            let t = if s.r#loop { time.rem_euclid(1.0) * clip.length } else { time.clamp(0.0, 1.0) * clip.length };
            for (k, tr) in clip.tracks.iter().enumerate() {
                let Some(&e) = self.bones.get(&tr.path) else { continue };
                let rest = self.rest.get(&e).copied().unwrap_or((Quat::IDENTITY, Vec3::ZERO));
                let (r, p) = clip.sample(k, t);
                // additive: a running product from identity; otherwise a weighted sum (from zero)
                let entry = out.entry(e).or_insert((if additive { Quat::IDENTITY } else { Quat::from_xyzw(0.0, 0.0, 0.0, 0.0) }, Vec3::ZERO, 0.0));
                let ww = w * layer_w;
                if additive {
                    let (r0, p0) = clip.sample(k, 0.0);
                    let dr = match (r, r0) {
                        (Some(r), Some(r0)) => r0.inverse() * r,
                        _ => Quat::IDENTITY,
                    };
                    let dp = match (p, p0) {
                        (Some(p), Some(p0)) => p - p0,
                        _ => Vec3::ZERO,
                    };
                    entry.0 = Quat::IDENTITY.slerp(dr, ww) * entry.0;
                    entry.1 += dp * ww;
                } else {
                    // same hemisphere as the rest pose, so opposite-signed samples don't cancel
                    let r = r.unwrap_or(rest.0);
                    let r = if rest.0.dot(r) < 0.0 { -r } else { r };
                    entry.0 = Quat::from_vec4(Vec4::from(entry.0) + Vec4::from(r) * ww);
                    entry.1 += p.unwrap_or(rest.1) * ww;
                    entry.2 += ww;
                }
            }
        }
    }

    /// The final local pose of every animated bone.
    pub fn pose(&self, c: &Controller) -> HashMap<Entity, (Quat, Vec3)> {
        let mut base: HashMap<Entity, (Quat, Vec3)> = self.rest.iter().map(|(e, r)| (*e, *r)).collect();
        for (li, run) in self.layers.iter().enumerate() {
            let layer = &c.def.layers[li];
            let m = &c.def.state_machines[layer.state_machine];
            let a = if run.prev.is_some() { (run.fade / run.fade_len.max(1e-4)).clamp(0.0, 1.0) } else { 1.0 };
            let mut acc = HashMap::new();
            if let Some(p) = run.prev {
                self.state_pose(c, &m.states[p.state], p.time, layer.additive, &mut acc, 1.0 - a);
            }
            self.state_pose(c, &m.states[run.cur.state], run.cur.time, layer.additive, &mut acc, a);
            if li == 0 || !layer.additive {
                for (e, (q, p, w)) in acc {
                    if w <= 1e-5 {
                        continue;
                    }
                    let rest = base.get(&e).copied().unwrap_or((Quat::IDENTITY, Vec3::ZERO));
                    // weight short of 1 (bones only some clips animate): the rest pose fills in
                    let v = Vec4::from(q) + Vec4::from(rest.0) * (1.0 - w).max(0.0);
                    let q = if v.length() > 1e-4 && v.is_finite() { Quat::from_vec4(v.normalize()) } else { rest.0 };
                    let p = p + rest.1 * (1.0 - w).max(0.0);
                    base.insert(e, (q, if p.is_finite() { p } else { rest.1 }));
                }
            } else {
                let lw = if li == 0 { 1.0 } else { layer.weight };
                for (e, (dq, dp, _)) in acc {
                    if let Some(b) = base.get_mut(&e)
                        && dq.is_finite()
                    {
                        b.0 = (b.0 * Quat::IDENTITY.slerp(dq, lw)).normalize();
                        b.1 += dp * lw;
                    }
                }
            }
        }
        base
    }

    /// Testing: the pose of one clip alone at `t` seconds (rest pose for bones it doesn't key).
    pub fn clip_pose(&self, c: &Controller, name: &str, t: f32) -> HashMap<Entity, (Quat, Vec3)> {
        let mut base: HashMap<Entity, (Quat, Vec3)> = self.rest.iter().map(|(e, r)| (*e, *r)).collect();
        if let Some(clip) = c.clips.get(name) {
            let t = if clip.length > 0.0 { t.rem_euclid(clip.length) } else { 0.0 };
            for (k, tr) in clip.tracks.iter().enumerate() {
                let Some(&e) = self.bones.get(&tr.path) else { continue };
                let (r, p) = clip.sample(k, t);
                let b = base.entry(e).or_insert((Quat::IDENTITY, Vec3::ZERO));
                if let Some(r) = r {
                    b.0 = r;
                }
                if let Some(p) = p {
                    b.1 = p;
                }
            }
        }
        base
    }

    /// Write the pose to the bones' transforms.
    pub fn apply(&self, c: &Controller, tf: &mut Query<&mut Transform, Without<crate::tdm_bots::PlayerBot>>) {
        for (e, (r, p)) in self.pose(c) {
            if let Ok(mut t) = tf.get_mut(e) {
                t.rotation = r;
                t.translation = p;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directional_blend_hits_points() {
        let pts = [[0.0, 1.0], [0.0, 0.6], [0.6, 0.0], [-0.6, 0.0], [0.0, -0.6], [0.0, 0.0]];
        for (k, p) in pts.iter().enumerate() {
            let w = gradient_bands(&pts, Vec2::from(*p), true);
            assert!(w[k] > 0.99, "{k}: {w:?}");
        }
        let w = gradient_bands(&pts, Vec2::new(0.3, 0.0), true);
        assert!((w[2] - 0.5).abs() < 0.1 && (w[5] - 0.5).abs() < 0.1, "{w:?}");
    }

    #[test]
    fn one_d() {
        assert_eq!(blend_1d(&[-1.0, 0.0, 1.0], 0.5), vec![0.0, 0.5, 0.5]);
    }
}
