#!/usr/bin/env python3
"""Export a map's baked Unity navigation mesh (where enemies can walk) for the Bevy rewrite.

    tools/venv/bin/python scripts/export_navmesh.py arena [hill ...]

Writes private/export/maps/<map>/navmesh.json:
    {"verts": [[x,y,z], ...], "polys": [[v0, v1, ...], ...],
     "links": [[[neighbour poly, [portal start xyz], [portal end xyz]], ...] per poly]}
Coordinates are already mirrored (x -> -x) like the rest of the export.

Format notes (Unity 2022 NavMeshData tile, reverse engineered from the files):
    header 72 bytes: 'VAND'(int) version(16) x y layer polyCount vertCount detailMeshCount
                     detailVertCount detailTriCount bvNodeCount | bmin[3] bmax[3] bvQuantFactor
    verts:        vertCount   x 3 floats (world space, Unity coords)
    polys:        polyCount   x 32 bytes: u16 verts[6], u16 neis[6], u32 flags, u8 vertCount,
                                          u8 area, 2 pad
    detailMeshes: polyCount   x 12, detailVerts x 12, detailTris x 8, bvNodes x 16
Neighbours across tiles are not stored by index, so we connect polygons that share an edge
(same two vertex positions), and polygons whose edges overlap along a tile seam.
"""
import collections
import json
import os
import pathlib
import struct
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import export_map as em  # noqa: E402

HEADER = 72
POLY = 32


def scene_navmesh(env, scene_path):
    for o in env.objects:
        if o.type.name != "NavMeshSettings":
            continue
        parent = getattr(getattr(o.assets_file, "parent", None), "path", "") or ""
        if o.assets_file.name.startswith("BuildPlayer-") or os.path.abspath(parent) == os.path.abspath(scene_path):
            return em.deref(o.read().m_NavMeshData)
    return None


def parse_tile(b):
    magic, version, _x, _y, _layer, pc, vc = struct.unpack_from("<7i", b, 0)
    if magic != 0x444E4156 or version != 16:
        raise ValueError(f"unknown tile magic/version {magic:#x}/{version}")
    verts = [struct.unpack_from("<3f", b, HEADER + i * 12) for i in range(vc)]
    off = HEADER + vc * 12
    polys = []
    for i in range(pc):
        f = struct.unpack_from("<6H6HIBB", b, off + i * POLY)
        n = f[13]
        area = f[14] & 0x3F
        kind = f[14] >> 6  # 0 = ground polygon, 1 = off-mesh connection
        if kind == 0 and 3 <= n <= 6:
            polys.append(([verts[v] for v in f[0:n]], area))
    return polys


def export(map_name):
    env, scene_path, _ = em.load_env(map_name)
    nav = scene_navmesh(env, scene_path)
    if nav is None:
        print(f"{map_name}: no navmesh")
        return
    key = lambda p: (round(-p[0], 3), round(p[1], 3), round(p[2], 3))  # mirror X, weld
    vindex, verts, polys = {}, [], []
    for t in nav.m_NavMeshTiles:
        for pverts, _area in parse_tile(bytes(t.m_MeshData)):
            ids = []
            for p in pverts:
                k = key(p)
                if k not in vindex:
                    vindex[k] = len(verts)
                    verts.append(list(k))
                ids.append(vindex[k])
            # mirroring flips winding; keep counter-clockwise when seen from above
            polys.append(ids[::-1])
    edges = collections.defaultdict(list)
    for pi, ids in enumerate(polys):
        for a, b in zip(ids, ids[1:] + ids[:1]):
            edges[(min(a, b), max(a, b))].append(pi)
    links = [[] for _ in polys]
    border = []  # edges used by one polygon only
    for (a, b), ps in edges.items():
        if len(ps) == 1:
            border.append((ps[0], a, b))
        for p in ps:
            for q in ps:
                if p != q:
                    links[p].append([q, verts[a], verts[b]])
    # tile seams: edges along the same axis-aligned line that overlap
    seams = collections.defaultdict(list)
    for p, a, b in border:
        va, vb = verts[a], verts[b]
        for axis, other in ((0, 2), (2, 0)):
            if abs(va[axis] - vb[axis]) < 0.01:
                lo, hi = sorted((va[other], vb[other]))
                seams[(axis, round(va[axis], 1))].append((p, lo, hi, (va[1] + vb[1]) / 2, axis, other, va[axis]))
    joined = 0
    for group in seams.values():
        for i, (p, lo, hi, y, axis, other, c) in enumerate(group):
            for q, lo2, hi2, y2, *_ in group[i + 1:]:
                if p == q or abs(y - y2) > 0.6:
                    continue
                a, b = max(lo, lo2), min(hi, hi2)
                if b - a < 0.2:
                    continue
                pa, pb = [0.0, y, 0.0], [0.0, y, 0.0]
                pa[axis] = pb[axis] = c
                pa[other], pb[other] = a, b
                links[p].append([q, pa, pb])
                links[q].append([p, pa, pb])
                joined += 1
    out = em.OUT_ROOT / map_name
    out.mkdir(parents=True, exist_ok=True)
    (out / "navmesh.json").write_text(json.dumps({"verts": verts, "polys": polys, "links": links}))
    lonely = sum(1 for l in links if not l)
    print(f"{map_name}: {len(nav.m_NavMeshTiles)} tiles, {len(polys)} polygons, {len(verts)} vertices, "
          f"{joined} tile-seam links, {lonely} without neighbours -> {out / 'navmesh.json'}")


if __name__ == "__main__":
    for m in sys.argv[1:]:
        export(m)
