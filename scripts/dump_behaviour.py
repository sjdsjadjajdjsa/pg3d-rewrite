#!/usr/bin/env python3
"""Dump the serialized fields of MonoBehaviours found by find_behaviours.py as JSON.

    tools/venv/bin/python scripts/dump_behaviour.py <index.tsv> <out.json> [GameObjectName ...]

With no names, dumps every row of the index (one JSON object keyed by GameObject name).
Field layouts come from Cpp2IL's dummy DLLs (private/cpp2il/dummydll).
"""
import collections
import json
import os
import pathlib
import sys

import UnityPy
import gamepaths

ROOT = pathlib.Path(__file__).resolve().parent.parent
GAME = str(gamepaths.DATA)


def main():
    index, out, *names = sys.argv[1:]
    rows = [line.rstrip("\n").split("\t") for line in open(index, encoding="utf-8")]
    if names:
        rows = [r for r in rows if r[2] in names]
    by_file = collections.defaultdict(list)
    for path, cls, go, pid in rows:
        by_file[path].append((go, int(pid)))
    gen = gamepaths.typetree_generator()
    result = {}
    for path, items in by_file.items():
        assets = UnityPy.load(os.path.join(GAME, path))
        assets.typetree_generator = gen
        want = dict((pid, go) for go, pid in items)
        for o in assets.objects:
            if o.path_id in want:
                try:
                    result[want[o.path_id]] = o.read_typetree()
                except Exception as e:
                    print(f"{want[o.path_id]}: {e!r}", file=sys.stderr)
    pathlib.Path(out).write_text(json.dumps(result, indent=1, default=str))
    print(f"{len(result)} objects -> {out}")


if __name__ == "__main__":
    main()
