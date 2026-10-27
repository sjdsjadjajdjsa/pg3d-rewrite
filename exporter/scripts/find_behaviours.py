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
import gamepaths

GAME = str(gamepaths.DATA)
TARGET = set(sys.argv[1:])


# The game's top-level files (levels, sharedassets, resources) keep their MonoScripts in
# globalgamemanagers.assets; bundles carry their own. Load the shared ones once.
SHARED_SCRIPTS = {}


def shared_scripts():
    if not SHARED_SCRIPTS:
        for o in UnityPy.load(f"{GAME}/globalgamemanagers.assets").objects:
            if o.type.name == "MonoScript":
                try:
                    SHARED_SCRIPTS[("globalgamemanagers.assets", o.path_id)] = o.read().m_ClassName
                except Exception:
                    pass
    return SHARED_SCRIPTS


def script_name(o, ptr, local):
    """Class name of a MonoBehaviour's script, following cross-file references by name."""
    fid, pid = ptr.m_FileID, ptr.m_PathID
    if fid == 0:
        return local.get((o.assets_file.name, pid))
    ext = o.assets_file.externals[fid - 1].path.split("/")[-1]
    return local.get((ext, pid)) or shared_scripts().get((ext, pid))


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
        bundle = "/bundles/" in path
        if bundle and not TARGET & set(scripts.values()):
            return out
        if not bundle and not TARGET & set(shared_scripts().values()) | set(scripts.values()):
            return out
        for o in assets.objects:
            if o.type.name != "MonoBehaviour":
                continue
            try:
                mb = o.read(check_read=False)
                name = script_name(o, mb.m_Script, scripts)
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
             + [p for p in glob.glob(f"{GAME}/level*") if not p.endswith(".resS")]
             + glob.glob(f"{GAME}/StreamingAssets/Cache/bundles/*/*/*"))
    with ProcessPoolExecutor(os.cpu_count()) as ex:
        for res in ex.map(scan, files, chunksize=4):
            for row in res:
                print("\t".join(map(str, row)), flush=True)


if __name__ == "__main__":
    main()
