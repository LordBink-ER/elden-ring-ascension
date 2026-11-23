#!/usr/bin/env python3
"""Author the Ascension 0.3 power library (config/powers.json).

Rebalances the passive 0.2 cards (same ids, same mechanics, stronger values)
and appends the 0.3 cards: real-spell procs, hit/kill events, Great-Rune style
attributes, divine intervention, weapon-type and steering cards. Native-graph
cards whose values live in generated rows (Rune Echo, Relentless Momentum,
Constellation Chorus, Perfect Reversal) are left unchanged.

Idempotent: rerunning on its own output rewrites the same cards.
Run: python3 tools/author_powers.py
"""
from __future__ import annotations
import json
from pathlib import Path

PROJECT = Path(__file__).resolve().parents[1]
PATH = PROJECT / "config/powers.json"

C = ["common", "uncommon", "rare", "epic", "legendary"]
U = ["uncommon", "rare", "rare", "epic", "legendary"]
R = ["rare", "epic", "epic", "legendary", "mythic"]
E = ["epic", "epic", "legendary", "legendary", "mythic"]
L = ["legendary", "legendary", "legendary", "mythic", "mythic"]
M = ["mythic"] * 5
ALWAYS = {"kind": "always"}


def cond(kind, ratio=None, **kw):
    d = {"kind": kind}
    d.update(kw)
    if ratio is not None:
        d["ratio"] = ratio
    return d


def both(*cs):
    return {"kind": "all", "conditions": list(cs)}


def x(v):
    return f"×{v:.2f}"


def pct(v):
    return f"{(v - 1) * 100:+.0f}%"


# 0.8: every card grows in even steps; level I is 40% of its level-V power,
# then 55%, 70%, 85% and 100% (the value written for V is the reference).
STEPS = [0.40, 0.55, 0.70, 0.85, 1.0]
ADDITIVE = {"stamina_recover_change_speed", "item_drop_rate", "magic_effect_time_change", "change_hp_rate",
            "change_hp_point", "change_mp_rate", "change_mp_point", "poizon_attack_power", "disease_attack_power",
            "blood_attack_power", "freeze_attack_power", "sleep_attack_power", "madness_attack_power",
            "curse_attack_power", "dexterity_cancel_system_only_add_dexterity", "add_vigor", "add_mind",
            "add_endurance", "add_strength", "add_dexterity", "add_intelligence", "add_faith", "add_arcane"}
INTEGER = ADDITIVE - {"item_drop_rate", "magic_effect_time_change", "change_hp_rate", "change_mp_rate"}


def step(field, top, i):
    if field in ADDITIVE:
        value = top * STEPS[i]
    else:
        value = 1 + (top - 1) * STEPS[i]
    if field in INTEGER:
        rounded = int(round(value))
        if rounded == 0 and top != 0:
            rounded = 1 if top > 0 else -1
        return rounded
    return round(value, 3)


def at(values, i):
    out = {}
    for k, v in values.items():
        if isinstance(v, list) and len(v) == 5 and k != "motion_interval":
            out[k] = step(k, v[4], i)
        else:
            out[k] = v[i] if isinstance(v, list) else v
    return out


# ---------------------------------------------------------------- rebalance ---
SURGE_TEXT = " Rośnie z Malice (+4% buildupu za punkt). Odporni wrogowie pozostają odporni."
# 0.7.1: card texts that no longer mention the Mind channel budget.
DESCRIPTIONS = {
    "blood_communion": "Krew przyzywa krew: buildup krwawienia na broni. Wszystkie statusy z kart działają naraz.",
    "winterwake": "Mróz idzie za ostrzem: buildup odmrożenia na broni. Wszystkie statusy z kart działają naraz.",
    "miasma_craft": "Trucizna i zgnilizna w każdym trafieniu; oba statusy działają razem z innymi kartami.",
    "dreaming_touch": "Sen i szaleństwo w każdym trafieniu; działają razem ze wszystkimi innymi statusami.",
    "dual_covenant": "Wyszkolony rezonans łączy szkoły magii: tańsze sorcery i inkantacje.",
    "wandering_anvil": "Furia i hart razem: styl oparty na przełamywaniu postury.",
}
# id -> (fields with five values, level text)
REBALANCE = {
    "steady_pulse": ({"change_hp_rate": [-.15, -.25, -.35, -.5, -.7], "motion_interval": 1},
                     lambda v: f"Poniżej 50% HP: regeneracja {-v['change_hp_rate']:g}% maksymalnego HP na sekundę. Kończy się przy 50% HP."),
    "green_well": ({"stamina_recover_change_speed": [4, 7, 10, 14, 19]},
                   lambda v: f"Naturalna regeneracja staminy: +{v['stamina_recover_change_speed']} punktów do szybkości odzyskiwania."),
    "measured_breath": ({"consume_stamina_rate": [.95, .92, .89, .85, .80]},
                        lambda v: f"Przy staminie co najmniej 70%: koszt staminy działań {x(v['consume_stamina_rate'])}."),
    "second_wind": ({"stamina_recover_change_speed": [10, 15, 20, 26, 33]},
                    lambda v: f"Poniżej 25% staminy: +{v['stamina_recover_change_speed']} do szybkości jej naturalnej regeneracji."),
    "patient_scholar": ({"change_mp_point": [-1, -2, -3, -4, -5], "motion_interval": 1},
                        lambda v: f"Co najmniej 90% HP i 8 s bez utraty HP: odzyskujesz {-v['change_mp_point']} FP na sekundę."),
    "emergency_reserve": ({"change_mp_point": [-2, -3, -4, -5, -7], "motion_interval": 1},
                          lambda v: f"Poniżej 15% FP: odzyskujesz {-v['change_mp_point']} FP na sekundę. Regeneracja zatrzymuje się przy 15% FP."),
    "crimson_drought": ({"change_hp_estus_flask_correct_rate": [1.10, 1.16, 1.23, 1.31, 1.40]},
                        lambda v: f"Poniżej 35% HP: leczenie karmazynowego flakonu {x(v['change_hp_estus_flask_correct_rate'])}."),
    "cerulean_drought": ({"change_mp_estus_flask_correct_rate": [1.10, 1.17, 1.25, 1.34, 1.45]},
                         lambda v: f"Poniżej 20% FP: odzysk FP z błękitnego flakonu {x(v['change_mp_estus_flask_correct_rate'])}."),
    "amber_vessel": ({"max_hp_rate": [1.04, 1.07, 1.10, 1.14, 1.20]},
                     lambda v: f"Maksymalne HP {x(v['max_hp_rate'])}."),
    "memory_vessel": ({"max_mp_rate": [1.05, 1.09, 1.14, 1.20, 1.28]},
                      lambda v: f"Maksymalne FP {x(v['max_mp_rate'])}."),
    "unbroken_reservoir": ({"max_stamina_rate": [1.05, 1.09, 1.13, 1.18, 1.24]},
                           lambda v: f"Maksymalna stamina {x(v['max_stamina_rate'])}."),
    "wayfarers_frame": ({"equip_weight_change_rate": [1.05, 1.09, 1.14, 1.20, 1.28]},
                        lambda v: f"Maksymalny udźwig wyposażenia {x(v['equip_weight_change_rate'])}."),
    "soft_landing": ({"fall_damage_rate": [.85, .70, .55, .40, .25]},
                     lambda v: f"Obrażenia od nieśmiertelnych upadków {x(v['fall_damage_rate'])}. Nie zmienia granicy śmiertelnej wysokości."),
    "heavy_footing": ({"toughness_damage_cut_rate": [.94, .89, .84, .78, .70]},
                      lambda v: f"Przy staminie co najmniej 80%: otrzymywane obrażenia stabilności {x(v['toughness_damage_cut_rate'])}."),
    "stonebreaker": ({"sa_attack_power_rate": [1.06, 1.11, 1.17, 1.24, 1.32], "stamina_attack_rate": [1.05, 1.09, 1.14, 1.20, 1.28]},
                     lambda v: f"Obrażenia postury {x(v['sa_attack_power_rate'])}; obrażenia staminy przeciwnika {x(v['stamina_attack_rate'])}."),
    "guardkeeper": ({"guard_stamina_mult": [.95, .90, .85, .79, .72]},
                    lambda v: f"Koszt staminy gardy {x(v['guard_stamina_mult'])}."),
    "reprisal": ({"physics_attack_rate": [1.08, 1.13, 1.19, 1.26, 1.35], "toughness_damage_cut_rate": [.97, .94, .90, .85, .80]},
                 lambda v: f"Przez 4 s po utracie HP: obrażenia fizyczne {x(v['physics_attack_rate'])}, otrzymywane obrażenia stabilności {x(v['toughness_damage_cut_rate'])}."),
    "cinder_reserve": ({"fire_attack_rate": [1.08, 1.14, 1.21, 1.29, 1.40]},
                       lambda v: f"Poniżej 25% FP: zadawane obrażenia ognia {x(v['fire_attack_rate'])}. Nie dodaje ognia broni, która go nie posiada."),
    "storm_capacitor": ({"thunder_attack_rate": [1.08, 1.14, 1.21, 1.29, 1.40], "arts_consumption_rate": 1.06},
                        lambda v: f"Przy FP co najmniej 75%: obrażenia błyskawic {x(v['thunder_attack_rate'])}; koszt FP umiejętności ×1.06."),
    "full_moon_covenant": ({"magic_attack_rate": [1.10, 1.16, 1.23, 1.31, 1.42], "magic_damage_cut_rate": 1.08},
                           lambda v: f"Przy FP co najmniej 80%: obrażenia magiczne {x(v['magic_attack_rate'])}, otrzymywane obrażenia magiczne ×1.08."),
    "golden_shelter": ({"dark_damage_cut_rate": [.93, .88, .83, .77, .70], "magic_damage_cut_rate": [.95, .91, .86, .81, .75]},
                       lambda v: f"Przy HP co najmniej 80%: otrzymywane obrażenia święte {x(v['dark_damage_cut_rate'])}, magiczne {x(v['magic_damage_cut_rate'])}."),
    "ashskin": ({"fire_damage_cut_rate": [.90, .84, .78, .71, .62], "toughness_damage_cut_rate": [.95, .91, .86, .80, .73]},
                lambda v: f"Podczas gardy: obrażenia ognia {x(v['fire_damage_cut_rate'])}; otrzymywane obrażenia stabilności {x(v['toughness_damage_cut_rate'])}."),
    "iron_refuge": ({"slash_damage_cut_rate": [.90, .85, .79, .72, .64], "blow_damage_cut_rate": [.90, .85, .79, .72, .64],
                     "thrust_damage_cut_rate": [.90, .85, .79, .72, .64], "neutral_damage_cut_rate": [.90, .85, .79, .72, .64]},
                    lambda v: f"Poniżej 15% staminy: otrzymywane obrażenia fizyczne wszystkich czterech odmian {x(v['slash_damage_cut_rate'])}."),
    "cleansing_bloom": ({"regist_poizon_change_rate": [1.12, 1.20, 1.30, 1.42, 1.58], "regist_disease_change_rate": [1.12, 1.20, 1.30, 1.42, 1.58],
                         "poison_def_damage_rate": [.95, .90, .84, .77, .68], "disease_def_damage_rate": [.95, .90, .84, .77, .68]},
                        lambda v: f"Odporność na truciznę i zgniliznę {x(v['regist_poizon_change_rate'])}; ich obrażenia {x(v['poison_def_damage_rate'])}."),
    "bloodless_resolve": ({"regist_blood_change_rate": [1.15, 1.25, 1.38, 1.53, 1.75], "regist_freeze_change_rate": [1.15, 1.25, 1.38, 1.53, 1.75]},
                          lambda v: f"Poniżej 45% HP: odporność na krwawienie i odmrożenie {x(v['regist_blood_change_rate'])}."),
    "waking_vigil": ({"regist_sleep_change_rate": [1.15, 1.25, 1.38, 1.53, 1.75], "regist_madness_change_rate": [1.15, 1.25, 1.38, 1.53, 1.75]},
                     lambda v: f"Przy FP co najmniej 50%: odporność na sen i szaleństwo {x(v['regist_sleep_change_rate'])}."),
    "grave_whisper": ({"regist_curse_change_rate": [1.15, 1.25, 1.38, 1.53, 1.75], "item_drop_rate": [5, 8, 12, 17, 24]},
                      lambda v: f"Odporność na Death Blight {x(v['regist_curse_change_rate'])}; odkrywanie przedmiotów +{v['item_drop_rate']} punktów."),
    "oracle_lens": ({"item_drop_rate": [8, 13, 19, 26, 35]},
                    lambda v: f"Odkrywanie przedmiotów +{v['item_drop_rate']} punktów. Nie jest to gwarantowany procent szansy na łup."),
    "gilded_thread": ({"soul_rate": [1.06, 1.10, 1.15, 1.21, 1.30]},
                      lambda v: f"Runy z zabitych przeciwników {x(v['soul_rate'])}. Mnoży się z premią Arcane i Gold Scarab."),
    "long_invocation": ({"magic_effect_time_change": [3, 6, 10, 15, 21]},
                        lambda v: f"Czas trwania kwalifikujących się efektów magii +{v['magic_effect_time_change']} s."),
    "quiet_formula": ({"magic_consumption_rate": [.96, .92, .87, .81, .74]},
                      lambda v: f"Koszt FP sorcery {x(v['magic_consumption_rate'])}."),
    "devout_measure": ({"miracle_consumption_rate": [.96, .92, .87, .81, .74]},
                       lambda v: f"Koszt FP inkantacji {x(v['miracle_consumption_rate'])}."),
    "martial_clarity": ({"arts_consumption_rate": [.94, .88, .82, .75, .66]},
                        lambda v: f"Koszt FP umiejętności broni {x(v['arts_consumption_rate'])}."),
    "dual_covenant": ({"magic_consumption_rate": [.94, .88, .82, .75, .66], "miracle_consumption_rate": [.94, .88, .82, .75, .66]},
                      lambda v: f"Przy Resonance co najmniej 5: koszt FP sorcery i inkantacji {x(v['magic_consumption_rate'])}."),
    "spellblade_cradle": ({"arts_consumption_rate": [.93, .87, .80, .72, .62], "consume_stamina_rate": 1.04},
                          lambda v: f"Przy FP co najmniej 70%: koszt FP umiejętności {x(v['arts_consumption_rate'])}; koszt staminy działań ×1.04."),
    "anchored_rhythm": ({"guard_stamina_mult": [.93, .87, .80, .72, .62], "change_sa_recovery_velocity": [1.06, 1.12, 1.19, 1.27, 1.37]},
                        lambda v: f"Przy HP co najmniej 60% i staminie co najmniej 50%: koszt staminy gardy {x(v['guard_stamina_mult'])}, regeneracja postury {x(v['change_sa_recovery_velocity'])}."),
    "needle_bargain": ({"thrust_attack_rate": [1.08, 1.14, 1.21, 1.29, 1.40], "toughness_damage_cut_rate": 1.08},
                       lambda v: f"Przy staminie co najmniej 70%: obrażenia pchnięć {x(v['thrust_attack_rate'])}; otrzymywane obrażenia stabilności ×1.08."),
    "wandering_anvil": ({"sa_attack_power_rate": [1.08, 1.14, 1.21, 1.29, 1.40], "toughness_damage_cut_rate": [.97, .94, .90, .85, .79]},
                        lambda v: f"Przy Ferocity i Fortitude co najmniej 5: obrażenia postury {x(v['sa_attack_power_rate'])}, otrzymywane obrażenia stabilności {x(v['toughness_damage_cut_rate'])}."),
    "eclipse_balance": ({"neutral_damage_cut_rate": [.90, .85, .79, .72, .64], "slash_damage_cut_rate": [.90, .85, .79, .72, .64],
                         "blow_damage_cut_rate": [.90, .85, .79, .72, .64], "thrust_damage_cut_rate": [.90, .85, .79, .72, .64],
                         "magic_consumption_rate": 1.08, "miracle_consumption_rate": 1.08},
                        lambda v: f"Poniżej 50% HP i przy FP co najmniej 50%: otrzymywane obrażenia fizyczne {x(v['neutral_damage_cut_rate'])}; koszt FP sorcery i inkantacji ×1.08."),
    "graceful_reversal": ({"physics_attack_rate": [1.08, 1.13, 1.19, 1.26, 1.35], "sa_attack_power_rate": [1.08, 1.13, 1.19, 1.26, 1.35]},
                          lambda v: f"Przez 1.2 s po zakończeniu uniku: obrażenia fizyczne i obrażenia postury {x(v['physics_attack_rate'])}."),
    "moving_bastion": ({"stamina_recover_change_speed": [5, 8, 12, 17, 23], "toughness_damage_cut_rate": [.96, .92, .88, .83, .77]},
                       lambda v: f"Podczas gardy: +{v['stamina_recover_change_speed']} do regeneracji staminy, otrzymywane obrażenia stabilności {x(v['toughness_damage_cut_rate'])}."),
    "patient_colossus": ({"toughness_damage_cut_rate": [.94, .89, .83, .76, .68], "sa_attack_power_rate": [1.06, 1.11, 1.17, 1.24, 1.32]},
                         lambda v: f"Przy ciężkim obciążeniu: otrzymywane obrażenia stabilności {x(v['toughness_damage_cut_rate'])}, zadawane obrażenia postury {x(v['sa_attack_power_rate'])}."),
    "long_hunt": ({"stamina_recover_change_speed": [6, 10, 15, 21, 28], "consume_stamina_rate": [.96, .92, .88, .83, .77]},
                  lambda v: f"Po 20 s walki z bossem: +{v['stamina_recover_change_speed']} do regeneracji staminy, koszt staminy {x(v['consume_stamina_rate'])}. Reset po zakończeniu starcia."),
    "crimson_harvest": ({"change_hp_rate": [-.5, -.9, -1.4, -2.0, -2.8], "change_hp_point": [-5, -10, -16, -24, -35]},
                        lambda v: f"Po zabiciu przeciwnika odzyskujesz {-v['change_hp_rate']:g}% maksymalnego HP + {-v['change_hp_point']} HP (pojedynczy impuls)."),
    "azure_harvest": ({"change_mp_point": [-2, -3, -5, -7, -10]},
                      lambda v: f"Po zabiciu przeciwnika odzyskujesz {-v['change_mp_point']} FP (pojedynczy impuls)."),
    # 0.7.1: status cards were far too weak (0.7.0 log: +7 sleep/madness against
    # gauges of 2120). Values roughly x3, all statuses work at once (no Mind
    # channel limit), potency grows with Mind, level and NG+, and Status Surge
    # breaks through resistance (src/surge.rs).
    "blood_communion": ({"blood_attack_power": [24, 32, 41, 52, 66]},
                        lambda v: f"Buildup krwawienia broni: {v['blood_attack_power']} na trafienie." + SURGE_TEXT),
    "winterwake": ({"freeze_attack_power": [24, 32, 41, 52, 66]},
                   lambda v: f"Buildup odmrożenia broni: {v['freeze_attack_power']} na trafienie." + SURGE_TEXT),
    "miasma_craft": ({"poizon_attack_power": [22, 29, 38, 48, 62], "disease_attack_power": [16, 21, 27, 35, 45]},
                     lambda v: f"Buildup trucizny {v['poizon_attack_power']} i zgnilizny {v['disease_attack_power']} na trafienie." + SURGE_TEXT),
    "dreaming_touch": ({"sleep_attack_power": [22, 29, 38, 48, 62], "madness_attack_power": [22, 29, 38, 48, 62]},
                       lambda v: f"Buildup snu i szaleństwa po {v['sleep_attack_power']} na trafienie." + SURGE_TEXT),
}
META_REBALANCE = {
    "fortunes_favor": (0.15, lambda i: f"W losowaniu kart: Common +0%, Mythic +{15 * (i + 1)}% do wagi; rzadkości pomiędzy proporcjonalnie. Sumuje się ze szczęściem Arcane."),
    "ascension_affinity": (0.15, lambda i: f"Wagi już posiadanych kart poniżej V ×{1 + .15 * (i + 1):.2f}. Nadal obowiązuje indywidualne zmniejszanie szans na kolejne poziomy."),
}

# ---------------------------------------------------------------- new cards ---
NEW = []


def card(id, name, cat, rarities, description, levels, meta=None, weight=1.0, curse=None, requires=None,
         family=None, hidden=False, trigger="passive", signature=None):
    entry = {"id": id, "name": name, "category": cat, "base_rarity": rarities[0], "level_rarities": rarities,
             "weight": weight, "description": description, "trigger": trigger, "required_capabilities": [],
             "meta": meta, "levels": levels}
    if curse:
        entry["curse"] = curse
    if requires:
        entry["requires"] = [[pid, lvl] for pid, lvl in requires]
    if family:
        entry["family"] = family
    if hidden:
        entry["hidden"] = True
    if signature:
        entry["signature"] = list(signature)
    NEW.append(entry)


def passive(id, name, cat, rarities, description, activation, fields, text, **kw):
    card(id, name, cat, rarities, description,
         [{"description": text(at(fields, i)), "activation": activation,
           "modifiers": [{"field": k, "value": v} for k, v in at(fields, i).items()]} for i in range(5)], **kw)


def proc(id, name, cat, rarities, description, proc_id, trigger, power, text, count=1, activation=ALWAYS,
         mastery=True, **kw):
    levels = []
    for i in range(5):
        t = {k: (v[i] if isinstance(v, list) else v) for k, v in trigger.items()}
        c = count[i] if isinstance(count, list) else count
        p = power[i]
        extra = ""
        if i == 4 and mastery:
            # 0.8 Mastery: a spell card at V fires one more projectile.
            c = min(6, c + 1)
            extra = " Mistrzostwo (V): +1 pocisk."
        levels.append({"description": text(t, p, c) + extra, "activation": activation, "modifiers": [],
                       "proc": {"proc": proc_id, "trigger": t, "power": p, "count": c}})
    card(id, name, cat, rarities, description, levels, **kw)


def scale(p):
    return f"siła {p:.2f}"


SCALING = {
    "glintstone_bolt": "Intelligence", "pest_threads": "Faith/Arcane", "storm_spear": "Faith/Dexterity",
    "ruin_stars": "Intelligence", "elden_stars": "Faith", "sky_bolt": "Faith/Dexterity", "black_flame": "Faith",
    "fly_swarm": "Arcane", "meteorite": "Strength/Intelligence", "bestial_sling": "Strength/Faith",
    "bloodflame_burst": "Arcane", "ghostflame_burst": "Arcane/Intelligence", "rancor_spirits": "Arcane/Intelligence",
    "fell_pyre": "Faith/Strength", "catch_flame": "Faith/Dexterity", "zamor_frost": "Intelligence/Endurance",
    "wrath_of_gold": "Faith", "rejection": "Faith/Vigor", "crystal_burst": "Intelligence/Strength",
    "flame_sling": "Faith/Strength",
    # 0.5 evolutions.
    "full_moon": "Intelligence", "ancient_lightning": "Faith/Dexterity", "black_flame_ritual": "Faith",
    "bloodboon": "Arcane", "death_rancor": "Arcane/Intelligence", "scarlet_aeonia": "Faith/Arcane",
    "astel_meteor": "Intelligence", "black_blade": "Faith/Strength", "placidusax_ruin": "Faith/Dexterity",
    # 0.6 reflexes.
    "frenzied_burst": "Faith/Arcane", "adula_moonblade": "Intelligence", "frozen_lightning": "Faith/Dexterity",
    "glintstone_icecrag": "Intelligence/Endurance", "loretta_greatbow": "Intelligence/Dexterity",
    "fortissax_spear": "Faith",
}
TAIL = " Obrażenia rosną z poziomem postaci, atrybutami ({}) i cyklem NG+."


def interval_text(spell):
    def text(t, p, c):
        n = f"{c} pociski" if c > 1 else "pocisk"
        return f"W walce co {t['seconds']:g} s: {n} {spell} w najbliższego zaangażowanego wroga ({scale(p)})."
    return text


def hit_text(spell):
    return lambda t, p, c: f"Trafienie: {t['chance'] * 100:.0f}% szansy na {spell} w cel (odnowienie {t['cooldown']:g} s, {scale(p)})."


def streak_text(spell):
    def text(t, p, c):
        n = f"{c} × {spell}" if c > 1 else spell
        return f"Seria {t['hits']} trafień w {t.get('window', 6):g} s: {n} w cel (odnowienie {t['cooldown']:g} s, {scale(p)})."
    return text


def kill_text(spell):
    return lambda t, p, c: f"Zabicie: {t['chance'] * 100:.0f}% szansy, że ciało eksploduje — {spell} (odnowienie {t['cooldown']:g} s, {scale(p)})."


def dodge_text(spell):
    return lambda t, p, c: f"Początek uniku: {spell} wokół postaci (odnowienie {t['cooldown']:g} s, {scale(p)})."


def hurt_text(spell):
    return lambda t, p, c: f"Cios za co najmniej {t['min_share'] * 100:.0f}% maks. HP: {spell} wokół postaci (odnowienie {t['cooldown']:g} s, {scale(p)})."


def p(id, name, cat, rarities, desc, proc_id, trigger, power, text, **kw):
    proc(id, name, cat, rarities, desc + TAIL.format(SCALING[proc_id]), proc_id, trigger, power, text, **kw)


# Automatic casters (Megabonk-style weapons).
p("glintstone_familiar", "Glintstone Familiar", "arcane_arsenal", C,
  "Kamień blasku krąży przy ramieniu i sam strzela w wrogów, z którymi walczysz.",
  "glintstone_bolt", {"on": "interval", "seconds": [3.5, 3.0, 2.6, 2.2, 1.8]}, [0.45, 0.55, 0.66, 0.72, 0.85],
  interval_text("Glintstone Pebble"), count=[1, 1, 1, 2, 2])
p("pest_familiar", "Pest Familiar", "arcane_arsenal", U,
  "Nici szkodników wyrywają się z ciała i same oplatają przeciwnika.",
  "pest_threads", {"on": "interval", "seconds": [5.0, 4.5, 4.0, 3.5, 3.0]}, [0.25, 0.30, 0.36, 0.43, 0.52],
  interval_text("Pest Threads (pięć nici)"))
p("flame_familiar", "Flame Familiar", "arcane_arsenal", U,
  "Ognista kula sama wylatuje z dłoni w stronę walczącego z tobą wroga.",
  "flame_sling", {"on": "interval", "seconds": [5.0, 4.4, 3.8, 3.3, 2.8]}, [0.60, 0.70, 0.82, 0.90, 1.05],
  interval_text("Flame Sling"), count=[1, 1, 1, 2, 2])
p("storm_herald", "Storm Herald", "arcane_arsenal", R,
  "Burza ciska włócznie błyskawic w każdego, kto stanie z tobą do walki.",
  "storm_spear", {"on": "interval", "seconds": [6.0, 5.2, 4.5, 3.8, 3.0]}, [0.80, 0.95, 1.10, 1.20, 1.40],
  interval_text("Lightning Spear"), count=[1, 1, 1, 2, 2])
p("ruin_comet", "Ruin Comet", "arcane_arsenal", M,
  "Gwiazdy ruiny krążą wokół ciebie i samonaprowadzają się na wrogów.",
  "ruin_stars", {"on": "interval", "seconds": [4.0, 3.5, 3.0, 2.5, 2.0]}, [0.40, 0.48, 0.57, 0.66, 0.78],
  interval_text("Stars of Ruin"), count=[2, 2, 3, 3, 4])
p("elden_constellation", "Elden Constellation", "arcane_arsenal", M,
  "Nad głową formuje się gwiazdozbiór Elden Stars, który zasypuje wrogów świętymi gwiazdami.",
  "elden_stars", {"on": "interval", "seconds": [14.0, 12.0, 10.0, 8.5, 7.0]}, [0.35, 0.42, 0.50, 0.60, 0.72],
  interval_text("Elden Stars"))
# On hit / streak.
p("thunderhead", "Thunderhead", "elemental", R,
  "Twoje ciosy przyzywają piorun z nieba prosto w trafionego wroga.",
  "sky_bolt", {"on": "hit", "chance": [0.15, 0.18, 0.21, 0.25, 0.30], "cooldown": [2.5, 2.2, 1.9, 1.6, 1.3]},
  [0.90, 1.05, 1.20, 1.40, 1.65], hit_text("Lightning Strike"))
p("bestial_volley", "Bestial Volley", "offense", C,
  "Trafienia czasem wyrzucają w cel garść zwierzęcych kamieni.",
  "bestial_sling", {"on": "hit", "chance": [0.10, 0.13, 0.16, 0.19, 0.23], "cooldown": [2.5, 2.3, 2.1, 1.9, 1.6]},
  [0.25, 0.30, 0.36, 0.42, 0.50], hit_text("Bestial Sling"))
p("fly_swarm_curse", "Swarm Curse", "status", U,
  "Krwawe muchy rzucają się na trafionego przeciwnika i otwierają jego rany.",
  "fly_swarm", {"on": "hit", "chance": [0.12, 0.15, 0.18, 0.22, 0.27], "cooldown": [3.0, 2.7, 2.4, 2.1, 1.8]},
  [0.35, 0.42, 0.50, 0.60, 0.72], hit_text("Swarm of Flies (krwawienie)"))
p("crystal_riposte", "Crystal Riposte", "offense", U,
  "Szybka seria ciosów kończy się wybuchem kryształowych odłamków.",
  "crystal_burst", {"on": "streak", "hits": 3, "cooldown": [7.0, 6.0, 5.0, 4.2, 3.5]},
  [0.30, 0.36, 0.43, 0.50, 0.60], streak_text("Crystal Burst"))
p("black_flame_mark", "Black Flame Mark", "elemental", E,
  "Seria ciosów naznacza wroga czarnym płomieniem bogobójców.",
  "black_flame", {"on": "streak", "hits": [5, 4, 4, 4, 4], "window": [7.0, 6.0, 5.5, 5.0, 4.5],
                  "cooldown": [6.0, 5.5, 5.0, 4.5, 4.0]},
  [0.90, 1.10, 1.30, 1.50, 1.75], streak_text("Black Flame (spala % HP)"), count=[1, 1, 1, 1, 2])
p("meteor_call", "Meteor Call", "arcane_arsenal", L,
  "Długa seria ciosów ściąga z nieba meteoryt na twój cel.",
  "meteorite", {"on": "streak", "hits": [4, 4, 4, 3, 3], "cooldown": [8.0, 7.0, 6.0, 5.5, 5.0]},
  [1.20, 1.40, 1.60, 1.85, 2.20], streak_text("Meteorite"))
# On kill: corpse explosions.
p("bloodflame_bloom", "Bloodflame Bloom", "status", R,
  "Ciała zabitych wybuchają krwawym płomieniem, który rani i wywołuje krwawienie u sąsiadów.",
  "bloodflame_burst", {"on": "kill", "chance": [0.5, 0.6, 0.7, 0.85, 1.0], "cooldown": [1.5, 1.3, 1.1, 0.9, 0.7]},
  [0.60, 0.72, 0.85, 1.00, 1.20], kill_text("Bloodflame Talons"))
p("ghostflame_pyre", "Ghostflame Pyre", "status", E,
  "Śmierć wroga rozpala upiorny płomień, który mrozi wszystko dookoła.",
  "ghostflame_burst", {"on": "kill", "chance": [0.35, 0.42, 0.5, 0.6, 0.7], "cooldown": [4.0, 3.5, 3.0, 2.5, 2.0]},
  [0.80, 0.95, 1.10, 1.30, 1.50], kill_text("Explosive Ghostflame (odmrożenie)"))
p("rancor_vengeance", "Rancor Vengeance", "arcane_arsenal", R,
  "Z ciał zabitych wyrywają się mściwe duchy, które same szukają kolejnych ofiar.",
  "rancor_spirits", {"on": "kill", "chance": [0.4, 0.5, 0.6, 0.7, 0.8], "cooldown": [3.0, 2.6, 2.2, 1.8, 1.4]},
  [0.45, 0.54, 0.64, 0.76, 0.90], kill_text("Rancorcall"))
p("fell_gods_pyre", "Fell God's Pyre", "elemental", M,
  "Zabity wróg staje się stosem — płomień upadłego boga rozlewa się wokół jego ciała.",
  "fell_pyre", {"on": "kill", "chance": [0.25, 0.30, 0.36, 0.43, 0.50], "cooldown": [12.0, 10.0, 8.5, 7.0, 6.0]},
  [0.80, 0.95, 1.10, 1.30, 1.50], kill_text("Flame of the Fell God"))
# Dodge / hurt reactions.
p("ember_step", "Ember Step", "mobility", C,
  "Każdy unik zostawia za tobą wybuch płomieni.",
  "catch_flame", {"on": "dodge", "cooldown": [3.0, 2.6, 2.2, 1.8, 1.4]}, [0.45, 0.55, 0.65, 0.78, 0.92],
  dodge_text("Catch Flame"))
p("frost_step", "Frost Step", "mobility", R,
  "Unik rozpętuje wokół ciebie burzę lodu Zamora.",
  "zamor_frost", {"on": "dodge", "cooldown": [6.0, 5.2, 4.5, 3.8, 3.0]}, [0.60, 0.70, 0.82, 0.96, 1.12],
  dodge_text("Zamor Ice Storm (odmrożenie)"))
p("wrath_of_the_wounded", "Wrath of the Wounded", "defense", E,
  "Potężny cios w ciebie wyzwala złoty gniew, który odrzuca i rani wszystkich wokół.",
  "wrath_of_gold", {"on": "hurt", "min_share": [0.12, 0.11, 0.10, 0.09, 0.08], "cooldown": [10.0, 9.0, 8.0, 7.0, 6.0]},
  [1.00, 1.20, 1.40, 1.65, 1.95], hurt_text("Wrath of Gold"))
p("rebuke", "Rebuke", "defense", U,
  "Bolesne trafienie odpychasz falą świętego odrzucenia.",
  "rejection", {"on": "hurt", "min_share": [0.15, 0.14, 0.13, 0.12, 0.10], "cooldown": [8.0, 7.0, 6.0, 5.0, 4.0]},
  [0.35, 0.42, 0.50, 0.60, 0.72], hurt_text("Rejection"))

# Hit / kill event passives (the 0.3 world scanner confirms the player's hits).
ALL_ATTACK = ["physics_attack_rate", "magic_attack_rate", "fire_attack_rate", "thunder_attack_rate", "dark_attack_rate"]
passive("blade_rhythm", "Blade Rhythm", "offense", C, "Utrzymane tempo ciosów wzmacnia kolejne trafienia.",
        cond("after_event", event="hit_landed", seconds=4, minimum_stacks=3),
        {"physics_attack_rate": [1.04, 1.07, 1.10, 1.14, 1.19]},
        lambda v: f"Przy co najmniej 3 trafieniach w 6 s (trwa 4 s po ostatnim): obrażenia fizyczne {x(v['physics_attack_rate'])}.")
passive("battle_trance", "Battle Trance", "endurance", R, "Długa wymiana ciosów wprawia w trans, który oszczędza siły.",
        cond("after_event", event="hit_landed", seconds=4, minimum_stacks=5),
        {"consume_stamina_rate": [.92, .88, .84, .79, .73], "toughness_damage_cut_rate": [.95, .91, .87, .82, .76]},
        lambda v: f"Przy co najmniej 5 trafieniach w 6 s (trwa 4 s po ostatnim): koszt staminy {x(v['consume_stamina_rate'])}, otrzymywane obrażenia stabilności {x(v['toughness_damage_cut_rate'])}.")
passive("executioners_focus", "Executioner's Focus", "offense", L, "Bezlitosna seria ciosów łamie każdą gardę.",
        cond("after_event", event="hit_landed", seconds=4, minimum_stacks=7),
        {"sa_attack_power_rate": [1.15, 1.22, 1.30, 1.40, 1.52], "physics_attack_rate": [1.06, 1.09, 1.13, 1.18, 1.24]},
        lambda v: f"Przy co najmniej 7 trafieniach w 6 s (trwa 4 s po ostatnim): obrażenia postury {x(v['sa_attack_power_rate'])}, obrażenia fizyczne {x(v['physics_attack_rate'])}.")
passive("bloodlust", "Bloodlust", "offense", U, "Każde zabójstwo rozpala żądzę kolejnej walki.",
        cond("after_event", event="kill", seconds=8, minimum_stacks=1),
        {"physics_attack_rate": [1.06, 1.10, 1.15, 1.21, 1.28]},
        lambda v: f"Przez 8 s po zabójstwie: obrażenia fizyczne {x(v['physics_attack_rate'])}.")
passive("killing_spree", "Killing Spree", "offense", E, "Trzy zabójstwa w krótkim czasie zmieniają cię w żywą burzę.",
        cond("after_event", event="kill", seconds=8, minimum_stacks=3),
        {f: [1.10, 1.15, 1.21, 1.28, 1.36] for f in ALL_ATTACK},
        lambda v: f"Przy co najmniej 3 zabójstwach w 8 s: wszystkie zadawane obrażenia {x(v['physics_attack_rate'])}.")
passive("carrion_feast", "Carrion Feast", "recovery", U, "Zabity wróg na chwilę zasila twoją regenerację.",
        cond("after_event", event="kill", seconds=8, minimum_stacks=1),
        {"change_hp_rate": [-.3, -.45, -.6, -.8, -1.05], "motion_interval": 1},
        lambda v: f"Przez 8 s po zabójstwie: regeneracja {-v['change_hp_rate']:g}% maksymalnego HP na sekundę.")

# Great-Rune style attributes (they also feed Attribute Evolution).
ATTR_FIELDS = ["add_vigor", "add_mind", "add_endurance", "add_strength", "add_dexterity", "add_intelligence", "add_faith", "add_arcane"]
passive("ascendant_blood", "Ascendant Blood", "attribute_synergy", L, "Krew wznosi się ponad śmiertelność: wszystkie atrybuty rosną, razem z ich nowymi funkcjami.",
        ALWAYS, {f: [2, 3, 5, 7, 10] for f in ATTR_FIELDS},
        lambda v: f"Wszystkie osiem atrybutów +{v['add_vigor']}. Liczy się także dla Attribute Evolution.")
for cid, name, main, side, flavour in [
    ("grafted_might", "Grafted Might", "add_strength", "add_endurance", "Przeszczepiona siła: więcej Strength i Endurance."),
    ("moonlit_mind", "Moonlit Mind", "add_intelligence", "add_mind", "Światło księżyca wyostrza umysł: więcej Intelligence i Mind."),
    ("erdtree_devotion", "Erdtree Devotion", "add_faith", "add_vigor", "Oddanie Złotemu Drzewu: więcej Faith i Vigor."),
    ("serpents_luck", "Serpent's Luck", "add_arcane", "add_dexterity", "Wężowe szczęście: więcej Arcane i Dexterity."),
    ("swift_hands", "Swift Hands", "add_dexterity", "add_endurance", "Zręczne dłonie: więcej Dexterity i Endurance."),
]:
    label = {"add_strength": "Strength", "add_endurance": "Endurance", "add_intelligence": "Intelligence", "add_mind": "Mind",
             "add_faith": "Faith", "add_vigor": "Vigor", "add_arcane": "Arcane", "add_dexterity": "Dexterity"}
    passive(cid, name, "attribute_synergy", U, flavour + " Atrybuty liczą się także dla Attribute Evolution.", ALWAYS,
            {main: [3, 5, 8, 11, 15], side: [1, 2, 3, 4, 6]},
            (lambda m, s: (lambda v: f"{label[m]} +{v[m]}, {label[s]} +{v[s]}."))(main, side))

# Divine intervention cards (combine with Faith's own: best of each value).
card("marikas_mercy", "Marika's Mercy", "defense", M,
     "Śmiertelny cios zatrzymuje się na 1 HP. Łaska przywraca część zdrowia i na chwilę osłania złotą aurą.",
     [{"description": f"Raz na {cd:g} s: przeżywasz śmiertelny cios, odzyskujesz {h * 100:.0f}% maks. HP, przez 2.5 s otrzymujesz obrażenia ×{w:.2f}. Upadki w przepaść nadal zabijają.",
       "activation": ALWAYS, "modifiers": [],
       "intervention": {"cooldown": cd, "heal_share": h, "ward_seconds": 2.5, "ward_cut": w}}
      for cd, h, w in zip([600, 480, 390, 320, 260], [0.25, 0.30, 0.35, 0.42, 0.50], [0.5, 0.45, 0.4, 0.35, 0.3])])
card("duelists_oath", "Duelist's Oath", "defense", L,
     "Przysięga pojedynku: w walce z bossem śmierć musi poczekać.",
     [{"description": f"Tylko w walce z bossem, raz na {cd:g} s: przeżywasz śmiertelny cios i odzyskujesz {h * 100:.0f}% maks. HP.",
       "activation": {"kind": "boss_combat"}, "modifiers": [],
       "intervention": {"cooldown": cd, "heal_share": h, "ward_seconds": 2.0, "ward_cut": 0.5}}
      for cd, h in zip([360, 300, 250, 210, 180], [0.20, 0.24, 0.28, 0.32, 0.36])])

# Weapon types.
passive("edge_discipline", "Edge Discipline", "offense", C, "Wyszkolone cięcia.", ALWAYS,
        {"slash_attack_rate": [1.04, 1.08, 1.12, 1.16, 1.21]},
        lambda v: f"Obrażenia cięte {x(v['slash_attack_rate'])}.")
passive("crushing_weight", "Crushing Weight", "offense", C, "Miażdżące uderzenia łamią posturę.", ALWAYS,
        {"blow_attack_rate": [1.04, 1.08, 1.12, 1.16, 1.21], "sa_attack_power_rate": [1.03, 1.05, 1.08, 1.10, 1.13]},
        lambda v: f"Obrażenia obuchowe {x(v['blow_attack_rate'])}, obrażenia postury {x(v['sa_attack_power_rate'])}.")
passive("windstep", "Windstep", "mobility", C, "Unik przywraca oddech.",
        cond("after_dodge", seconds=1.5), {"stamina_recover_change_speed": [8, 12, 16, 21, 27]},
        lambda v: f"Przez 1.5 s po uniku: +{v['stamina_recover_change_speed']} do regeneracji staminy.")

# 0.4: Ascended Elites.
card("ember_seeker", "Ember Seeker", "roguelite", U,
     "Wyczuwasz żar wzniesionych: Ascended Elites pojawiają się częściej i dają więcej Embers. Przy okazji więcej znajdujesz.",
     [{"description": f"Szansa na Ascended Elite ×{1 + ch:.2f}; Embers za elitę +{eb}; odkrywanie przedmiotów +{fd}.",
       "activation": ALWAYS, "modifiers": [{"field": "item_drop_rate", "value": fd}],
       "elite_lure": {"chance_bonus": ch, "ember_bonus": eb}}
      for ch, eb, fd in zip([0.3, 0.5, 0.75, 1.0, 1.4], [0, 0, 1, 1, 2], [3, 5, 8, 11, 15])])

passive("trophy_hunter", "Trophy Hunter", "offense", R, "Pokonany Ascended Elite to trofeum: przez chwilę walczysz jak on.",
        cond("after_event", event="elite_kill", seconds=20, minimum_stacks=1),
        {f: [1.10, 1.14, 1.19, 1.24, 1.30] for f in ALL_ATTACK} | {"stamina_recover_change_speed": [5, 8, 11, 15, 20]},
        lambda v: f"Przez 20 s po zabiciu Ascended Elite: wszystkie zadawane obrażenia {x(v['physics_attack_rate'])}, regeneracja staminy +{v['stamina_recover_change_speed']}.")

# Draw steering (meta).
for cid, name, category, label in [
    ("warriors_calling", "Warrior's Calling", "offense", "Offense"),
    ("arcanists_calling", "Arcanist's Calling", "arcane_arsenal", "Arcane Arsenal"),
]:
    card(cid, name, "roguelite", U, f"Kieruje losowanie w stronę kart {label}.",
         [{"description": f"Wagi kart kategorii {label} ×{1 + .15 * (i + 1):.2f}. Inne kategorie zachowują swoje wagi.",
           "activation": ALWAYS, "modifiers": []} for i in range(5)],
         meta={"kind": "category_bias", "category": category, "per_level": 0.15})


# --------------------------------------------------------- 0.5 curse cards ---
# A strong power with a permanent drawback, shown in red on the card. The
# drawback is fixed; only the power grows with the level.
CUTS = ["slash_damage_cut_rate", "blow_damage_cut_rate", "thrust_damage_cut_rate", "neutral_damage_cut_rate",
        "magic_damage_cut_rate", "fire_damage_cut_rate", "thunder_damage_cut_rate", "dark_damage_cut_rate"]
PHYS_CUTS = CUTS[:4]
passive("glass_cannon", "Glass Cannon", "cursed", R,
        "Ciało jak szkło, cios jak młot: zadajesz ogromne obrażenia, ale każdy cios w ciebie boli bardziej.",
        ALWAYS, {f: [1.18, 1.23, 1.28, 1.34, 1.42] for f in ALL_ATTACK} | {f: 1.30 for f in CUTS},
        lambda v: f"Wszystkie zadawane obrażenia {x(v['physics_attack_rate'])}.",
        curse="Otrzymywane obrażenia wszystkich rodzajów ×1.30.", weight=0.8)
card("blood_price", "Blood Price", "cursed", E,
     "Krew zamiast skupienia: czary, inkantacje i umiejętności broni prawie nie kosztują FP, płacisz za nie życiem.",
     [{"description": f"Koszt FP sorcery, inkantacji i umiejętności broni ×0.35; każdy wydany punkt FP kosztuje {bp:g} HP.",
       "activation": ALWAYS,
       "modifiers": [{"field": f, "value": 0.35} for f in ["magic_consumption_rate", "miracle_consumption_rate", "arts_consumption_rate"]],
       "blood_price": bp} for bp in [3.4, 3.0, 2.6, 2.2, 1.8]],
     curse="Każdy wydany punkt FP zabiera też HP (3.4 HP na I, 1.8 HP na V). HP nigdy nie spada przez to poniżej 1.",
     weight=0.8)
passive("berserkers_oath", "Berserker's Oath", "cursed", U,
        "Przysięga berserkera: nie osłaniasz się, tylko atakujesz.",
        ALWAYS, {"physics_attack_rate": [1.10, 1.14, 1.19, 1.24, 1.30], "stamina_recover_change_speed": [5, 7, 10, 13, 16],
                 "guard_stamina_mult": 1.6} | {f: 1.15 for f in PHYS_CUTS},
        lambda v: f"Obrażenia fizyczne {x(v['physics_attack_rate'])}; regeneracja staminy +{v['stamina_recover_change_speed']}.",
        curse="Koszt staminy gardy ×1.60; otrzymywane obrażenia fizyczne ×1.15.", weight=0.8)
passive("hollow_arcanum", "Hollow Arcanum", "cursed", E,
        "Pustka w dłoniach, gwiazdy w umyśle: magia i żywioły rosną kosztem siły samej broni.",
        ALWAYS, {f: [1.15, 1.20, 1.26, 1.32, 1.40] for f in ALL_ATTACK[1:]} | {"physics_attack_rate": 0.70},
        lambda v: f"Obrażenia magiczne, ognia, błyskawic i święte {x(v['magic_attack_rate'])}.",
        curse="Obrażenia fizyczne ×0.70.", weight=0.8)
passive("golden_greed", "Golden Greed", "cursed", U,
        "Chciwość złota: świat płaci ci więcej, a ty płacisz krwią.",
        ALWAYS, {"soul_rate": [1.30, 1.45, 1.60, 1.80, 2.00], "item_drop_rate": [5, 8, 12, 17, 24]} | {f: 1.20 for f in CUTS},
        lambda v: f"Runy z zabitych przeciwników {x(v['soul_rate'])}; odkrywanie przedmiotów +{v['item_drop_rate']}.",
        curse="Otrzymywane obrażenia wszystkich rodzajów ×1.20.", weight=0.8)
passive("withered_flask", "Withered Flask", "cursed", R,
        "Uschnięty flakon: łzy leczą słabiej, ale krew sama zasklepia rany.",
        ALWAYS, {"change_hp_rate": [-.30, -.38, -.47, -.58, -.70], "motion_interval": 1, "change_hp_estus_flask_correct_rate": 0.5},
        lambda v: f"Stała regeneracja {-v['change_hp_rate']:g}% maksymalnego HP na sekundę.",
        curse="Leczenie karmazynowego flakonu ×0.50.", weight=0.8)
card("ember_debt", "Ember Debt", "cursed", R,
     "Dług wobec żaru: Ascended Elites przychodzą po ciebie częściej i płacą więcej, ale walczysz osłabiony.",
     [{"description": f"Szansa na Ascended Elite ×{1 + ch:.2f}; Embers za elitę +{eb}.",
       "activation": ALWAYS, "modifiers": [{"field": f, "value": 1.15} for f in CUTS],
       "elite_lure": {"chance_bonus": ch, "ember_bonus": eb}}
      for ch, eb in zip([0.8, 1.1, 1.4, 1.7, 2.0], [1, 1, 2, 2, 3])],
     curse="Otrzymywane obrażenia wszystkich rodzajów ×1.15.", weight=0.8)
passive("frenzied_pact", "Frenzied Pact", "cursed", L,
        "Pakt z Płomieniem Szaleństwa: twój ogień płonie jaśniej, ale pochłania także ciebie.",
        ALWAYS, {"fire_attack_rate": [1.25, 1.32, 1.40, 1.50, 1.60], "fire_damage_cut_rate": 1.5,
                 "magic_consumption_rate": 1.25, "miracle_consumption_rate": 1.25},
        lambda v: f"Obrażenia ognia {x(v['fire_attack_rate'])}.",
        curse="Otrzymywane obrażenia od ognia ×1.50; koszt FP sorcery i inkantacji ×1.25.", weight=0.8)

# ------------------------------------------------------- 0.5 evolutions ---
# Two parent cards at V unlock a Mythic evolution with a new mechanic. It
# joins the next offer (guaranteed) until it is taken; later levels are drawn.
EVO_TAIL = " Ewolucja: wymaga obu kart bazowych na poziomie V."
EV = 4.0
p("moonlight_familiar", "Moonlight Familiar", "evolution", M,
  "Familiar wchłonął pełnię Rennali: co kilka sekund nad twoim ramieniem wschodzi księżyc i spada na wroga." + EVO_TAIL,
  "full_moon", {"on": "interval", "seconds": [12.0, 10.5, 9.0, 7.5, 6.0]}, [1.60, 1.85, 2.10, 2.40, 2.80],
  interval_text("Rennala's Full Moon"), requires=[("glintstone_familiar", 5), ("full_moon_covenant", 5)], weight=EV)
p("dragonlords_tempest", "Dragonlord's Tempest", "evolution", M,
  "Burza i niebo stają się jednym: twoje ciosy ściągają czerwone pioruny starożytnych smoków." + EVO_TAIL,
  "ancient_lightning", {"on": "hit", "chance": [0.30, 0.34, 0.38, 0.42, 0.48], "cooldown": [2.2, 1.9, 1.6, 1.4, 1.2]},
  [1.30, 1.50, 1.70, 1.95, 2.25], hit_text("Ancient Dragons' Lightning Strike"),
  requires=[("storm_herald", 5), ("thunderhead", 5)], weight=EV)
p("godslayers_ritual", "Godslayer's Ritual", "evolution", M,
  "Czarny płomień i stos upadłego boga: seria ciosów wznosi wokół wroga kolumny czarnego ognia." + EVO_TAIL,
  "black_flame_ritual", {"on": "streak", "hits": [5, 5, 4, 4, 4], "window": [6.0, 6.0, 6.0, 5.5, 5.0],
                         "cooldown": [10.0, 9.0, 8.0, 7.0, 6.0]},
  [1.20, 1.40, 1.60, 1.85, 2.10], streak_text("Black Flame Ritual"),
  requires=[("black_flame_mark", 5), ("fell_gods_pyre", 5)], weight=EV)
p("lord_of_blood", "Lord of Blood", "evolution", M,
  "Krwawa łaska Mohga: z ciał zabitych tryska Bloodboon, krwawy wybuch, który rozrywa sąsiadów." + EVO_TAIL,
  "bloodboon", {"on": "kill", "chance": [0.50, 0.60, 0.70, 0.85, 1.0], "cooldown": [4.0, 3.5, 3.0, 2.5, 2.0]},
  [1.10, 1.30, 1.50, 1.75, 2.00], kill_text("Bloodboon"),
  requires=[("bloodflame_bloom", 5), ("blood_communion", 5)], weight=EV)
p("ancient_death", "Ancient Death", "evolution", M,
  "Upiorny płomień i mściwe duchy budzą starożytną śmierć: z ciał zabitych wyrywają się duchy, które same polują na kolejnych." + EVO_TAIL,
  "death_rancor", {"on": "kill", "chance": [0.45, 0.55, 0.65, 0.75, 0.85], "cooldown": [4.0, 3.5, 3.0, 2.5, 2.0]},
  [0.90, 1.05, 1.20, 1.40, 1.60], kill_text("Ancient Death Rancor"), count=[1, 1, 1, 2, 2],
  requires=[("rancor_vengeance", 5), ("ghostflame_pyre", 5)], weight=EV)
p("scarlet_bloom", "Scarlet Bloom", "evolution", M,
  "Szkodniki i muchy niosą nasiona szkarłatnego kwiatu: pod twoim wrogiem rozkwita Scarlet Aeonia i rozsiewa zgniliznę." + EVO_TAIL,
  "scarlet_aeonia", {"on": "interval", "seconds": [14.0, 12.5, 11.0, 9.5, 8.0]}, [1.00, 1.15, 1.30, 1.50, 1.75],
  interval_text("Scarlet Aeonia"), requires=[("pest_familiar", 5), ("fly_swarm_curse", 5)], weight=EV)
p("astels_cataclysm", "Astel's Cataclysm", "evolution", M,
  "Meteoryt i gwiazdy ruiny przywołują Astela: seria ciosów zrzuca na cel meteoryty z pustki." + EVO_TAIL,
  "astel_meteor", {"on": "streak", "hits": [5, 5, 4, 4, 4], "window": [6.0, 6.0, 6.0, 5.5, 5.0],
                   "cooldown": [12.0, 11.0, 10.0, 9.0, 8.0]},
  [1.50, 1.75, 2.00, 2.30, 2.60], streak_text("Meteorite of Astel"), count=[1, 1, 2, 2, 3],
  requires=[("meteor_call", 5), ("ruin_comet", 5)], weight=EV)
p("destined_death", "Destined Death", "evolution", M,
  "Złoty gniew i odrzucenie budzą Czarne Ostrze Maliketha: potężny cios w ciebie odpowiada falą przeznaczonej śmierci." + EVO_TAIL,
  "black_blade", {"on": "hurt", "min_share": [0.10, 0.09, 0.08, 0.07, 0.06], "cooldown": [9.0, 8.0, 7.0, 6.0, 5.0]},
  [1.30, 1.50, 1.70, 1.95, 2.20], hurt_text("Black Blade"),
  requires=[("wrath_of_the_wounded", 5), ("rebuke", 5)], weight=EV)
p("ruinous_step", "Ruinous Step", "evolution", M,
  "Ogień i lód pod stopami: każdy unik ściąga w miejsce, które opuszczasz, czerwone pioruny Placidusaxa." + EVO_TAIL,
  "placidusax_ruin", {"on": "dodge", "cooldown": [6.0, 5.2, 4.5, 3.8, 3.0]}, [0.90, 1.05, 1.20, 1.40, 1.60],
  dodge_text("Placidusax's Ruin"), requires=[("ember_step", 5), ("frost_step", 5)], weight=EV)
passive("elden_vessel", "Vessel of the Elden Lord", "evolution", M,
        "Dwa naczynia stają się jednym: ciało Pana Elden mieści więcej życia i skupienia, więcej siły i lepiej znosi ciosy." + EVO_TAIL,
        ALWAYS, {"max_hp_rate": [1.08, 1.10, 1.12, 1.15, 1.18], "max_mp_rate": [1.08, 1.10, 1.12, 1.15, 1.18],
                 "max_stamina_rate": [1.06, 1.08, 1.10, 1.12, 1.15]} | {f: [.96, .95, .94, .93, .92] for f in CUTS},
        lambda v: f"Maksymalne HP i FP {x(v['max_hp_rate'])}, stamina {x(v['max_stamina_rate'])}; otrzymywane obrażenia {x(v['slash_damage_cut_rate'])}.",
        requires=[("amber_vessel", 5), ("memory_vessel", 5)], weight=EV)
passive("bladestorm", "Bladestorm", "evolution", M,
        "Rytm i szkoła cięć: kiedy ostrze tańczy, każdy kolejny cios jest szybszy w oddechu i cięższy w skutkach." + EVO_TAIL,
        cond("after_event", event="hit_landed", seconds=4, minimum_stacks=3),
        {"physics_attack_rate": [1.10, 1.13, 1.16, 1.19, 1.23], "slash_attack_rate": [1.06, 1.08, 1.10, 1.12, 1.15],
         "consume_stamina_rate": [.90, .87, .84, .81, .78]},
        lambda v: f"Przy co najmniej 3 trafieniach w 6 s (trwa 4 s): obrażenia fizyczne {x(v['physics_attack_rate'])}, cięte dodatkowo {x(v['slash_attack_rate'])}, koszt staminy {x(v['consume_stamina_rate'])}.",
        requires=[("blade_rhythm", 5), ("edge_discipline", 5)], weight=EV)
passive("colossus", "Colossus", "evolution", M,
        "Miażdżący ciężar i łamacz kamieni: stajesz się kolosem, który łamie postawę wroga i sam się nie chwieje." + EVO_TAIL,
        ALWAYS, {"sa_attack_power_rate": [1.15, 1.20, 1.25, 1.30, 1.36], "blow_attack_rate": [1.06, 1.08, 1.10, 1.12, 1.15],
                 "toughness_damage_cut_rate": [.90, .87, .84, .82, .80]},
        lambda v: f"Obrażenia postury {x(v['sa_attack_power_rate'])}, obuchowe {x(v['blow_attack_rate'])}; otrzymywane obrażenia stabilności {x(v['toughness_damage_cut_rate'])}.",
        requires=[("crushing_weight", 5), ("stonebreaker", 5)], weight=EV)



# ------------------------------------------------------- 0.6 "Reflexes" ---
# New combat events read by the world scan (reflexes.rs): perfect dodge (a
# dodge through an enemy attack), block, guard counter, critical (riposte or
# backstab), stance break and status procs on enemies. Category "reflex".
def pdodge_text(spell):
    return lambda t, p, c: f"Perfekcyjny unik (unik przez atak wroga): {spell} w atakującego (odnowienie {t['cooldown']:g} s, {scale(p)})."


def block_text(spell):
    return lambda t, p, c: f"Zablokowany cios: {t['chance'] * 100:.0f}% szansy na {spell} (odnowienie {t['cooldown']:g} s, {scale(p)})."


def counter_text(spell):
    return lambda t, p, c: f"Kontra po gardzie (trafienie do 1.5 s po bloku): {spell} w cel (odnowienie {t['cooldown']:g} s, {scale(p)})."


def critical_text(spell):
    return lambda t, p, c: f"Riposta lub cios w plecy: {spell} w ofiarę (odnowienie {t['cooldown']:g} s, {scale(p)})."


def stance_text(spell):
    def text(t, p, c):
        n = f"{c} × {spell}" if c > 1 else spell
        return f"Złamanie postawy wroga: {n} w niego (odnowienie {t['cooldown']:g} s, {scale(p)})."
    return text


STATUS_PL = {"frostbite": "odmrożenie", "blood_loss": "krwotok", "sleep": "sen", "madness": "szaleństwo",
             "poison": "trucizna lub zgnilizna", "any": "dowolny status", "death_blight": "Death Blight"}


def status_text(spell, status):
    def text(t, p, c):
        n = f"{c} × {spell}" if c > 1 else spell
        return f"Gdy u wroga wybuchnie {STATUS_PL[status]}: {n} w niego (odnowienie {t['cooldown']:g} s, {scale(p)})."
    return text


def st(status, cooldown):
    return {"on": "status", "status": status, "cooldown": cooldown}


p("phantom_reprisal", "Phantom Reprisal", "reflex", R,
  "Unik w ostatniej chwili zostawia za tobą widmowy łuk Loretty, który odpowiada strzałą.",
  "loretta_greatbow", {"on": "perfect_dodge", "cooldown": [4.0, 3.6, 3.2, 2.8, 2.4]}, [0.90, 1.05, 1.20, 1.40, 1.65],
  pdodge_text("Loretta's Greatbow"))
p("thunderclap_evasion", "Thunderclap Evasion", "reflex", U,
  "Gdy wymykasz się ciosowi, niebo uderza w tego, kto go zadał.",
  "sky_bolt", {"on": "perfect_dodge", "cooldown": [5.0, 4.5, 4.0, 3.5, 3.0]}, [0.70, 0.82, 0.95, 1.10, 1.30],
  pdodge_text("Lightning Strike"))
p("golden_bulwark", "Golden Bulwark", "reflex", R,
  "Cios zatrzymany na tarczy odbija się złotym gniewem.",
  "wrath_of_gold", {"on": "block", "chance": [0.30, 0.37, 0.44, 0.52, 0.60], "cooldown": [8.0, 7.2, 6.4, 5.7, 5.0]},
  [0.80, 0.95, 1.10, 1.30, 1.50], block_text("Wrath of Gold"))
p("counter_spark", "Counter Spark", "reflex", U,
  "Kontra po udanej gardzie niesie włócznię błyskawicy.",
  "storm_spear", {"on": "guard_counter", "cooldown": [3.0, 2.7, 2.4, 2.1, 1.8]}, [0.60, 0.72, 0.85, 1.00, 1.20],
  counter_text("Lightning Spear"))
p("executioners_pyre", "Executioner's Pyre", "reflex", E,
  "Riposta i cios w plecy kończą się stosem upadłego boga pod ofiarą.",
  "fell_pyre", {"on": "critical", "cooldown": [6.0, 5.2, 4.5, 3.7, 3.0]}, [1.00, 1.20, 1.40, 1.65, 1.90],
  critical_text("Flame of the Fell God"))
p("moonblade_riposte", "Moonblade Riposte", "reflex", R,
  "Krytyczny cios wypuszcza zimne ostrze księżyca Adula.",
  "adula_moonblade", {"on": "critical", "cooldown": [4.0, 3.5, 3.0, 2.5, 2.0]}, [0.90, 1.05, 1.20, 1.40, 1.60],
  critical_text("Adula's Moonblade (odmrożenie)"))
p("shattering_quake", "Shattering Quake", "reflex", L,
  "Gdy postawa wroga pęka, z nieba spada na niego meteoryt.",
  "meteorite", {"on": "stance_break", "cooldown": [10.0, 9.0, 8.0, 7.0, 6.0]}, [1.30, 1.55, 1.80, 2.10, 2.40],
  stance_text("Meteorite"), count=[1, 1, 1, 2, 2])
p("frost_shatter", "Frost Shatter", "reflex", R,
  "Odmrożony wróg pęka jak lód: w jego stronę leci odłamek lodowca blasku.",
  "glintstone_icecrag", st("frostbite", [5.0, 4.5, 4.0, 3.5, 3.0]), [0.90, 1.05, 1.20, 1.40, 1.60],
  status_text("Glintstone Icecrag", "frostbite"))
p("hemorrhage_rite", "Hemorrhage Rite", "reflex", R,
  "Krwotok wroga to obrzęd: z jego krwi wybucha krwawy płomień.",
  "bloodflame_burst", st("blood_loss", [5.0, 4.5, 4.0, 3.5, 3.0]), [0.70, 0.82, 0.95, 1.10, 1.30],
  status_text("Bloodflame Talons", "blood_loss"))
p("dream_eater", "Dream Eater", "reflex", E,
  "Śpiący wróg przyzywa mściwe duchy, które pożerają jego sen.",
  "rancor_spirits", st("sleep", [6.0, 5.4, 4.8, 4.2, 3.5]), [0.80, 0.95, 1.10, 1.30, 1.50],
  status_text("Rancorcall", "sleep"), count=[1, 1, 2, 2, 3])
p("contagious_frenzy", "Contagious Frenzy", "reflex", L,
  "Szaleństwo jest zaraźliwe: oszalały wróg płonie żółtym płomieniem.",
  "frenzied_burst", st("madness", [6.0, 5.2, 4.5, 3.8, 3.0]), [1.00, 1.20, 1.40, 1.65, 1.90],
  status_text("Frenzied Burst (szaleństwo)", "madness"))
p("pestilent_bloom", "Pestilent Bloom", "reflex", U,
  "Trucizna i zgnilizna przyciągają rój krwawych much.",
  "fly_swarm", st("poison", [5.0, 4.5, 4.0, 3.5, 3.0]), [0.50, 0.60, 0.70, 0.82, 0.95],
  status_text("Swarm of Flies", "poison"))

ev = lambda event, seconds, stacks=1: cond("after_event", event=event, seconds=seconds, minimum_stacks=stacks)
passive("phantom_footwork", "Phantom Footwork", "reflex", R, "Po perfekcyjnym uniku twoje ciosy niosą widmową siłę.",
        ev("perfect_dodge", 5), {f: [1.10, 1.13, 1.17, 1.21, 1.26] for f in ALL_ATTACK},
        lambda v: f"Przez 5 s po perfekcyjnym uniku: wszystkie zadawane obrażenia {x(v['physics_attack_rate'])}.")
passive("second_breath", "Second Breath", "reflex", U, "Unik o włos od ciosu przywraca oddech.",
        ev("perfect_dodge", 3), {"stamina_recover_change_speed": [15, 20, 26, 33, 42], "consume_stamina_rate": [.92, .89, .86, .83, .80]},
        lambda v: f"Przez 3 s po perfekcyjnym uniku: regeneracja staminy +{v['stamina_recover_change_speed']}, koszt staminy {x(v['consume_stamina_rate'])}.")
passive("bulwark_stance", "Bulwark Stance", "reflex", C, "Każdy zatrzymany cios utwierdza cię w gardzie.",
        ev("block", 3), {f: [.95, .92, .89, .85, .80] for f in CUTS} | {"guard_stamina_mult": [.95, .92, .89, .86, .82]},
        lambda v: f"Przez 3 s po zablokowaniu ciosu: otrzymywane obrażenia {x(v['slash_damage_cut_rate'])}, koszt staminy gardy {x(v['guard_stamina_mult'])}.")
passive("counter_tempo", "Counter Tempo", "reflex", U, "Kontra po gardzie nadaje rytm kolejnym ciosom.",
        ev("guard_counter", 5), {"physics_attack_rate": [1.08, 1.12, 1.16, 1.20, 1.24], "sa_attack_power_rate": [1.06, 1.09, 1.12, 1.15, 1.18]},
        lambda v: f"Przez 5 s po kontrze po gardzie: obrażenia fizyczne {x(v['physics_attack_rate'])}, obrażenia postury {x(v['sa_attack_power_rate'])}.")
passive("headsmans_mark", "Headsman's Mark", "reflex", U, "Krytyczny cios wyostrza zmysły kata.",
        ev("critical_hit", 10), {"physics_attack_rate": [1.06, 1.09, 1.12, 1.16, 1.20], "stamina_recover_change_speed": [5, 7, 10, 12, 15]},
        lambda v: f"Przez 10 s po ripoście lub ciosie w plecy: obrażenia fizyczne {x(v['physics_attack_rate'])}, regeneracja staminy +{v['stamina_recover_change_speed']}.")
passive("shatterpoint", "Shatterpoint", "reflex", R, "Złamana postawa wroga otwiera drogę do jego końca.",
        ev("poise_break", 8), {f: [1.08, 1.12, 1.16, 1.20, 1.24] for f in ALL_ATTACK},
        lambda v: f"Przez 8 s po złamaniu postawy wroga: wszystkie zadawane obrażenia {x(v['physics_attack_rate'])}.")
passive("riven_veins", "Riven Veins", "reflex", U, "Krwotok wroga karmi twoje rany.",
        ev("bleed_proc", 6), {"change_hp_rate": [-.40, -.55, -.70, -.90, -1.20], "motion_interval": 1},
        lambda v: f"Przez 6 s po krwotoku u wroga: regeneracja {-v['change_hp_rate']:g}% maksymalnego HP na sekundę.")
passive("lucid_nightmare", "Lucid Nightmare", "reflex", R, "Śpiący wróg jest bezbronny wobec twojego koszmaru.",
        ev("sleep_proc", 8), {f: [1.10, 1.14, 1.19, 1.24, 1.30] for f in ALL_ATTACK},
        lambda v: f"Przez 8 s po uśpieniu wroga: wszystkie zadawane obrażenia {x(v['physics_attack_rate'])}.")
passive("cold_blooded", "Cold Blooded", "reflex", U, "Mróz na wrogu chłodzi także twoją krew: ciosy mniej bolą.",
        ev("frostbite_proc", 8), {f: [.94, .91, .88, .85, .82] for f in CUTS},
        lambda v: f"Przez 8 s po odmrożeniu wroga: otrzymywane obrażenia {x(v['slash_damage_cut_rate'])}.")
passive("madmans_insight", "Madman's Insight", "reflex", R, "Cudze szaleństwo podsyca twój płomień i umysł.",
        ev("madness_proc", 8), {"change_mp_point": [-1, -2, -2, -3, -4], "motion_interval": 1, "fire_attack_rate": [1.08, 1.11, 1.14, 1.18, 1.22]},
        lambda v: f"Przez 8 s po szaleństwie u wroga: odzyskujesz {-v['change_mp_point']} FP na sekundę, obrażenia ognia {x(v['fire_attack_rate'])}.")

# Dexterity's nimble hands, as a card: faster flasks and items.
card("swift_draught", "Swift Draught", "recovery", C,
     "Wprawne dłonie: flakony i przedmioty używasz szybciej. Sumuje się z premią Dexterity.",
     [{"description": f"Animacje picia z flakonów i użycia przedmiotów szybsze o {sp * 100:.0f}%.",
       "activation": ALWAYS, "modifiers": [], "item_speed": sp} for sp in [0.06, 0.09, 0.12, 0.16, 0.22]])

# Fate cards: Lock and Skip.
card("thread_of_fate", "Thread of Fate", "roguelite", U,
     "Nić przeznaczenia: zablokowanie karty na następną ofertę kosztuje mniej Embers.",
     [{"description": "Blokada karty (Lock) kosztuje 1 Ember zamiast 2." if i < 4 else "Blokada karty (Lock) jest darmowa.",
       "activation": ALWAYS, "modifiers": []} for i in range(5)],
     meta={"kind": "lock_discount"})
card("fates_ledger", "Fate's Ledger", "roguelite", U,
     "Księga losu: pominięta nagroda płaci więcej run, a na wyższych poziomach także Embers.",
     [{"description": f"Pominięcie nagrody (Skip): runy ×{1 + .15 * (i + 1):.2f}" + ("" if i < 2 else f", +{1 + (i >= 4)} Ember") + ".",
       "activation": ALWAYS, "modifiers": []} for i in range(5)],
     meta={"kind": "skip_bonus", "per_level": 0.15})

# Three-card evolutions (0.6): every parent at V.
EVO3 = " Ewolucja: wymaga wszystkich trzech kart bazowych na poziomie V."
p("moonlit_reflex", "Moonlit Reflex", "evolution", M,
  "Widmowy łuk, widmowe kroki i oddech wiatru: perfekcyjny unik wznosi nad tobą pełnię Rennali." + EVO3,
  "full_moon", {"on": "perfect_dodge", "cooldown": [7.0, 6.2, 5.5, 4.8, 4.0]}, [1.50, 1.75, 2.00, 2.30, 2.60],
  pdodge_text("Rennala's Full Moon"),
  requires=[("phantom_reprisal", 5), ("phantom_footwork", 5), ("windstep", 5)], weight=EV)
p("fortress_of_gold", "Fortress of Gold", "evolution", M,
  "Złota tarcza, postawa i strażnik gardy stają się twierdzą: zablokowany cios rozpala nad tobą gwiazdozbiór Elden." + EVO3,
  "elden_stars", {"on": "block", "chance": [0.50, 0.57, 0.64, 0.72, 0.80], "cooldown": [8.0, 7.2, 6.4, 5.7, 5.0]},
  [0.45, 0.53, 0.62, 0.70, 0.80], block_text("Elden Stars"),
  requires=[("golden_bulwark", 5), ("bulwark_stance", 5), ("guardkeeper", 5)], weight=EV)
p("fortissax_judgment", "Fortissax's Judgment", "evolution", M,
  "Meteoryt, punkt złamania i łamacz kamieni: złamana postawa ściąga czerwone włócznie Fortissaxa." + EVO3,
  "fortissax_spear", {"on": "stance_break", "cooldown": [9.0, 8.0, 7.0, 6.0, 5.0]}, [1.40, 1.65, 1.90, 2.15, 2.40],
  stance_text("Fortissax's Lightning Spear"), count=[1, 1, 2, 2, 3],
  requires=[("shattering_quake", 5), ("shatterpoint", 5), ("stonebreaker", 5)], weight=EV)
p("frozen_sovereign", "Frozen Sovereign", "evolution", M,
  "Lód blasku, zimna krew i krok mrozu: odmrożony wróg ściąga na siebie zamarznięte włócznie błyskawic." + EVO3,
  "frozen_lightning", st("frostbite", [5.0, 4.4, 3.8, 3.2, 2.6]), [1.20, 1.40, 1.60, 1.85, 2.10],
  status_text("Frozen Lightning Spear", "frostbite"), count=[1, 1, 2, 2, 3],
  requires=[("frost_shatter", 5), ("cold_blooded", 5), ("frost_step", 5)], weight=EV)
p("lord_of_frenzy", "Lord of Frenzied Flame", "evolution", M,
  "Zaraźliwe szaleństwo, wgląd szaleńca i przeklęty pakt: każdy status wroga zapala w nim Płomień Szaleństwa." + EVO3,
  "frenzied_burst", st("any", [6.0, 5.2, 4.5, 3.8, 3.0]), [1.10, 1.30, 1.50, 1.75, 2.00],
  status_text("Frenzied Burst", "any"), count=[1, 2, 2, 3, 3],
  requires=[("contagious_frenzy", 5), ("madmans_insight", 5), ("frenzied_pact", 5)], weight=EV)


# --------------------------------------------------------- 0.7 "Wygląd" ---
# Parry and backstab (reflexes.rs): an enemy attack that breaks off early in
# its parryable window (or a riposte without a stance break), and a critical
# with the victim facing away.
def parry_text(spell):
    def text(t, p, c):
        n = f"{c} × {spell}" if c > 1 else spell
        return f"Parowanie ciosu wroga: {n} w sparowanego (odnowienie {t['cooldown']:g} s, {scale(p)})."
    return text


def backstab_text(spell):
    def text(t, p, c):
        n = f"{c} × {spell}" if c > 1 else spell
        return f"Cios w plecy: {n} w ofiarę (odnowienie {t['cooldown']:g} s, {scale(p)})."
    return text


p("crystal_deflection", "Crystal Deflection", "reflex", U,
  "Sparowany cios rozbija się o kryształ: odłamki eksplodują w twarz napastnika.",
  "crystal_burst", {"on": "parry", "cooldown": [3.0, 2.7, 2.4, 2.1, 1.8]}, [0.70, 0.82, 0.95, 1.10, 1.30],
  parry_text("Crystal Burst"))
p("golden_retort", "Golden Retort", "reflex", E,
  "Odparty atak odpowiada złotym gniewem Złotego Porządku.",
  "wrath_of_gold", {"on": "parry", "cooldown": [5.0, 4.5, 4.0, 3.5, 3.0]}, [0.90, 1.05, 1.20, 1.40, 1.60],
  parry_text("Wrath of Gold"))
p("dragons_retort", "Dragon's Retort", "reflex", L,
  "Parowanie wzywa czerwoną błyskawicę starożytnych smoków na tego, kto zaatakował.",
  "ancient_lightning", {"on": "parry", "cooldown": [7.0, 6.2, 5.5, 4.8, 4.0]}, [1.20, 1.40, 1.60, 1.85, 2.10],
  parry_text("Ancient Dragons' Lightning Strike"))
p("shadows_kiss", "Shadow's Kiss", "reflex", U,
  "Cios w plecy zostawia w ofierze upiorny płomień śmierci.",
  "ghostflame_burst", {"on": "backstab", "cooldown": [4.0, 3.6, 3.2, 2.8, 2.4]}, [0.80, 0.95, 1.10, 1.25, 1.45],
  backstab_text("Ghostflame"))
p("crimson_ambush", "Crimson Ambush", "reflex", R,
  "Zasadzka kończy się krwawym płomieniem rozlanym z rany w plecach.",
  "bloodflame_burst", {"on": "backstab", "cooldown": [5.0, 4.5, 4.0, 3.5, 3.0]}, [0.90, 1.05, 1.20, 1.40, 1.60],
  backstab_text("Bloodflame Talons"), count=[1, 1, 1, 2, 2])
passive("riposte_tide", "Riposte Tide", "reflex", R, "Każde parowanie otwiera falę ripost.",
        ev("parry", 6), {f: [1.10, 1.13, 1.17, 1.21, 1.26] for f in ALL_ATTACK} | {"sa_attack_power_rate": [1.06, 1.09, 1.12, 1.15, 1.18]},
        lambda v: f"Przez 6 s po parowaniu: wszystkie zadawane obrażenia {x(v['physics_attack_rate'])}, obrażenia postury {x(v['sa_attack_power_rate'])}.")
passive("deflecting_calm", "Deflecting Calm", "reflex", U, "Spokój po odbitym ciosie: oddech wraca, ciało twardnieje.",
        ev("parry", 4), {"stamina_recover_change_speed": [12, 16, 21, 27, 34]} | {f: [.94, .91, .88, .85, .82] for f in CUTS},
        lambda v: f"Przez 4 s po parowaniu: regeneracja staminy +{v['stamina_recover_change_speed']}, otrzymywane obrażenia {x(v['slash_damage_cut_rate'])}.")
passive("shadow_step", "Shadow Step", "reflex", U, "Po ciosie w plecy rozpływasz się w cieniu.",
        ev("backstab", 8), {"physics_attack_rate": [1.06, 1.09, 1.12, 1.16, 1.20],
                            "sight_search_enemy_rate": [.85, .78, .70, .62, .55], "hearing_search_enemy_rate": [.85, .78, .70, .62, .55]},
        lambda v: f"Przez 8 s po ciosie w plecy: obrażenia fizyczne {x(v['physics_attack_rate'])}, wrogowie widzą i słyszą cię {x(v['sight_search_enemy_rate'])}.")
p("black_blade_verdict", "Verdict of the Black Blade", "evolution", M,
  "Złota riposta, fala ripost i znak kata: każde parowanie spuszcza na wroga czarne ostrze przeznaczenia." + EVO3,
  "black_blade", {"on": "parry", "cooldown": [6.0, 5.2, 4.5, 3.8, 3.2]}, [1.40, 1.65, 1.90, 2.20, 2.50],
  parry_text("Black Blade"), count=[1, 1, 2, 2, 3],
  requires=[("golden_retort", 5), ("riposte_tide", 5), ("headsmans_mark", 5)], weight=EV)


# 0.7.1: every status at once (user: "it would be nice if all of the effects
# appear at once at hit").
passive("litany_of_afflictions", "Litany of Afflictions", "status", E,
        "Każde trafienie niesie wszystkie sześć statusów naraz: krwawienie, odmrożenie, truciznę, zgniliznę, sen i szaleństwo.",
        ALWAYS, {f: [10, 14, 18, 23, 30] for f in ["blood_attack_power", "freeze_attack_power", "poizon_attack_power",
                                                     "disease_attack_power", "sleep_attack_power", "madness_attack_power"]},
        lambda v: f"Każde trafienie: +{v['blood_attack_power']} buildupu każdego z sześciu statusów." + SURGE_TEXT)


# ------------------------------------------------------------- 0.8 "Ascension" ---
# New cards for the new engine levers (attack speed, lifesteal, charged / jump /
# critical damage through talisman copies). Even steps, Mastery at V (+25%).
def art_card(id, name, cat, rarities, desc, field, top, text, family):
    levels = []
    for i, s in enumerate(STEPS):
        value = round(top * s * (1.25 if i == 4 else 1.0), 3)
        extra = " Mistrzostwo (V): +25%." if i == 4 else ""
        levels.append({"description": text(value) + extra, "activation": ALWAYS, "modifiers": [], field: value})
    card(id, name, cat, rarities, desc, levels, family=family)


pc = lambda v: f"{v * 100:.1f}%"
art_card("quickened_edge", "Quickened Edge", "offense", U, "Lżejsza dłoń, szybsze cięcia: ataki bronią przyspieszają.",
         "attack_speed", 0.06, lambda v: f"Animacje ataków szybsze o {pc(v)}.", "celerity")
art_card("whirling_fury", "Whirling Fury", "offense", E, "Ostrze tańczy szybciej niż oko wroga.",
         "attack_speed", 0.10, lambda v: f"Animacje ataków szybsze o {pc(v)}.", "celerity")
art_card("vampiric_edge", "Vampiric Edge", "recovery", R, "Ostrze pije krew i oddaje ją tobie.",
         "lifesteal", 0.03, lambda v: f"Leczysz {pc(v)} zadanych obrażeń.", "thirst")
art_card("sanguine_pact", "Sanguine Pact", "recovery", L, "Pakt krwi: każdy cios karmi twoje życie.",
         "lifesteal", 0.06, lambda v: f"Leczysz {pc(v)} zadanych obrażeń.", "thirst")
art_card("earthshaker_grip", "Earthshaker Grip", "offense", U, "Naładowany cios spada jak głaz.",
         "charge_damage", 0.20, lambda v: f"Naładowane ataki zadają +{pc(v)} obrażeń (jak Axe Talisman).", "ferocity")
art_card("sky_splitter", "Sky Splitter", "offense", U, "Skok kończy się uderzeniem z nieba.",
         "jump_damage", 0.20, lambda v: f"Ataki z wyskoku zadają +{pc(v)} obrażeń (jak Claw Talisman).", "ferocity")
art_card("executioners_edge", "Executioner's Edge", "offense", R, "Riposta i cios w plecy trafiają w samo serce.",
         "critical_damage", 0.30, lambda v: f"Ciosy krytyczne zadają +{pc(v)} obrażeń (jak Dagger Talisman).", "ferocity")
passive("bastion_of_will", "Bastion of Will", "defense", R, "Wola twardsza niż stal: trudniej cię zachwiać.",
        ALWAYS, {"toughness_damage_cut_rate": [.95, .9, .85, .8, .72], "guard_stamina_mult": [.95, .9, .85, .8, .72]},
        lambda v: f"Otrzymywane obrażenia stabilności {x(v['toughness_damage_cut_rate'])}, koszt staminy gardy {x(v['guard_stamina_mult'])}.",
        family="fortitude")


# Hidden powers switched on by the Ascension stats (perks at 10/20/30), Bonds
# (two stats at 10+) and Oaths (src/stats.rs). Never offered, never in the codex.
def hidden(id, name, family, desc, activation, fields, trigger="passive", cat="perk"):
    level = {"description": desc, "activation": activation,
             "modifiers": [{"field": k, "value": v} for k, v in fields.items()]}
    card(id, name, cat, M, desc, [dict(level) for _ in range(5)], family=family, hidden=True, trigger=trigger)


def hidden_proc(id, name, family, desc, proc_id, trigger, power, count=1, cat="perk"):
    proc(id, name, cat, M, desc, proc_id, trigger, [power] * 5, lambda t, p, c: desc, count=count,
         mastery=False, family=family, hidden=True)


ALL = lambda v: {f: v for f in ALL_ATTACK}
CUT = lambda v: {f: v for f in CUTS}
evt = lambda event, seconds: cond("after_event", event=event, seconds=seconds, minimum_stacks=1)
hidden("perk_bonebreaker", "Bonebreaker", "ferocity", "Ferocity 5: krytyk leczy 2% HP na sekundę przez 3 s.",
       evt("critical_hit", 3.0), {"change_hp_rate": -2, "motion_interval": 1})
hidden_proc("perk_cataclysm", "Cataclysm", "ferocity", "Ferocity 20: złamanie postawy wroga ściąga na niego meteoryt.",
            "meteorite", {"on": "stance_break", "cooldown": 8.0}, 1.6)
hidden("perk_titans_wrath", "Titan's Wrath", "ferocity", "Ferocity 30: przez 8 s po złamaniu postawy wszystkie obrażenia ×1.25.",
       evt("poise_break", 8.0), ALL(1.25))
hidden("perk_momentum", "Momentum", "celerity", "Celerity 10: przy serii 4 trafień wszystkie obrażenia ×1.10.",
       {"kind": "streak", "hits": 4}, ALL(1.10))
hidden_proc("perk_tempest", "Tempest", "celerity", "Celerity 30: seria 8 trafień przywołuje uderzenie pioruna w cel.",
            "sky_bolt", {"on": "streak", "hits": 8, "window": 6.0, "cooldown": 3.0}, 1.2)
hidden_proc("perk_arcane_storm", "Arcane Storm", "resonance", "Resonance 30: w walce co 10 s pełnia Rennali spada na wroga.",
            "full_moon", {"on": "interval", "seconds": 10.0}, 1.6)
hidden("perk_iron_will", "Iron Will", "fortitude", "Fortitude 10: poniżej 30% HP otrzymywane obrażenia ×0.85.",
       cond("hp_below", 0.3), CUT(0.85))
hidden("perk_juggernaut", "Juggernaut", "fortitude", "Fortitude 30: przez 2 s po trafieniu postura +40%, otrzymywane obrażenia ×0.90.",
       evt("hit_landed", 2.0), {"toughness_damage_cut_rate": 0.6} | CUT(0.9))
hidden("perk_second_wind", "Quickened Breath", "evasion", "Evasion 10: po perfekcyjnym uniku stamina wraca błyskawicznie przez 2 s.",
       evt("perfect_dodge", 2.0), {"stamina_recover_change_speed": 35})
hidden("perk_counterstrike", "Counterstrike", "evasion", "Evasion 20: przez 3 s po perfekcyjnym uniku wszystkie obrażenia ×1.35.",
       evt("perfect_dodge", 3.0), ALL(1.35))
hidden_proc("perk_phantom", "Phantom", "evasion", "Evasion 30: perfekcyjny unik wystrzeliwuje widmową strzałę Loretty w atakującego.",
            "loretta_greatbow", {"on": "perfect_dodge", "cooldown": 2.0}, 1.5)
hidden("perk_feast", "Feast", "thirst", "Thirst 10: zabicie wroga przywraca 3% HP.",
       ALWAYS, {"change_hp_rate": -3}, trigger="on_kill")
hidden("perk_crimson_tide", "Crimson Tide", "thirst", "Thirst 20: zabicie wroga przywraca 5 FP.",
       ALWAYS, {"change_mp_point": -5}, trigger="on_kill")
hidden("perk_midas", "Midas", "fortune", "Fortune 30: runy ×1.10.", ALWAYS, {"soul_rate": 1.1})


# --------------------------------------------------------------- 0.9 "Legion" ---
# Command (allies), Execution / Rampage events, deeper small families.
def ally_card(id, name, rarities, desc, fields, text):
    """Ally fields (ally_damage / ally_hp / ally_defense) in even steps, +25% at V."""
    levels = []
    for i, s in enumerate(STEPS):
        values = {k: round(v * s * (1.25 if i == 4 else 1.0), 3) for k, v in fields.items()}
        extra = " Mistrzostwo (V): +25%." if i == 4 else ""
        level = {"description": text(values) + extra, "activation": ALWAYS, "modifiers": []}
        level.update(values)
        levels.append(level)
    card(id, name, "command", rarities, desc, levels, family="command")


ap = lambda v: f"+{v * 100:.0f}%"
ally_card("spirit_callers_bell", "Spirit Caller's Bell", U, "Dzwon przyzywacza: duchy walczą zacieklej.",
          {"ally_damage": 0.25}, lambda v: f"Sojusznicy (duchy, Torrent) zadają {ap(v['ally_damage'])} obrażeń.")
ally_card("ancestral_vigor", "Ancestral Vigor", U, "Siła przodków wzmacnia ciała przywołanych.",
          {"ally_hp": 0.30}, lambda v: f"Sojusznicy mają {ap(v['ally_hp'])} HP.")
ally_card("mimics_mirror", "Mimic's Mirror", R, "Lustro mima: twoje odbicie i duchy rosną w siłę.",
          {"ally_damage": 0.18, "ally_hp": 0.18},
          lambda v: f"Sojusznicy: obrażenia {ap(v['ally_damage'])}, HP {ap(v['ally_hp'])}.")
ally_card("banner_of_the_fallen", "Banner of the Fallen", E, "Sztandar poległych: duchy nie cofają się przed niczym.",
          {"ally_damage": 0.25, "ally_hp": 0.25, "ally_defense": 0.12},
          lambda v: f"Sojusznicy: obrażenia {ap(v['ally_damage'])}, HP {ap(v['ally_hp'])}, otrzymywane obrażenia −{v['ally_defense'] * 100:.0f}%.")
ally_card("lord_of_spirits", "Lord of Spirits", L, "Władca duchów: twoja armia zmarłych jest legionem.",
          {"ally_damage": 0.40, "ally_hp": 0.40, "ally_defense": 0.18},
          lambda v: f"Sojusznicy: obrażenia {ap(v['ally_damage'])}, HP {ap(v['ally_hp'])}, otrzymywane obrażenia −{v['ally_defense'] * 100:.0f}%.")
passive("warband_spirit", "Warband Spirit", "command", U, "Ramię w ramię z sojusznikiem walczysz mocniej.",
        {"kind": "ally_near"}, {f: [1.03, 1.05, 1.07, 1.09, 1.12] for f in ALL_ATTACK},
        lambda v: f"Gdy sojusznik walczy w pobliżu (15 m): wszystkie zadawane obrażenia {x(v['physics_attack_rate'])}.",
        family="command")

# Execution (kill with a critical) and Rampage (three kills in 6 s).
passive("headsmans_toll", "Headsman's Toll", "reflex", R, "Każda egzekucja hartuje kata.",
        evt("execution", 10.0), {f: [1.08, 1.12, 1.17, 1.23, 1.30] for f in ALL_ATTACK},
        lambda v: f"Przez 10 s po egzekucji (zabójstwo krytykiem): wszystkie obrażenia {x(v['physics_attack_rate'])}.",
        family="ferocity")
p("deathblow_pyre", "Deathblow Pyre", "reflex", E, "Ciało straconego zamienia się w stos czarnego płomienia.",
  "black_flame_ritual", {"on": "execution", "cooldown": [8.0, 7.2, 6.4, 5.7, 5.0]}, [1.2, 1.35, 1.5, 1.65, 1.8],
  lambda t, pw, c: f"Egzekucja (zabójstwo krytykiem): Black Flame Ritual wokół ciała (odnowienie {t['cooldown']:g} s, {scale(pw)}).")
passive("bloodthirst_frenzy", "Bloodthirst Frenzy", "offense", R, "Rzeź napędza rzeź.",
        evt("rampage", 8.0), {f: [1.08, 1.11, 1.14, 1.17, 1.20] for f in ALL_ATTACK} | {"stamina_recover_change_speed": [8, 11, 14, 17, 20]},
        lambda v: f"Przez 8 s po rzezi (3 zabójstwa w 6 s): obrażenia {x(v['physics_attack_rate'])}, regeneracja staminy +{v['stamina_recover_change_speed']}.",
        family="celerity")
p("carnage_bloom", "Carnage Bloom", "reflex", L, "Z rzezi wyrasta szkarłatny kwiat.",
  "scarlet_aeonia", {"on": "rampage", "cooldown": [10.0, 9.0, 8.0, 7.0, 6.0]}, [1.4, 1.55, 1.7, 1.85, 2.0],
  lambda t, pw, c: f"Rzeź (3 zabójstwa w 6 s): Scarlet Aeonia na ostatnim ciele (odnowienie {t['cooldown']:g} s, {scale(pw)}).")

# Thirst, Evasion, Celerity, Fortune depth.
def single(id, name, cat, rarities, desc, activation, field, top, text, family, trigger="passive"):
    levels = []
    for i, s in enumerate(STEPS):
        value = round(top * s * (1.25 if i == 4 else 1.0), 3)
        extra = " Mistrzostwo (V): +25%." if i == 4 else ""
        levels.append({"description": text(value) + extra, "activation": activation, "modifiers": [], field: value})
    card(id, name, cat, rarities, desc, levels, family=family, trigger=trigger)


single("leeching_hex", "Leeching Hex", "recovery", R, "Klątwa pijawki: wybuch statusu otwiera żyły wroga.",
       evt("status_proc", 6.0), "lifesteal", 0.05, lambda v: f"Przez 6 s po wybuchu statusu u wroga leczysz {v * 100:.1f}% zadanych obrażeń.", "thirst")
card("gravefeast", "Gravefeast", "recovery", U, "Uczta nad grobem: każde zabójstwo cię karmi.",
     [{"description": f"Zabicie wroga przywraca {hp} HP.", "activation": ALWAYS,
       "modifiers": [{"field": "change_hp_point", "value": -hp}]} for hp in [16, 22, 28, 34, 50]],
     family="thirst", trigger="on_kill")
passive("crimson_reserve", "Crimson Reserve", "recovery", C, "Głębszy łyk: flakony szkarłatne leczą więcej.",
        ALWAYS, {"change_hp_estus_flask_correct_rate": [1.04, 1.06, 1.09, 1.12, 1.15]},
        lambda v: f"Leczenie flakonów szkarłatnych {x(v['change_hp_estus_flask_correct_rate'])}.", family="thirst")
passive("mistwalker", "Mistwalker", "mobility", U, "Mgła po uniku osłania twoje ciało.",
        cond("after_dodge", seconds=2.0), {f: [.96, .94, .91, .88, .85] for f in CUTS},
        lambda v: f"Przez 2 s po uniku: otrzymywane obrażenia {x(v['slash_damage_cut_rate'])}.", family="evasion")
p("mirage_step", "Mirage Step", "mobility", E, "Każdy unik zostawia kryształowy miraż, który pęka.",
  "crystal_burst", {"on": "dodge", "cooldown": [4.0, 3.6, 3.2, 2.8, 2.4]}, [0.6, 0.7, 0.8, 0.9, 1.0],
  dodge_text("Crystal Burst"))
passive("phantoms_grace", "Phantom's Grace", "mobility", R, "Po perfekcyjnym uniku ciało jest lekkie jak widmo.",
        evt("perfect_dodge", 4.0), {"stamina_recover_change_speed": [10, 14, 18, 22, 25], "consume_stamina_rate": [.95, .93, .9, .88, .85]},
        lambda v: f"Przez 4 s po perfekcyjnym uniku: regeneracja staminy +{v['stamina_recover_change_speed']}, koszt staminy {x(v['consume_stamina_rate'])}.",
        family="evasion")
single("second_hand", "Second Hand", "recovery", U, "Druga para rąk: przedmioty i flakony błyskawicznie.",
       ALWAYS, "item_speed", 0.15, lambda v: f"Animacje flakonów i przedmiotów szybsze o {v * 100:.0f}%.", "celerity")
passive("fleetfoot_charm", "Fleetfoot Charm", "endurance", C, "Lekki krok, szybki oddech.",
        ALWAYS, {"stamina_recover_change_speed": [4, 6, 8, 10, 12], "consume_stamina_rate": [.98, .97, .95, .94, .92]},
        lambda v: f"Regeneracja staminy +{v['stamina_recover_change_speed']}, koszt staminy {x(v['consume_stamina_rate'])}.",
        family="celerity")
passive("runebearers_toll", "Runebearer's Toll", "economy", U, "Każdy pokonany płaci daninę w runach.",
        ALWAYS, {"soul_rate": [1.04, 1.06, 1.09, 1.12, 1.15]},
        lambda v: f"Runy z wrogów {x(v['soul_rate'])}.", family="fortune")

# Hidden: Command perk 30, Bonds with Command, Oath of the Legion's toll.
hidden("perk_warlord", "Warlord", "command", "Command 30: gdy sojusznik walczy obok, obrażenia ×1.15, otrzymywane ×0.90.",
       {"kind": "ally_near"}, ALL(1.15) | CUT(0.9))


# ------------------------------------------------------ 2.0 signature cards ---
# Every great boss has its own card: its first defeat always offers it, and
# once that boss is beaten the card can also show up in any later reward.
# Legendary at I, Mythic from IV. A spell in the boss's style, plus a small
# passive bonus. Values go from I to V in even steps.
def lerp(a, b, i):
    return round(a + (b - a) * i / 4, 3)


def sig(id, name, bosses, family, proc_id=None, trigger=None, trigger_v=None, power=(1.0, 1.8), count=(1, 2),
        mods=None, extra=None, activation=ALWAYS):
    levels = []
    for i in range(5):
        level = {"description": "-", "activation": activation,
                 "modifiers": [{"field": f, "value": v} for f, v in at({k: [v] * 5 for k, v in (mods or {}).items()}, i).items()]}
        if proc_id:
            t = {}
            for k, v in trigger.items():
                v5 = (trigger_v or {}).get(k, v)
                if isinstance(v, (int, float)) and not isinstance(v, bool):
                    t[k] = lerp(v, v5, i)
                    if k == "hits":
                        t[k] = int(round(t[k]))
                else:
                    t[k] = v
            c = count[0] if i < 2 else count[1] if i == 4 else max(count[0], count[1] - 1)
            level["proc"] = {"proc": proc_id, "trigger": t, "power": lerp(power[0], power[1], i), "count": c}
        for k, (a, b) in (extra or {}).items():
            level[k] = lerp(a, b, i)
        levels.append(level)
    card(id, name, "signature", L, "-", levels, family=family, signature=bosses)


DMG = lambda v: {f: v for f in ALL_ATTACK}
# Base game.
sig("sig_margit", "Fell Omen's Daggers", ["boss_10000850"], "evasion", "wrath_of_gold",
    {"on": "perfect_dodge", "cooldown": 5.0}, {"cooldown": 2.5}, (1.0, 1.8), mods={"dark_attack_rate": 1.12})
sig("sig_godrick", "Grafted Dragon", ["boss_10000800"], "ferocity", "catch_flame",
    {"on": "stance_break", "cooldown": 6.0}, {"cooldown": 3.0}, (1.3, 2.3), mods={"sa_attack_power_rate": 1.2})
sig("sig_red_wolf", "Red Wolf's Glintblades", ["boss_14000850"], "celerity", "glintstone_bolt",
    {"on": "dodge", "cooldown": 3.0}, {"cooldown": 1.5}, (0.6, 1.1), (2, 3), extra={"attack_speed": (0.03, 0.07)})
sig("sig_rennala", "Queen of the Full Moon", ["boss_14000800"], "resonance", "full_moon",
    {"on": "interval", "seconds": 12.0}, {"seconds": 7.0}, (1.2, 2.1), mods={"magic_consumption_rate": 0.8})
sig("sig_loretta", "Loretta's Mastery", ["boss_1035500800", "boss_15000850"], "evasion", "loretta_greatbow",
    {"on": "critical", "cooldown": 4.0}, {"cooldown": 2.0}, (1.1, 1.9), (2, 3), mods={"magic_attack_rate": 1.1})
sig("sig_radahn", "Starscourge", ["boss_1252380800"], "ferocity", "meteorite",
    {"on": "streak", "hits": 5, "window": 6.0, "cooldown": 8.0}, {"cooldown": 4.5}, (1.4, 2.4), (1, 2),
    mods={"physics_attack_rate": 1.12})
sig("sig_rykard", "Blasphemous Hunger", ["boss_16000800"], "thirst", "rancor_spirits",
    {"on": "kill", "chance": 0.4, "cooldown": 3.0}, {"chance": 0.8, "cooldown": 1.5}, (0.8, 1.5),
    mods={"fire_attack_rate": 1.1}, extra={"lifesteal": (0.02, 0.05)})
sig("sig_godskin_duo", "Godskin Black Flame", ["boss_13000850", "boss_34130800", "boss_1042550800"], "resonance",
    "black_flame", {"on": "interval", "seconds": 9.0}, {"seconds": 5.0}, (0.9, 1.6), (1, 2))
sig("sig_morgott", "Omen King's Retort", ["boss_11000800"], "fortitude", "wrath_of_gold",
    {"on": "parry", "cooldown": 4.0}, {"cooldown": 2.0}, (1.2, 2.1), (1, 2), mods=CUT(0.88))
sig("sig_mohg", "Mohg's Bloodboon", ["boss_12050800", "boss_35000800"], "malice", "bloodboon",
    {"on": "status", "status": "blood_loss", "cooldown": 6.0}, {"cooldown": 3.0}, (1.2, 2.2),
    mods={"blood_attack_power": 45})
sig("sig_godfrey", "Hoarah Loux's Stomp", ["boss_11050800", "boss_11000850"], "fortitude", "rejection",
    {"on": "guard_counter", "cooldown": 4.0}, {"cooldown": 2.0}, (1.2, 2.2), mods={"toughness_damage_cut_rate": 0.7})
sig("sig_fire_giant", "Flame of the Fell God", ["boss_1252520800"], "fortitude", "fell_pyre",
    {"on": "hurt", "min_share": 0.1, "cooldown": 10.0}, {"cooldown": 5.0}, (1.2, 2.1),
    mods={"fire_damage_cut_rate": 0.75, "max_hp_rate": 1.08})
sig("sig_niall", "Frozen Leg of Niall", ["boss_1051570800"], "malice", "zamor_frost",
    {"on": "hit", "chance": 0.12, "cooldown": 3.0}, {"chance": 0.25, "cooldown": 1.8}, (0.9, 1.6),
    mods={"freeze_attack_power": 40})
sig("sig_placidusax", "Time-Ruined Lord", ["boss_13000830"], "celerity", "placidusax_ruin",
    {"on": "streak", "hits": 6, "window": 6.0, "cooldown": 7.0}, {"cooldown": 4.0}, (1.3, 2.2), (1, 2),
    mods={"stamina_recover_change_speed": 15})
sig("sig_maliketh", "Maliketh's Black Blade", ["boss_13000800"], "ferocity", "black_blade",
    {"on": "critical", "cooldown": 5.0}, {"cooldown": 2.5}, (1.5, 2.6), extra={"critical_damage": (0.1, 0.25)})
sig("sig_malenia", "Waterfowl's Blessing", ["boss_15000800"], "thirst", "scarlet_aeonia",
    {"on": "perfect_dodge", "cooldown": 8.0}, {"cooldown": 4.0}, (1.1, 1.9), extra={"lifesteal": (0.03, 0.06)})
sig("sig_regal_ancestor", "Regal Ancestor's Rite", ["boss_12090800"], "command", "rancor_spirits",
    {"on": "kill", "chance": 0.35, "cooldown": 3.0}, {"chance": 0.7, "cooldown": 1.5}, (0.8, 1.4),
    extra={"ally_damage": (0.15, 0.35), "ally_hp": (0.15, 0.35)})
sig("sig_astel", "Naturalborn of the Void", ["boss_12040800", "boss_32110800"], "resonance", "astel_meteor",
    {"on": "interval", "seconds": 16.0}, {"seconds": 9.0}, (1.3, 2.3), (1, 2))
sig("sig_fortissax", "Death Lightning", ["boss_12030850"], "evasion", "fortissax_spear",
    {"on": "perfect_dodge", "cooldown": 6.0}, {"cooldown": 3.0}, (1.3, 2.3), (1, 2))
sig("sig_elden_beast", "Elden Ring's Light", ["boss_19000800"], "resonance", "elden_stars",
    {"on": "interval", "seconds": 14.0}, {"seconds": 8.0}, (0.6, 1.1), (1, 2), mods={"dark_attack_rate": 1.15})
# Shadow of the Erdtree.
sig("sig_dancing_lion", "Divine Beast's Storm", ["boss_20000800"], "celerity", "frozen_lightning",
    {"on": "hit", "chance": 0.15, "cooldown": 3.0}, {"chance": 0.3, "cooldown": 1.8}, (1.0, 1.8),
    mods={"thunder_attack_rate": 1.1})
sig("sig_rellana", "Twin Moon Knight", ["boss_2048440800"], "resonance", "adula_moonblade",
    {"on": "streak", "hits": 4, "window": 6.0, "cooldown": 6.0}, {"cooldown": 3.5}, (1.0, 1.8), (1, 2),
    mods={"magic_attack_rate": 1.08, "fire_attack_rate": 1.08})
sig("sig_metyr", "Mother of Fingers", ["boss_25000800", "boss_2051450800"], "resonance", "ruin_stars",
    {"on": "interval", "seconds": 6.0}, {"seconds": 3.5}, (0.45, 0.8), (2, 4))
sig("sig_messmer", "Impaler's Flame", ["boss_21010800"], "ferocity", "black_flame",
    {"on": "hit", "chance": 0.15, "cooldown": 3.0}, {"chance": 0.3, "cooldown": 1.8}, (0.9, 1.6),
    mods={"fire_attack_rate": 1.12})
sig("sig_romina", "Scarlet Bud", ["boss_2044450800"], "malice", "scarlet_aeonia",
    {"on": "status", "status": "any", "cooldown": 8.0}, {"cooldown": 4.0}, (1.0, 1.8),
    mods={"disease_attack_power": 40})
sig("sig_putrescent", "Putrescent Ghostflame", ["boss_22000800"], "malice", "ghostflame_burst",
    {"on": "kill", "chance": 0.4, "cooldown": 3.0}, {"chance": 0.8, "cooldown": 1.5}, (0.9, 1.6),
    mods={"freeze_attack_power": 30})
sig("sig_gaius", "Gaius' Charge", ["boss_2049480800"], "fortitude", "meteorite",
    {"on": "guard_counter", "cooldown": 6.0}, {"cooldown": 3.0}, (1.2, 2.1), mods={"toughness_damage_cut_rate": 0.75})
sig("sig_bayle", "Bayle's Dread", ["boss_2054390800"], "ferocity", "ancient_lightning",
    {"on": "streak", "hits": 5, "window": 6.0, "cooldown": 6.0}, {"cooldown": 3.0}, (1.2, 2.1), (1, 2),
    mods={"thunder_attack_rate": 1.1, "fire_attack_rate": 1.1})
sig("sig_scadutree", "Scadutree Blessing", ["boss_2050480800"], "thirst",
    mods={"change_hp_rate": -0.5, "motion_interval": 1, "dark_damage_cut_rate": 0.85})
sig("sig_midra", "Frenzied Flame", ["boss_28000800"], "malice", "frenzied_burst",
    {"on": "interval", "seconds": 10.0}, {"seconds": 6.0}, (1.0, 1.8), mods={"madness_attack_power": 45})
sig("sig_consort_radahn", "Consort's Light", ["boss_20010800"], "ferocity", "meteorite",
    {"on": "streak", "hits": 6, "window": 6.0, "cooldown": 8.0}, {"cooldown": 4.0}, (1.5, 2.6), (2, 3),
    mods=DMG(1.1))


# ------------------------------------------------------- 3.0 Keystone cards ---
# Two per Path. Each one bends a build around one idea, often with a price
# or a demanding condition. Offered only once its Path has 6 points (the
# DLL excludes it from draws below that: runtime.rs KEYSTONE_GATE).
INT_FIELDS = {"stamina_recover_change_speed", "change_mp_point", "change_hp_point", "poizon_attack_power",
              "disease_attack_power", "blood_attack_power", "freeze_attack_power", "sleep_attack_power",
              "madness_attack_power", "curse_attack_power"}


def key(id, name, family, activation=ALWAYS, mods=None, const=None, extra=None, proc_id=None, trigger=None,
        trigger_v=None, power=(1.0, 1.6), count=(1, 1), meta=None, lure=None, trigger_kind="passive"):
    levels = []
    for i in range(5):
        values = {f: lerp(a, b, i) for f, (a, b) in (mods or {}).items()}
        values.update(const or {})
        for f in list(values):
            if f in INT_FIELDS or f.startswith("add_"):
                values[f] = int(round(values[f]))
        level = {"description": "-", "activation": activation,
                 "modifiers": [{"field": f, "value": v} for f, v in values.items()]}
        for k, (a, b) in (extra or {}).items():
            level[k] = lerp(a, b, i)
        if lure:
            level["elite_lure"] = {"chance_bonus": lerp(lure[0][0], lure[0][1], i),
                                   "ember_bonus": int(round(lerp(lure[1][0], lure[1][1], i)))}
        if proc_id:
            t = {}
            for k, v in trigger.items():
                v5 = (trigger_v or {}).get(k, v)
                if isinstance(v, (int, float)) and not isinstance(v, bool):
                    t[k] = lerp(v, v5, i)
                else:
                    t[k] = v
            c = count[0] if i < 3 else count[1]
            level["proc"] = {"proc": proc_id, "trigger": t, "power": lerp(power[0], power[1], i), "count": c}
        levels.append(level)
    card(id, name, "keystone", L, "-", levels, family=family, meta=meta, weight=0.9, trigger=trigger_kind)


DMG_R = lambda a, b: {f: (a, b) for f in ALL_ATTACK}
CUT_R = lambda a, b: {f: (a, b) for f in CUTS}
# Ferocity: heavy blows.
key("key_colossus_grip", "Colossus Grip", "ferocity",
    mods={"sa_attack_power_rate": (1.25, 1.45)}, const={"consume_stamina_rate": 1.12},
    extra={"charge_damage": (0.15, 0.35), "jump_damage": (0.15, 0.35)})
key("key_wrath_unbound", "Wrath Unbound", "ferocity", cond("hp_below", 0.5),
    mods={"physics_attack_rate": (1.15, 1.30), "sa_attack_power_rate": (1.20, 1.40)})
# Celerity: speed.
key("key_flurry_covenant", "Flurry Covenant", "celerity", {"kind": "streak", "hits": 4},
    mods=DMG_R(1.06, 1.12), extra={"attack_speed": (0.08, 0.16)})
key("key_windborne", "Windborne", "celerity", cond("load_below", 0.3),
    mods={**DMG_R(1.10, 1.18), "stamina_recover_change_speed": (8, 16), "consume_stamina_rate": (0.92, 0.84)})
# Malice: statuses.
key("key_hexblade_oath", "Hexblade Oath", "malice",
    mods={"blood_attack_power": (25, 55), "freeze_attack_power": (25, 55)}, const={"physics_attack_rate": 0.9})
key("key_rot_harvest", "Rot Harvest", "malice", evt("status_proc", 3.0),
    mods={"change_hp_rate": (-2.0, -3.0), "change_mp_point": (-4, -8)}, const={"motion_interval": 1})
# Resonance: card spells.
key("key_arcane_constellation", "Arcane Constellation", "resonance", proc_id="glintstone_icecrag",
    trigger={"on": "interval", "seconds": 7.0}, trigger_v={"seconds": 4.0}, power=(0.7, 1.2), count=(1, 2),
    const={"magic_consumption_rate": 1.2, "miracle_consumption_rate": 1.2, "arts_consumption_rate": 1.2})
key("key_spellblade_echo", "Spellblade Echo", "resonance", proc_id="adula_moonblade",
    trigger={"on": "hit", "chance": 0.2, "cooldown": 1.6}, trigger_v={"chance": 0.35, "cooldown": 1.0},
    power=(0.6, 1.0), count=(1, 1))
# Fortitude: defense.
key("key_living_fortress", "Living Fortress", "fortitude", {"kind": "heavy_load"},
    mods={**CUT_R(0.88, 0.80), "toughness_damage_cut_rate": (0.77, 0.62)})
key("key_aegis_oath", "Aegis Oath", "fortitude", {"kind": "guarding"},
    mods={"guard_stamina_mult": (0.7, 0.5), **CUT_R(0.85, 0.75)})
# Evasion: dodging.
key("key_duelists_edge", "Duelist's Edge", "evasion", evt("perfect_dodge", 3.0),
    mods={**DMG_R(1.25, 1.45), "stamina_recover_change_speed": (10, 20)})
key("key_phantom_veil", "Phantom Veil", "evasion", evt("perfect_dodge", 2.0),
    mods={**CUT_R(0.65, 0.45), "consume_stamina_rate": (0.85, 0.75)})
# Thirst: healing.
key("key_sanguine_covenant", "Sanguine Covenant", "thirst",
    const={"change_hp_estus_flask_correct_rate": 0.6}, extra={"lifesteal": (0.03, 0.06)})
key("key_crimson_frenzy", "Crimson Frenzy", "thirst", cond("hp_below", 0.4),
    mods=DMG_R(1.10, 1.20), extra={"lifesteal": (0.04, 0.08)})
# Fortune: runes and luck.
key("key_golden_wager", "Golden Wager", "fortune", mods={"soul_rate": (1.15, 1.35)},
    lure=((0.6, 1.2), (1, 3)))
key("key_fates_hand", "Fate's Hand", "fortune", mods={"item_drop_rate": (10, 30)},
    meta={"kind": "category_bias", "category": "keystone", "per_level": 0.25})
# Command: allies.
key("key_warlords_banner", "Warlord's Banner", "command",
    const=CUT(1.10), extra={"ally_damage": (0.30, 0.60), "ally_hp": (0.30, 0.60)})
key("key_shared_fate", "Shared Fate", "command", {"kind": "ally_near"},
    mods=CUT_R(0.88, 0.80), extra={"lifesteal": (0.02, 0.04)})

# ------------------------------------------------------- 3.2 No Quarter ---
# Your blows land on a critical's victim while it gets back up (normally
# invincible). I: the first 1 s of it; II: 1.5 s and +10% damage then; III:
# 2.5 s, +15%; IV: the whole time, +20%; V: the whole time, +35%. The DLL
# deals the damage of your usual hit on that foe (pursuit.rs).
card("no_quarter", "No Quarter", "reflex", R, "-",
     [{"description": "-", "activation": ALWAYS, "modifiers": [],
       "downed_strike": {"window": w, "bonus": b}}
      for w, b in [(1.0, 0.0), (1.5, 0.10), (2.5, 0.15), (99.0, 0.20), (99.0, 0.35)]],
     family="ferocity")

# ----------------------------------------------------- 3.4 Prismatic cards ---
# Ultra rare (a small fixed chance per card slot, by boss tier: draw.rs), all
# colours at once, a MAX WIN reveal. One per Path (two for Fortitude), each
# ridiculous on its own. Values go from I to V; no Mastery.
def prism(id, name, family, activation=ALWAYS, mods=None, const=None, extra=None, proc_id=None, trigger=None,
          trigger_v=None, power=(1.0, 1.6), count=(1, 2), lure=None, intervention=None):
    levels = []
    for i in range(5):
        values = {f: lerp(a, b, i) for f, (a, b) in (mods or {}).items()}
        values.update(const or {})
        for f in list(values):
            if f in INT_FIELDS or f.startswith("add_"):
                values[f] = int(round(values[f]))
        level = {"description": "-", "activation": activation,
                 "modifiers": [{"field": f, "value": v} for f, v in values.items()]}
        for k, (a, b) in (extra or {}).items():
            level[k] = lerp(a, b, i)
        if lure:
            level["elite_lure"] = {"chance_bonus": lerp(lure[0][0], lure[0][1], i),
                                   "ember_bonus": int(round(lerp(lure[1][0], lure[1][1], i)))}
        if intervention:
            level["intervention"] = {k: lerp(a, b, i) for k, (a, b) in intervention.items()}
        if proc_id:
            t = {}
            for k, v in trigger.items():
                v5 = (trigger_v or {}).get(k, v)
                t[k] = lerp(v, v5, i) if isinstance(v, (int, float)) and not isinstance(v, bool) else v
            c = int(round(lerp(count[0], count[1], i)))
            level["proc"] = {"proc": proc_id, "trigger": t, "power": lerp(power[0], power[1], i), "count": c}
        levels.append(level)
    card(id, name, "prismatic", M, "-", levels, family=family, weight=1.0)


ATTR_ALL = lambda a, b: {f: (a, b) for f in ATTR_FIELDS}
prism("prism_elden_overflow", "Elden Overflow", "ferocity",
      mods={**DMG_R(1.35, 1.6), "sa_attack_power_rate": (1.4, 1.8)})
prism("prism_tireless_titan", "Tireless Titan", "celerity",
      mods={"stamina_recover_change_speed": (30, 50), "consume_stamina_rate": (0.65, 0.45)},
      extra={"attack_speed": (0.08, 0.15)})
prism("prism_chaos_sovereign", "Chaos Sovereign", "malice",
      mods={f: (90, 160) for f in ["blood_attack_power", "freeze_attack_power", "poizon_attack_power",
                                   "disease_attack_power", "sleep_attack_power", "madness_attack_power"]})
prism("prism_starfall_apocalypse", "Starfall Apocalypse", "resonance", proc_id="astel_meteor",
      trigger={"on": "interval", "seconds": 6.0}, trigger_v={"seconds": 3.0}, power=(1.2, 2.0), count=(2, 4))
prism("prism_deathless", "Deathless", "fortitude", mods=CUT_R(0.85, 0.75),
      intervention={"cooldown": (120.0, 45.0), "heal_share": (0.5, 0.8), "ward_seconds": (3.0, 4.0),
                    "ward_cut": (0.4, 0.2)})
prism("prism_ascendant_form", "Ascendant Form", "fortitude", mods=ATTR_ALL(12, 30))
prism("prism_phantom_sovereign", "Phantom Sovereign", "evasion", proc_id="fortissax_spear",
      trigger={"on": "perfect_dodge", "cooldown": 1.5}, trigger_v={"cooldown": 0.8}, power=(1.4, 2.2), count=(2, 4))
prism("prism_endless_vigor", "Endless Vigor", "thirst",
      mods={"change_hp_rate": (-1.5, -2.5), "change_hp_estus_flask_correct_rate": (1.3, 1.6)},
      const={"motion_interval": 1}, extra={"lifesteal": (0.05, 0.10)})
prism("prism_golden_apotheosis", "Golden Apotheosis", "fortune",
      mods={"soul_rate": (2.0, 3.0), "item_drop_rate": (40, 75)}, lure=((2.0, 3.0), (3, 5)))
prism("prism_lord_of_the_legion", "Lord of the Legion", "command",
      extra={"ally_damage": (0.8, 1.0), "ally_hp": (0.8, 1.0), "ally_defense": (0.3, 0.5)})

# Families, Mastery at V and stat requirements, applied to the whole library.
STATUS_FIELDS = {"poizon_attack_power", "disease_attack_power", "blood_attack_power", "freeze_attack_power",
                 "sleep_attack_power", "madness_attack_power", "curse_attack_power"}
ATTRIBUTE_STAT = {"strength": "ferocity", "dexterity": "celerity", "intelligence": "resonance", "faith": "resonance",
                  "arcane": "malice", "vigor": "fortitude", "mind": "resonance", "endurance": "fortitude"}
REFLEX_STAT = {"perfect_dodge": "evasion", "dodge": "evasion", "block": "fortitude", "guard_counter": "fortitude",
               "parry": "fortitude", "critical_hit": "ferocity", "critical": "ferocity", "backstab": "ferocity",
               "poise_break": "ferocity", "stance_break": "ferocity"}
MASTERY = {
    "ferocity": ("sa_attack_power_rate", 1.10, "obrażenia postury ×1.10"),
    "celerity": ("consume_stamina_rate", 0.95, "koszt staminy ×0.95"),
    "resonance": ("magic_consumption_rate", 0.90, "koszt FP sorcery ×0.90"),
    "fortitude": ("toughness_damage_cut_rate", 0.90, "postura +10%"),
    "evasion": ("stamina_recover_change_speed", 5, "regeneracja staminy +5"),
    "thirst": ("change_hp_estus_flask_correct_rate", 1.10, "leczenie flakonów ×1.10"),
    "fortune": ("soul_rate", 1.05, "runy ×1.05"),
    "command": ("neutral_damage_cut_rate", 0.95, "otrzymywane obrażenia fizyczne ×0.95"),
}
MASTERY_MARK = "Mistrzostwo (V)"
FAMILY_OVERRIDE = {"dual_covenant": "resonance", "wandering_anvil": "ferocity", "ascendant_blood": "fortitude"}


def family_of(p, by_id):
    if p.get("family"):
        return p["family"]
    cat = p["category"]
    v = p["levels"][4]
    fields = {m["field"] for m in v["modifiers"]}
    if fields & STATUS_FIELDS or cat == "status":
        return "malice"
    proc = v.get("proc")
    if proc:
        return "malice" if proc["trigger"]["on"] == "status" else "resonance"
    if cat == "evolution" and p.get("requires"):
        parent = by_id.get(p["requires"][0][0])
        return family_of(parent, by_id) if parent else "resonance"
    if cat == "reflex":
        def events(a):
            if a.get("event"):
                yield a["event"]
            for c in a.get("conditions", []):
                yield from events(c)
        for e in events(v["activation"]):
            if e in REFLEX_STAT:
                return REFLEX_STAT[e]
            if e.endswith("_proc"):
                return "malice"
        return "ferocity"
    if p["id"] in FAMILY_OVERRIDE:
        return FAMILY_OVERRIDE[p["id"]]
    if cat == "attribute_synergy":
        for f in [m["field"] for m in v["modifiers"]]:
            if f.startswith("add_"):
                name = {"add_vigor": "vigor", "add_mind": "mind", "add_endurance": "endurance", "add_strength": "strength",
                        "add_dexterity": "dexterity", "add_intelligence": "intelligence", "add_faith": "faith",
                        "add_arcane": "arcane"}.get(f)
                if name:
                    return ATTRIBUTE_STAT[name]
        return "fortitude"
    return {"offense": "ferocity", "defense": "fortitude", "recovery": "thirst", "endurance": "celerity",
            "magic": "resonance", "arcane_arsenal": "resonance", "elemental": "resonance", "experimental": "resonance",
            "mobility": "evasion", "exploration": "fortune", "economy": "fortune", "roguelite": "fortune",
            "cursed": "ferocity" if fields & set(ALL_ATTACK) else "fortitude"}.get(cat, "fortune")


def convert_requirements(activation, family):
    """2.0: cards have no attribute or stat gates (the condition is dropped)."""
    kind = activation["kind"]
    if kind in ("attribute_at_least", "stat_at_least"):
        return None
    if kind == "all":
        parts = [convert_requirements(c, family) for c in activation["conditions"]]
        parts = [c for c in parts if c is not None]
        unique = []
        for c in parts:
            if c not in unique:
                unique.append(c)
        if not unique:
            return None
        return unique[0] if len(unique) == 1 else {"kind": "all", "conditions": unique}
    return activation


def add_mastery(p):
    family = p.get("family")
    if p.get("hidden") or p["trigger"] != "passive" or family not in MASTERY:
        return
    levels = p["levels"]
    if not levels[4]["modifiers"] or MASTERY_MARK in levels[4]["description"]:
        return
    field, value, text = MASTERY[family]
    present = any(m["field"] == field for m in levels[4]["modifiers"])
    for i, level in enumerate(levels):
        if present:
            if i == 4:
                for m in level["modifiers"]:
                    if m["field"] == field:
                        if field in ADDITIVE:
                            m["value"] = m["value"] + value
                        else:
                            m["value"] = round(m["value"] * value, 3)
        else:
            neutral = 0 if field in ADDITIVE else 1
            level["modifiers"].append({"field": field, "value": value if i == 4 else neutral})
    levels[4]["description"] += f" {MASTERY_MARK}: {text}."


def status_mastery(p):
    """Malice cards at V: +20% buildup of their own statuses."""
    if p.get("hidden") or p.get("family") != "malice":
        return
    levels = p["levels"]
    if not levels[4]["modifiers"] or MASTERY_MARK in levels[4]["description"]:
        return
    touched = False
    for m in levels[4]["modifiers"]:
        if m["field"] in STATUS_FIELDS:
            m["value"] = int(round(m["value"] * 1.2))
            touched = True
    if touched:
        levels[4]["description"] += f" {MASTERY_MARK}: buildup tej karty ×1.20."


def main():
    library = json.loads(PATH.read_text())
    powers = library["powers"]
    by_id = {p["id"]: p for p in powers}
    for pid, (fields, text) in REBALANCE.items():
        power = by_id[pid]
        old = [m["field"] for m in power["levels"][0]["modifiers"] if m["field"] in fields]
        if sorted(old) != sorted(fields):
            raise SystemExit(f"{pid}: field set changed {old} -> {list(fields)}")
        for i, level in enumerate(power["levels"]):
            values = at(fields, i)
            level["modifiers"] = [{"field": m, "value": values[m]} for m in old]
            level["description"] = text(values)
    for pid, text in DESCRIPTIONS.items():
        by_id[pid]["description"] = text
    for pid, (per_level, text) in META_REBALANCE.items():
        power = by_id[pid]
        power["meta"]["per_level"] = per_level
        for i, level in enumerate(power["levels"]):
            level["description"] = text(i)
    new_ids = {n["id"] for n in NEW}
    # 2.0: the 0.8 Bonds and Oaths are gone (hidden, never owned).
    kept = [p for p in powers if p["id"] not in new_ids and not p["id"].startswith(("bond_", "oath_"))]
    existing = {p["id"]: p for p in powers}
    # Every card owns 8 row ids; the DLL refuses a library with overlaps.
    # Keep a card's earlier block when it is still free, else take a new one
    # above everything used (0.4.0 shipped two new cards on a taken block).
    used = {p["effect_row_base"] for p in kept}
    next_base = max([b for p in powers for b in [p["effect_row_base"]]]) + 8
    for new in NEW:
        new = dict(new)
        base = existing.get(new["id"], {}).get("effect_row_base")
        if base is None or base in used:
            base = next_base
            next_base += 8
        new["effect_row_base"] = base
        used.add(base)
        kept.append(new)
    # 0.8 post-pass over the whole library: families, stat requirements
    # instead of vanilla ones, Mastery at V (passive cards).
    final = {p["id"]: p for p in kept}
    fresh = new_ids | set(REBALANCE)
    for power in kept:
        if power.get("hidden"):
            continue
        power["family"] = family_of(power, final)
        for level in power["levels"]:
            converted = convert_requirements(level["activation"], power["family"])
            level["activation"] = converted if converted is not None else {"kind": "always"}
        # 3.0: Keystones are complete as authored (no Mastery at V).
        if power["id"] not in fresh or power["category"] in ("keystone", "prismatic"):
            continue
        if power["family"] == "malice":
            status_mastery(power)
        else:
            add_mastery(power)
    if max(used) + 7 > 95_004_004:
        raise SystemExit("effect row reservations exhausted")
    rows = [p["effect_row_base"] + i for p in kept for i in range(5)]
    if len(rows) != len(set(rows)):
        raise SystemExit("effect row ids overlap")
    library["powers"] = kept
    PATH.write_text(json.dumps(library, ensure_ascii=False, indent=2) + "\n")
    # 0.8: synergies need cards only (no vanilla attribute gates).
    syn_path = PROJECT / "config/synergies.json"
    synergies = json.loads(syn_path.read_text())
    for synergy in synergies["synergies"]:
        synergy.pop("attributes", None)
    syn_path.write_text(json.dumps(synergies, ensure_ascii=False, indent=2) + "\n")
    print(f"{len(kept)} powers ({len(NEW)} authored since 0.3), rebalanced {len(REBALANCE)} + {len(META_REBALANCE)} meta")


if __name__ == "__main__":
    main()
