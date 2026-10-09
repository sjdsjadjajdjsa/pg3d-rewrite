#!/usr/bin/env python3
"""Export the crosshairs: every weapon prefab carries a WeaponCustomCrosshair whose _dataId
picks one of the 54 CrosshairData entries of the CrosshairsSo asset (resources.assets).
Each entry names atlas sprites (GameAtlas "aim_N") for the centre, the top, bottom and
left arms (the right arm is the left one mirrored) and the corners, with an offset from
the centre and a size in NGUI pixels; AimCrosshairController lays them out and pushes the
arms out with the weapon's scatter.

    tools/venv/bin/python scripts/export_crosshairs.py

Writes private/export/ui/crosshairs/crosshairs.json
    {"crosshairs": {id: {part: {"sprite", "offset": [x, y], "size": [w, h]}}},
     "weapons": {"Weapon2": 3, ...}}
and the sprites as <sprite>.png, cut from the in-game atlas (export_ngui.py ... ingame).
"""
import json
import pathlib
import subprocess
import sys
import tempfile

from PIL import Image

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "private/export/ui/crosshairs"
UI = ROOT / "private/export/ui/ingame"
PARTS = ("center", "up", "down", "left", "leftUp", "leftDown")


def dump(rows):
    with tempfile.TemporaryDirectory() as d:
        idx, js = pathlib.Path(d, "i.tsv"), pathlib.Path(d, "o.json")
        idx.write_text("".join("\t".join(r) + "\n" for r in rows))
        subprocess.run([sys.executable, str(ROOT / "scripts/dump_behaviour.py"), str(idx), str(js)], capture_output=True)
        return json.loads(js.read_text()) if js.exists() else {}


def main():
    found = subprocess.run([sys.executable, str(ROOT / "scripts/find_behaviours.py"), "CrosshairsSo", "WeaponCustomCrosshair"],
                           capture_output=True, text=True).stdout.splitlines()
    rows = [l.split("\t") for l in found if l.count("\t") == 3]
    so = next(r for r in rows if r[1] == "CrosshairsSo")
    data = dump([[so[0], so[1], "CrosshairsSo", so[3]]])["CrosshairsSo"]["Crosshairs"]
    exported = {p.name for p in (ROOT / "private/export/weapons").iterdir() if (p / "weapon.gltf").exists()}
    wanted, seen = [], set()
    for r in rows:
        if r[1] == "WeaponCustomCrosshair" and r[2] in exported and r[2] not in seen:
            seen.add(r[2])
            wanted.append(r)
    weapons = {name: int(v.get("_dataId", 0)) for name, v in dump(wanted).items()}

    OUT.mkdir(parents=True, exist_ok=True)
    atlases = json.loads((UI / "atlases.json").read_text())
    atlas = next(a for a in atlases.values() if "aim_1" in a["sprites"])
    sheet = Image.open(UI / atlas["png"]).convert("RGBA")
    crosshairs, sprites = {}, set()
    for c in data:
        parts = {}
        for p in PARTS:
            s = c.get(p) or {}
            if s.get("spriteName") and s["spriteName"] in atlas["sprites"]:
                parts[p] = {"sprite": s["spriteName"], "offset": [s["offset"]["x"], s["offset"]["y"]],
                            "size": [s["spriteSize"]["x"], s["spriteSize"]["y"]]}
                sprites.add(s["spriteName"])
        crosshairs[str(c["ID"])] = parts
    for name in sorted(sprites):
        x, y, w, h = atlas["sprites"][name][:4]
        sheet.crop((x, y, x + w, y + h)).save(OUT / f"{name}.png")
    (OUT / "crosshairs.json").write_text(json.dumps({"crosshairs": crosshairs, "weapons": weapons}, indent=1))
    print(f"crosshairs: {len(crosshairs)} kinds, {len(sprites)} sprites, {len(weapons)} weapons -> {OUT}")


if __name__ == "__main__":
    main()
