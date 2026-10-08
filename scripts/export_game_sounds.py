#!/usr/bin/env python3
"""Export the game's non-weapon sounds the remake uses, named after the original fields:

  sounds/player/<field>.wav   Player_move_c (damage, death, headshot, pickups, weapon switch
                              per category, ...), SkinName (jump, land, crouch, walk) and
                              PlayerMovementsSoundsController (footstep lists: <field>_<n>.wav)
                              of the Player prefab
  sounds/ui/click.wav         ButtonClickSound.Click: the click every NGUI button plays
  sounds/lobby/<field>.wav    LobbyMainMenuSounds (level17 MainMenuNGUI)

Writes private/export/sounds/... and sounds/index.json ({group: {field: [files]}}).

    tools/venv/bin/python scripts/export_game_sounds.py
"""
import json
import os
import pathlib
import sys

import UnityPy

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import export_weapons as ew  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
GAME = os.path.expanduser("~/.local/share/Steam/steamapps/common/Pixel Gun 3D PC Edition/Pixel Gun 3D_Data")
OUT = ROOT / "private/export/sounds"

# (group, asset file, script class, GameObject name)
TARGETS = [
    ("player", "resources.assets", "Player_move_c", "PlayerMoveC"),
    ("player", "resources.assets", "SkinName", "Player"),
    ("player", "resources.assets", "PlayerMovementsSoundsController", "Player"),
    ("lobby", "level17", "LobbyMainMenuSounds", "MainMenuNGUI"),
    ("match", "resources.assets", "InGameGUI", "InGameGUI"),
    ("match", "resources.assets", "NetworkStartTableNGUIController", "NetworkStartTableNGUI"),
    ("arena", "resources.assets", "ZombieCreator", "ArenaMode"),
    ("level", "sharedassets1.assets", "ExperienceController", "ExperienceController"),
]


def clips(tree, sf, files):
    """field -> [AudioClip objects] (arrays keep their order)."""
    out = {}

    def obj(ptr):
        fid, pid = ptr["m_FileID"], ptr["m_PathID"]
        if not pid:
            return None
        target = sf if fid == 0 else files.get(sf.externals[fid - 1].path.split("/")[-1]) if fid <= len(sf.externals) else None
        o = target.objects.get(pid) if target is not None else None
        return o if o is not None and o.type.name == "AudioClip" else None

    for k, v in tree.items():
        if isinstance(v, dict) and set(v) == {"m_FileID", "m_PathID"}:
            o = obj(v)
            if o is not None:
                out[k] = [o]
        elif isinstance(v, list) and v and isinstance(v[0], dict) and set(v[0]) == {"m_FileID", "m_PathID"}:
            os_ = [o for o in map(obj, v) if o is not None]
            if os_:
                out[k] = os_
    return out


def main():
    import subprocess
    from UnityPy.helpers.TypeTreeGenerator import TypeTreeGenerator
    index = {}
    gen = TypeTreeGenerator("2022.3.76f1")
    gen.load_local_dll_folder(str(ROOT / "private/cpp2il/dummydll"))
    classes = sorted({t[2] for t in TARGETS})
    rows = [l.split("\t") for l in subprocess.run([sys.executable, str(ROOT / "scripts/find_behaviours.py"), *classes],
                                                    capture_output=True, text=True).stdout.splitlines()]
    picked = {}
    for path, cls, go, pid in rows:
        for group, fname, want, go_name in TARGETS:
            if path == fname and cls == want and (go_name is None or go == go_name):
                picked.setdefault((fname, want, go_name), (group, int(pid)))
    extra = [f"{GAME}/resources.assets", f"{GAME}/globalgamemanagers.assets"] + sorted(str(p) for p in pathlib.Path(GAME).glob("sharedassets*.assets"))
    for fname in sorted({k[0] for k in picked}):
        env = UnityPy.load(*([f"{GAME}/{fname}"] + [e for e in extra if not e.endswith("/" + fname)]))
        env.typetree_generator = gen
        files = {f.name: f for f in env.files.values()}
        sf = files[fname]
        for (f, want, go_name), (group, pid) in picked.items():
            if f != fname:
                continue
            tree = sf.objects[pid].read_typetree()
            (OUT / group).mkdir(parents=True, exist_ok=True)
            for field, objs in clips(tree, sf, files).items():
                names = []
                for n, c in enumerate(objs):
                    clip = c.read()
                    rel = f"{group}/{ew.em.safe(field)}{'' if len(objs) == 1 else f'_{n}'}.wav"
                    for _k, data in clip.samples.items():
                        (OUT / rel).write_bytes(data)
                        names.append(rel)
                        break
                if names:
                    index.setdefault(group, {})[field] = names
    # ButtonClickSound.Click lives on a singleton made at run time (the prefab buttons' own
    # Click fields are empty): the game's generic button sound is resources.assets
    # "snd_button_click"
    res = UnityPy.load(f"{GAME}/resources.assets")
    for o in res.objects:
        if o.type.name == "AudioClip" and o.peek_name() == "snd_button_click":
            (OUT / "ui").mkdir(parents=True, exist_ok=True)
            for _k, data in o.read().samples.items():
                (OUT / "ui/click.wav").write_bytes(data)
                index.setdefault("ui", {})["Click"] = ["ui/click.wav"]
                break
            break
    (OUT / "index.json").write_text(json.dumps(index, indent=1))
    print("sounds: " + ", ".join(f"{g} {len(v)}" for g, v in index.items()) + f" -> {OUT}")


if __name__ == "__main__":
    main()
