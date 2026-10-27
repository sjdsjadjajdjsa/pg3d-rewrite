#!/usr/bin/env python3
"""One command from a Pixel Gun 3D install to the files the Rust rewrite plays.

    python scripts/setup.py                  find the game, install the packages, export everything
    python scripts/setup.py --game <folder>  use this install (remembered for next time)
    python scripts/setup.py --find [folder]  only print where the game is (exit 1: not found)
    python scripts/setup.py arena hill       export just these maps (plus menus, weapons, bots)

Works on Linux and Windows with any Python 3.9+. Everything it makes stays in this folder:
tools/ (Python packages, Cpp2IL) and private/ (the exported files). The game install is
only read, never changed.
"""
import hashlib
import os
import pathlib
import shutil
import subprocess
import sys
import urllib.request

import gamepaths

ROOT = gamepaths.ROOT
VENV = ROOT / "tools/venv"
VENV_PY = VENV / ("Scripts/python.exe" if sys.platform == "win32" else "bin/python")
REQUIREMENTS = ROOT / "requirements.txt"
CPP2IL_VERSION = "2022.1.0-pre-release.21"
CPP2IL_ASSET = {"win32": "Windows.exe", "darwin": "OSX"}.get(sys.platform, "Linux")
CPP2IL = ROOT / "tools/cpp2il" / ("cpp2il.exe" if sys.platform == "win32" else "cpp2il")
DUMMYDLL = ROOT / "private/cpp2il/dummydll"


def say(text):
    print(f"[setup] {text}", flush=True)


def packages():
    if not VENV_PY.exists():
        say("making a Python environment in tools/venv")
        if subprocess.run([sys.executable, "-m", "venv", str(VENV)]).returncode != 0:
            sys.exit("could not make a Python environment (on Debian/Ubuntu: sudo apt install python3-venv)")
    stamp = VENV / "requirements.sha1"
    want = hashlib.sha1(REQUIREMENTS.read_bytes()).hexdigest()
    if stamp.exists() and stamp.read_text().strip() == want:
        return
    say("installing Python packages (UnityPy, numpy, pillow)")
    r = subprocess.run([str(VENV_PY), "-m", "pip", "install", "-q", "--disable-pip-version-check", "-r", str(REQUIREMENTS)])
    if r.returncode != 0:
        sys.exit("installing the Python packages failed (see above)")
    stamp.write_text(want)


def game_build(install):
    data = gamepaths.data_dir(install)
    asm = install / "GameAssembly.dll"
    return f"{gamepaths.unity_version(data)} {asm.stat().st_size if asm.exists() else 0}"


def cpp2il(install):
    """Field layouts of the game's scripts (Cpp2IL dummy DLLs) make the menus, HUD and some
    sounds readable. Newer game builds encrypt the metadata Cpp2IL needs: then the layouts
    saved in layouts.json (recorded from the October 2026 build) are used instead."""
    marker = ROOT / "private/cpp2il/build.txt"
    build = game_build(install)
    if marker.exists() and marker.read_text().strip() == build:
        return
    marker.parent.mkdir(parents=True, exist_ok=True)
    if not CPP2IL.exists():
        url = f"https://github.com/SamboyCoding/Cpp2IL/releases/download/{CPP2IL_VERSION}/Cpp2IL-{CPP2IL_VERSION}-{CPP2IL_ASSET}"
        say("downloading Cpp2IL (reads the game's script layouts)")
        CPP2IL.parent.mkdir(parents=True, exist_ok=True)
        try:
            urllib.request.urlretrieve(url, CPP2IL)
            CPP2IL.chmod(0o755)
        except OSError as e:
            say(f"Cpp2IL download failed ({e}); menus will use the simple versions")
            return
    say("reading the game's script layouts with Cpp2IL")
    exe = gamepaths.data_dir(install).name[: -len("_Data")]
    for old in DUMMYDLL.glob("*.dll"):
        old.unlink()
    try:
        # attributeinjector keeps [SerializeField] on private fields (NGUI's mWidth, mColor, ...)
        subprocess.run([str(CPP2IL), "--game-path", str(install), "--exe-name", exe, "--use-processor", "attributeinjector",
                        "--output-as", "dummydll",
                        "--output-to", str(DUMMYDLL)], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                       stderr=subprocess.DEVNULL, timeout=1800)
    except (OSError, subprocess.TimeoutExpired):
        pass
    if any(DUMMYDLL.glob("*.dll")):
        say("script layouts read from this game build")
    else:
        say("this game build encrypts its script layouts: using the saved ones (layouts.json)")
    marker.write_text(build)


def new_game_build(install):
    """Indexes and script layouts belong to one game build: start over when it changed."""
    stamp = ROOT / "private/game_build.txt"
    build = f"{install} {game_build(install)}"
    if stamp.exists() and stamp.read_text(encoding="utf-8").strip() == build:
        return
    if stamp.exists():
        say("different game build: re-indexing")
    for old in [ROOT / "private/index", ROOT / "private/cpp2il"]:
        shutil.rmtree(old, ignore_errors=True)
    stamp.parent.mkdir(parents=True, exist_ok=True)
    stamp.write_text(build, encoding="utf-8")


def main():
    args = sys.argv[1:]
    if sys.version_info < (3, 9):
        sys.exit("Python 3.9 or newer is needed")
    if args[:1] == ["--find"]:
        found = gamepaths.install_dir(args[1]) if len(args) > 1 else gamepaths.find()
        if found is None:
            sys.exit(1)
        print(found)
        return
    if args[:1] == ["--game"]:
        if len(args) < 2:
            sys.exit("usage: setup.py --game <folder>")
        found = gamepaths.install_dir(args[1])
        if found is None:
            sys.exit(f"not a Pixel Gun 3D install: {args[1]}")
        gamepaths.save(found)
        args = args[2:]
    install = gamepaths.find()
    if install is None:
        sys.exit("Pixel Gun 3D not found. Install \"Pixel Gun 3D: PC Edition\" from Steam (free), "
                 "or pass its folder: python scripts/setup.py --game <folder>")
    say(f"game: {install}")
    packages()
    new_game_build(install)
    cpp2il(install)
    say("exporting (a few minutes)")
    env = dict(os.environ, PG3D_GAME=str(install))
    sys.exit(subprocess.run([str(VENV_PY), str(ROOT / "scripts/export_all.py"), *args], cwd=ROOT, env=env).returncode)


if __name__ == "__main__":
    main()
