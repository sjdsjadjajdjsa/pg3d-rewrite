#!/usr/bin/env python3
"""Export the game's SceneInfo assets (one per map: display names, description, size) as
private/export/maps/scenes.json: {map (lower case): {"name", "short", "description", "gravity"}} with
the en-gb strings filled in. The mode picker shows "short" (keyTranslateShortName).

    tools/venv/bin/python scripts/export_scene_info.py
"""
import json
import pathlib
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
PY = sys.executable


def main():
    strings = json.loads((ROOT / "private/export/ui/strings_en-gb.json").read_text())
    tr = lambda k: strings.get(k, k if k and not k.startswith("Key_") else "").replace("\n", " ").strip()
    with tempfile.TemporaryDirectory() as d:
        idx, dump = pathlib.Path(d, "s.tsv"), pathlib.Path(d, "s.json")
        idx.write_text(subprocess.run([PY, ROOT / "scripts/find_behaviours.py", "SceneInfo"],
                                      capture_output=True, text=True).stdout)
        subprocess.run([PY, ROOT / "scripts/dump_behaviour.py", idx, dump], capture_output=True)
        raw = json.loads(dump.read_text())
    out = {k.lower(): {"name": tr(v.get("keyTranslateName", "")), "short": tr(v.get("keyTranslateShortName", "")),
                       "description": tr(v.get("keyTranslateDescription", "")),
                       # Physics.gravity.y on this map (-9.81 normally; Matrix -4.9, Space -6.5)
                       "gravity": float(v.get("gravity", -9.81) or -9.81),
                       "slope_limit": float(v.get("playerSlopeLimit", 0) or 0),
                       "sliding": bool(v.get("playerSurfacesSlidingEnabled", 0))}
           for k, v in raw.items() if isinstance(v, dict)}
    # The Arena-mode maps' SceneInfos carry copied placeholder keys ("Cupid" on the ice
    # rink, "Winter" on the mine): the game never shows their names (Arena picks the map
    # itself). Use the game's own strings where it has them, else the wiki's names
    # (pg3d.wiki.gg, pixelgun.fandom.com/wiki/Arena_(PG3D)).
    fixed = {
        "arena_swamp": tr("Key_0517"),        # Foggy Swamp
        "arena_space": tr("Key_0519"),        # Space Station
        "arena_underwater": tr("Key_0529"),   # Atlantis
        "arena_hockey": "Ice Hockey Arena",
        "arena_mine": "Abandoned Mineshaft",
        "arena_castle": "Medieval Castle",
        "arena": tr("Key_0509"),              # Deadly Arena
        "arena2": tr("Key_0509"),
        # "New Map 2" is an untranslated placeholder; the starting map
        "new_start_map3": tr("Key_11236"),    # Training ground
    }
    for k, v in fixed.items():
        if v:
            out.setdefault(k, {"name": v, "short": v, "description": ""}).update(name=v, short=v)
    p = ROOT / "private/export/maps/scenes.json"
    p.write_text(json.dumps(out, indent=1, ensure_ascii=False))
    print(f"scenes: {len(out)} -> {p}")


if __name__ == "__main__":
    main()
