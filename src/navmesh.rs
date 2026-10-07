//! Path finding on the map's original navigation mesh (exported by
//! ../pixel-gun-3d-re/scripts/export_navmesh.py into maps/<map>/navmesh.json).
//!
//! A navigation mesh is a set of convex polygons covering everywhere an enemy can stand.
//! To get from A to B we (1) find the polygons A and B are on, (2) search the polygon
//! graph with A* for a chain of neighbouring polygons, (3) turn that chain into a few
//! straight-line waypoints ("string pulling").

use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap},
};

use bevy::prelude::*;
use serde::Deserialize;

use crate::{DataDir, map::CurrentMap};

pub struct NavMeshPlugin;

impl Plugin for NavMeshPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, load_navmesh);
    }
}

#[derive(Deserialize)]
struct NavFile {
    verts: Vec<[f32; 3]>,
    polys: Vec<Vec<usize>>,
    /// per polygon: [neighbour, portal start, portal end]
    links: Vec<Vec<(usize, [f32; 3], [f32; 3])>>,
}

struct Link {
    to: usize,
    a: Vec3,
    b: Vec3,
}

struct Poly {
    verts: Vec<Vec3>,
    center: Vec3,
    links: Vec<Link>,
}

#[derive(Resource, Default)]
pub struct NavMesh {
    polys: Vec<Poly>,
}

fn load_navmesh(mut commands: Commands, data: Res<DataDir>, map: Res<CurrentMap>) {
    let path = data.0.join("maps").join(&map.0).join("navmesh.json");
    let file: NavFile = match std::fs::read_to_string(&path).map(|s| serde_json::from_str(&s)) {
        Ok(Ok(f)) => f,
        other => {
            warn!("{}: {:?} (no path finding; run export_navmesh.py)", path.display(), other.err());
            commands.insert_resource(NavMesh::default());
            return;
        }
    };
    let v = |i: usize| Vec3::from(file.verts[i]);
    let polys: Vec<Poly> = file
        .polys
        .iter()
        .zip(file.links)
        .map(|(ids, links)| {
            let verts: Vec<Vec3> = ids.iter().map(|&i| v(i)).collect();
            let center = verts.iter().copied().sum::<Vec3>() / verts.len() as f32;
            let links = links.into_iter().map(|(to, a, b)| Link { to, a: a.into(), b: b.into() }).collect();
            Poly { verts, center, links }
        })
        .collect();
    info!("navmesh: {} polygons", polys.len());
    commands.insert_resource(NavMesh { polys });
}

fn cross2(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

fn xz(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.z)
}

/// Do 2D segments p0-p1 and q0-q1 (seen from above) intersect?
fn segments_cross(p0: Vec2, p1: Vec2, q0: Vec2, q1: Vec2) -> bool {
    let r = p1 - p0;
    let s = q1 - q0;
    let d = cross2(r, s);
    if d.abs() < 1e-9 {
        return false;
    }
    let t = cross2(q0 - p0, s) / d;
    let u = cross2(q0 - p0, r) / d;
    (-1e-4..=1.0 + 1e-4).contains(&t) && (-1e-4..=1.0 + 1e-4).contains(&u)
}

#[derive(PartialEq)]
struct Open(f32, usize);
impl Eq for Open {}
impl Ord for Open {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.total_cmp(&self.0) // min-heap
    }
}
impl PartialOrd for Open {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl NavMesh {
    pub fn is_empty(&self) -> bool {
        self.polys.is_empty()
    }

    /// Polygon under `p`: contains it when seen from above and is the closest in height.
    pub fn find_poly(&self, p: Vec3) -> Option<usize> {
        let q = xz(p);
        let mut best: Option<(usize, f32)> = None;
        for (i, poly) in self.polys.iter().enumerate() {
            let n = poly.verts.len();
            // convex polygon: point is inside if it is on the same side of every edge
            let mut sign = 0.0f32;
            let mut inside = true;
            for k in 0..n {
                let a = xz(poly.verts[k]);
                let b = xz(poly.verts[(k + 1) % n]);
                let c = cross2(b - a, q - a);
                if c.abs() < 1e-6 {
                    continue;
                }
                if sign == 0.0 {
                    sign = c.signum();
                } else if c.signum() != sign {
                    inside = false;
                    break;
                }
            }
            if !inside {
                continue;
            }
            let dy = (poly.center.y - p.y).abs();
            if dy < 3.0 && best.is_none_or(|(_, d)| dy < d) {
                best = Some((i, dy));
            }
        }
        best.map(|(i, _)| i).or_else(|| {
            // off the mesh: nearest polygon centre
            self.polys
                .iter()
                .enumerate()
                .map(|(i, poly)| (i, poly.center.distance_squared(p)))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .filter(|(_, d)| *d < 9.0)
                .map(|(i, _)| i)
        })
    }

    /// Waypoints from `from` to `to` (excluding `from`, including `to`), or None if there is
    /// no connection.
    pub fn find_path(&self, from: Vec3, to: Vec3) -> Option<Vec<Vec3>> {
        let start = self.find_poly(from)?;
        let goal = self.find_poly(to)?;
        if start == goal {
            return Some(vec![to]);
        }
        // A* over polygons, moving between portal midpoints
        let mut came: HashMap<usize, (usize, usize)> = HashMap::new(); // poly -> (prev poly, link index)
        let mut cost: HashMap<usize, f32> = HashMap::from([(start, 0.0)]);
        let mut pos: HashMap<usize, Vec3> = HashMap::from([(start, from)]);
        let mut open = BinaryHeap::from([Open(0.0, start)]);
        let mut found = false;
        let mut expanded = 0;
        while let Some(Open(_, p)) = open.pop() {
            if p == goal {
                found = true;
                break;
            }
            expanded += 1;
            if expanded > 20_000 {
                break;
            }
            let here = pos[&p];
            for (li, link) in self.polys[p].links.iter().enumerate() {
                let mid = (link.a + link.b) * 0.5;
                let g = cost[&p] + here.distance(mid);
                if cost.get(&link.to).is_none_or(|&c| g < c) {
                    cost.insert(link.to, g);
                    pos.insert(link.to, mid);
                    came.insert(link.to, (p, li));
                    open.push(Open(g + mid.distance(to), link.to));
                }
            }
        }
        if !found {
            return None;
        }
        // chain of portals from start to goal
        let mut portals = Vec::new();
        let mut p = goal;
        while p != start {
            let (prev, li) = came[&p];
            let l = &self.polys[prev].links[li];
            portals.push((l.a, l.b));
            p = prev;
        }
        portals.reverse();
        Some(string_pull(from, to, &portals))
    }
}

/// Turn a chain of portals into few waypoints: from the current point, jump to the
/// furthest portal midpoint whose straight line still passes through every portal in
/// between.
fn string_pull(from: Vec3, to: Vec3, portals: &[(Vec3, Vec3)]) -> Vec<Vec3> {
    // aim slightly inside each portal so we don't scrape the corner
    let points: Vec<Vec3> = portals
        .iter()
        .map(|(a, b)| (*a + *b) * 0.5)
        .chain(std::iter::once(to))
        .collect();
    let shrunk: Vec<(Vec2, Vec2)> = portals
        .iter()
        .map(|(a, b)| {
            let (a, b) = (xz(*a), xz(*b));
            let d = (b - a).normalize_or_zero() * ((b - a).length() * 0.15).min(0.4);
            (a + d, b - d)
        })
        .collect();
    let mut out = Vec::new();
    let mut cur = from;
    let mut i = 0; // first portal not yet passed
    while i < points.len() {
        // furthest j such that cur -> points[j] crosses all portals i..j
        let mut best = i;
        for j in i..points.len() {
            let ok = (i..j.min(shrunk.len())).all(|k| segments_cross(xz(cur), xz(points[j]), shrunk[k].0, shrunk[k].1));
            if ok {
                best = j;
            }
        }
        out.push(points[best]);
        cur = points[best];
        i = best + 1;
    }
    out
}
