#!/usr/bin/env python3
"""Where Pixel Gun 3D (PC Edition) is installed, on Linux and Windows.

Every script reads the game through INSTALL (the folder with "Pixel Gun 3D.exe") and DATA
(its "Pixel Gun 3D_Data" folder). Lookup order: PG3D_GAME, the folder saved by
`setup.py --game`, then every Steam library on this computer.

    python scripts/gamepaths.py              print the install folder (exit 1: not found)
    python scripts/gamepaths.py <folder>     check a folder (the install, its _Data folder or the .exe)
"""
import json
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


LAYOUTS = ROOT / "layouts.json"
REPORT = ROOT / "private/index/layout_report.json"
MAX_VALUES = 64      # recorded value fingerprints per field


def _script_key(obj):
    """"Assembly/Namespace.Class" of a MonoBehaviour's script, the key in layouts.json."""
    s = obj.parse_monobehaviour_head().m_Script.deref_parse_as_object()
    full = f"{s.m_Namespace}.{s.m_ClassName}" if s.m_Namespace else s.m_ClassName
    return f"{s.m_AssemblyName.removesuffix('.dll')}/{full}"


class SavedLayouts:
    """Field layouts of the game's scripts from layouts.json, for builds whose Cpp2IL output
    can't be made (Unity 6 builds encrypt global-metadata.dat). The layouts were recorded
    from the October 2026 build. A script that changed since is lined up again by
    layout_repair.py (see repaired())."""

    def __init__(self):
        raw = json.loads(LAYOUTS.read_text(encoding="utf-8")) if LAYOUTS.exists() else {}
        self.layouts = raw.get("classes", {})
        self.values = raw.get("values", {})
        self.cache = {}
        self.index = None
        self.plans = {}
        self.scripts = {}
        self.report = {"adjusted": {}, "unsure": [], "unreadable": {}}
        _install_repair()
        import atexit
        atexit.register(self._save_report)

    def get_nodes_up(self, assembly, fullname):
        from UnityPy.helpers.TypeTreeNode import TypeTreeNode

        key = f"{assembly.removesuffix('.dll')}/{fullname}"
        if key not in self.cache:
            nodes = self.layouts.get(key)
            self.cache[key] = nodes and TypeTreeNode.from_list(
                [TypeTreeNode(level, type_, name, 0, 0, m_MetaFlag=flag) for level, type_, name, flag in nodes]
            )
        return self.cache[key]

    def _same_script(self, obj):
        """Every MonoBehaviour in obj's file with obj's script (matched on the m_Script
        reference in the header's bytes 16-28)."""
        sf = obj.assets_file
        if id(sf) not in self.scripts:
            groups = {}
            for o in sf.objects.values():
                if o.type.name == "MonoBehaviour":
                    try:
                        groups.setdefault(bytes(o.get_raw_data()[16:28]), []).append(o)
                    except Exception:
                        pass
            self.scripts[id(sf)] = groups
        return self.scripts[id(sf)].get(bytes(obj.get_raw_data()[16:28]), [obj])

    def repaired(self, obj, read):
        """A layout that reads obj when its saved one doesn't (the game changed the script),
        or None. read(node) reads obj with a layout, raising when it doesn't fit."""
        import layout_repair as lr
        from UnityPy.helpers.TypeTreeNode import TypeTreeNode

        try:
            key = _script_key(obj)
        except Exception:
            return None
        assembly, fullname = key.split("/", 1)
        saved = self.get_nodes_up(assembly, fullname)
        if saved is None:  # a script no exporter needed when the layouts were recorded
            return None
        if self.index is None:
            self.index = lr.field_index(self.layouts)
        sf = obj.assets_file
        files_key = (id(sf), key)
        if files_key not in self.plans:
            files = lr.Files(sf, self.index)
            same = self._same_script(obj)
            step = max(1, len(same) // (lr.SAMPLE * 2))
            samples = [lr.Data(o.get_raw_data(), files) for o in ([obj] + same[::step])[:lr.SAMPLE]]
            known = {k: set(v) for k, v in self.values.get(key, {}).items()}
            rep = lr.Repair(saved, known)
            plan, unsure = rep.plan(samples)
            self.plans[files_key] = (rep, plan, files)
            if unsure and key not in self.report["unsure"]:
                self.report["unsure"].append(key)
        rep, plan, files = self.plans[files_key]

        def reads(edits):
            try:
                read(rep.node(edits, TypeTreeNode))
                return True
            except Exception:
                return False

        edits = rep.edits_for(lr.Data(obj.get_raw_data(), files), plan, reads)
        bucket = "adjusted" if edits is not None else "unreadable"
        self.report[bucket][key] = self.report[bucket].get(key, 0) + 1
        return None if edits is None else rep.node(edits, TypeTreeNode)

    def _save_report(self):
        if not any(self.report.values()):
            return
        old = json.loads(REPORT.read_text(encoding="utf-8")) if REPORT.exists() else {}
        for bucket in ("adjusted", "unreadable"):
            merged = old.get(bucket, {})
            for k, n in self.report[bucket].items():
                merged[k] = merged.get(k, 0) + n
            old[bucket] = merged
        old["unsure"] = sorted(set(old.get("unsure", [])) | set(self.report["unsure"]))
        REPORT.parent.mkdir(parents=True, exist_ok=True)
        REPORT.write_text(json.dumps(old, indent=1), encoding="utf-8")


_installed = False


def _install_repair():
    """Every MonoBehaviour read that fails with a saved layout gets a repaired one."""
    global _installed
    if _installed:
        return
    _installed = True
    from UnityPy.files.ObjectReader import ObjectReader

    original = ObjectReader.read_typetree

    def read_typetree(self, nodes=None, wrap=False, check_read=True):
        try:
            return original(self, nodes, wrap, check_read)
        except Exception:
            gen = getattr(getattr(self.assets_file, "environment", None), "typetree_generator", None)
            if nodes is not None or self.type.name != "MonoBehaviour" or not isinstance(gen, SavedLayouts):
                raise
            node = gen.repaired(self, lambda n: original(self, n, False, True))
            if node is None:
                raise
            return original(self, node, wrap, check_read)

    ObjectReader.read_typetree = read_typetree


def _record(gen):
    """PG3D_RECORD_LAYOUTS=1: add every layout this run reads to layouts.json, and
    fingerprints of the values each field had (they tell a repaired layout's right reading
    from a shifted one)."""
    import atexit

    import layout_repair as lr
    from UnityPy.files.ObjectReader import ObjectReader

    seen, values, keys = {}, {}, {}
    real = gen.get_nodes_up

    def get_nodes_up(assembly, fullname):
        node = real(assembly, fullname)
        if node is not None:
            key = f"{assembly.removesuffix('.dll')}/{fullname}"
            seen[key] = [[n.m_Level, n.m_Type, n.m_Name, n.m_MetaFlag or 0] for n in node.traverse()]
            keys[id(node)] = key
        return node

    original = ObjectReader.read_typetree

    class NoChecks:
        externals, ids = 1 << 30, None

        def __init__(self, sf):
            self.externals, self.ids = len(sf.externals), sf.objects.keys()

        def check(self, ptype, pid):
            return 0

        def external_ids(self, fid):
            return None

    def read_typetree(self, nodes=None, wrap=False, check_read=True):
        out = original(self, nodes, wrap, check_read)
        if nodes is None and self.type.name == "MonoBehaviour":
            try:
                node = self._get_typetree_node()
                key = keys.get(id(node))
                if key:
                    fields = values.setdefault(key, {})
                    for name, f in lr.values(node, lr.Data(self.get_raw_data(), NoChecks(self.assets_file))).items():
                        s = fields.setdefault(name, set())
                        if len(s) < MAX_VALUES:
                            s.add(f)
            except Exception:
                pass
        return out

    ObjectReader.read_typetree = read_typetree

    def save():
        if not seen:
            return
        data = json.loads(LAYOUTS.read_text(encoding="utf-8")) if LAYOUTS.exists() else {"classes": {}}
        data["source"] = "Pixel Gun 3D PC Edition, Steam manifest 8917309360682204919 (October 2026)"
        data["classes"].update(seen)
        data["classes"] = dict(sorted(data["classes"].items()))
        old = data.get("values", {})
        for key, fields in values.items():
            mine = old.setdefault(key, {})
            for name, s in fields.items():
                mine[name] = sorted(set(mine.get(name, [])) | s)[:MAX_VALUES]
        data["values"] = dict(sorted(old.items()))
        text = json.dumps({k: data[k] for k in ("source", "classes", "values") if k in data}, separators=(",", ":"))
        LAYOUTS.write_text(text.replace('],["', '],\n["'), encoding="utf-8")

    gen.get_nodes_up = get_nodes_up
    atexit.register(save)
    return gen


def typetree_generator():
    """Reads MonoBehaviours whose files carry no type trees: from Cpp2IL's dummy DLLs when
    they exist (older game builds), else from the saved layouts (layouts.json)."""
    dlls = ROOT / "private/cpp2il/dummydll"
    if not any(dlls.glob("*.dll")):
        return SavedLayouts()
    from UnityPy.helpers.TypeTreeGenerator import TypeTreeGenerator

    gen = TypeTreeGenerator(unity_version(_require()[1]))
    gen.load_local_dll_folder(str(dlls))
    return _record(gen) if os.environ.get("PG3D_RECORD_LAYOUTS") else gen


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
