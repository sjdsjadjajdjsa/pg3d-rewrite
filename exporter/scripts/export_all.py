#!/usr/bin/env python3
"""Export everything the Rust/Bevy rewrite needs from your Pixel Gun 3D install.

    python scripts/export_all.py               every map in the install + menus, weapons, bots
    python scripts/export_all.py arena hill    just these maps (the rest is always redone)

Run it with the Python that has the packages (setup.py does all of this for you). Each step
prints "[i/N] what" and then "ok", "FAILED" or "skipped" with one line of detail; a failed
step does not stop the others. Exit code 1 when no map could be exported.
"""
import os
import pathlib
import shutil
import subprocess
import sys

import gamepaths

ROOT = pathlib.Path(__file__).resolve().parent.parent
INDEX = ROOT / "private/index"
EXPORT = ROOT / "private/export"
BC6H = ROOT / "helpers/bc6h-decode"


def py(script, *args, out=None):
    return {"cmd": [sys.executable, str(ROOT / "scripts" / script), *args], "out": out}


# the rewrite's map list (Arena campaign, Team Fight, sandbox); installs that ship every map
# (older builds: 600+) export just these, installs that download maps on demand export
# whichever of these they have, or all they have when none of these is there
DEFAULT_MAPS = ["arena", "hill", "ice_palace", "matrix", "arena_mine", "duelarenaspace", "arena_swamp", "new_start_map3",
                "castle_campaign5", "arena_castle", "arena_hockey", "arena_space", "arena2", "arena_underwater", "ants",
                "area52labs", "dust", "heaven", "nuclearcity", "pool", "sky_islands", "space"]


def installed_maps():
    r = subprocess.run(py("export_map.py", "--list")["cmd"], cwd=ROOT, capture_output=True, text=True)
    names = [m.strip() for m in r.stdout.splitlines() if m.strip() and not m.strip().endswith(" maps")]
    if r.returncode != 0:
        return []
    known = [m for m in DEFAULT_MAPS if m in names]
    return known if len(known) >= 5 or len(names) > 50 else names


def lobby_prefab():
    """The lobby's season decoration: the autumn one the rewrite was matched against, else
    the default one, else any."""
    bundles = gamepaths.DATA / "StreamingAssets/Cache/bundles"
    seasons = sorted(p.name for p in bundles.glob("slo_seasonlobby_*"))
    prefer = [s for s in ("slo_seasonlobby_autumn_fairytale_season", "slo_seasonlobby_default") if s in seasons]
    pick = prefer[0] if prefer else (seasons[0] if seasons else None)
    return [] if pick is None else ["--prefab", f"{pick}:SeasonLobbyRoot"]


def steps(maps):
    s = []
    for name, classes in [
        ("weaponsounds", ["WeaponSounds"]),
        ("bots", ["MeleeBot", "ShootingBot", "FiringShotBot", "KamikadzeBot", "MeleeShootBot", "ShootingBossBot", "StalkerBot"]),
        ("rockets", ["RocketSettings"]),
    ]:
        if not (INDEX / f"{name}.tsv").exists():
            s.append((f"index: {name}", py("find_behaviours.py", *classes, out=INDEX / f"{name}.tsv")))
    for m in maps:
        s += [
            (f"map {m}", py("export_map.py", m)),
            (f"map {m}: navmesh", py("export_navmesh.py", m)),
            (f"map {m}: light probes", py("export_probes.py", m)),
        ]
    s += [
        ("lobby screen + strings", py("export_ngui.py", "level17", "MainMenuNGUI", "lobby")),
        ("lobby scene", py("export_map.py", "level17", "--out", "lobby", "--skip", "MainMenuNGUI", "FriendsWindowGUI", *lobby_prefab())),
        ("player character", py("export_character.py")),
        ("death angel", py("export_death.py")),
        ("player animations", py("export_player_anims.py")),
        ("player skins", py("export_skins.py")),
        ("fonts", py("export_fonts.py")),
        ("hit marker", py("export_hitmarker.py")),
        ("post-processing", py("export_postprocess.py")),
        ("game modes per map", py("export_scene_modes.py")),
        ("mode picker screen", py("export_ngui.py", "resources.assets", "AllModesView", "modes",
                                  "--graft=ModeSelectorHolder=ModeSelector_default", "--graft=MapSelectorHolder=MapSelector_grid",
                                  "--graft=ModeInfoHolder=ModeInfo_default")),
        ("mode icons", py("export_mode_icons.py")),
        ("scene info", py("export_scene_info.py")),
        ("game sounds", py("export_game_sounds.py")),
        ("score table screen", py("export_ngui.py", "resources.assets", "NetworkStartTableNGUI", "table")),
        ("team fight HUD", py("export_ngui.py", "resources.assets", "InGameGUIPart_TeamFight", "teampart")),
        ("key icons", py("export_keys.py")),
        ("map pictures", py("export_previews.py")),
        ("settings screen", py("export_ngui.py", "resources.assets", "SettingsWindow", "settings")),
        ("pause screen", py("export_ngui.py", "resources.assets", "SettingsPanel", "pause")),
        ("results screen", py("export_ngui.py", "level6", "UI Root (2D)", "results", "--graft=ShopButton=BankShopViewPrefab")),
        ("match HUD", py("export_ngui.py", "resources.assets", "InGameGUI", "ingame", "--graft=shopView=BankShopViewPrefab")),
        ("arena HUD", py("export_ngui.py", "resources.assets", "InGameGUIPart_Arena", "arenapart")),
        ("armory screen", py("export_ngui.py", "resources.assets", "ShopNGUI", "armory",
                             "--graft=1_FirstGrid=ArmoryCell", "--graft=CoinsPanel=BankShopViewPrefab")),
        ("armory background", py("make_armory_bg.py")),
        ("cursor", py("export_ngui.py", "resources.assets", "GamepadCursor", "cursor")),
        ("pickups", py("export_pickups.py")),
        ("weapons", py("export_weapons.py")),
        ("weapon icons", py("export_weapon_icons.py")),
        ("weapon rays", py("export_rays.py")),
        ("weapon prices", py("export_balance.py")),
        ("weapon effects", py("export_effects.py")),
        ("weapon profiles", py("export_profiles.py")),
        ("weapon clips", py("export_weapon_clips.py")),
        ("weapon skins", py("export_weapon_skins.py")),
        ("crosshairs", py("export_crosshairs.py")),
        ("rockets", py("export_rockets.py")),
        ("bots", py("export_bots.py")),
        ("player bot", py("export_player_bot.py")),
    ]
    return s


def last_line(text):
    lines = [l.strip() for l in text.splitlines() if l.strip() and not l.startswith(("  File ", "    "))]
    return lines[-1][:200] if lines else ""


def build_bc6h():
    """HDR lightmaps need the small Rust decoder; without it they are clamped to 8 bits."""
    exe = BC6H / "target/release" / ("bc6h-decode.exe" if sys.platform == "win32" else "bc6h-decode")
    if exe.exists() or not shutil.which("cargo") or not (BC6H / "Cargo.toml").exists():
        return
    print("building the HDR lightmap decoder", flush=True)
    subprocess.run(["cargo", "build", "--release", "-q"], cwd=BC6H, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def main():
    INDEX.mkdir(parents=True, exist_ok=True)
    build_bc6h()
    if not (INDEX / "bundles.json").exists():
        print("indexing the game's bundles", flush=True)
        subprocess.run(py("bundle_index.py")["cmd"], cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    maps = sys.argv[1:] or installed_maps()
    if not maps:
        sys.exit("no maps found in the game install")
    todo = steps(maps)
    env = dict(os.environ, PYTHONIOENCODING="utf-8")
    failed, maps_ok = [], 0
    for i, (label, step) in enumerate(todo, 1):
        print(f"[{i}/{len(todo)}] {label}", flush=True)
        try:
            r = subprocess.run(step["cmd"], cwd=ROOT, capture_output=True, text=True, encoding="utf-8",
                               errors="replace", timeout=3600, env=env)
            code, stdout, stderr = r.returncode, r.stdout, r.stderr
        except subprocess.TimeoutExpired:
            code, stdout, stderr = 1, "", "took longer than an hour"
        if code == 0 and step["out"] is not None:
            step["out"].write_text(stdout, encoding="utf-8")
        if code == 0:
            print(f"      ok  {last_line(stdout)}", flush=True)
            if label.startswith("map ") and ":" not in label:
                maps_ok += 1
        else:
            if step["out"] is not None and step["out"].exists():
                step["out"].unlink()
            failed.append(label)
            print(f"      FAILED  {last_line(stderr) or last_line(stdout)}", flush=True)
    print(f"done: {len(todo) - len(failed)} of {len(todo)} steps ok, {maps_ok} maps -> {EXPORT}", flush=True)
    if failed:
        print("failed: " + ", ".join(failed), flush=True)
    sys.exit(0 if maps_ok else 1)


if __name__ == "__main__":
    main()
