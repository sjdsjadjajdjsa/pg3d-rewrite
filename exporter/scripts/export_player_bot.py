#!/usr/bin/env python3
"""Export the tuning of the game's player bots (the PlayerBot prefab in resources.assets:
AIBotController, AIBotTargetDetector, PlayerBotMovement) as private/export/bots/player_bot.json.

    tools/venv/bin/python scripts/export_player_bot.py
"""
import json
import pathlib
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
CLASSES = ["AIBotController", "AIBotTargetDetector", "PlayerBotMovement"]


def main():
    rows = [l.split("\t") for l in subprocess.run([sys.executable, str(ROOT / "scripts/find_behaviours.py"), *CLASSES],
                                                  capture_output=True, text=True).stdout.splitlines()]
    rows = [r for r in rows if len(r) == 4 and r[2] == "PlayerBot"]
    with tempfile.TemporaryDirectory() as d:
        idx, dump = pathlib.Path(d, "b.tsv"), pathlib.Path(d, "b.json")
        # name each row by its class so the dump keeps all three
        idx.write_text("".join(f"{p}\t{c}\t{c}\t{pid}\n" for p, c, _g, pid in rows))
        subprocess.run([sys.executable, str(ROOT / "scripts/dump_behaviour.py"), str(idx), str(dump)], capture_output=True)
        raw = json.loads(dump.read_text())
    out = {c: {k: v for k, v in raw.get(c, {}).items() if not k.startswith("m_") and not isinstance(v, dict)} for c in CLASSES}
    p = ROOT / "private/export/bots/player_bot.json"
    p.write_text(json.dumps(out, indent=1))
    print(f"player bot: {', '.join(f'{c} {len(v)}' for c, v in out.items())} -> {p}")


if __name__ == "__main__":
    main()
