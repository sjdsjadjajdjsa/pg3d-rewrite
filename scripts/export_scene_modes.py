#!/usr/bin/env python3
"""Which maps each game mode offers, from the game's built-in SceneInfoController config.

SceneInfoController (sharedassets1.assets) carries `jsonConfig`, the default of the
server-side scene config: {"all": {scene: {...}}, "modes": {ModeName: {scene: {"L": unlock
level, "C": ...}}}}. Written as private/export/maps/modes.json: {ModeName: [scene, ...]}.
(TimeBattle is the "Arena" survival mode; the Arena_* variant scenes are only offered by it.)

    tools/venv/bin/python scripts/export_scene_modes.py
"""
import json
import pathlib

import UnityPy
import gamepaths

ROOT = pathlib.Path(__file__).resolve().parent.parent
GAME = str(gamepaths.DATA)


def main():
    gen = gamepaths.typetree_generator()
    env = UnityPy.load(f"{GAME}/sharedassets1.assets")
    env.typetree_generator = gen
    for o in env.objects:
        if o.type.name != "MonoBehaviour":
            continue
        try:
            d = o.read_typetree()
        except Exception:
            continue
        if "jsonConfig" not in d or "modeInfo" not in d:
            continue
        modes = json.loads(d["jsonConfig"])["modes"]
        out = {m: sorted(maps) for m, maps in modes.items()}
        dst = ROOT / "private/export/maps/modes.json"
        dst.write_text(json.dumps(out, indent=1))
        print(f"{len(out)} modes -> {dst}")
        return
    raise SystemExit("SceneInfoController not found")


if __name__ == "__main__":
    main()
