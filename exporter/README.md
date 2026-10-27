# Pixel Gun 3D — static reverse engineering

Offline, static study of Pixel Gun 3D (PC edition, IL2CPP; Unity 2022.3.76f1 until
October 2026, Unity 6 since) to recover its code structure (classes, functions, game
systems), and the exporters that turn your own install into files the Rust/Bevy rewrite
in `../pixel-gun-3d-rs` plays.

## Quick start (Linux and Windows)

Needs Python 3.9+ and an installed "Pixel Gun 3D: PC Edition" (free on Steam).

    python scripts/setup.py                  # finds the game, installs packages, exports everything
    python scripts/setup.py --game <folder>  # a copy outside Steam's libraries

The rewrite runs this for you on its first start (an import screen). Exports land in
`private/export/`; a failed step is listed and the rest carries on.

Since the Unity 6 update the game encrypts `global-metadata.dat`, so Cpp2IL can no longer
read the script layouts: maps, weapons, bots, characters, map pictures and strings still
export (their bundles carry type trees), the NGUI screens (lobby, HUD, settings, pause,
results, armory), game sounds, weapon effects and crosshairs do not, and the rewrite uses
its own simpler versions of those. The October 2026 build from before the update (Steam
console: `download_depot 2524890 2524891 8917309360682204919`, then
`python scripts/setup.py --game <the downloaded folder>`) exports everything, including all
maps (it ships every map instead of downloading them on demand).

## Layout

| Path       | What                                                        | In git? |
|------------|-------------------------------------------------------------|---------|
| `tools/`   | Downloaded tools (Cpp2IL, Ghidra plugins, Python venv)      | no      |
| `private/` | Everything extracted from the game (Cpp2IL output, Ghidra project, bundles) | no |
| `scripts/` | Our own helper scripts                                      | yes     |
| `notes/`   | Our own notes / findings                                    | yes     |

Game install (read-only, never modify, never copy into git): found by `scripts/gamepaths.py`
(`PG3D_GAME`, the folder saved by `setup.py --game`, then every Steam library on Linux and
Windows).

## Pipeline

1. `tools/cpp2il/cpp2il --game-path <install> --exe-name "Pixel Gun 3D" --use-processor attributeinjector --output-as diffable-cs --output-to private/cpp2il/cs`
   (and `--output-as dummydll --output-to private/cpp2il/dummydll` for DLLs you can open in ILSpy/dnSpy).
2. `python3 scripts/cs_index.py` → `private/index/{types,methods,fields}.tsv` + `summary.md`.
3. Ghidra project `private/ghidra/PixelGun3D` (import + auto-analysis, headless).
4. `scripts/apply_names.sh` → runs `scripts/ghidra/ApplyCpp2IL.java`: names every method
   `Namespace::Class::Method`, adds C# signatures as comments, creates `/Il2Cpp/*_o` structs.

5. `tools/venv/bin/python scripts/bundle_index.py` → `private/index/bundles.json` (which bundle holds which internal file).
6. `tools/venv/bin/python scripts/export_map.py <map>` → `private/export/maps/<map>/` (glTF + colliders + spawn
   markers) for the Rust/Bevy rewrite in `../pixel-gun-3d-rs`. `--list` shows the 626 map names.

7. `tools/venv/bin/python scripts/find_behaviours.py WeaponSounds > private/index/weaponsounds.tsv`, then
   `tools/venv/bin/python scripts/export_weapons.py [Weapon2 ...]` → `private/export/weapons/` (stats of all
   ~1650 weapons + first-person models and animation timings of selected ones).

8. `tools/venv/bin/python scripts/find_behaviours.py MeleeBot ShootingBot FiringShotBot KamikadzeBot MeleeShootBot
   ShootingBossBot StalkerBot > private/index/bots.tsv`, then `tools/venv/bin/python scripts/export_bots.py`
   → `private/export/bots/` (stats of all ~250 campaign enemies + models/animations of selected ones).

9. `tools/venv/bin/python scripts/export_navmesh.py <map>` → `private/export/maps/<map>/navmesh.json` (walkable
   polygons for enemy path finding). Weapon/bot exports now also write `sounds/*.wav` and muzzle-flash particle data.

10. `tools/venv/bin/python scripts/find_behaviours.py RocketSettings > private/index/rockets.tsv`, then
    `tools/venv/bin/python scripts/export_rockets.py` → `private/export/rockets/` (all projectile settings +
    models of rockets fired by exported weapons). Shooting bots' bullet colour/size is in bots.json.

11. Menu: `scripts/export_ngui.py level17 MainMenuNGUI lobby` (the lobby's NGUI screen: widget tree,
    atlases, bitmap fonts, English strings from bundle l_en-gb), `scripts/export_map.py level17 --out lobby
    --skip MainMenuNGUI FriendsWindowGUI --prefab slo_seasonlobby_current:SeasonLobbyRoot` (3D lobby + season
    decoration + music) and `scripts/export_character.py` (default player model with the lobby idle animation).
    The lobby scene is `level17` = Assets/Sources/Scenes/Menu_Custom.unity.
    `scripts/export_ngui.py resources.assets AllModesView modes` exports the "To Battle" game mode hub and
    `scripts/export_previews.py` the map loading pictures (bundles level_loading_previews_*).
    `scripts/export_ngui.py resources.assets SettingsWindow settings` exports the settings window and
    `scripts/export_ngui.py resources.assets SettingsPanel pause` the in-match pause screen,
    `scripts/export_ngui.py level6 "UI Root (2D)" results` the end-of-match results (level6 = LevelComplete), and
    `scripts/export_pickups.py` the in-match bonus models/sounds (prefab BonusGeneric).
    `scripts/find_behaviours.py` now also searches the built-in scenes (levelN) and resolves their scripts
    through globalgamemanagers.assets.

12. `tools/venv/bin/python scripts/export_probes.py <map>` → `private/export/maps/<map>/probes.{json,bin}`:
    the scene's baked light probes (LightmapSettings.m_LightProbes, L2 spherical harmonics) resampled
    into a Bevy irradiance volume (ambient cubes) that lights zombies, weapons and pickups.
    Exported model materials now honour the Cubic/Universal `UNLIT` keyword (the first-person arms are
    unlit, the guns' Optimized/MultiShader is lit); map exports ignore it (lightmaps light the level).

HDR lightmaps (BC6H) are decoded by `helpers/bc6h-decode` (build once:
`cd helpers/bc6h-decode && cargo build --release`); without it they are clamped to 8 bits.
Cubic/Universal emission (`_EmissionTex` alpha mask, e.g. glowing crystals) becomes a glTF
emissive texture. Explosive props (ExplosionObjectRespawnController -> Bomb_TNT prefab) are
placed under their markers with `extras.explosive` settings.
`scripts/export_ngui.py resources.assets InGameGUI ingame` exports the in-match HUD.

Textures keep their Unity filter mode: most of the game's textures are Point filtered, so the
exporters write glTF NEAREST samplers for them (bilinear made everything blurry).

Reference material from the real game (screenshots, clips, settings tabs) is in
`private/reference/` (not in git).

**Shortcut:** `scripts/export_all.py [maps...]` runs all of the above (indexes only if missing;
every map in the install by default). `scripts/setup.py` sets up `tools/` and then runs it.

Other helpers: `scripts/dump_behaviour.py` (any MonoBehaviour's fields as JSON), `scripts/find_text.py`
(find a string in the asset files), `scripts/probe_settings.py` (colour space, physics, time settings).

Obfuscated identifiers (CJK glyphs) get token aliases: `T_<token>` types, `m_<token>` methods,
`f_<token>` fields. The token matches the `[Token]` in the Cpp2IL C# output.

## Rules

- Game files and anything derived from them **never get committed**. `tools/` and
  `private/` are git-ignored.
- Static analysis only: no debuggers, hooks, Frida or Cheat Engine on the running
  game; never touch its servers or accounts.
