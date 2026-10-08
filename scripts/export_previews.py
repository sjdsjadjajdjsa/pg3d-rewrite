#!/usr/bin/env python3
"""Export the map loading/preview pictures (bundles level_loading_previews_*).

    tools/venv/bin/python scripts/export_previews.py

Maps without one get their full loading screen (levelloadings/loading_<map>.png).
Writes private/export/previews/<map>.png where <map> is the lower-case map name
(Loading_Arena_preview -> arena.png), used by the map picker.
"""
import glob
import pathlib
import re

import UnityPy

ROOT = pathlib.Path(__file__).resolve().parent.parent
BUNDLES = (pathlib.Path.home() / ".local/share/Steam/steamapps/common/Pixel Gun 3D PC Edition"
           / "Pixel Gun 3D_Data/StreamingAssets/Cache/bundles")
OUT = ROOT / "private/export/previews"


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    n = 0
    for f in sorted(glob.glob(str(BUNDLES / "level_loading_previews*" / "*" / "*"))):
        for o in UnityPy.load(f).objects:
            if o.type.name != "Texture2D":
                continue
            t = o.read()
            m = re.fullmatch(r"Loading_(.+)_preview", t.m_Name)
            if not m:
                continue
            img = t.image.convert("RGB")  # some have an unused, empty alpha channel
            img.thumbnail((512, 512))
            img.save(OUT / f"{m.group(1).lower()}.png")
            n += 1
    # maps without a preview there (the campaign arenas): their full loading screen,
    # assets/editor/resources/levelloadings/loading_<map>.png in the map's _data bundle
    index = ROOT / "private/index/bundle_containers.tsv"
    if index.exists():
        for line in open(index, encoding="utf-8"):
            bundle, path = line.rstrip("\n").split("\t")[:2]
            m = re.fullmatch(r"assets/editor/resources/levelloadings/loading_(.+)\.png", path)
            if not m or (OUT / f"{m.group(1).lower()}.png").exists():
                continue
            env = UnityPy.load(str(BUNDLES / bundle))
            for cpath, obj in env.container.items():
                if cpath == path:
                    img = obj.read().image.convert("RGB")
                    img.thumbnail((512, 512))
                    img.save(OUT / f"{m.group(1).lower()}.png")
                    n += 1
    print(f"previews: {n} -> {OUT}")


if __name__ == "__main__":
    main()
