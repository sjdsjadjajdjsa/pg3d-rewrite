#!/usr/bin/env python3
"""Export the game's balance tables that the Armory needs (prices, shop stats).

The game ships its balance as JSON documents inside resources.assets, each with a
"configId": BalanceEconomic (price tables per rarity and level), BalanceWeaponsEconomic
(per weapon number: unlock level L/L3, grade G, price coefficient PC, currency flag BC)
and BalanceWeapons (per weapon: shop numbers SR fire rate, A clip, SM mobility, ...).

ItemRecord.get_PriceItemRecord (GameAssembly RVA 0x8352B0 / 0x835640) prices a weapon as
    round(WeaponPrices[<grade><currency>][level - 1] * PC / 5) * 5   (at least 5)
with grades 0 and 1 using the "R" (rare) row. Writes private/export/weapons/balance.json.
"""
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parent.parent
GAME = pathlib.Path.home() / ".local/share/Steam/steamapps/common/Pixel Gun 3D PC Edition/Pixel Gun 3D_Data"
OUT = ROOT / "private/export/weapons/balance.json"


def documents(data: bytes):
    """Every length-prefixed JSON object in the file that has a configId."""
    found = {}
    for m in re.finditer(rb'\{"configId"\s*:\s*"([A-Za-z]+)"', data):
        s = m.start()
        n = int.from_bytes(data[s - 4:s], "little")
        if 0 < n < 50_000_000:
            try:
                found.setdefault(m.group(1).decode(), json.loads(data[s:s + n]))
            except ValueError:
                pass
    return found


def main():
    docs = documents((GAME / "resources.assets").read_bytes())
    eco = docs["BalanceEconomic"]
    weco = docs["BalanceWeaponsEconomic"]["WeaponsEconomic"]
    wb = docs["BalanceWeapons"]["Weapons"]
    out = {
        "prices": eco["WeaponPrices"],
        "economy": {k: {f: v.get(f) for f in ("L", "L3", "G", "PC", "BC", "UC")} for k, v in weco.items()},
        "shop": {k: {f: v.get(f) for f in ("SR", "A", "SM", "DC", "L")} for k, v in wb.items()},
        # reload speed per weapon number (Weapons "R"; BalanceController hands it to the
        # Reload animation's speed, default 1): reload time = Reload clip length / R
        "reload_speed": {k: v["R"] for k, v in wb.items() if v.get("R")},
        # headshot damage multiplier per weapon number (WeaponsCore "H", else Weapons "H");
        # weapons without one use BalanceOthers HeadShotCoef
        "head": {k: v["H"] for k, v in {**{k: v for k, v in wb.items() if "H" in v},
                                       **{k: v for k, v in docs["BalanceWeaponsCore"]["WeaponsCore"].items() if "H" in v}}.items()},
        "head_default": next((e["XP"] for e in docs["BalanceOthers"]["SpecialEvents"] if e.get("Event") == "HeadShotCoef"), 1.0),
        # match rewards per mode by place (XP, coins C, gems G) and the level table
        "rewards": docs["BalanceRewards"]["Rewards"],
        "levels": docs["BalanceLeveling"]["LevellingV3"],
    }
    OUT.write_text(json.dumps(out))
    print(f"balance: {len(out['economy'])} weapon prices, {len(out['shop'])} shop stats, {len(out['head'])} headshot coefs (default {out['head_default']}) -> {OUT}")


if __name__ == "__main__":
    main()
