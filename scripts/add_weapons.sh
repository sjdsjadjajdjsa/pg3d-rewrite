#!/usr/bin/env bash
# Add weapons to the remake: everything one weapon needs, in one go.
#   scripts/add_weapons.sh Weapon97 Weapon98 ...
# The model with its arms, Armory pose, first-person clips, icon, ray beam and rocket model.
# Weapon names and their in-game titles: private/export/weapons/weapons.json
# (localizeWeaponKey -> private/export/ui/strings_en-gb.json). The game picks them up on
# its next start: they show in the Armory, and bots may carry them.
set -uo pipefail
cd "$(dirname "$0")/.."
PY=tools/venv/bin/python
[ $# -gt 0 ] || { echo "usage: $0 WeaponN [WeaponM ...]"; exit 1; }
[ -f private/index/weaponsounds.tsv ] || $PY scripts/find_behaviours.py WeaponSounds > private/index/weaponsounds.tsv
[ -f private/index/rockets.tsv ] || $PY scripts/find_behaviours.py RocketSettings > private/index/rockets.tsv
$PY scripts/export_weapons.py "$@" 2>/dev/null | grep -E '^(weapons|model)'
ok=()
for w in "$@"; do
  if [ -f "private/export/weapons/$w/weapon.gltf" ]; then ok+=("$w"); else echo "   $w: no model exported"; fi
done
[ ${#ok[@]} -gt 0 ] || exit 1
$PY scripts/export_weapon_icons.py "${ok[@]}" 2>/dev/null | grep -E '^icons'
$PY scripts/export_rays.py 2>/dev/null | grep -E '^rays'
$PY scripts/export_profiles.py "${ok[@]}" 2>/dev/null | grep -E '^profiles'
python3 scripts/export_weapon_clips.py "${ok[@]}" | grep -E '^weapon clips'
python3 scripts/export_weapon_skins.py >/dev/null 2>&1
$PY scripts/export_rockets.py 2>/dev/null | grep -E '^model'
$PY scripts/export_crosshairs.py 2>/dev/null | grep -E '^crosshairs'
echo "added: ${ok[*]}"
