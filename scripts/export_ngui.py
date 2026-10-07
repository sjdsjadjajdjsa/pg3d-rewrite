#!/usr/bin/env python3
"""Export an NGUI user interface (the game's menus) for the Bevy rewrite.

    tools/venv/bin/python scripts/export_ngui.py level17 MainMenuNGUI lobby

  level17       game scene file holding the UI (level17 = Assets/Sources/Scenes/Menu_Custom.unity)
  MainMenuNGUI  name of the UI root GameObject in that scene
  lobby         output name -> private/export/ui/lobby/

NGUI (the old Unity UI plugin the game uses) builds screens out of "widgets": sprites cut
from texture atlases, bitmap-font labels and plain textures, placed with transform offsets
and edge anchors. We write:
    ui.json            flat list of nodes (parent index, transform, widget/sprite/label/... data)
    atlases/<a>.png    atlas textures      + atlases.json  sprite rectangles and 9-slice borders
    fonts/<f>.png      bitmap font pages   + fonts.json    glyph metrics
    textures/<t>.png   textures used by UITexture widgets
"""
import json
import os
import pathlib
import sys

import UnityPy

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import export_weapons as ew  # noqa: E402
import find_behaviours as fb  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
G = fb.GAME
WIDGETS = {"UIWidget", "UISprite", "UILabel", "UITexture", "UI2DSprite"}
KEEP = WIDGETS | {"UIPanel", "UIButton", "UIRoot", "UIToggle", "UIScrollView", "UIGrid", "UITable", "UISlider", "UIProgressBar"}


class Ngui:
    def __init__(self, level):
        files = [f"{G}/{level}", f"{G}/resources.assets", f"{G}/globalgamemanagers.assets"]
        files += sorted(str(p) for p in pathlib.Path(G).glob("sharedassets*.assets"))
        self.env = UnityPy.load(*files)
        self.by_name = {os.path.basename(k): f for k, f in self.env.files.items()}
        self.level = self.by_name[level]
        self.gen = ew.typetree_generator()
        self.filters = {}
        self.classes = {}
        self.atlases, self.fonts, self.textures = {}, {}, {}
        self.grafts = {}

    def prefab_root(self, name):
        """A prefab's root GameObject (no parent transform) in resources.assets."""
        f = self.by_name["resources.assets"]
        for o in f.objects.values():
            if o.type.name in ("Transform", "RectTransform"):
                t = o.read(check_read=False)
                if t.m_Father.path_id == 0:
                    g = t.m_GameObject.read()
                    if g.m_Name == name:
                        return g
        raise KeyError(name)

    # ---- low level helpers
    def resolve(self, owner_file, ptr):
        """PPtr dict/object -> ObjectReader (or None)."""
        fid = ptr["m_FileID"] if isinstance(ptr, dict) else ptr.m_FileID
        pid = ptr["m_PathID"] if isinstance(ptr, dict) else ptr.m_PathID
        if not pid:
            return None
        f = owner_file if fid == 0 else self.by_name.get(owner_file.externals[fid - 1].path.split("/")[-1])
        return None if f is None else f.objects.get(pid)

    def cls(self, o):
        key = (o.assets_file.name, o.path_id)
        if key not in self.classes:
            self.env.typetree_generator = None  # plain read keeps m_Script intact
            try:
                self.classes[key] = fb.script_name(o, o.read(check_read=False).m_Script, {})
            except Exception:
                self.classes[key] = None
        return self.classes[key]

    def ref(self, owner, ptr):
        """Remember an object reference; turned into a node index after the walk."""
        if not ptr or not ptr.get("m_PathID"):
            return None
        o = self.resolve(owner, ptr)
        return None if o is None else ["ref", o.assets_file.name, o.path_id]

    def tree(self, o):
        self.env.typetree_generator = self.gen
        return o.read_typetree()

    # ---- assets
    def texture_png(self, tex_obj, folder, out):
        name = f"{tex_obj.read().m_Name}_{tex_obj.path_id & 0xffffff:x}"
        path = f"{folder}/{em_safe(name)}.png"
        # Unity filter mode (0 = point: drawn with nearest-pixel sampling, e.g. GameAtlas)
        self.filters[path] = tex_obj.read().m_TextureSettings.m_FilterMode
        if not (out / path).exists():
            (out / folder).mkdir(parents=True, exist_ok=True)
            tex_obj.read().image.save(out / path)
        return path

    def material_texture(self, mat_obj):
        mat = mat_obj.read()
        for k, te in mat.m_SavedProperties.m_TexEnvs:
            name = k if isinstance(k, str) else getattr(k, "name", str(k))
            if name == "_MainTex" and te.m_Texture.m_PathID:
                return self.resolve(mat_obj.assets_file, te.m_Texture)
        return None

    def atlas(self, owner, ptr, out):
        o = self.resolve(owner, ptr)
        seen = 0
        while o is not None:
            d = self.tree(o)
            rep = self.resolve(o.assets_file, d.get("mReplacement", {"m_FileID": 0, "m_PathID": 0}))
            if rep is None or seen > 5:
                break
            o, seen = rep, seen + 1
        if o is None:
            return None
        key = f"{o.assets_file.name}:{o.path_id}"
        if key not in self.atlases:
            mat = self.resolve(o.assets_file, d["material"])
            tex = self.material_texture(mat) if mat else None
            if tex is None:
                self.atlases[key] = None
                return None
            sprites = {s["name"]: [s["x"], s["y"], s["width"], s["height"],
                                   s["borderLeft"], s["borderRight"], s["borderTop"], s["borderBottom"],
                                   s["paddingLeft"], s["paddingRight"], s["paddingTop"], s["paddingBottom"]]
                       for s in d["mSprites"]}
            img = tex.read()
            # 13th value: 1 if a 9-slice sprite's middle is fully transparent (a frame);
            # NGUI never samples it, but a stretched GPU texture would smear the border in
            alpha = img.image.convert("RGBA").getchannel("A")
            for name, v in sprites.items():
                x, y, w, h, bl, br, bt, bb = v[:8]
                clear = 0
                if bl + br + bt + bb > 0 and w - bl - br > 0 and h - bt - bb > 0:
                    box = (int(x + bl), int(y + bt), int(x + w - br), int(y + h - bb))
                    lo, hi = alpha.crop(box).getextrema()
                    clear = 1 if hi < 16 else 0
                v.append(clear)
            self.atlases[key] = {"name": o.read(check_read=False).m_GameObject.read().m_Name if False else key,
                                 "png": self.texture_png(tex, "atlases", out),
                                 "size": [img.m_Width, img.m_Height], "pixel_size": d.get("mPixelSize", 1.0),
                                 "sprites": sprites}
        return key if self.atlases[key] else None

    def font(self, owner, ptr, out):
        o = self.resolve(owner, ptr)
        seen = 0
        while o is not None:
            d = self.tree(o)
            rep = self.resolve(o.assets_file, d.get("mReplacement", {"m_FileID": 0, "m_PathID": 0}))
            if rep is None or seen > 5:
                break
            o, seen = rep, seen + 1
        if o is None:
            return None
        key = f"{o.assets_file.name}:{o.path_id}"
        if key in self.fonts:
            return key if self.fonts[key] else None
        bm = d["mFont"]
        entry = {"size": bm["mSize"], "base": bm["mBase"], "glyphs": {}}
        offset = [0, 0]
        if d["mAtlas"]["m_PathID"]:
            # font living inside an atlas sprite: glyph rects are relative to that sprite
            a = self.atlas(o.assets_file, d["mAtlas"], out)
            if a is None:
                self.fonts[key] = None
                return None
            sp = self.atlases[a]["sprites"].get(bm["mSpriteName"])
            if sp:
                offset = [sp[0], sp[1]]
            entry["png"], entry["page"] = self.atlases[a]["png"], self.atlases[a]["size"]
        else:
            mat = self.resolve(o.assets_file, d["mMat"])
            tex = self.material_texture(mat) if mat else None
            if tex is None:
                self.fonts[key] = None
                return None
            entry["png"] = self.texture_png(tex, "fonts", out)
            img = tex.read()
            entry["page"] = [img.m_Width, img.m_Height]
        for g in bm["mSaved"]:
            entry["glyphs"][str(g["index"])] = [g["x"] + offset[0], g["y"] + offset[1], g["width"], g["height"],
                                                g["offsetX"], g["offsetY"], g["advance"]]
            # BMGlyph.kerning: flat [previous char, amount, ...] pairs
            k = g.get("kerning") or []
            if k:
                entry.setdefault("kerning", {})[str(g["index"])] = {str(k[i]): k[i + 1] for i in range(0, len(k) - 1, 2)}
        self.fonts[key] = entry
        return key

    # ---- the widget tree
    def export(self, root_name, out):
        root = next(o.read() for o in self.level.objects.values()
                    if o.type.name == "GameObject" and o.read().m_Name == root_name)
        nodes, tr_index, pending_anchors = [], {}, []

        def transform_of(go):
            for c in go.m_Component:
                p = getattr(c, "component", c)
                r = p.deref()
                if r.type.name in ("Transform", "RectTransform"):
                    return r
            return None

        def walk(go, parent):
            t_obj = transform_of(go)
            t = t_obj.read()
            idx = len(nodes)
            tr_index[(t_obj.assets_file.name, t_obj.path_id)] = idx
            tr_index[(go.object_reader.assets_file.name, go.object_reader.path_id)] = idx
            p, s = t.m_LocalPosition, t.m_LocalScale
            q = t.m_LocalRotation
            node = {"name": go.m_Name, "parent": parent, "active": bool(go.m_IsActive),
                    "pos": [p.x, p.y, p.z], "scale": [s.x, s.y], "rot_z": z_angle(q)}
            nodes.append(node)
            for c in go.m_Component:
                r = getattr(c, "component", c).deref()
                if r.type.name != "MonoBehaviour":
                    continue
                k = self.cls(r)
                # every script on the node, so game code that moves/scales it at run time
                # (layout helpers, controllers) can be found
                node.setdefault("scripts", []).append(k)
                if k not in KEEP:
                    continue
                try:
                    d = self.tree(r)
                except Exception as e:
                    print(f"  {go.m_Name} {k}: {e!r}"[:160], file=sys.stderr)
                    continue
                self.component(node, k, d, r, out, pending_anchors, idx)
            for ch in t.m_Children:
                cobj = ch.deref()
                walk(cobj.read().m_GameObject.read(), idx)
            # prefabs the game instantiates under this node at run time (--graft node=prefab)
            if go.m_Name in self.grafts:
                gi = len(nodes)
                walk(self.prefab_root(self.grafts[go.m_Name]), idx)
                nodes[gi]["name"] += "(Clone)"

        walk(root, None)
        # anchors point at transforms/GameObjects/widgets: turn them into node indices
        for idx, side, a, owner in pending_anchors:
            tgt = self.resolve(owner, a["target"])
            ti = None
            if tgt is not None:
                ti = tr_index.get((tgt.assets_file.name, tgt.path_id))
                if ti is None and tgt.type.name == "MonoBehaviour":
                    g = tgt.read(check_read=False).m_GameObject
                    gobj = self.resolve(tgt.assets_file, g)
                    ti = tr_index.get((gobj.assets_file.name, gobj.path_id)) if gobj else None
            nodes[idx]["widget"]["anchors"][side] = {"target": ti, "rel": a["relative"], "abs": a["absolute"]}
        # other references (slider foreground / thumb) -> node indices
        for nd in nodes:
            sl = nd.get("slider")
            if not sl:
                continue
            for k in ("fg", "thumb"):
                v = sl.get(k)
                if isinstance(v, list):
                    f = self.by_name.get(v[1])
                    o = f.objects.get(v[2]) if f else None
                    ti = tr_index.get((v[1], v[2]))
                    if ti is None and o is not None and o.type.name == "MonoBehaviour":
                        g = self.resolve(o.assets_file, o.read(check_read=False).m_GameObject)
                        ti = tr_index.get((g.assets_file.name, g.path_id)) if g else None
                    sl[k] = ti
        return nodes

    def component(self, node, k, d, r, out, pending, idx):
        owner = r.assets_file
        if k in WIDGETS:
            c = d["mColor"]
            w = {"w": d["mWidth"], "h": d["mHeight"], "depth": d["mDepth"], "pivot": d["mPivot"],
                 "color": [c["r"], c["g"], c["b"], c["a"]], "enabled": bool(d["m_Enabled"]), "anchors": {},
                 # keepAspectRatio: 0 free, 1 based on width, 2 based on height
                 "aspect": [d.get("keepAspectRatio", 0), d.get("aspectRatio", 1.0)]}
            for side in ("leftAnchor", "rightAnchor", "bottomAnchor", "topAnchor"):
                a = d.get(side)
                if a and a["target"]["m_PathID"]:
                    pending.append((idx, side[:-6], a, owner))
            node["widget"] = w
            node["kind"] = k
        if k == "UISprite":
            a = self.atlas(owner, d["mAtlas"], out) if d["mAtlas"]["m_PathID"] else None
            node["sprite"] = {"atlas": a, "name": d["mSpriteName"], "type": d["mType"], "flip": d["mFlip"],
                              "fill_center": bool(d.get("mFillCenter", 1)), "fill_amount": d.get("mFillAmount", 1.0),
                              "fill_dir": d.get("mFillDirection", 4)}
        elif k == "UILabel":
            f = self.font(owner, d["mFont"], out) if d["mFont"]["m_PathID"] else None
            ec = d["mEffectColor"]
            node["label"] = {"text": d["mText"], "font": f, "size": d["mFontSize"], "align": d["mAlignment"],
                             "overflow": d["mOverflow"], "effect": d["mEffectStyle"],
                             "effect_color": [ec["r"], ec["g"], ec["b"], ec["a"]],
                             "effect_dist": [d["mEffectDistance"]["x"], d["mEffectDistance"]["y"]],
                             "spacing": [d["mSpacingX"], d["mSpacingY"]], "max_lines": d["mMaxLineCount"],
                             "gradient": bool(d.get("mApplyGradient", 0)),
                             "gradient_top": [d.get("mGradientTop", {}).get(c, 1.0) for c in "rgba"],
                             "gradient_bottom": [d.get("mGradientBottom", {}).get(c, 1.0) for c in "rgba"],
                             "key": d.get("mTextTerm", "")}
        elif k == "UITexture":
            tex = self.resolve(owner, d["mTexture"]) if d["mTexture"]["m_PathID"] else None
            rc = d["mRect"]
            node["texture"] = {"png": self.texture_png(tex, "textures", out) if tex and tex.type.name == "Texture2D" else None,
                               "uv": [rc["x"], rc["y"], rc["width"], rc["height"]]}
            # a custom material (e.g. the SDF icon shader of the PC key tips): its shader name
            # and number/colour settings
            mat = self.resolve(owner, d["mMat"]) if d.get("mMat", {}).get("m_PathID") else None
            if mat is not None and mat.type.name == "Material":
                m = mat.read()
                sh = m.m_Shader.deref()
                props = m.m_SavedProperties
                node["texture"]["material"] = {
                    "shader": sh.read().m_ParsedForm.m_Name if sh is not None and hasattr(sh.read(), "m_ParsedForm") else (sh.read().m_Name if sh else None),
                    "floats": {(k if isinstance(k, str) else k.name): v for k, v in props.m_Floats},
                    "colors": {(k if isinstance(k, str) else k.name): [v.r, v.g, v.b, v.a] for k, v in props.m_Colors},
                }
        elif k == "UIPanel":
            node["panel"] = {"alpha": d["mAlpha"], "depth": d["mDepth"], "clipping": d["mClipping"],
                             "clip": [d["mClipRange"]["x"], d["mClipRange"]["y"], d["mClipRange"]["z"], d["mClipRange"]["w"]]}
        elif k == "UIButton":
            node["button"] = {"hover": d.get("hoverSprite", ""), "pressed": d.get("pressedSprite", ""),
                              "on_click": [e.get("mMethodName", "") for e in d.get("onClick", [])]}
        elif k == "UIRoot":
            node["root"] = {"manual_height": d["manualHeight"], "manual_width": d["manualWidth"],
                            "fit_width": bool(d["fitWidth"]), "fit_height": bool(d["fitHeight"])}
        elif k in ("UISlider", "UIProgressBar"):
            node["slider"] = {"value": d.get("mValue", d.get("rawValue", 1.0)), "steps": d.get("numberOfSteps", 0),
                              "fill_dir": d.get("mFill", 0),
                              "fg": self.ref(r.assets_file, d.get("mFG")), "thumb": self.ref(r.assets_file, d.get("thumb"))}
        elif k in ("UIToggle", "UIScrollView", "UIGrid", "UITable"):
            node.setdefault("tags", []).append(k)


def em_safe(s):
    return "".join(ch if ch.isalnum() or ch in "-_" else "_" for ch in s)[:80]


def z_angle(q):
    import math
    return math.degrees(2 * math.atan2(q.z, q.w))


def export_strings(lang="en-gb"):
    """The game's translations live in bundles l_<lang>: Key_xxxx -> text. Labels store keys."""
    import glob
    files = glob.glob(f"{G}/StreamingAssets/Cache/bundles/l_{lang}/*/*")
    if not files:
        return
    a = UnityPy.load(files[0])
    a.typetree_generator = ew.typetree_generator()
    for o in a.objects:
        if o.type.name == "MonoBehaviour" and len(o.get_raw_data()) > 100000:
            d = o.read_typetree()
            if "mTerms" in d:
                strings = {t["term"]: t["translation"] for t in d["mTerms"]}
                out = ROOT / "private/export/ui" / f"strings_{lang}.json"
                out.write_text(json.dumps(strings, ensure_ascii=False))
                print(f"strings: {len(strings)} ({lang}) -> {out}")
                return


def main():
    level, root, name = sys.argv[1:4]
    out = ROOT / "private/export/ui" / name
    out.mkdir(parents=True, exist_ok=True)
    ng = Ngui(level)
    for a in sys.argv[4:]:
        if a.startswith("--graft="):
            node, prefab = a[len("--graft="):].split("=")
            ng.grafts[node] = prefab
    nodes = ng.export(root, out)
    (out / "ui.json").write_text(json.dumps(nodes))
    (out / "atlases.json").write_text(json.dumps({k: v for k, v in ng.atlases.items() if v}))
    (out / "fonts.json").write_text(json.dumps({k: v for k, v in ng.fonts.items() if v}))
    (out / "filters.json").write_text(json.dumps(ng.filters))
    kinds = {}
    for n in nodes:
        kinds[n.get("kind", "-")] = kinds.get(n.get("kind", "-"), 0) + 1
    export_strings()
    print(f"{name}: {len(nodes)} nodes {kinds}, {len(ng.atlases)} atlases, {len(ng.fonts)} fonts -> {out}")


if __name__ == "__main__":
    main()
