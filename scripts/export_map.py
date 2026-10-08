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
MARKER_RE = re.compile(r"spawn|createzone|creationzone|respawn|point", re.I)


# ---------------------------------------------------------------- loading

def load_level_env(level: str):
    """Scenes built into the game itself (levelN files, e.g. level17 = the lobby)."""
    files = [GAME_DATA / level, GAME_DATA / "resources.assets", GAME_DATA / "globalgamemanagers.assets"]
    files += sorted(GAME_DATA.glob("sharedassets*.assets"))
    # texture/mesh data lives next to the asset files in .resS / .resource streams
    names = {x.name for x in files}
    files += [f for f in GAME_DATA.iterdir() if f.suffix in (".resS", ".resource")
              and (f.stem in names or f.stem + ".assets" in names or f.name.startswith(level))]
    files += [GAME_DATA / "Resources/unity default resources", GAME_DATA / "Resources/unity_builtin_extra"]
    env = UnityPy.load(*[str(f) for f in files])
    return env, str(GAME_DATA / level), len(files)


def load_env(map_name: str):
    if re.fullmatch(r"level\d+", map_name):
        return load_level_env(map_name)
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


def serialized_in(f):
    """All SerializedFiles inside one loaded file (a bundle can hold several)."""
    from UnityPy.files import SerializedFile
    if isinstance(f, SerializedFile):
        return [f]
    return [s for c in (getattr(f, "files", None) or {}).values() for s in serialized_in(c)]


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
        self.samplers = {(1, 0, 0): 0}  # (Unity filter mode, wrap U, wrap V) -> glTF sampler
        self.mesh_cache = {}     # (mesh key) -> MeshHandler data
        self.node_by_transform = {}  # (file, path_id) of a Unity Transform -> glTF node
        self.pending_skins = []  # (node index, SkinnedMeshRenderer, mesh data)
        self.vis_meshes = {}     # (mesh key, submesh range, materials) -> gltf mesh index
        self.col_meshes = {}
        self.materials = {}      # material key -> gltf material index
        self.textures = {}       # texture key -> gltf texture index
        self.colliders = []
        self.lights = []
        # honour Cubic/Universal's UNLIT keyword (weapons, bots, characters). Map exports
        # switch this off: their surfaces are lit by baked lightmaps instead.
        self.unlit_keywords = True
        self.in_explosive = False
        self.cameras = []
        self.sounds = {}         # filled by export_weapons.export_sounds for prefabs
        self.skip_names = set()  # GameObject names to leave out (with their children)
        self.force_active = set()  # inactive GameObjects to export anyway (switched on by game code)
        self.markers = []        # spawn zones etc. (empty GameObjects with telling names)
        self.lightmaps = []      # lightmap index -> png path (or None)
        self.lightmap_scale = 1.0
        self.extra_lightmaps = {}  # MeshLightmapDataController textures -> png path
        self.render = {}
        self.skybox = None       # Skybox/Cubemap material drawn on a mesh (see export_cubemap)
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
                    joints, weights = skin_weights(h, vc)
                    bindposes = [unity_matrix(m) for m in (getattr(mesh, "m_BindPose", None) or [])]
                    data = dict(name=mesh.m_Name, pos=pos, nrm=nrm, uv0=uv0, uv1=uv1, subs=subs, acc={},
                                joints=joints, weights=weights, bindposes=bindposes)
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
            nrm = md["nrm"]
            if nrm is None and not with_uv:
                nrm = np.tile(np.array([0, 1, 0], np.float32), (len(md["pos"]), 1))
            if nrm is not None:
                attrs["NORMAL"] = g.accessor(nrm.astype(np.float32), "VEC3", 34962)
            if with_uv:
                if md["uv0"] is not None:
                    attrs["TEXCOORD_0"] = g.accessor(md["uv0"], "VEC2", 34962)
                if md["uv1"] is not None:
                    attrs["TEXCOORD_1"] = g.accessor(md["uv1"], "VEC2", 34962)
            md["acc"][k] = attrs
        return md["acc"][k]

    # ---- materials / textures
    def sampler(self, tex):
        """glTF sampler matching the texture's Unity import settings. Most of the game's
        pixel-art textures use Point filtering (crisp texels); bilinear would blur them."""
        ts = getattr(tex, "m_TextureSettings", None)
        filt = int(getattr(ts, "m_FilterMode", 1)) if ts is not None else 1
        wu = int(getattr(ts, "m_WrapU", getattr(ts, "m_WrapMode", 0))) if ts is not None else 0
        wv = int(getattr(ts, "m_WrapV", getattr(ts, "m_WrapMode", 0))) if ts is not None else 0
        key = (min(filt, 1), wu, wv)
        if key not in self.samplers:
            wrap = {0: 10497, 1: 33071, 2: 33648}  # repeat, clamp, mirror
            s = {"wrapS": wrap.get(wu, 10497), "wrapT": wrap.get(wv, 10497)}
            if filt == 0:
                s.update(magFilter=9728, minFilter=9984)  # nearest, nearest mipmap
            else:
                s.update(magFilter=9729, minFilter=9987)
            self.vis.doc["samplers"].append(s)
            self.samplers[key] = len(self.vis.doc["samplers"]) - 1
        self.stats[f"textures_filter{filt}"] += 1
        return self.samplers[key]

    def emission_texture(self, texenvs, emis, strength):
        """Cubic/Universal emission: _MainTex.rgb * _EmissionTex (rgb tint, alpha = mask)
        * _Emission, baked into one PNG for the glTF emissiveTexture."""
        main = texenvs.get("_MainTex")
        mt, et = deref(main.m_Texture) if main else None, deref(emis.m_Texture)
        if mt is None or et is None:
            return None
        key = ("emission", ptr_key(main.m_Texture), ptr_key(emis.m_Texture), round(float(strength), 3))
        if key in self.textures:
            return self.textures[key]
        try:
            from PIL import Image
            m = np.asarray(mt.image.convert("RGB"), dtype=np.float32) / 255.0
            e = et.image.convert("RGBA")
            if e.size != (m.shape[1], m.shape[0]):
                e = e.resize((m.shape[1], m.shape[0]), Image.NEAREST)
            e = np.asarray(e, dtype=np.float32) / 255.0
            out = np.clip(m * e[:, :, :3] * e[:, :, 3:4] * float(strength), 0, 1)
            fname = f"{len(self.textures):04d}_{safe(mt.m_Name)}_emission.png"
            Image.fromarray((out * 255 + 0.5).astype(np.uint8), "RGB").save(self.out / "textures" / fname)
            g = self.vis.doc
            g["images"].append({"uri": f"textures/{fname}", "name": mt.m_Name + "_emission"})
            g["textures"].append({"source": len(g["images"]) - 1, "sampler": self.sampler(mt)})
            idx = len(g["textures"]) - 1
            self.stats["emission_textures"] += 1
        except Exception as ex:
            print(f"  emission {mt.m_Name}: {ex!r}"[:200], file=sys.stderr)
            idx = None
        self.textures[key] = idx
        return idx

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
                    g["textures"].append({"source": len(g["images"]) - 1, "sampler": self.sampler(tex)})
                    idx = len(g["textures"]) - 1
                    self.stats["textures"] += 1
                except Exception as e:
                    self.stats["texture_errors"] += 1
                    print(f"  texture {tex.m_Name}: {e!r}"[:200], file=sys.stderr)
            self.textures[key] = idx
        return self.textures[key]

    def material(self, mat_ptr, no_lightmap=False):
        """`no_lightmap`: the renderer has no baked lightmap, so Cubic/Universal's UNLIT
        keyword counts even in map exports (season lobby decor, props placed by code)."""
        key = (ptr_key(mat_ptr), no_lightmap and not self.unlit_keywords)
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
            floats = dict(as_pairs(props.m_Floats))
            if shader_name == "Skybox/Cubemap" and self.skybox is None:
                self.skybox = self.export_cubemap(texenvs.get("_Tex"), colors.get("_Tint"), floats)
            keywords = str(getattr(mat, "m_ShaderKeywords", "") or "").split()
            keywords += [str(k) for k in (getattr(mat, "m_ValidKeywords", None) or [])]
            if keywords:
                out["extras"]["keywords"] = keywords
            emis = texenvs.get("_EmissionTex")
            if emis is not None and ("EMISSION_ENABLE" in keywords or floats.get("_EmissionEnable", 0) == 1):
                ei = self.emission_texture(texenvs, emis, floats.get("_Emission", 1.0))
                if ei is not None:
                    # glowing parts (crystals, lamps): the main texture, masked by the
                    # emission texture's alpha, shown at full brightness whatever the lighting
                    out["emissiveTexture"] = {"index": ei}
                    out["emissiveFactor"] = [1.0, 1.0, 1.0]
                    out["extras"]["emission"] = float(floats.get("_Emission", 1.0))
            s = shader_name.lower()
            # Cubic/Universal (most game objects) switches lighting off per material with the
            # UNLIT keyword (set by its misleadingly named _LightingEnable toggle)
            unlit_material = (self.unlit_keywords or no_lightmap) and ("UNLIT" in keywords or "particle" in mat.m_Name.lower())
            if "cutout" in s or "alphatest" in s or "cutoff" in s:
                out["alphaMode"], out["alphaCutoff"] = "MASK", 0.5
                out["doubleSided"] = True
            elif "transparent" in s or "alpha" in s or "additive" in s or "particle" in s or "blend" in s:
                out["alphaMode"] = "BLEND"
            # Cubic/Universal picks its blending per material: _SrcBlend/_DstBlend are Unity
            # BlendMode values (1 One, 5 SrcAlpha, 10 OneMinusSrcAlpha). One as the
            # destination adds the colour on top (glowing light cones); the game reads
            # extras.blend == "add". _CullMode 0 draws both sides.
            src, dst = floats.get("_SrcBlend"), floats.get("_DstBlend")
            if dst == 1.0 and src in (1.0, 5.0):
                out["alphaMode"] = "BLEND"
                out["extras"]["blend"] = "add"
            elif dst == 10.0 and "alphaMode" not in out:
                out["alphaMode"] = "BLEND"
            if floats.get("_CullMode") == 0.0:
                out["doubleSided"] = True
            # the sky (SKY_SYSTEM: dome, clouds, far mountains). MADFINGER "Scroll 2 Layers"
            # shaders draw texture x colour x _MMultiplier/_AMultiplier, unlit, the texture
            # scrolling by _ScrollX/_ScrollY per second; "NO_FOG" and Skybox ones ignore fog.
            # Unlit Cubic/Universal SKY_ materials (mountains) are unlit but fogged.
            sky = None
            if "madfinger/environment/scroll" in s:
                sky = {"mult": float(floats.get("_MMultiplier", floats.get("_AMultiplier", 1.0))),
                       "scroll": [float(floats.get("_ScrollX", 0.0)), float(floats.get("_ScrollY", 0.0))],
                       "fog": not ("no_fog" in s or "skybox" in s)}
            elif mat.m_Name.upper().startswith("SKY") and "UNLIT" in keywords:
                sky = {"mult": 1.0, "scroll": [0.0, 0.0], "fog": "FOG_ENABLE" in keywords}
            if sky is not None:
                out["extras"]["sky"] = sky
                unlit_material = True
                if "ALPHATEST_ENABLE" in keywords:
                    out["alphaMode"], out["alphaCutoff"] = "MASK", float(floats.get("_CutOut", 0.5))
            if "unlit" in s or "particle" in s or "additive" in s or unlit_material:
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
                self.export_lightmaps(obj.read())
            elif obj.type.name == "RenderSettings":
                self.render_settings(obj.read())
        roots.sort(key=lambda t: root_order(t))
        for t in roots:
            n = self.walk(t, np.eye(4))
            if n is not None:
                self.vis.doc["scenes"][0]["nodes"].append(n)
        if not self.col.doc["nodes"]:
            self.col.node(name="empty")
        self.col.doc["scenes"][0]["nodes"] = list(range(len(self.col.doc["nodes"])))

    # ---- baked lighting
    def export_lightmaps(self, settings):
        """Write each baked lightmap as a PNG.

        The game uses Unity's *gamma* colour space: lighting is multiplied in gamma space.
        Bevy works in linear space, so we store lightmap^2.2 (≈ linear) divided by a
        per-map scale, encoded as sRGB PNG. The game multiplies it back with
        StandardMaterial::lightmap_exposure = scale (see info.json "lightmap_scale").
        """
        maps = []
        for i, lm in enumerate(settings.m_Lightmaps or []):
            tex = deref(lm.m_Lightmap)
            if tex is None:
                maps.append(None)
                continue
            rgb = texture_float_rgb(tex)
            if rgb is None:
                maps.append(None)
                self.stats["lightmap_errors"] += 1
                continue
            maps.append(np.power(np.clip(rgb, 0, None), 2.2))
        peak = max([float(np.percentile(m, 99.9)) for m in maps if m is not None] or [1.0])
        self.lightmap_scale = max(1.0, float(np.ceil(peak)))
        from PIL import Image
        for i, m in enumerate(maps):
            if m is None:
                self.lightmaps.append(None)
                continue
            v = np.clip(m / self.lightmap_scale, 0, 1)
            srgb = np.where(v <= 0.0031308, v * 12.92, 1.055 * np.power(v, 1 / 2.4) - 0.055)
            img = Image.fromarray((srgb * 255 + 0.5).astype(np.uint8), "RGB")
            name = f"textures/lightmap_{i}.png"
            img.save(self.out / name)
            self.lightmaps.append(name)
        self.stats["lightmaps"] = sum(1 for m in self.lightmaps if m)

    def export_cubemap(self, te, tint, floats):
        """A Skybox/Cubemap material's cubemap as textures/skybox.png: the six faces stacked
        top to bottom in wgpu order (+X, -X, +Y, -Y, +Z, -Z).

        Unity is left-handed and the export mirrors X, so the X faces swap places and every
        face is mirrored left to right. Unity draws tex x _Tint x 2 x _Exposure (gamma space);
        "brightness" is that factor, still in gamma."""
        import texture2ddecoder
        from PIL import Image
        tex = deref(te.m_Texture) if te is not None else None
        if tex is None:
            return None
        raw = bytes(tex.image_data) or stream_bytes(tex)
        w, fmt, mips = tex.m_Width, int(tex.m_TextureFormat), max(1, tex.m_MipCount)
        decoders = {10: (texture2ddecoder.decode_bc1, 8), 12: (texture2ddecoder.decode_bc3, 16)}
        if fmt not in decoders:
            print(f"  skybox {tex.m_Name}: format {fmt} not handled", file=sys.stderr)
            return None
        dec, block = decoders[fmt]
        face_bytes = sum(max(1, (w >> i) // 4) ** 2 * block for i in range(mips))
        if len(raw) < 6 * face_bytes:
            print(f"  skybox {tex.m_Name}: {len(raw)} bytes, expected {6 * face_bytes}", file=sys.stderr)
            return None
        faces = []
        for k in range(6):
            d = raw[k * face_bytes:k * face_bytes + (w // 4) ** 2 * block]
            faces.append(Image.frombytes("RGBA", (w, w), dec(d, w, w), "raw", "BGRA").convert("RGB")
                         .transpose(Image.FLIP_LEFT_RIGHT))
        faces[0], faces[1] = faces[1], faces[0]
        strip = Image.new("RGB", (w, 6 * w))
        for k, f in enumerate(faces):
            strip.paste(f, (0, k * w))
        (self.out / "textures").mkdir(parents=True, exist_ok=True)
        strip.save(self.out / "textures/skybox.png")
        t = [tint.r, tint.g, tint.b] if tint is not None else [0.5, 0.5, 0.5]
        k = 2.0 * float(floats.get("_Exposure", 1.0))
        return {"png": "textures/skybox.png", "tint": [c * k for c in t],
                "rotation": float(floats.get("_Rotation", 0.0))}

    def render_settings(self, rs):
        def col(c):
            return [c.r, c.g, c.b] if c is not None else None
        self.render = {
            "ambient_mode": getattr(rs, "m_AmbientMode", None),
            "ambient_sky": col(getattr(rs, "m_AmbientSkyColor", None)),
            "ambient_equator": col(getattr(rs, "m_AmbientEquatorColor", None)),
            "ambient_ground": col(getattr(rs, "m_AmbientGroundColor", None)),
            "ambient_intensity": getattr(rs, "m_AmbientIntensity", 1.0),
            "fog": bool(getattr(rs, "m_Fog", False)),
            "fog_color": col(getattr(rs, "m_FogColor", None)),
            "fog_mode": getattr(rs, "m_FogMode", None),
            "fog_start": getattr(rs, "m_LinearFogStart", None),
            "fog_end": getattr(rs, "m_LinearFogEnd", None),
            "fog_density": getattr(rs, "m_FogDensity", None),
        }

    def behaviour(self, comps, class_name):
        """The MonoBehaviour of script `class_name` among a GameObject's components, or None."""
        for ptr in comps.get("MonoBehaviour", []):
            try:
                mb = deref(ptr)
                sc = deref(mb.m_Script)
                if sc is not None and sc.m_ClassName == class_name:
                    return mb
            except Exception:
                pass
        return None

    def controller_lightmap(self, comps):
        """Season lobby decor brings its own baked lightmap: MeshLightmapDataController sets
        the renderer's lightmap texture and scale/offset at run time."""
        mb = self.behaviour(comps, "MeshLightmapDataController")
        if mb is None:
            return None
        tex = deref(getattr(mb, "lightmap", None))
        so = getattr(mb, "m_lightmapScaleOffset", None)
        if tex is None or so is None:
            return None
        key = ptr_key(mb.lightmap)
        if key not in self.extra_lightmaps:
            self.extra_lightmaps[key] = self.write_lightmap(tex, f"textures/lightmap_x{len(self.extra_lightmaps)}.png")
        name = self.extra_lightmaps[key]
        if name is None:
            return None
        self.stats["controller_lightmaps"] += 1
        return {"lightmap": name, "lightmap_rect": [so.z, 1 - so.y - so.w, so.z + so.x, 1 - so.w]}

    def write_lightmap(self, tex, name):
        """One lightmap texture as an sRGB PNG, encoded like export_lightmaps (linear / scale)."""
        from PIL import Image
        rgb = texture_float_rgb(tex)
        if rgb is None:
            return None
        v = np.clip(np.power(np.clip(rgb, 0, None), 2.2) / self.lightmap_scale, 0, 1)
        srgb = np.where(v <= 0.0031308, v * 12.92, 1.055 * np.power(v, 1 / 2.4) - 0.055)
        (self.out / "textures").mkdir(parents=True, exist_ok=True)
        Image.fromarray((srgb * 255 + 0.5).astype(np.uint8), "RGB").save(self.out / name)
        return name

    def season_fog(self, comps):
        """LobbySeasonFogController: the season lobby's own ambient colour and fog."""
        mb = self.behaviour(comps, "LobbySeasonFogController")
        if mb is None:
            return
        try:
            a, f = mb._ambientColor, mb._fog
            self.render.update(ambient_mode=3, ambient_sky=[a.r, a.g, a.b], ambient_intensity=1.0,
                               fog=bool(f.Active), fog_color=[f.Color.r, f.Color.g, f.Color.b],
                               fog_mode=int(f.Mode), fog_start=float(f.Start), fog_end=float(f.End))
            self.stats["season_fog"] += 1
        except Exception as e:
            print(f"  season fog: {e!r}"[:160], file=sys.stderr)

    def lightmap_extras(self, mr, md, static=False):
        """glTF node extras telling the game which lightmap + UV rect this renderer uses.

        Static batching bakes the renderer's lightmap scale/offset into the combined mesh's
        second UV set (its UVs already equal the rect), so those use the whole lightmap.
        """
        idx = getattr(mr, "m_LightmapIndex", 0xFFFF)
        if idx >= 0xFFFE or idx >= len(self.lightmaps) or not self.lightmaps[idx]:
            return None
        so = getattr(mr, "m_LightmapTilingOffset", None) or mr.m_LightmapScaleOffset
        # Unity: lm_uv = uv * (x, y) + (z, w), V pointing up. We flip V (v' = 1 - v), which
        # turns the V range into [1 - y - w, 1 - w]. Bevy: uv = mix(min, max, uv_1).
        if static:
            return {"lightmap": self.lightmaps[idx], "lightmap_rect": [0.0, 0.0, 1.0, 1.0]}
        return {"lightmap": self.lightmaps[idx],
                "lightmap_rect": [so.z, 1 - so.y - so.w, so.z + so.x, 1 - so.w]}

    def scene_objects(self, scene_path):
        for sf in iter_serialized(self.env):
            # objects that live in the scene bundle itself
            p = getattr(getattr(sf, "parent", None), "path", "") or ""
            if (os.path.abspath(p) == os.path.abspath(scene_path) or sf.name.startswith("BuildPlayer-")
                    or sf.name == os.path.basename(scene_path)):
                yield from sf.objects.values()

    def walk(self, t, parent_world):
        go = deref(t.m_GameObject)
        if go is None:
            return None
        if (not getattr(go, "m_IsActive", True) and go.m_Name not in self.force_active) or go.m_Name in self.skip_names:
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
        self.season_fog(comps)
        mesh_idx = self.visual(comps)
        static_batched, extras, skinned = False, None, None
        if isinstance(mesh_idx, tuple):
            mesh_idx, static_batched, extras, skinned = mesh_idx
        idx = self.vis.node(**node)
        self.node_by_transform[obj_key(t)] = idx
        if skinned is not None:
            self.pending_skins.append((idx, skinned[0], skinned[1]))
        children = []
        if mesh_idx is not None:
            if static_batched:
                # combined static-batch meshes are already in world space: hang them on a
                # child node that cancels this node's world transform
                inv = np.linalg.inv(world)
                children.append(self.vis.node(name=go.m_Name + "_static", mesh=mesh_idx,
                                              matrix=inv.T.flatten().tolist(), extras=extras))
            else:
                self.vis.doc["nodes"][idx]["mesh"] = mesh_idx
                if extras:
                    self.vis.doc["nodes"][idx]["extras"] = extras
        particle = self.particle_texture(comps)
        if particle:
            self.vis.doc["nodes"][idx].setdefault("extras", {}).update(particle_texture=particle,
                                                                        particle=particle_settings(comps))
        flash = self.flash_fire(comps)
        if flash:
            self.vis.doc["nodes"][idx].setdefault("extras", {}).update(flash_fire=flash)
        if not self.in_explosive:
            self.collect_colliders(go.m_Name, comps, world)
        explosive = self.explosive_prefab(comps)
        if explosive is not None:
            # ExplosionObjectRespawnController: the game spawns an explosive barrel/TNT here
            # (and respawns it); we place it as a child node tagged for the game to handle
            prefab_t, info = explosive
            fx = info.pop("_fx", None)
            self.in_explosive = True
            c = self.walk(prefab_t, world)
            if c is not None and fx is not None:
                # the blast effect (particle systems), fired by the game when it explodes
                fx_go = deref(fx.m_GameObject)
                self.force_active.add(fx_go.m_Name)
                f = self.walk(fx, world)
                if f is not None:
                    self.vis.doc["nodes"][f]["translation"] = [0.0, 0.3, 0.0]
                    self.vis.doc["nodes"][c].setdefault("children", []).append(f)
                    info["effect"] = fx_go.m_Name
            self.in_explosive = False
            if c is not None:
                self.vis.doc["nodes"][c]["translation"] = [0.0, 0.0, 0.0]
                self.vis.doc["nodes"][c].setdefault("extras", {})["explosive"] = info
                children.append(c)
                self.stats["explosives"] += 1
        if MARKER_RE.search(go.m_Name):
            self.markers.append({"name": go.m_Name, "position": world[:3, 3].tolist()})
        for cam in comps.get("Camera", []):
            self.camera(go.m_Name, cam, world)
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

    def flash_fire(self, comps):
        """FlashFire (weapons): StartGunFlash switches gunFlashObj on for timeFireAction
        seconds after each shot (Update switches it off when the time runs out), so its
        looping particle systems play while the gun keeps firing."""
        for ptr in comps.get("MonoBehaviour", []):
            try:
                mb = deref(ptr)
                sc = deref(mb.m_Script)
                if sc is None or sc.m_ClassName != "FlashFire":
                    continue
                tt = ptr.deref().read_typetree()
                g = deref(mb.gunFlashObj) if getattr(mb, "gunFlashObj", None) is not None else None
                return {"time": float(tt.get("timeFireAction", 0.15)), "object": g.m_Name if g is not None else None}
            except Exception as e:
                print(f"  flash fire: {e!r}"[:160], file=sys.stderr)
        return None

    def explosive_prefab(self, comps):
        """(prefab root Transform, settings) of an ExplosionObjectRespawnController, or None."""
        for ptr in comps.get("MonoBehaviour", []):
            try:
                mb = deref(ptr)
                sc = deref(mb.m_Script)
                if sc is None or sc.m_ClassName != "ExplosionObjectRespawnController":
                    continue
                go = deref(mb.explosionObjectPrefab)
                info = {"respawn": float(getattr(mb, "timeToNextRespawn", 90.0))}
                tr = None
                for c in go.m_Component:
                    r = deref(getattr(c, "component", c))
                    if type(r).__name__ == "Transform":
                        tr = r
                    elif type(r).__name__ == "MonoBehaviour":
                        rs = deref(r.m_Script)
                        if rs is not None and rs.m_ClassName == "DamagedExplosionObject":
                            info.update(radius=float(r.radiusExplosion), radius_max=float(r.radiusMaxExplosion),
                                        damage_percent=float(r.damagePercent))
                            fx = deref(r.explosionEffect)
                            if fx is not None:
                                for fc in fx.m_Component:
                                    ft = deref(getattr(fc, "component", fc))
                                    if type(ft).__name__ == "Transform":
                                        info["_fx"] = ft
                    elif type(r).__name__ == "BoxCollider":
                        info.update(box_center=[-r.m_Center.x, r.m_Center.y, r.m_Center.z],
                                    box_size=[r.m_Size.x, r.m_Size.y, r.m_Size.z])
                if tr is not None:
                    return tr, info
            except Exception as e:
                print(f"  explosive: {e!r}"[:160], file=sys.stderr)
        return None

    def particle_texture(self, comps):
        """Texture of a ParticleSystemRenderer's material (muzzle flashes, smoke, ...), as
        a path relative to the glTF, or None. The particle simulation itself is not exported."""
        if "ParticleSystemRenderer" not in comps:
            return None
        r = deref(comps["ParticleSystemRenderer"][0])
        mats = list(getattr(r, "m_Materials", []) or []) if r is not None else []
        if not mats:
            return None
        mi = self.material(mats[0])
        info = self.vis.doc["materials"][mi].get("pbrMetallicRoughness", {}).get("baseColorTexture")
        if info is None:
            return None
        tex = self.vis.doc["textures"][info["index"]]
        return self.vis.doc["images"][tex["source"]]["uri"]

    def visual(self, comps):
        if "MeshRenderer" in comps and "MeshFilter" in comps:
            mr = deref(comps["MeshRenderer"][0])
            mf = deref(comps["MeshFilter"][0])
            mesh_ptr = mf.m_Mesh if mf is not None else None
        elif "SkinnedMeshRenderer" in comps:
            # Skinned meshes (weapon + arms, characters): exported with their skeleton
            # (glTF skin) so animations can move them.
            mr = deref(comps["SkinnedMeshRenderer"][0])
            mesh_ptr = mr.m_Mesh if mr is not None else None
        else:
            return None
        if mr is None or mesh_ptr is None or not getattr(mr, "m_Enabled", True):
            return None
        md = self.mesh_data(mesh_ptr)
        if md is None:
            return None
        sbi = getattr(mr, "m_StaticBatchInfo", None)
        first, count = 0, len(md["subs"])
        static = False
        if sbi is not None and getattr(sbi, "subMeshCount", 0):
            first, count, static = sbi.firstSubMesh, sbi.subMeshCount, True
        mats = list(mr.m_Materials)
        mat_keys = tuple(ptr_key(m) for m in mats)
        extras = self.lightmap_extras(mr, md, static) or self.controller_lightmap(comps)
        is_skinned = (type(mr).__name__ == "SkinnedMeshRenderer" and md["joints"] is not None
                      and len(getattr(mr, "m_Bones", []) or []) > 0)
        key = (id(md), first, count, mat_keys, extras is not None, is_skinned)
        if key not in self.vis_meshes:
            attrs = self.vertex_accessors(self.vis, md, True)
            if is_skinned:
                attrs = dict(attrs,
                             JOINTS_0=self.vis.accessor(md["joints"], "VEC4", 34962),
                             WEIGHTS_0=self.vis.accessor(md["weights"], "VEC4", 34962))
            if extras is not None and "TEXCOORD_1" not in attrs and "TEXCOORD_0" in attrs:
                # Unity falls back to the first UV set for lightmapping
                attrs = dict(attrs, TEXCOORD_1=attrs["TEXCOORD_0"])
            prims = []
            for i in range(count):
                si = first + i
                if si >= len(md["subs"]) or len(md["subs"][si]) == 0:
                    continue
                tri = md["subs"][si]
                prim = {"attributes": attrs,
                        "indices": self.vis.accessor(tri.flatten().astype(np.uint32), "SCALAR", 34963)}
                if mats:
                    prim["material"] = self.material(mats[min(i, len(mats) - 1)], no_lightmap=extras is None)
                prims.append(prim)
                self.stats["triangles"] += len(tri)
            self.vis.doc["meshes"].append({"name": md["name"], "primitives": prims} if prims else None)
            self.vis_meshes[key] = len(self.vis.doc["meshes"]) - 1 if prims else None
            if not prims:
                self.vis.doc["meshes"].pop()
        self.stats["renderers"] += 1
        m = self.vis_meshes[key]
        if extras is not None:
            self.stats["lightmapped_renderers"] += 1
        if is_skinned:
            self.stats["skinned"] += 1
        return None if m is None else (m, static, extras, (mr, md) if is_skinned else None)

    def finalize_skins(self):
        """Turn recorded SkinnedMeshRenderers into glTF skins (needs all nodes to exist)."""
        S = np.diag([-1.0, 1.0, 1.0, 1.0])  # our X-mirroring as a matrix
        for node_idx, smr, md in self.pending_skins:
            joints = []
            for b in smr.m_Bones:
                joints.append(self.node_by_transform.get(ptr_key(b)))
            if not joints or any(j is None for j in joints) or len(md["bindposes"]) < len(joints):
                self.stats["skin_errors"] += 1
                continue
            # Unity bindpose = bone.worldToLocal * renderer.localToWorld at bind time, which is
            # exactly glTF's inverse bind matrix; convert it to our mirrored space: S * M * S.
            ibm = np.array([(S @ md["bindposes"][i] @ S).T.flatten() for i in range(len(joints))], np.float32)
            g = self.vis.doc
            g.setdefault("skins", []).append({"joints": joints, "inverseBindMatrices": self.vis.accessor(ibm, "MAT4")})
            g["nodes"][node_idx]["skin"] = len(g["skins"]) - 1

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

    def camera(self, name, ptr, world):
        c = deref(ptr)
        if c is None or not getattr(c, "m_Enabled", True):
            return
        fwd = world[:3, :3] @ np.array([0, 0, 1.0])  # mirrored Unity +Z
        up = world[:3, :3] @ np.array([0, 1.0, 0])
        self.cameras.append({"name": name, "position": world[:3, 3].tolist(), "forward": fwd.tolist(),
                             "up": up.tolist(), "fov": getattr(c, "field of view", None) or getattr(c, "m_FieldOfView", 60.0),
                             "orthographic": bool(getattr(c, "orthographic", False)), "depth": getattr(c, "m_Depth", 0)})

    def light(self, name, ptr, world):
        l = deref(ptr)
        if l is None or not getattr(l, "m_Enabled", True):
            return
        c = l.m_Color
        # a Unity light shines along its +Z (mirrored like the cameras' forward)
        fwd = world[:3, :3] @ np.array([0, 0, 1.0])
        fwd = fwd / max(float(np.linalg.norm(fwd)), 1e-9)
        self.lights.append({"name": name, "type": int(l.m_Type), "color": [c.r, c.g, c.b],
                            "intensity": l.m_Intensity, "range": l.m_Range,
                            "spot_angle": float(getattr(l, "m_SpotAngle", 30.0)),
                            "position": world[:3, 3].tolist(), "direction": fwd.tolist(),
                            "baked": int(getattr(l, "m_Lightmapping", getattr(getattr(l, "m_BakingOutput", None), "lightmapBakeType", 0)) or 0)})

    def save(self, info):
        self.finalize_skins()
        self.vis.save(self.out)
        self.col.save(self.out)
        (self.out / "colliders.json").write_text(json.dumps(self.colliders))
        info.update(stats=dict(self.stats), lightmap_scale=self.lightmap_scale, render=self.render,
                    markers=self.markers, lights=self.lights, cameras=self.cameras, skybox=self.skybox)
        (self.out / "info.json").write_text(json.dumps(info, indent=1))


# ---------------------------------------------------------------- small helpers

BC6H_DECODER = ROOT / "helpers/bc6h-decode/target/release/bc6h-decode"


def decode_bc6h(raw, w, h):
    """BC6H blocks -> float32 HxWx3 (Unity row order) via helpers/bc6h-decode
    (build it once with `cargo build --release` in that folder)."""
    import subprocess
    import tempfile
    if not BC6H_DECODER.exists():
        print(f"  BC6H lightmap clamped to 8 bits: build {BC6H_DECODER.parent.parent.parent}", file=sys.stderr)
        return None
    with tempfile.TemporaryDirectory() as d:
        src, dst = pathlib.Path(d, "in.bc6h"), pathlib.Path(d, "out.f32")
        src.write_bytes(bytes(raw))
        subprocess.run([str(BC6H_DECODER), str(src), str(dst), str(w), str(h)], check=True)
        return np.frombuffer(dst.read_bytes(), dtype=np.float32).reshape(h, w, 3).copy()


def stream_bytes(tex):
    """The texture's bytes from its .resS stream. UnityPy sometimes returns empty image_data
    for streamed textures (e.g. hill's BC6H lightmap); read the stream from the bundle."""
    sd = tex.m_StreamData
    if not sd or not sd.size:
        return b""
    parent = getattr(tex.assets_file, "parent", None)
    name = sd.path.split("/")[-1]
    r = getattr(parent, "files", {}).get(name)
    if r is None:
        # built-in levels stream from files next to the game's .assets
        f = GAME_DATA / name
        if not f.exists():
            return b""
        with open(f, "rb") as fh:
            fh.seek(sd.offset)
            return fh.read(sd.size)
    r.Position = sd.offset
    return bytes(r.read_bytes(sd.size))


def texture_float_rgb(tex):
    """Texture2D -> float32 HxWx3 array, top row first. Handles half/float HDR formats
    exactly and falls back to UnityPy's 8-bit decoder for everything else."""
    fmt = int(tex.m_TextureFormat)
    w, h = tex.m_Width, tex.m_Height
    try:
        raw = tex.image_data
        if not raw:
            raw = stream_bytes(tex)
        if fmt == 17:   # RGBAHalf
            a = np.frombuffer(raw, dtype=np.float16, count=w * h * 4).reshape(h, w, 4)[:, :, :3]
        elif fmt == 20:  # RGBAFloat
            a = np.frombuffer(raw, dtype=np.float32, count=w * h * 4).reshape(h, w, 4)[:, :, :3]
        elif fmt == 24:  # BC6H: HDR; the Python decoders clamp it to 8 bits
            a = decode_bc6h(raw, w, h)
            if a is None:
                return np.asarray(tex.image.convert("RGB"), dtype=np.float32) / 255.0
        else:
            return np.asarray(tex.image.convert("RGB"), dtype=np.float32) / 255.0
        return np.ascontiguousarray(a[::-1]).astype(np.float32)  # Unity stores bottom row first
    except Exception as e:
        print(f"  lightmap {tex.m_Name} fmt {fmt}: {e!r}"[:200], file=sys.stderr)
        return None


def _minmax(x, default):
    """Unity MinMaxCurve -> one representative value (constant or middle of two constants)."""
    if not isinstance(x, dict):
        return default
    state, hi, lo = x.get("minMaxState", 0), x.get("scalar", default), x.get("minScalar", default)
    return (hi + lo) / 2 if state == 3 else hi


def _gradient_color(mm, t=0.0):
    """Colour of a Unity MinMaxGradient: constant (state 0), gradient (1), two colours (2,
    averaged) or two gradients (3), sampled at t (0..1)."""
    white = {"r": 1.0, "g": 1.0, "b": 1.0, "a": 1.0}

    def grad(g):
        if not g:
            return white
        nc, na = g.get("m_NumColorKeys", 0), g.get("m_NumAlphaKeys", 0)
        keys = [(g.get(f"ctime{i}", 0) / 65535.0, g.get(f"key{i}", white)) for i in range(max(nc, 1))]
        akeys = [(g.get(f"atime{i}", 0) / 65535.0, g.get(f"key{i}", white).get("a", 1.0)) for i in range(max(na, 1))]

        def lerp(ks, f):
            if t <= ks[0][0]:
                return f(ks[0][1])
            for (t0, v0), (t1, v1) in zip(ks, ks[1:]):
                if t <= t1:
                    u = (t - t0) / max(t1 - t0, 1e-6)
                    return [a + (b - a) * u for a, b in zip(f(v0), f(v1))]
            return f(ks[-1][1])
        rgb = lerp(keys, lambda k: [k.get("r", 1), k.get("g", 1), k.get("b", 1)])
        alpha = lerp(akeys, lambda v: [v])[0]
        return {"r": rgb[0], "g": rgb[1], "b": rgb[2], "a": alpha}
    state = mm.get("minMaxState", 0)
    if state == 1:
        return grad(mm.get("maxGradient"))
    if state == 2:
        a, b = mm.get("minColor", white), mm.get("maxColor", white)
        return {ch: (a.get(ch, 1) + b.get(ch, 1)) / 2 for ch in "rgba"}
    if state == 3:
        a, b = grad(mm.get("minGradient")), grad(mm.get("maxGradient"))
        return {ch: (a[ch] + b[ch]) / 2 for ch in "rgba"}
    c = mm.get("maxColor", white)
    return {ch: c.get(ch, 1.0) for ch in "rgba"}


N_SAMPLES = 9


def _eval_curve(keys, t):
    """Unity AnimationCurve (Hermite keyframes) at t."""
    if not keys:
        return 1.0
    if t <= keys[0]["time"]:
        return keys[0]["value"]
    for a, b in zip(keys, keys[1:]):
        if t <= b["time"]:
            dt = max(b["time"] - a["time"], 1e-6)
            u = (t - a["time"]) / dt
            m0, m1 = a.get("outSlope", 0) * dt, b.get("inSlope", 0) * dt
            if abs(m0) == float("inf") or abs(m1) == float("inf"):
                return a["value"]  # stepped
            u2, u3 = u * u, u * u * u
            return (2 * u3 - 3 * u2 + 1) * a["value"] + (u3 - 2 * u2 + u) * m0 + (-2 * u3 + 3 * u2) * b["value"] + (u3 - u2) * m1
    return keys[-1]["value"]


def _curve_range(x, default):
    """Unity MinMaxCurve -> [[min, max] at N_SAMPLES evenly spaced points of 0..1] (a
    constant gives equal points). Random-between-two-constants / curves keep both ends."""
    if not isinstance(x, dict):
        return [[default, default]] * N_SAMPLES
    state = x.get("minMaxState", 0)
    hi, lo = x.get("scalar", default), x.get("minScalar", default)
    out = []
    for k in range(N_SAMPLES):
        t = k / (N_SAMPLES - 1)
        if state == 0:
            v = (hi, hi)
        elif state == 1:
            c = hi * _eval_curve(x.get("maxCurve", {}).get("m_Curve", []), t)
            v = (c, c)
        elif state == 2:
            v = (hi * _eval_curve(x.get("minCurve", {}).get("m_Curve", []), t), hi * _eval_curve(x.get("maxCurve", {}).get("m_Curve", []), t))
        else:
            v = (lo, hi)
        out.append([round(min(v), 5), round(max(v), 5)])
    return out


def _start_range(x, default):
    """MinMaxCurve read at the start (startLifetime, startSize ...): [min, max]."""
    r = _curve_range(x, default)
    return r[0]


def _shader_blend(r):
    """How a particle renderer's material blends: "add", "mul" or "alpha" (by shader name)."""
    try:
        mat = deref(list(r.m_Materials)[0])
        name = shader_display_name(deref(mat.m_Shader)).lower()
    except Exception:
        return "add", ""
    try:
        floats = {(k if isinstance(k, str) else getattr(k, "name", str(k))): v for k, v in mat.m_SavedProperties.m_Floats}
    except Exception:
        floats = {}
    # Cubic/Universal: _SrcBlend/_DstBlend are Unity BlendModes (1 One, 5 SrcAlpha,
    # 10 OneMinusSrcAlpha, 2 DstColor, 0 Zero)
    src, dst = floats.get("_SrcBlend"), floats.get("_DstBlend")
    if dst == 1.0:
        return "add", f"{name} {src}/{dst}"
    if src == 2.0 or (src == 0.0 and dst == 3.0):
        return "mul", f"{name} {src}/{dst}"
    if dst == 10.0:
        return "alpha", f"{name} {src}/{dst}"
    if "multiply" in name:
        return "mul", name
    if "additive" in name or "add" in name.split("/")[-1] or "glow" in name:
        return "add", name
    return "alpha", name


def particle_settings(comps):
    """A ParticleSystem's settings for the game's small particle runtime (particles.rs):
    emission (bursts and rate over the duration), start values as [min, max] ranges, the
    emitter shape, the over-lifetime modules sampled at N_SAMPLES points (size, colour,
    velocity, texture-sheet frame), and how the renderer draws (billboard / stretched,
    additive / alpha)."""
    ps = comps.get("ParticleSystem")
    if not ps:
        return {}
    try:
        d = ps[0].deref().read_typetree()
    except Exception:
        return {}
    if os.environ.get("PG3D_DUMP_PS"):
        try:
            name = ps[0].deref().read().m_GameObject.read().m_Name
        except Exception:
            name = "?"
        with open(os.environ["PG3D_DUMP_PS"], "a") as f:
            f.write(json.dumps({"name": name, "ps": d}, default=str) + "\n")
    init, uv = d.get("InitialModule", {}), d.get("UVModule", {})
    start_col = init.get("startColor", {})
    c = _gradient_color(start_col)
    col = d.get("ColorModule", {})
    out = {
        # the old single values (kept for older readers)
        "life": _minmax(init.get("startLifetime"), 0.2),
        "size": _minmax(init.get("startSize"), 0.5),
        "color": [c["r"], c["g"], c["b"], c["a"]],
        "tiles": [uv.get("tilesX", 1), uv.get("tilesY", 1)] if uv.get("enabled") else [1, 1],
        "row": uv.get("rowIndex", 0),
        "single_row": uv.get("animationType", 0) == 1,
        "max": init.get("maxNumParticles", 0),
        "v": 2,
        "duration": d.get("lengthInSec", 1.0),
        "play_on_awake": bool(d.get("playOnAwake", 1)),
        "looping": bool(d.get("looping", 0)),
        "world": d.get("moveWithTransform", 0) == 1,
        "sim_speed": d.get("simulationSpeed", 1.0),
        "life_range": _start_range(init.get("startLifetime"), 0.2),
        "speed_range": _start_range(init.get("startSpeed"), 0.0),
        "size_range": _start_range(init.get("startSize"), 0.5),
        "rotation_range": _start_range(init.get("startRotation"), 0.0),
        "gravity": _minmax(init.get("gravityModifier"), 0.0),
        # start colour: the two ends of a random-between (equal for one colour)
        "color_range": [
            [c2[ch] for ch in "rgba"]
            for c2 in (
                (_gradient_color({"minMaxState": 0, "maxColor": start_col.get("minColor", {})}), _gradient_color({"minMaxState": 0, "maxColor": start_col.get("maxColor", {})}))
                if start_col.get("minMaxState") == 2
                else (c, c)
            )
        ],
    }
    em = d.get("EmissionModule", {})
    if em.get("enabled", 1):
        out["rate"] = _minmax(em.get("rateOverTime"), 0.0)
        bursts = []
        for b in em.get("m_Bursts", []):
            if "countCurve" in b:
                lo, hi = _start_range(b["countCurve"], 0)
            else:
                lo, hi = b.get("minCount", 0), b.get("maxCount", 0)
            bursts.append({"time": b.get("time", 0.0), "min": lo, "max": hi, "cycles": b.get("cycleCount", 1), "interval": b.get("repeatInterval", 0.01)})
        out["bursts"] = bursts
    else:
        out["rate"], out["bursts"] = 0.0, []
    sh = d.get("ShapeModule", {})
    if sh.get("enabled"):
        rad = sh.get("radius", 1.0)
        rad = rad.get("value", 1.0) if isinstance(rad, dict) else rad
        out["shape"] = {"type": sh.get("type", 0), "angle": sh.get("angle", 25.0), "radius": rad, "length": sh.get("length", 5.0),
                        "random_dir": sh.get("randomDirectionAmount", sh.get("randomDirection", 0)) or 0}
    size = d.get("SizeModule", {})
    if size.get("enabled"):
        out["size_curve"] = [r[1] for r in _curve_range(size.get("curve"), 1.0)]
    rot = d.get("RotationModule", {})
    if rot.get("enabled"):
        out["spin_range"] = _start_range(rot.get("curve"), 0.0)
    if col.get("enabled"):
        out["color_curve"] = [[round(v, 4) for v in (lambda g: [g["r"], g["g"], g["b"], g["a"]])(_gradient_color(col.get("gradient", {}), k / (N_SAMPLES - 1)))] for k in range(N_SAMPLES)]
    vel = d.get("VelocityModule", {})
    if vel.get("enabled"):
        out["velocity"] = {"x": _start_range(vel.get("x"), 0.0), "y": _start_range(vel.get("y"), 0.0), "z": _start_range(vel.get("z"), 0.0),
                           "world": vel.get("inWorldSpace", 0) == 1}
    if uv.get("enabled"):
        out["frames"] = [r[1] for r in _curve_range(uv.get("frameOverTime"), 0.0)]
        out["start_frame"] = _start_range(uv.get("startFrame"), 0.0)
        out["uv_cycles"] = uv.get("cycles", 1.0)
    lights = d.get("LightsModule", {})
    if lights.get("enabled"):
        out["lights"] = {"ratio": lights.get("ratio", 0.0), "range": lights.get("rangeCurve", {}).get("scalar", 1.0),
                         "intensity": lights.get("intensityCurve", {}).get("scalar", 1.0), "use_color": bool(lights.get("color", 1))}
    if "ParticleSystemRenderer" in comps:
        r = deref(comps["ParticleSystemRenderer"][0])
        if r is not None:
            blend, shader = _shader_blend(r)
            out["render"] = {"mode": int(getattr(r, "m_RenderMode", 0)), "length_scale": float(getattr(r, "m_LengthScale", 2.0)),
                             "velocity_scale": float(getattr(r, "m_VelocityScale", 0.0)), "blend": blend, "shader": shader,
                             "max_size": float(getattr(r, "m_MaxParticleSize", 0.5))}
    return out


def obj_key(o):
    r = o.object_reader
    return (r.assets_file.name, r.path_id)


def unity_matrix(m):
    return np.array([[m.e00, m.e01, m.e02, m.e03], [m.e10, m.e11, m.e12, m.e13],
                     [m.e20, m.e21, m.e22, m.e23], [m.e30, m.e31, m.e32, m.e33]], dtype=np.float64)


def skin_weights(h, vc):
    """Per-vertex bone indices/weights padded to 4 (glTF JOINTS_0 / WEIGHTS_0), or (None, None)."""
    bi = getattr(h, "m_BoneIndices", None)
    if not bi or len(bi) < vc:
        return None, None
    joints = np.zeros((vc, 4), np.uint16)
    weights = np.zeros((vc, 4), np.float32)
    bw = getattr(h, "m_BoneWeights", None)
    for v in range(vc):
        idx = list(bi[v])[:4]
        joints[v, :len(idx)] = idx
        if bw and len(bw) >= vc:
            w = list(bw[v])[:4]
            weights[v, :len(w)] = w
        else:
            weights[v, 0] = 1.0  # one bone per vertex
    s = weights.sum(axis=1, keepdims=True)
    weights = np.where(s > 0, weights / np.maximum(s, 1e-9), np.array([1, 0, 0, 0], np.float32))
    return joints, weights


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


def place_prefab(ex, bundle, anchor):
    """Export the root object of a bundle (e.g. the season lobby decoration) and hang it in
    the scene at the node called `anchor` (e.g. SeasonLobbyRoot)."""
    idx = json.loads(INDEX.read_text())
    paths = glob.glob(f"{idx['root']}/{bundle}/*/*")
    if not paths:
        print(f"  no bundle {bundle}", file=sys.stderr)
        return
    seen, queue = {paths[0]}, [paths[0]]
    while queue:
        e = UnityPy.load(queue.pop())
        for sf in iter_serialized(e):
            for ext in sf.externals:
                rel = idx["cab"].get(ext.path.split("/")[-1])
                if rel and os.path.join(idx["root"], rel) not in seen:
                    seen.add(os.path.join(idx["root"], rel))
                    queue.append(os.path.join(idx["root"], rel))
    env = UnityPy.load(*sorted(seen))
    saved_env, ex.env = ex.env, env
    target = next((i for i, n in enumerate(ex.vis.doc["nodes"]) if n["name"] == anchor), None)
    bundle_files = [sf for k, f in env.files.items() if os.path.abspath(k) == os.path.abspath(paths[0])
                    for sf in serialized_in(f)]
    for o in (x for f in bundle_files for x in f.objects.values()):
        if o.type.name == "AudioClip":
            # season lobby music
            try:
                clip = o.read()
                (ex.out / "sounds").mkdir(exist_ok=True)
                for fname, data in clip.samples.items():
                    name = "music.wav" if "music" in clip.m_Name.lower() else safe(clip.m_Name) + ".wav"
                    (ex.out / "sounds" / name).write_bytes(data)
                    print(f"  sound {clip.m_Name} -> sounds/{name}")
                    break
            except Exception as e:
                print(f"  music: {e!r}"[:160], file=sys.stderr)
            continue
        if o.type.name != "Transform":
            continue
        t = o.read()
        if t.m_Father.m_PathID:
            continue
        n = ex.walk(t, np.eye(4))
        if n is None:
            continue
        if target is not None:
            ex.vis.doc["nodes"][target].setdefault("children", []).append(n)
        else:
            ex.vis.doc["scenes"][0]["nodes"].append(n)
        print(f"  placed {t.m_GameObject.read().m_Name} under {anchor if target is not None else 'scene root'}")
    ex.env = saved_env


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
    ap.add_argument("--out", help="output folder name (default: the map name)")
    ap.add_argument("--skip", nargs="*", default=[], help="GameObject names to leave out")
    ap.add_argument("--animate", nargs="*", default=[], help="objects whose legacy Animation clips to export")
    ap.add_argument("--prefab", nargs="*", default=[],
                    help="extra bundle prefabs to place: <bundle>:<scene object to put it under>")
    a = ap.parse_args()
    if a.list or not a.map:
        return list_maps()
    env, scene_path, nbundles = load_env(a.map)
    out = OUT_ROOT / (a.out or a.map)
    ex = Exporter(env, out)
    ex.unlit_keywords = False
    ex.skip_names = set(a.skip)
    ex.run(scene_path)
    for spec in a.prefab:
        place_prefab(ex, *spec.split(":", 1))
    clips = {}
    if a.animate:
        import export_weapons as ew
        for name in a.animate:
            clips[name] = ew.export_animations(ex, env, name)
    ex.save({"map": a.map, "bundles_loaded": nbundles, "clips": clips})
    print(json.dumps(dict(map=a.map, bundles=nbundles, **ex.stats)))
    print(f"-> {out}")


if __name__ == "__main__":
    main()
