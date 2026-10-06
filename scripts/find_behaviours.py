#!/usr/bin/env python3
"""Find which bundles/assets contain MonoBehaviours of the given script classes.

    tools/venv/bin/python scripts/find_behaviours.py WeaponSounds [OtherClass ...]

Prints: bundle (relative), class, GameObject name, path_id. Read-only on the Steam install.
"""
import glob
import os
import sys
from concurrent.futures import ProcessPoolExecutor

import UnityPy

GAME = os.path.expanduser("~/.local/share/Steam/steamapps/common/Pixel Gun 3D PC Edition/Pixel Gun 3D_Data")
TARGET = set(sys.argv[1:])


def scan(path):
    out = []
    try:
        assets = UnityPy.load(path)
        scripts = {}
        for o in assets.objects:
            if o.type.name == "MonoScript":
                try:
                    scripts[(o.assets_file.name, o.path_id)] = o.read().m_ClassName
                except Exception:
                    pass
        if not TARGET & set(scripts.values()):
            return out
        for o in assets.objects:
            if o.type.name != "MonoBehaviour":
                continue
            try:
                mb = o.read(check_read=False)
                r = mb.m_Script.deref()
                name = scripts.get((r.assets_file.name, r.path_id))
                if name in TARGET:
                    go = mb.m_GameObject.read() if mb.m_GameObject.m_PathID else None
                    out.append((os.path.relpath(path, GAME), name, go.m_Name if go else "", o.path_id))
            except Exception:
                pass
    except Exception:
        pass
    return out


def main():
    files = ([f"{GAME}/resources.assets"] + glob.glob(f"{GAME}/sharedassets*.assets")
             + glob.glob(f"{GAME}/StreamingAssets/Cache/bundles/*/*/*"))
    with ProcessPoolExecutor(os.cpu_count()) as ex:
        for res in ex.map(scan, files, chunksize=4):
            for row in res:
                print("\t".join(map(str, row)), flush=True)


if __name__ == "__main__":
    main()
