#!/usr/bin/env python3
"""Reads game scripts whose layout changed since layouts.json was recorded.

A MonoBehaviour is its fields one after another, with no names or markers in the data. When
the game adds, removes or changes a field, the saved layout reads past the change shifted
and fails. This lines the saved fields up with the bytes again, like aligning two texts:
going field by field through a sample of the script's objects at once, it keeps the best few
ways of reading so far (a beam), where a way may drop a saved field (the game removed it)
or skip a field the game added (a few bytes, a string or a list). Every reading has to stay
plausible in every sample (object references that resolve to the right kind of object,
readable strings, lists that fit, floats that aren't garbage, zero alignment padding) and end
exactly at each object's size. Ways are ranked by how many fields read as values they had in
the recorded build (most values survive an update), minus a cost per change. Fields the game
added come back unnamed; the saved fields keep their names.

    Repair(saved_root, known).plan(samples) -> edits for every object of the script
"""
import json
import math
import struct
import zlib

ALIGN = 0x4000
SIZES = {"SInt8": 1, "UInt8": 1, "char": 1, "bool": 1, "short": 2, "SInt16": 2, "unsigned short": 2, "UInt16": 2,
         "int": 4, "SInt32": 4, "unsigned int": 4, "UInt32": 4, "Type*": 4, "float": 4,
         "long long": 8, "SInt64": 8, "unsigned long long": 8, "UInt64": 8, "FileSize": 8, "double": 8}
UNPACK = {"SInt8": "<b", "UInt8": "<B", "char": "<B", "bool": "<?", "short": "<h", "SInt16": "<h",
          "unsigned short": "<H", "UInt16": "<H", "int": "<i", "SInt32": "<i", "unsigned int": "<I", "UInt32": "<I",
          "Type*": "<I", "float": "<f", "long long": "<q", "SInt64": "<q", "unsigned long long": "<Q",
          "UInt64": "<Q", "FileSize": "<Q", "double": "<d"}
UNPACK = {k: struct.Struct(v).unpack_from for k, v in UNPACK.items()}
HEADER = ("m_GameObject", "m_Enabled", "m_Script", "m_Name")
MAX_EDITS = 3
EDIT_COST = 3        # per object: a change has to buy more than this in matching values
BEAM = 128
SAMPLE = 8
MATCH, EMPTY_MATCH, MISMATCH = 3, 1, -1
_i32 = struct.Struct("<i").unpack_from
_i64 = struct.Struct("<q").unpack_from


class Bad(Exception):
    pass


class Unsupported(Exception):
    pass


# ---------------------------------------------------------------- values

def canon(v):
    """A value without what changes between game builds (object ids), for comparing."""
    if isinstance(v, dict):
        if set(v) == {"m_FileID", "m_PathID"}:
            return None
        return {k: canon(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [canon(x) for x in v]
    if isinstance(v, float):
        return round(v, 5)
    if isinstance(v, (bytes, bytearray)):
        return bytes(v).hex()
    if isinstance(v, bool):
        return int(v)
    return v


def _empty(v):
    if isinstance(v, dict):
        return all(_empty(x) for x in v.values())
    if isinstance(v, list):
        return all(_empty(x) for x in v)
    return v in (None, 0, "", False)


def fingerprint(v):
    """A small hash of a field's value. Zero and empty values hash to their negative, so
    they can weigh less."""
    c = canon(v)
    h = zlib.crc32(json.dumps(c, sort_keys=True, default=str).encode()) + 1
    return -h if _empty(c) else h


def field_index(classes):
    """class name (full and short) -> its field names, from layouts.json's "classes"."""
    out = {}
    for key, rows in classes.items():
        full = key.split("/", 1)[1]
        names = tuple(r[2] for r in rows if r[0] == 1)[len(HEADER):]
        out[full] = names
        out.setdefault(full.rsplit(".", 1)[-1], names)
    return out


# ---------------------------------------------------------------- what references point to

# native Unity classes and the classes a reference to them also accepts (None: any)
SUBCLASSES = {
    "Object": None, "Component": None, "Behaviour": None, "ScriptableObject": {"MonoBehaviour"},
    "Transform": {"Transform", "RectTransform"},
    "Renderer": {"MeshRenderer", "SkinnedMeshRenderer", "SpriteRenderer", "ParticleSystemRenderer", "LineRenderer",
                 "TrailRenderer", "BillboardRenderer"},
    "Collider": {"BoxCollider", "SphereCollider", "CapsuleCollider", "MeshCollider", "TerrainCollider", "WheelCollider"},
    "Texture": {"Texture2D", "RenderTexture", "Cubemap", "Texture3D", "Texture2DArray", "CustomRenderTexture"},
}
NATIVE = {"MonoBehaviour", "GameObject", "AudioClip", "Material", "Mesh", "Sprite", "Shader", "Font", "AnimationClip",
          "Animator", "Animation", "Camera", "Light", "AudioSource", "Rigidbody", "TextAsset", "Texture2D",
          "ParticleSystem", "MeshFilter", "MeshRenderer", "SkinnedMeshRenderer", "SpriteRenderer", "RectTransform",
          "Canvas", "CanvasGroup", "LineRenderer", "TrailRenderer", "RenderTexture", "Cubemap", "AudioMixer",
          "RuntimeAnimatorController", "AnimatorController", "PhysicMaterial", "BoxCollider", "SphereCollider",
          "CapsuleCollider", "MeshCollider", "CharacterController", "NavMeshAgent", "Terrain", "ReflectionProbe"}


class Files:
    """What references in one asset file point to: Unity class, and script class for scripts."""

    def __init__(self, sf, layouts=None):
        self.sf, self.layouts, self.kinds = sf, layouts or {}, {}
        self.externals, self.ids = len(sf.externals), sf.objects.keys()
        self._ext = {}

    def external_ids(self, fid):
        """The object ids of the file a reference with this file id points into, when that
        file is loaded, else None."""
        if fid not in self._ext:
            name = self.sf.externals[fid - 1].path.rsplit("/", 1)[-1].lower()
            found = None
            env = getattr(self.sf, "environment", None)
            stack = list((getattr(env, "files", None) or {}).values()) if env else []
            while stack and found is None:
                f = stack.pop()
                if (getattr(f, "name", None) or "").lower() == name and hasattr(f, "objects"):
                    found = f.objects.keys()
                stack.extend((getattr(f, "files", None) or {}).values())
            self._ext[fid] = found
        return self._ext[fid]

    def kind(self, pid):
        if pid not in self.kinds:
            o = self.sf.objects[pid]
            script = None
            if o.type.name == "MonoBehaviour":
                try:
                    s = o.parse_monobehaviour_head().m_Script.deref_parse_as_object()
                    script = f"{s.m_Namespace}.{s.m_ClassName}" if s.m_Namespace else s.m_ClassName
                except Exception:
                    script = None
            self.kinds[pid] = (o.type.name, script)
        return self.kinds[pid]

    def check(self, ptype, pid):
        """1 for a reference of type PPtr<ptype> to object pid of exactly that class, 0 when
        it may be; Bad when it can't be."""
        want = ptype[5:-1].lstrip("$")
        native, script = self.kind(pid)
        if want in SUBCLASSES:
            ok = SUBCLASSES[want]
            if ok is not None and native not in ok and native != want:
                raise Bad
            return 0
        if script is None:
            if want in NATIVE:
                if native != want:
                    raise Bad
                return 1
            return 0
        if script.rsplit(".", 1)[-1] == want:
            return 1
        base, mine = self.layouts.get(want), self.layouts.get(script)
        if base is not None and mine is not None and mine[:len(base)] != base:
            raise Bad
        return 0


class Data:
    """One object's bytes plus what its references may point to."""

    def __init__(self, raw, files):
        self.b, self.n, self.files = bytes(raw), len(raw), files


# ---------------------------------------------------------------- reading

def _align(d, o):
    a = (o + 3) & ~3
    if a > d.n or any(d.b[o:a]):
        raise Bad
    return a


def _check_float(v):
    if math.isnan(v) or (v != 0 and not math.isinf(v) and abs(v) < 1e-30) or (not math.isinf(v) and abs(v) > 3.5e38):
        raise Bad


def _min_size(node):
    t = node.m_Type
    if t in SIZES:
        return SIZES[t]
    if t == "string" or (node.m_Children and node.m_Children[0].m_Type == "Array"):
        return 4
    return sum(_min_size(c) for c in node.m_Children)


def read(node, d, o):
    """One field at offset o: (new offset, type evidence, value). Raises Bad when the bytes
    can't be this field."""
    t = node.m_Type
    ev = 0
    # the layouts lack the header's m_Enabled alignment (Unity aligns it; UnityPy gets away
    # with it because the next field realigns)
    align = node.m_MetaFlag & ALIGN or node.m_Name == "m_Enabled"
    if t in SIZES:
        s = SIZES[t]
        if o + s > d.n:
            raise Bad
        v = UNPACK[t](d.b, o)[0]
        if t == "bool" and d.b[o] > 1:
            raise Bad
        if t in ("float", "double"):
            _check_float(v)
        o += s
    elif t == "string":
        if o + 4 > d.n:
            raise Bad
        ln = _i32(d.b, o)[0]
        if ln < 0 or o + 4 + ln > d.n:
            raise Bad
        try:
            v = d.b[o + 4:o + 4 + ln].decode("utf-8")
        except UnicodeDecodeError:
            raise Bad from None
        if any(c < " " and c not in "\t\n\r" for c in v):
            raise Bad
        o = _align(d, o + 4 + ln)
        align = 0
    elif t == "TypelessData":
        if o + 4 > d.n:
            raise Bad
        ln = _i32(d.b, o)[0]
        if ln < 0 or o + 4 + ln > d.n:
            raise Bad
        v = d.b[o + 4:o + 4 + ln]
        o += 4 + ln
    elif t in ("ReferencedObject", "ManagedReferencesRegistry"):
        raise Unsupported
    elif node.m_Children and node.m_Children[0].m_Type == "Array":
        arr = node.m_Children[0]
        align |= arr.m_MetaFlag & ALIGN
        sub = arr.m_Children[1]
        if o + 4 > d.n:
            raise Bad
        count = _i32(d.b, o)[0]
        o += 4
        size = _min_size(sub)
        if count < 0 or count * max(size, 1) > d.n - o:
            raise Bad
        if sub.m_Type in SIZES and sub.m_Type not in ("bool", "float", "double"):
            v = d.b[o:o + count * size]
            o += count * size
            if sub.m_MetaFlag & ALIGN:
                o = _align(d, o)
        else:
            v = []
            for _ in range(count):
                o, e, x = read(sub, d, o)
                ev += e
                v.append(x)
    elif t.startswith("PPtr<"):
        if o + 12 > d.n:
            raise Bad
        fid, pid = _i32(d.b, o)[0], _i64(d.b, o + 4)[0]
        if fid < 0 or fid > d.files.externals or (fid == 0 and pid != 0 and pid not in d.files.ids) \
                or (fid > 0 and pid == 0) or 0 < (abs(pid) >> 32) < 1 << 16:
            # (object ids are small numbers or random 64-bit hashes; two small numbers side
            # by side read as one id are a shifted reading)
            raise Bad
        if fid > 0:
            ids = d.files.external_ids(fid)
            if ids is not None and pid not in ids:
                raise Bad
        if fid == 0 and pid:
            ev = d.files.check(t, pid)
        v = {"m_FileID": fid, "m_PathID": pid}
        o += 12
    else:
        v = {}
        for c in node.m_Children:
            o, e, v[c.m_Name] = read(c, d, o)
            ev += e
    if align:
        o = _align(d, o)
    return o, ev, v


def values(root, d):
    """field name -> fingerprint, reading d with the layout as it is (for recording)."""
    out, o = {}, 0
    for f in root.m_Children:
        o, _, v = read(f, d, o)
        out[f.m_Name] = fingerprint(v)
    if o != d.n:
        raise Bad
    return out


# ---------------------------------------------------------------- what the game may have added

def _rows_string(name):
    return [(1, "string", name, 0), (2, "Array", "Array", ALIGN), (3, "int", "size", 0), (3, "char", "data", 0)]


def _rows_vector(name, k):
    rows = [(1, "vector", name, 0), (2, "Array", "Array", ALIGN), (3, "int", "size", 0)]
    if k == 1:
        return rows + [(3, "UInt8", "data", 0)]
    return rows + [(3, "Element", "data", 0)] + [(4, "int", f"i{j}", 0) for j in range(k // 4)]


def _rows_bytes(name, k):
    return [(1, "UInt8", f"{name}_{j}", 0) for j in range(k)]


# a field the game added: fixed bytes, a string, or a list of k-byte elements
ADDED = [("bytes", k) for k in (4, 8, 12, 16, 1, 2, 3, 20, 24, 28, 32, 36, 40, 48, 64)] + \
        [("string", 0)] + [("vector", k) for k in (4, 12, 1, 8, 16, 24, 28, 32)]


def _added_rows(kind, k, name):
    if kind == "string":
        return _rows_string(name)
    if kind == "vector":
        return _rows_vector(name, k)
    return _rows_bytes(name, k)


_ADDED_NODES = {}


def _added_node(kind, k):
    """A single-field node for reading an added field with read()."""
    key = (kind, k)
    if key not in _ADDED_NODES:
        from UnityPy.helpers.TypeTreeNode import TypeTreeNode
        rows = [(0, "Added", "Base", 0)] + _added_rows(kind, k, "x")
        _ADDED_NODES[key] = TypeTreeNode.from_list([TypeTreeNode(lv, t, nm, 0, 0, m_MetaFlag=fl) for lv, t, nm, fl in rows])
    return _ADDED_NODES[key]


def _skip(kind, k, d, o):
    if kind == "bytes":
        if o + k > d.n:
            raise Bad
        return o + k
    node = _added_node(kind, k)
    for c in node.m_Children:
        o = read(c, d, o)[0]
    return o


# ---------------------------------------------------------------- repair

class Repair:
    def __init__(self, root, known=None):
        """root: the saved layout. known: field name -> fingerprints of its values in the
        recorded build."""
        self.root, self.known = root, known or {}
        self.fields = list(root.m_Children)
        names = tuple(f.m_Name for f in self.fields[:len(HEADER)])
        self.first = len(HEADER) if names == HEADER else 0

    def _score(self, name, ev, v):
        seen = self.known.get(name)
        if not seen:
            return ev
        f = fingerprint(v)
        return ev + ((MATCH if f > 0 else EMPTY_MATCH) if f in seen else MISMATCH)

    def _read_all(self, i, offs, samples):
        """Field i in every sample: (new offsets, score), or None."""
        new, score = [], 0
        try:
            for o, d in zip(offs, samples):
                o2, ev, v = read(self.fields[i], d, o)
                new.append(o2)
                score += self._score(self.fields[i].m_Name, ev, v)
        except Bad:
            return None
        return tuple(new), score

    def search(self, samples, keep=2):
        """The best ways to read every sample under one set of edits: [(score, edits)], edits
        = [(field index, "drop") | (field index, (kind, k))] in order, best first."""
        F, S = len(self.fields), len(samples)
        # hypothesis: score, offsets per sample, edits, changes used, gaps right before field i
        beam = [(0, (0,) * S, (), 0, 0)]
        try:
            for i in range(F):
                editable = i >= self.first
                layer = list(beam)
                if editable:
                    frontier, kinds = beam, ADDED
                    for _ in range(2):
                        grown = []
                        for score, offs, path, used, gaps in frontier:
                            if used >= MAX_EDITS:
                                continue
                            for kind, k in kinds:
                                try:
                                    new = tuple(_skip(kind, k, d, o) for o, d in zip(offs, samples))
                                except Bad:
                                    continue
                                grown.append((score - EDIT_COST * S, new, path + ((i, (kind, k)),), used + 1, gaps + 1))
                        layer += grown
                        # a second added field right after a string or list (fixed-size runs
                        # are already covered by the larger byte counts)
                        frontier = [h for h in grown if h[2][-1][1][0] != "bytes"]
                        kinds = [("bytes", 4), ("string", 0), ("vector", 4), ("vector", 12)]
                nxt = {}
                for score, offs, path, used, gaps in layer:
                    r = self._read_all(i, offs, samples)
                    if r is not None:
                        h = (score + r[1], r[0], path, used, 0)
                        key = (r[0], used)
                        if key not in nxt or nxt[key][0] < h[0]:
                            nxt[key] = h
                    if editable and used < MAX_EDITS and not gaps:
                        h = (score - EDIT_COST * S, offs, path + ((i, "drop"),), used + 1, 0)
                        key = (offs, used + 1, "d")
                        if key not in nxt or nxt[key][0] < h[0]:
                            nxt[key] = h
                beam = sorted(nxt.values(), key=lambda h: -h[0])[:BEAM]
                if not beam:
                    return []
        except Unsupported:
            return []
        done = []
        for score, offs, path, used, _ in beam:
            if all(o == d.n for o, d in zip(offs, samples)):
                done.append((score, list(path)))
            elif used < MAX_EDITS:
                for kind, k in ADDED:
                    try:
                        if all(_skip(kind, k, d, o) == d.n for o, d in zip(offs, samples)):
                            done.append((score - EDIT_COST * S, list(path) + [(F, (kind, k))]))
                            break
                    except Bad:
                        continue
        done.sort(key=lambda x: -x[0])
        # the same edits can be reached twice (a drop and a skip that read alike)
        out, seen = [], set()
        for score, path in done:
            key = tuple(path)
            if key not in seen:
                seen.add(key)
                out.append((score, path))
            if len(out) >= keep:
                break
        return out

    def plan(self, samples):
        """Edits for the script from a sample of its objects: (edits, unsure). unsure: another
        set of edits scored about as well, so some fields may read the wrong values."""
        found = self.search(samples[:SAMPLE])
        if not found:
            return None, False
        unsure = len(found) > 1 and found[0][0] - found[1][0] < EDIT_COST * min(len(samples), SAMPLE)
        return found[0][1], unsure

    def edits_for(self, d, plan, reads):
        """This object's edits: the script's plan when the object reads with it, else its own."""
        if plan and reads(plan):
            return plan
        found = self.search([d], keep=1)
        if found and reads(found[0][1]):
            return found[0][1]
        return None

    def node(self, edits, node_cls):
        """The saved layout with these edits, as a TypeTreeNode."""
        flat = [(0, self.root.m_Type, self.root.m_Name, self.root.m_MetaFlag or 0)]
        k = 0
        for i in range(len(self.fields) + 1):
            dropped = False
            while k < len(edits) and edits[k][0] == i:
                e = edits[k][1]
                if e == "drop":
                    dropped = True
                else:
                    flat += _added_rows(e[0], e[1], f"_added{k}")
                k += 1
            if i < len(self.fields) and not dropped:
                flat += [(n.m_Level, n.m_Type, n.m_Name, n.m_MetaFlag or 0) for n in self.fields[i].traverse()]
        return node_cls.from_list([node_cls(lv, t, nm, 0, 0, m_MetaFlag=fl) for lv, t, nm, fl in flat])
