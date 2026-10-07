# Pixel Gun 3D — static reverse engineering

Offline, static study of Pixel Gun 3D (PC edition, Unity 2022.3.76f1, IL2CPP)
to recover its code structure (classes, functions, game systems).

## Layout

| Path       | What                                                        | In git? |
|------------|-------------------------------------------------------------|---------|
| `tools/`   | Downloaded tools (Cpp2IL, Ghidra plugins, Python venv)      | no      |
| `private/` | Everything extracted from the game (Cpp2IL output, Ghidra project, bundles) | no |
| `scripts/` | Our own helper scripts                                      | yes     |
| `notes/`   | Our own notes / findings                                    | yes     |

Game install (read-only, never modify, never copy into git):
`~/.local/share/Steam/steamapps/common/Pixel Gun 3D PC Edition/`

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

**Shortcut:** `scripts/export_all.sh [maps...]` runs all of the above (indexes only if missing).

Other helpers: `scripts/dump_behaviour.py` (any MonoBehaviour's fields as JSON), `scripts/find_text.py`
(find a string in the asset files), `scripts/probe_settings.py` (colour space, physics, time settings).

Obfuscated identifiers (CJK glyphs) get token aliases: `T_<token>` types, `m_<token>` methods,
`f_<token>` fields. The token matches the `[Token]` in the Cpp2IL C# output.

## Rules

- Game files and anything derived from them **never get committed**. `tools/` and
  `private/` are git-ignored.
- Static analysis only: no debuggers, hooks, Frida or Cheat Engine on the running
  game; never touch its servers or accounts.
