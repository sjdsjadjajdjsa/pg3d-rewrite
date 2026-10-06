#!/usr/bin/env python3
"""Index the game's asset bundles: which bundle file contains which internal "CAB" file.

Unity bundles reference each other by internal file name (CAB-<hash>), e.g. a map's scene
bundle points to meshes living in "archive:/CAB-4c9e.../CAB-4c9e...". This builds the
lookup table CAB name -> bundle path so the exporter can load dependencies.

Output: private/index/bundles.json   (read-only access to the Steam install)
"""
import json
import os
import pathlib
import sys
from concurrent.futures import ProcessPoolExecutor

import UnityPy

ROOT = pathlib.Path(__file__).resolve().parent.parent
GAME = pathlib.Path(os.path.expanduser(
    "~/.local/share/Steam/steamapps/common/Pixel Gun 3D PC Edition/Pixel Gun 3D_Data"))
BUNDLES = GAME / "StreamingAssets/Cache/bundles"
OUT = ROOT / "private/index/bundles.json"


def scan(path: str):
    try:
        env = UnityPy.load(path)
        names = []
        for bundle in env.files.values():
            names.extend(getattr(bundle, "files", {}).keys())
        return path, names, None
    except Exception as e:  # corrupt/unsupported bundle: record and move on
        return path, [], repr(e)


def main():
    paths = sorted(str(p) for p in BUNDLES.glob("*/*/*") if p.is_file())
    cab, bundles, errors = {}, {}, {}
    with ProcessPoolExecutor(max_workers=os.cpu_count()) as ex:
        for i, (path, names, err) in enumerate(ex.map(scan, paths, chunksize=8)):
            rel = os.path.relpath(path, BUNDLES)
            bundles[rel] = names
            for n in names:
                cab[n] = rel
            if err:
                errors[rel] = err
            if i % 250 == 0:
                print(f"{i}/{len(paths)}", file=sys.stderr)
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps({"root": str(BUNDLES), "cab": cab, "bundles": bundles, "errors": errors}, indent=1))
    print(f"bundles={len(paths)} cab_files={len(cab)} errors={len(errors)} -> {OUT}")


if __name__ == "__main__":
    main()
