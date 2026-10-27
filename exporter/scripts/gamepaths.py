#!/usr/bin/env python3
"""Where Pixel Gun 3D (PC Edition) is installed, on Linux and Windows.

Every script reads the game through INSTALL (the folder with "Pixel Gun 3D.exe") and DATA
(its "Pixel Gun 3D_Data" folder). Lookup order: PG3D_GAME, the folder saved by
`setup.py --game`, then every Steam library on this computer.

    python scripts/gamepaths.py              print the install folder (exit 1: not found)
    python scripts/gamepaths.py <folder>     check a folder (the install, its _Data folder or the .exe)
"""
import os
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SAVED = ROOT / "private/game_path.txt"
STEAM_DIR = "Pixel Gun 3D PC Edition"


def data_dir(install):
    """The install's Unity data folder, or None when this is not a Pixel Gun 3D install."""
    for d in sorted(pathlib.Path(install).glob("*_Data")):
        if (d / "globalgamemanagers").is_file() and (d / "StreamingAssets").is_dir():
            return d
    return None


def install_dir(path):
    """The install folder for a path the user picked: the install, its _Data folder, a file in it."""
    p = pathlib.Path(path).expanduser()
    if p.is_file():
        p = p.parent
    for c in (p, p.parent, p / STEAM_DIR):
        if data_dir(c):
            return c.resolve()
    return None


def steam_roots():
    roots = []
    if sys.platform == "win32":
        try:
            import winreg
            for hive in (winreg.HKEY_CURRENT_USER, winreg.HKEY_LOCAL_MACHINE):
                for key in (r"Software\Valve\Steam", r"Software\WOW6432Node\Valve\Steam"):
                    try:
                        with winreg.OpenKey(hive, key) as k:
                            for name in ("SteamPath", "InstallPath"):
                                try:
                                    roots.append(pathlib.Path(winreg.QueryValueEx(k, name)[0]))
                                except OSError:
                                    pass
                    except OSError:
                        pass
        except ImportError:
            pass
        for env in ("ProgramFiles(x86)", "ProgramFiles"):
            if os.environ.get(env):
                roots.append(pathlib.Path(os.environ[env]) / "Steam")
    else:
        home = pathlib.Path.home()
        roots += [
            home / ".local/share/Steam",
            home / ".steam/steam",
            home / ".steam/root",
            home / ".var/app/com.valvesoftware.Steam/.local/share/Steam",
            home / "snap/steam/common/.local/share/Steam",
        ]
    return roots


def steam_libraries():
    libs = []
    for root in steam_roots():
        libs.append(root)
        vdf = root / "steamapps/libraryfolders.vdf"
        if vdf.is_file():
            text = vdf.read_text(encoding="utf-8", errors="replace")
            libs += [pathlib.Path(p.replace("\\\\", "\\")) for p in re.findall(r'"path"\s+"([^"]+)"', text)]
    seen, out = set(), []
    for lib in libs:
        key = os.path.normcase(str(lib))
        if key not in seen:
            seen.add(key)
            out.append(lib)
    return out


def find():
    """The install folder, or None."""
    candidates = []
    if os.environ.get("PG3D_GAME"):
        candidates.append(os.environ["PG3D_GAME"])
    if SAVED.is_file():
        candidates.append(SAVED.read_text(encoding="utf-8").strip())
    candidates += [lib / "steamapps/common" / STEAM_DIR for lib in steam_libraries()]
    for c in candidates:
        found = c and install_dir(c)
        if found:
            return found
    return None


def save(install):
    SAVED.parent.mkdir(parents=True, exist_ok=True)
    SAVED.write_text(str(install), encoding="utf-8")


def unity_version(data):
    """The engine version the game was built with, from globalgamemanagers' header."""
    head = (pathlib.Path(data) / "globalgamemanagers").read_bytes()[:256]
    m = re.search(rb"\d{4}\.\d+\.\d+[abfpx]\d+", head)
    return m.group().decode() if m else "2022.3.76f1"


def typetree_generator():
    """Reads MonoBehaviours whose files carry no type trees, from Cpp2IL's dummy DLLs when
    they exist (older game builds; newer ones encrypt the metadata Cpp2IL needs)."""
    from UnityPy.helpers.TypeTreeGenerator import TypeTreeGenerator

    gen = TypeTreeGenerator(unity_version(_require()[1]))
    dlls = ROOT / "private/cpp2il/dummydll"
    if any(dlls.glob("*.dll")):
        gen.load_local_dll_folder(str(dlls))
    return gen


_found = None


def _require():
    global _found
    if _found is None:
        install = find()
        if install is None:
            sys.exit("Pixel Gun 3D not found. Install \"Pixel Gun 3D: PC Edition\" from Steam, "
                     "or run: python scripts/setup.py --game <its folder>")
        _found = install, data_dir(install)
    return _found


def __getattr__(name):
    if name == "INSTALL":
        return _require()[0]
    if name == "DATA":
        return _require()[1]
    raise AttributeError(name)


if __name__ == "__main__":
    found = install_dir(sys.argv[1]) if len(sys.argv) > 1 else find()
    if found is None:
        sys.exit(1)
    print(found)
