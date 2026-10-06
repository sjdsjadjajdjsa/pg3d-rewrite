#!/usr/bin/env python3
"""Export one Pixel Gun 3D map (Unity scene bundle) to glTF for the Bevy rewrite.

    tools/venv/bin/python scripts/export_map.py <map>        e.g. arena, hill, ice_palace
    tools/venv/bin/python scripts/export_map.py --list       list map names

Reads the Steam install read-only (needs private/index/bundles.json from bundle_index.py).
Writes private/export/maps/<map>/:
    map.gltf + map.bin + textures/*.png   visible geometry with materials
    collision.gltf + collision.bin        MeshCollider meshes (no materials)
    colliders.json                        box/sphere/capsule colliders in world space
    info.json                             stats, lights, skipped things

Coordinates: Unity is left-handed (+Z forward), glTF/Bevy right-handed. We mirror X:
position (x,y,z) -> (-x,y,z), rotation quaternion (x,y,z,w) -> (x,-y,-z,w), triangle
winding reversed, UV v -> 1-v.
"""
import argparse
import collections
import glob
import json
import os
import pathlib
import re
import sys

import numpy as np
import UnityPy

ROOT = pathlib.Path(__file__).resolve().parent.parent
GAME_DATA = pathlib.Path(os.path.expanduser(
    "~/.local/share/Steam/steamapps/common/Pixel Gun 3D PC Edition/Pixel Gun 3D_Data"))
INDEX = ROOT / "private/index/bundles.json"
OUT_ROOT = ROOT / "private/export/maps"

MAIN_TEX = ("_MainTex", "_BaseMap", "_Albedo", "_Diffuse", "_Texture")
MAIN_COLOR = ("_Color", "_BaseColor", "_TintColor", "_MainColor")
MARKER_RE = re.compile(r"spawn|createzone|respawn|point", re.I)


# ---------------------------------------------------------------- loading

def load_env(map_name: str):
    idx = json.loads(INDEX.read_text())
    root = idx["root"]
    scene = glob.glob(f"{root}/{map_name}_scene/*/*")
    if not scene:
        sys.exit(f"no scene bundle for '{map_name}' (try --list)")
    paths = [scene[0]] + glob.glob(f"{root}/{map_name}_data/*/*")
    seen = set(paths)
    queue = list(paths)
    # transitive dependency closure via CAB names
    while queue:
        env = UnityPy.load(queue.pop())
        for sf in iter_serialized(env):
            for ext in sf.externals:
                cab = ext.path.split("/")[-1]
                rel = idx["cab"].get(cab)
                if rel:
                    p = os.path.join(root, rel)
                    if p not in seen:
                        seen.add(p)
                        queue.append(p)
    builtin = [str(GAME_DATA / "Resources/unity default resources"), str(GAME_DATA / "Resources/unity_builtin_extra")]
    env = UnityPy.load(*sorted(seen), *builtin)
    return env, scene[0], len(seen)


def iter_serialized(env):
    out = []

    from UnityPy.files import SerializedFile

    def walk(f):
        if isinstance(f, SerializedFile):
            out.append(f)
        for c in (getattr(f, "files", None) or {}).values():
            walk(c)
    for f in env.files.values():
        walk(f)
    return out


def deref(ptr):
    """PPtr -> object instance (or None)."""
    if ptr is None or getattr(ptr, "m_PathID", 0) == 0:
        return None
    try:
        return ptr.read()
    except Exception:
        return None


def ptr_key(ptr):
    try:
        r = ptr.deref()
        return (r.assets_file.name, r.path_id)
    except Exception:
        return None


# ---------------------------------------------------------------- math

def unity_trs_to_gltf(t):
    p, r, s = t.m_LocalPosition, t.m_LocalRotation, t.m_LocalScale
    return [-p.x, p.y, p.z], [r.x, -r.y, -r.z, r.w], [s.x, s.y, s.z]


def trs_matrix(tr, rot, sc):
    x, y, z, w = rot
    R = np.array([
        [1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
        [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
        [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)],
    ])
    M = np.eye(4)
    M[:3, :3] = R * np.array(sc)
    M[:3, 3] = tr
    return M


# ---------------------------------------------------------------- glTF writer

class Gltf:
    def __init__(self, name):
        self.name = name
        self.bin = bytearray()
        self.doc = {"asset": {"version": "2.0", "generator": "pg3d export_map.py"},
                    "scene": 0, "scenes": [{"nodes": []}], "nodes": [], "meshes": [],
                    "buffers": [], "bufferViews": [], "accessors": [],
                    "materials": [], "textures": [], "images": [], "samplers": []}

    def accessor(self, arr: np.ndarray, kind: str, target=None, minmax=False):
        comp = {np.float32: 5126, np.uint32: 5125, np.uint16: 5123}[arr.dtype.type]
        while len(self.bin) % 4:
            self.bin.append(0)
        view = {"buffer": 0, "byteOffset": len(self.bin), "byteLength": arr.nbytes}
        if target:
            view["target"] = target
        self.bin += arr.tobytes()
        self.doc["bufferViews"].append(view)
        acc = {"bufferView": len(self.doc["bufferViews"]) - 1, "componentType": comp,
               "count": int(arr.shape[0]), "type": kind}
        if minmax:
            acc["min"] = arr.min(axis=0).tolist()
            acc["max"] = arr.max(axis=0).tolist()
        self.doc["accessors"].append(acc)
        return len(self.doc["accessors"]) - 1

    def node(self, **kw):
        self.doc["nodes"].append({k: v for k, v in kw.items() if v is not None})
        return len(self.doc["nodes"]) - 1

    def save(self, out_dir: pathlib.Path):
        d = self.doc
        d["buffers"] = [{"uri": f"{self.name}.bin", "byteLength": len(self.bin)}]
        for k in ("materials", "textures", "images", "samplers", "meshes"):
            if not d[k]:
                del d[k]
        (out_dir / f"{self.name}.bin").write_bytes(bytes(self.bin))
        (out_dir / f"{self.name}.gltf").write_text(json.dumps(d))


# ---------------------------------------------------------------- exporter

class Exporter:
    def __init__(self, env, out_dir: pathlib.Path):
        self.env = env
        self.out = out_dir
        (out_dir / "textures").mkdir(parents=True, exist_ok=True)
        self.vis = Gltf("map")
        self.col = Gltf("collision")
        self.vis.doc["samplers"].append({"magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497})
        self.mesh_cache = {}     # (mesh key) -> MeshHandler data
        self.vis_meshes = {}     # (mesh key, submesh range, materials) -> gltf mesh index
        self.col_meshes = {}
        self.materials = {}      # material key -> gltf material index
        self.textures = {}       # texture key -> gltf texture index
        self.colliders = []
        self.lights = []
        self.markers = []        # spawn zones etc. (empty GameObjects with telling names)
        self.stats = collections.Counter()

    # ---- mesh data
    def mesh_data(self, mesh_ptr):
        key = ptr_key(mesh_ptr)
        if key in self.mesh_cache:
            return self.mesh_cache[key]
        data = None
        mesh = deref(mesh_ptr)
        if mesh is not None:
            try:
                from UnityPy.helpers.MeshHelper import MeshHandler
                h = MeshHandler(mesh)
                h.process()
                vc = h.m_VertexCount
                pos = np.array(h.m_Vertices, dtype=np.float32).reshape(vc, -1)[:, :3].copy()
                if len(pos):
                    pos[:, 0] *= -1
                    nrm = None
                    if h.m_Normals and np.size(h.m_Normals) >= 3 * vc:
                        nrm = np.array(h.m_Normals, dtype=np.float32).reshape(vc, -1)[:, :3].copy()
                        nrm[:, 0] *= -1
                        n = np.linalg.norm(nrm, axis=1, keepdims=True)
                        nrm = np.where(n > 1e-6, nrm / np.maximum(n, 1e-6), np.array([0, 1, 0], np.float32))
                    uv0 = uv1 = None
                    if h.m_UV0 and np.size(h.m_UV0) >= 2 * vc:
                        uv0 = np.array(h.m_UV0, dtype=np.float32).reshape(vc, -1)[:, :2].copy()
                        uv0[:, 1] = 1 - uv0[:, 1]
                    if h.m_UV1 and np.size(h.m_UV1) >= 2 * vc:
                        uv1 = np.array(h.m_UV1, dtype=np.float32).reshape(vc, -1)[:, :2].copy()
                        uv1[:, 1] = 1 - uv1[:, 1]
                    subs = []
                    for tris in h.get_triangles():
                        t = np.array(tris, dtype=np.uint32).reshape(-1, 3)[:, [0, 2, 1]]
                        subs.append(t)
                    data = dict(name=mesh.m_Name, pos=pos, nrm=nrm, uv0=uv0, uv1=uv1, subs=subs, acc={})
            except Exception as e:
                self.stats["mesh_errors"] += 1
                print(f"  mesh {getattr(mesh, 'm_Name', '?')}: {e!r}"[:200], file=sys.stderr)
        self.mesh_cache[key] = data
        return data

    def vertex_accessors(self, g: Gltf, md, with_uv: bool):
        k = id(g)
        if k not in md["acc"]:
            attrs = {"POSITION": g.accessor(md["pos"], "VEC3", 34962, minmax=True)}
            # Normals also for collision meshes: without them Bevy generates flat normals by
            # un-indexing the mesh, and the physics engine then can't build a triangle mesh.
            if md["nrm"] is not None:
                attrs["NORMAL"] = g.accessor(md["nrm"].astype(np.float32), "VEC3", 34962)
            if with_uv:
                if md["uv0"] is not None:
                    attrs["TEXCOORD_0"] = g.accessor(md["uv0"], "VEC2", 34962)
                if md["uv1"] is not None:
                    attrs["TEXCOORD_1"] = g.accessor(md["uv1"], "VEC2", 34962)
            md["acc"][k] = attrs
        return md["acc"][k]

    # ---- materials / textures
    def texture(self, tex_ptr):
        key = ptr_key(tex_ptr)
        if key is None:
            return None
        if key not in self.textures:
            idx = None
            tex = deref(tex_ptr)
            if tex is not None and type(tex).__name__ == "Texture2D":
                try:
                    img = tex.image
                    fname = f"{len(self.textures):04d}_{safe(tex.m_Name)}.png"
                    img.save(self.out / "textures" / fname)
                    g = self.vis.doc
                    g["images"].append({"uri": f"textures/{fname}", "name": tex.m_Name})
                    g["textures"].append({"source": len(g["images"]) - 1, "sampler": 0})
                    idx = len(g["textures"]) - 1
                    self.stats["textures"] += 1
                except Exception as e:
                    self.stats["texture_errors"] += 1
                    print(f"  texture {tex.m_Name}: {e!r}"[:200], file=sys.stderr)
            self.textures[key] = idx
        return self.textures[key]

    def material(self, mat_ptr):
        key = ptr_key(mat_ptr)
        if key in self.materials:
            return self.materials[key]
        mat = deref(mat_ptr)
        out = {"name": "missing", "pbrMetallicRoughness": {"metallicFactor": 0.0, "roughnessFactor": 0.9}}
        if mat is not None:
            shader = deref(mat.m_Shader)
            shader_name = shader_display_name(shader)
            out["name"] = mat.m_Name
            out["extras"] = {"unity_shader": shader_name}
            props = mat.m_SavedProperties
            texenvs = dict(as_pairs(props.m_TexEnvs))
            colors = dict(as_pairs(props.m_Colors))
            pbr = out["pbrMetallicRoughness"]
            for n in MAIN_TEX:
                te = texenvs.get(n)
                if te is not None:
                    ti = self.texture(te.m_Texture)
                    if ti is not None:
                        info = {"index": ti}
                        sc, of = te.m_Scale, te.m_Offset
                        if (sc.x, sc.y, of.x, of.y) != (1, 1, 0, 0):
                            # Unity: uv*scale+offset with v up; converted for v' = 1-v
                            info["extensions"] = {"KHR_texture_transform": {
                                "scale": [sc.x, sc.y], "offset": [of.x, 1 - sc.y - of.y]}}
                            self.vis.doc.setdefault("extensionsUsed", [])
                            if "KHR_texture_transform" not in self.vis.doc["extensionsUsed"]:
                                self.vis.doc["extensionsUsed"].append("KHR_texture_transform")
                        pbr["baseColorTexture"] = info
                        break
            for n in MAIN_COLOR:
                c = colors.get(n)
                if c is not None:
                    pbr["baseColorFactor"] = [clamp01(c.r), clamp01(c.g), clamp01(c.b), clamp01(c.a)]
                    break
            s = shader_name.lower()
            if "cutout" in s or "alphatest" in s or "cutoff" in s:
                out["alphaMode"], out["alphaCutoff"] = "MASK", 0.5
                out["doubleSided"] = True
            elif "transparent" in s or "alpha" in s or "additive" in s or "particle" in s or "blend" in s:
                out["alphaMode"] = "BLEND"
            if "unlit" in s or "particle" in s or "additive" in s:
                out.setdefault("extensions", {})["KHR_materials_unlit"] = {}
                self.vis.doc.setdefault("extensionsUsed", [])
                if "KHR_materials_unlit" not in self.vis.doc["extensionsUsed"]:
                    self.vis.doc["extensionsUsed"].append("KHR_materials_unlit")
        self.vis.doc["materials"].append(out)
        self.materials[key] = len(self.vis.doc["materials"]) - 1
        return self.materials[key]

    # ---- scene walk
    def run(self, scene_path):
        roots = []
        for obj in self.scene_objects(scene_path):
            if obj.type.name == "Transform":
                t = obj.read()
                if deref_is_null(t.m_Father):
                    roots.append(t)
            elif obj.type.name == "LightmapSettings":
                self.stats["lightmaps"] = len(getattr(obj.read(), "m_Lightmaps", []) or [])
        roots.sort(key=lambda t: root_order(t))
        for t in roots:
            n = self.walk(t, np.eye(4))
            if n is not None:
                self.vis.doc["scenes"][0]["nodes"].append(n)
        if not self.col.doc["nodes"]:
            self.col.node(name="empty")
        self.col.doc["scenes"][0]["nodes"] = list(range(len(self.col.doc["nodes"])))

    def scene_objects(self, scene_path):
        for sf in iter_serialized(self.env):
            # objects that live in the scene bundle itself
            p = getattr(getattr(sf, "parent", None), "path", "") or ""
            if os.path.abspath(p) == os.path.abspath(scene_path) or sf.name.startswith("BuildPlayer-"):
                yield from sf.objects.values()

    def walk(self, t, parent_world):
        go = deref(t.m_GameObject)
        if go is None:
            return None
        if not getattr(go, "m_IsActive", True):
            self.stats["inactive_skipped"] += 1
            return None
        tr, rot, sc = unity_trs_to_gltf(t)
        world = parent_world @ trs_matrix(tr, rot, sc)
        node = {"name": go.m_Name, "translation": tr, "rotation": rot, "scale": sc}
        comps = {}
        for c in go.m_Component:
            ptr = getattr(c, "component", c)
            try:
                r = ptr.deref()
                comps.setdefault(r.type.name, []).append(ptr)
            except Exception:
                pass
        mesh_idx = self.visual(comps)
        static_batched = False
        if isinstance(mesh_idx, tuple):
            mesh_idx, static_batched = mesh_idx
        idx = self.vis.node(**node)
        children = []
        if mesh_idx is not None:
            if static_batched:
                # combined static-batch meshes are already in world space: hang them on a
                # child node that cancels this node's world transform
                inv = np.linalg.inv(world)
                children.append(self.vis.node(name=go.m_Name + "_static", mesh=mesh_idx,
                                              matrix=inv.T.flatten().tolist()))
            else:
                self.vis.doc["nodes"][idx]["mesh"] = mesh_idx
        self.collect_colliders(go.m_Name, comps, world)
        if MARKER_RE.search(go.m_Name):
            self.markers.append({"name": go.m_Name, "position": world[:3, 3].tolist()})
        for lt in comps.get("Light", []):
            self.light(go.m_Name, lt, world)
        for ch in t.m_Children:
            ct = deref(ch)
            if ct is not None:
                c = self.walk(ct, world)
                if c is not None:
                    children.append(c)
        if children:
            self.vis.doc["nodes"][idx]["children"] = children
        return idx

    def visual(self, comps):
        if "MeshRenderer" not in comps or "MeshFilter" not in comps:
            if "SkinnedMeshRenderer" in comps:
                self.stats["skinned_skipped"] += 1
            return None
        mr = deref(comps["MeshRenderer"][0])
        mf = deref(comps["MeshFilter"][0])
        if mr is None or mf is None or not getattr(mr, "m_Enabled", True):
            return None
        md = self.mesh_data(mf.m_Mesh)
        if md is None:
            return None
        sbi = getattr(mr, "m_StaticBatchInfo", None)
        first, count = 0, len(md["subs"])
        static = False
        if sbi is not None and getattr(sbi, "subMeshCount", 0):
            first, count, static = sbi.firstSubMesh, sbi.subMeshCount, True
        mats = list(mr.m_Materials)
        mat_keys = tuple(ptr_key(m) for m in mats)
        key = (id(md), first, count, mat_keys)
        if key not in self.vis_meshes:
            attrs = self.vertex_accessors(self.vis, md, True)
            prims = []
            for i in range(count):
                si = first + i
                if si >= len(md["subs"]) or len(md["subs"][si]) == 0:
                    continue
                tri = md["subs"][si]
                prim = {"attributes": attrs,
                        "indices": self.vis.accessor(tri.flatten().astype(np.uint32), "SCALAR", 34963)}
                if mats:
                    prim["material"] = self.material(mats[min(i, len(mats) - 1)])
                prims.append(prim)
                self.stats["triangles"] += len(tri)
            self.vis.doc["meshes"].append({"name": md["name"], "primitives": prims} if prims else None)
            self.vis_meshes[key] = len(self.vis.doc["meshes"]) - 1 if prims else None
            if not prims:
                self.vis.doc["meshes"].pop()
        self.stats["renderers"] += 1
        m = self.vis_meshes[key]
        return None if m is None else (m, static)

    # ---- colliders / lights
    def collect_colliders(self, name, comps, world):
        W = world.T.flatten().tolist()  # column-major like glTF
        for kind in ("BoxCollider", "SphereCollider", "CapsuleCollider", "MeshCollider"):
            for ptr in comps.get(kind, []):
                c = deref(ptr)
                if c is None or not getattr(c, "m_Enabled", True):
                    continue
                trig = bool(getattr(c, "m_IsTrigger", False))
                rec = {"name": name, "kind": kind.replace("Collider", "").lower(), "trigger": trig, "world": W}
                if kind == "BoxCollider":
                    rec.update(center=v3(c.m_Center), size=[c.m_Size.x, c.m_Size.y, c.m_Size.z])
                elif kind == "SphereCollider":
                    rec.update(center=v3(c.m_Center), radius=c.m_Radius)
                elif kind == "CapsuleCollider":
                    rec.update(center=v3(c.m_Center), radius=c.m_Radius, height=c.m_Height, direction=c.m_Direction)
                else:
                    md = self.mesh_data(c.m_Mesh)
                    if md is None:
                        continue
                    k = id(md)
                    if k not in self.col_meshes:
                        attrs = self.vertex_accessors(self.col, md, False)
                        tri = np.concatenate([s for s in md["subs"] if len(s)]) if md["subs"] else None
                        if tri is None or not len(tri):
                            continue
                        self.col.doc["meshes"].append({"name": md["name"], "primitives": [{
                            "attributes": attrs,
                            "indices": self.col.accessor(tri.flatten().astype(np.uint32), "SCALAR", 34963)}]})
                        self.col_meshes[k] = len(self.col.doc["meshes"]) - 1
                    rec["mesh"] = self.col_meshes[k]
                    if not trig:
                        self.col.node(name=name, mesh=self.col_meshes[k], matrix=W)
                self.colliders.append(rec)
                self.stats[f"collider_{rec['kind']}" + ("_trigger" if trig else "")] += 1

    def light(self, name, ptr, world):
        l = deref(ptr)
        if l is None:
            return
        c = l.m_Color
        fwd = world[:3, :3] @ np.array([0, 0, -1.0])  # glTF lights point down -Z
        self.lights.append({"name": name, "type": int(l.m_Type), "color": [c.r, c.g, c.b],
                            "intensity": l.m_Intensity, "range": l.m_Range,
                            "position": world[:3, 3].tolist(), "direction": fwd.tolist(),
                            "baked": int(getattr(l, "m_Lightmapping", getattr(getattr(l, "m_BakingOutput", None), "lightmapBakeType", 0)) or 0)})

    def save(self, info):
        self.vis.save(self.out)
        self.col.save(self.out)
        (self.out / "colliders.json").write_text(json.dumps(self.colliders))
        info.update(stats=dict(self.stats), markers=self.markers, lights=self.lights)
        (self.out / "info.json").write_text(json.dumps(info, indent=1))


# ---------------------------------------------------------------- small helpers

def safe(s):
    return "".join(ch if ch.isalnum() or ch in "-_" else "_" for ch in s)[:60] or "tex"


def clamp01(x):
    return max(0.0, min(1.0, float(x)))


def v3(v):
    return [-v.x, v.y, v.z]


def as_pairs(seq):
    for item in seq or []:
        if isinstance(item, tuple):
            k, v = item
        else:
            k, v = item[0], item[1]
        yield (k if isinstance(k, str) else getattr(k, "name", str(k))), v


def deref_is_null(ptr):
    return ptr is None or getattr(ptr, "m_PathID", 0) == 0


def root_order(t):
    try:
        return t.m_RootOrder
    except AttributeError:
        return 0


def shader_display_name(shader):
    if shader is None:
        return ""
    try:
        return shader.m_ParsedForm.m_Name
    except Exception:
        return getattr(shader, "m_Name", "") or ""


def list_maps():
    idx = json.loads(INDEX.read_text())
    root = pathlib.Path(idx["root"])
    names = sorted(p.name[:-6] for p in root.glob("*_scene"))
    print("\n".join(names))
    print(f"{len(names)} maps", file=sys.stderr)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("map", nargs="?")
    ap.add_argument("--list", action="store_true")
    a = ap.parse_args()
    if a.list or not a.map:
        return list_maps()
    env, scene_path, nbundles = load_env(a.map)
    out = OUT_ROOT / a.map
    ex = Exporter(env, out)
    ex.run(scene_path)
    ex.save({"map": a.map, "bundles_loaded": nbundles})
    print(json.dumps(dict(map=a.map, bundles=nbundles, **ex.stats)))
    print(f"-> {out}")


if __name__ == "__main__":
    main()
