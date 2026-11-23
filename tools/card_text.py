#!/usr/bin/env python3
"""2.0: every card text in short, plain English, written from the card's real
numbers (config/powers.json levels and config/synergies.json).

One line per level: "Below 50% HP: heal 0.7% HP per second." Run after
tools/author_powers.py; it only rewrites texts, never numbers, so it can run
any number of times.
"""
import json
from pathlib import Path

PROJECT = Path(__file__).resolve().parents[1]
POWERS = PROJECT / "config/powers.json"
SYNERGIES = PROJECT / "config/synergies.json"
PROCS = PROJECT / "config/procs.json"

ATTACK = ["physics_attack_rate", "magic_attack_rate", "fire_attack_rate", "thunder_attack_rate", "dark_attack_rate"]
PHYS_KINDS = ["slash_attack_rate", "blow_attack_rate", "thrust_attack_rate"]
CUTS = ["neutral_damage_cut_rate", "slash_damage_cut_rate", "blow_damage_cut_rate", "thrust_damage_cut_rate",
        "magic_damage_cut_rate", "fire_damage_cut_rate", "thunder_damage_cut_rate", "dark_damage_cut_rate"]
PHYS_CUTS = CUTS[:4]
ELEM_CUTS = CUTS[4:]
ELEM_ATTACK = ATTACK[1:]
RESISTS = ["regist_poizon_change_rate", "regist_disease_change_rate", "regist_blood_change_rate",
           "regist_freeze_change_rate", "regist_sleep_change_rate", "regist_madness_change_rate",
           "regist_curse_change_rate"]
SPELL_COSTS = ["magic_consumption_rate", "miracle_consumption_rate"]

# Multipliers: label, True when a higher value is better.
RATE = {
    "physics_attack_rate": ("physical damage", True), "slash_attack_rate": ("slash damage", True),
    "blow_attack_rate": ("strike damage", True), "thrust_attack_rate": ("pierce damage", True),
    "magic_attack_rate": ("magic damage", True), "fire_attack_rate": ("fire damage", True),
    "thunder_attack_rate": ("lightning damage", True), "dark_attack_rate": ("holy damage", True),
    "sa_attack_power_rate": ("stance damage", True), "stamina_attack_rate": ("stamina damage to foes", True),
    "neutral_damage_cut_rate": ("physical damage taken", False), "slash_damage_cut_rate": ("slash damage taken", False),
    "blow_damage_cut_rate": ("strike damage taken", False), "thrust_damage_cut_rate": ("pierce damage taken", False),
    "magic_damage_cut_rate": ("magic damage taken", False), "fire_damage_cut_rate": ("fire damage taken", False),
    "thunder_damage_cut_rate": ("lightning damage taken", False), "dark_damage_cut_rate": ("holy damage taken", False),
    "max_hp_rate": ("max HP", True), "max_mp_rate": ("max FP", True), "max_stamina_rate": ("max stamina", True),
    "consume_stamina_rate": ("stamina costs", False), "guard_stamina_mult": ("stamina lost when blocking", False),
    "equip_weight_change_rate": ("equip load", True), "magic_consumption_rate": ("sorcery FP cost", False),
    "miracle_consumption_rate": ("incantation FP cost", False), "arts_consumption_rate": ("skill FP cost", False),
    "change_hp_estus_flask_correct_rate": ("Crimson Flask healing", True),
    "change_mp_estus_flask_correct_rate": ("Cerulean Flask FP", True), "soul_rate": ("runes", True),
    "fall_damage_rate": ("fall damage", False), "sight_search_enemy_rate": ("enemy sight", False),
    "hearing_search_enemy_rate": ("enemy hearing", False),
    "regist_poizon_change_rate": ("poison resistance", True), "regist_disease_change_rate": ("rot resistance", True),
    "regist_blood_change_rate": ("bleed resistance", True), "regist_freeze_change_rate": ("frost resistance", True),
    "regist_sleep_change_rate": ("sleep resistance", True), "regist_madness_change_rate": ("madness resistance", True),
    "regist_curse_change_rate": ("death blight resistance", True),
    "poison_def_damage_rate": ("poison damage taken", False), "disease_def_damage_rate": ("rot damage taken", False),
    "change_sa_recovery_velocity": ("poise recovery speed", True),
}
STATUS = {"blood_attack_power": "blood loss", "poizon_attack_power": "poison", "disease_attack_power": "scarlet rot",
          "freeze_attack_power": "frostbite", "sleep_attack_power": "sleep", "madness_attack_power": "madness",
          "curse_attack_power": "death blight"}
ATTRS = {"add_vigor": "Vigor", "add_mind": "Mind", "add_endurance": "Endurance", "add_strength": "Strength",
         "add_dexterity": "Dexterity", "add_intelligence": "Intelligence", "add_faith": "Faith", "add_arcane": "Arcane"}
EVENTS = {
    "hit_landed": "a hit", "kill": "a kill", "perfect_dodge": "a perfect dodge", "status_proc": "a status pops on a foe",
    "critical_hit": "a critical (riposte, backstab, stance break)", "guard_counter": "a guard counter", "poise_break": "breaking a stance",
    "elite_kill": "killing an Ascended Elite", "block": "blocking a hit", "bleed_proc": "blood loss pops",
    "frostbite_proc": "frostbite pops", "poison_proc": "poison pops", "sleep_proc": "sleep pops",
    "madness_proc": "madness pops", "parry": "a parry", "backstab": "a backstab", "execution": "a killing critical",
    "rampage": "a rampage (3 kills in 6 s)",
}
STATUS_FILTER = {"any": "any status", "blood_loss": "blood loss", "frostbite": "frostbite", "poison": "poison or rot",
                 "sleep": "sleep", "madness": "madness", "death_blight": "death blight"}
PATHS = {"ferocity": "Ferocity", "celerity": "Celerity", "malice": "Malice", "resonance": "Resonance",
         "fortitude": "Fortitude", "evasion": "Evasion", "thirst": "Thirst", "fortune": "Fortune", "command": "Command"}
CATEGORIES = {"status": "Status", "offense": "Offense", "arcane_arsenal": "Spell", "defense": "Defense",
              "magic": "Magic", "recovery": "Recovery"}


def num(v):
    """3, 2.5, 0.35 - no trailing zeros."""
    v = float(v)
    if abs(v - round(v)) < 1e-6:
        return str(int(round(v)))
    if abs(v) >= 10:
        return str(int(v + 0.5)) if v > 0 else str(-int(-v + 0.5))
    text = f"{v:.1f}" if abs(v) >= 1 else f"{v:.2f}"
    return text.rstrip("0").rstrip(".")


def percent(m):
    """Multiplier -> signed percent text: 1.14 -> '+14%', 0.92 -> '-8%'."""
    p = (m - 1.0) * 100.0
    if abs(p) >= 1:
        n = int(abs(p) + 0.5)
        return f"{'+' if p > 0 else '-'}{n}%"
    return f"{'+' if p > 0 else '-'}{num(abs(p))}%"


def neutral(field, value):
    if field in RATE or field == "toughness_damage_cut_rate":
        return abs(value - 1.0) < 5e-3
    return abs(value) < 1e-4


def good(field, value):
    if field == "toughness_damage_cut_rate":
        return value < 1.0
    if field in RATE:
        higher = RATE[field][1]
        return (value > 1.0) == higher
    if field in ("change_hp_rate", "change_hp_point", "change_mp_rate", "change_mp_point"):
        return value < 0
    return value > 0


def effects(mods):
    """Modifier dict -> list of short phrases (grouped where possible)."""
    mods = {k: v for k, v in mods.items() if k != "motion_interval" and not neutral(k, v)}
    out = []

    def group(fields, label, sign_better_low=False):
        values = [mods.get(f) for f in fields]
        if all(v is not None for v in values) and len({round(v, 4) for v in values}) == 1:
            out.append(f"{percent(values[0])} {label}")
            for f in fields:
                mods.pop(f)

    group(ATTACK, "damage")
    group(PHYS_KINDS, "physical damage")
    group(CUTS, "damage taken")
    group(PHYS_CUTS, "physical damage taken")
    group(ELEM_CUTS, "elemental damage taken")
    group(RESISTS, "resistance to every status")
    if all(f in mods for f in SPELL_COSTS + ["arts_consumption_rate"]) and \
            len({round(mods[f], 4) for f in SPELL_COSTS + ["arts_consumption_rate"]}) == 1:
        out.append(f"{percent(mods['arts_consumption_rate'])} FP costs")
        for f in SPELL_COSTS + ["arts_consumption_rate"]:
            mods.pop(f)
    group(SPELL_COSTS, "spell FP cost")
    group(ELEM_ATTACK, "elemental damage")
    # 3.4: every attribute at once, and statuses sharing one value.
    attrs = list(ATTRS)
    if all(f in mods for f in attrs) and len({round(mods[f], 4) for f in attrs}) == 1:
        out.append(f"+{num(mods[attrs[0]])} to every attribute")
        for f in attrs:
            mods.pop(f)
    by_value = {}
    for f in [f for f in STATUS if f in mods]:
        by_value.setdefault(round(mods[f], 4), []).append(f)
    for value, fields in by_value.items():
        if len(fields) >= 3:
            names = [STATUS[f] for f in fields]
            joined = ", ".join(names[:-1]) + " and " + names[-1]
            out.append(f"+{num(value)} {joined} buildup on hits")
            for f in fields:
                mods.pop(f)
    merged = {}
    for field in list(mods):
        if field not in RATE:
            continue
        label = RATE[field][0]
        for suffix in (" damage taken", " damage", " resistance"):
            if label.endswith(suffix) and label != suffix.strip():
                key = (percent(mods[field]), suffix)
                merged.setdefault(key, []).append((field, label[: -len(suffix)]))
                break
    for (value, suffix), items in merged.items():
        if len(items) < 2:
            continue
        names = [n for _, n in items]
        joined = ", ".join(names[:-1]) + " and " + names[-1]
        out.append(f"{value} {joined}{suffix}")
        for f, _ in items:
            mods.pop(f)
    for field, value in mods.items():
        if field == "toughness_damage_cut_rate":
            out.append(f"{percent(2.0 - value)} poise")
        elif field in RATE:
            out.append(f"{percent(value)} {RATE[field][0]}")
        elif field == "stamina_recover_change_speed":
            out.append(f"{'+' if value > 0 else ''}{num(value)} stamina regen")
        elif field == "item_drop_rate":
            out.append(f"{'+' if value > 0 else ''}{int(round(value))} item discovery")
        elif field == "magic_effect_time_change":
            out.append(f"buffs last {num(value)} s longer")
        elif field == "change_hp_rate":
            out.append(f"heal {num(-value)}% HP per second" if value < 0 else f"lose {num(value)}% HP per second")
        elif field == "change_hp_point":
            out.append(f"heal {num(-value)} HP per second" if value < 0 else f"lose {num(value)} HP per second")
        elif field == "change_mp_rate":
            out.append(f"regain {num(-value)}% FP per second" if value < 0 else f"lose {num(value)}% FP per second")
        elif field == "change_mp_point":
            out.append(f"regain {num(-value)} FP per second" if value < 0 else f"lose {num(value)} FP per second")
        elif field in STATUS:
            out.append(f"+{num(value)} {STATUS[field]} buildup on hits")
        elif field in ATTRS:
            out.append(f"{'+' if value > 0 else ''}{num(value)} {ATTRS[field]}")
        elif field == "dexterity_cancel_system_only_add_dexterity":
            out.append(f"casts as if +{num(value)} Dexterity")
        else:
            out.append(f"{field} {num(value)}")
    return out


def condition(a):
    """Activation -> prefix like 'Below 50% HP' (empty for always)."""
    k = a["kind"]
    r = lambda: f"{int(a['ratio'] * 100 + 0.5)}%"
    if k == "always":
        return ""
    if k == "hp_below":
        return f"Below {r()} HP"
    if k == "hp_above":
        return f"At {r()}+ HP"
    if k == "fp_below":
        return f"Below {r()} FP"
    if k == "fp_above":
        return f"At {r()}+ FP"
    if k == "stamina_below":
        return f"Below {r()} stamina"
    if k == "stamina_above":
        return f"At {r()}+ stamina"
    if k == "load_below":
        return f"Under {r()} equip load"
    if k == "no_hp_loss_for":
        return f"After {num(a['seconds'])} s without damage"
    if k == "after_hp_loss":
        return f"For {num(a['seconds'])} s after you take damage"
    if k == "after_event":
        event = EVENTS.get(a["event"], a["event"].replace("_", " "))
        stacks = a.get("minimum_stacks", 1)
        if stacks > 1 and a["event"] == "hit_landed":
            return f"After {stacks} quick hits, for {num(a['seconds'])} s"
        if stacks > 1:
            return f"After {event} x{stacks}, for {num(a['seconds'])} s"
        return f"For {num(a['seconds'])} s after {event}"
    if k == "boss_combat":
        return "In boss fights"
    if k == "dodging":
        return "While dodging"
    if k == "guarding":
        return "While blocking"
    if k == "airborne":
        return "In the air"
    if k == "heavy_load":
        return "With a heavy load"
    if k == "after_dodge":
        return f"For {num(a['seconds'])} s after a dodge"
    if k == "combat_for":
        return f"After {num(a['seconds'])} s of a boss fight"
    if k == "streak":
        return f"During a {a['hits']}-hit streak"
    if k == "ally_near":
        return "With an ally nearby"
    if k == "path_at_least":
        return f"With {a['value']}+ {PATHS.get(a['path'], a['path'])} points"
    if k == "attribute_at_least":
        return f"With {a['value']}+ {a['attribute'].title()}"
    if k == "all":
        parts = [condition(c) for c in a["conditions"]]
        parts = [p for p in parts if p]
        return " and ".join([parts[0]] + [p[0].lower() + p[1:] for p in parts[1:]]) if parts else ""
    return k.replace("_", " ")


def spell_text(proc, spells):
    name = spells.get(proc["proc"], proc["proc"].replace("_", " "))
    c = proc.get("count", 1)
    spell = f"{c}x {name}" if c > 1 else name
    t = proc["trigger"]
    on = t["on"]
    cd = lambda: f" ({num(t['cooldown'])} s cooldown)"
    ch = lambda: f"{int(t['chance'] * 100 + 0.5)}%"
    if on == "interval":
        return f"Every {num(t['seconds'])} s in combat: {spell} at your foe"
    if on == "hit":
        return f"{ch()} chance on hit: {spell}" + cd()
    if on == "streak":
        return f"{t['hits']} hits in {num(t.get('window', 6))} s: {spell}" + cd()
    if on == "kill":
        return f"{ch()} chance a slain foe bursts: {spell}" + cd()
    if on == "dodge":
        return f"Dodging releases {spell}" + cd()
    if on == "hurt":
        return f"A hit for {int(t['min_share'] * 100 + 0.5)}%+ of your HP releases {spell}" + cd()
    if on == "perfect_dodge":
        return f"A perfect dodge fires {spell} at the attacker" + cd()
    if on == "block":
        return f"{ch()} chance when you block: {spell} at the attacker" + cd()
    if on == "guard_counter":
        return f"Guard counters release {spell}" + cd()
    if on == "critical":
        return f"Criticals (ripostes, backstabs, stance breaks) release {spell} once the foe can be hit" + cd()
    if on == "stance_break":
        return f"Breaking a stance calls {spell}" + cd()
    if on == "status":
        return f"When {STATUS_FILTER.get(t['status'], t['status'])} pops on a foe: {spell}" + cd()
    if on == "parry":
        return f"A parry releases {spell}" + cd()
    if on == "backstab":
        return f"Backstabs release {spell}" + cd()
    if on == "execution":
        return f"A killing critical releases {spell}" + cd()
    if on == "rampage":
        return f"3 kills in 6 s: {spell}" + cd()
    return f"{spell} ({on})"


def pc(v):
    return f"{num(v * 100)}%"


def special(level):
    out = []
    if level.get("lifesteal"):
        out.append(f"heal {pc(level['lifesteal'])} of the damage you deal")
    if level.get("attack_speed"):
        out.append(f"attacks {pc(level['attack_speed'])} faster")
    if level.get("item_speed"):
        out.append(f"flasks and items {pc(level['item_speed'])} faster")
    if level.get("charge_damage"):
        out.append(f"+{pc(level['charge_damage'])} charged attack damage")
    if level.get("jump_damage"):
        out.append(f"+{pc(level['jump_damage'])} jump attack damage")
    if level.get("critical_damage"):
        out.append(f"+{pc(level['critical_damage'])} critical damage")
    ally = []
    if level.get("ally_damage"):
        ally.append(f"+{pc(level['ally_damage'])} damage")
    if level.get("ally_hp"):
        ally.append(f"+{pc(level['ally_hp'])} HP")
    if level.get("ally_defense"):
        ally.append(f"-{pc(level['ally_defense'])} damage taken")
    if ally:
        out.append("allies get " + ", ".join(ally))
    if level.get("intervention"):
        i = level["intervention"]
        minutes = i["cooldown"] / 60
        when = f"every {num(round(minutes, 1))} min" if minutes >= 1 else f"every {num(i['cooldown'])} s"
        out.append(f"{when}: survive a deadly blow and heal {pc(i['heal_share'])} HP")
    if level.get("elite_lure"):
        e = level["elite_lure"]
        parts = []
        if e.get("chance_bonus"):
            parts.append(f"+{pc(e['chance_bonus'])} Ascended Elites")
        if e.get("ember_bonus"):
            parts.append(f"+{e['ember_bonus']} Ember{'s' if e['ember_bonus'] > 1 else ''} per Elite")
        out.append(", ".join(parts))
    if level.get("blood_price"):
        out.append(f"every FP you spend also costs {num(level['blood_price'])} HP")
    if level.get("downed_strike"):
        d = level["downed_strike"]
        span = "the whole time" if d["window"] >= 10 else f"for the first {num(d['window'])} s"
        text = f"after your critical, the foe can be hit while it gets back up ({span}): weapon, spells and all"
        if d.get("bonus"):
            text += f"; it takes +{pc(d['bonus'])} damage then"
        out.append(text)
    return out


def sentence(prefix, parts):
    parts = [p for p in parts if p]
    if not parts:
        return ""
    body = ", ".join(parts)
    if prefix:
        return f"{prefix}: {body}."
    return body[0].upper() + body[1:] + "."


def meta_text(meta, level):
    k = meta["kind"]
    if k == "fortune":
        return f"Better odds of rare cards (+{pc(meta['per_level'] * level)})."
    if k == "rerolls":
        n = meta["per_level"] * level
        return f"+{n} free reroll{'s' if n > 1 else ''} on every reward."
    if k == "category_bias":
        label = CATEGORIES.get(meta["category"], meta["category"].replace("_", " ").title())
        return f"{label} cards show up {pc(meta['per_level'] * level)} more often."
    if k == "affinity":
        return f"Cards you own show up {pc(meta['per_level'] * level)} more often."
    if k == "lock_discount":
        return "Locking a card is free." if level >= 5 else "Locking a card costs 1 Ember."
    if k == "skip_bonus":
        embers = 2 if level >= 5 else 1 if level >= 3 else 0
        text = f"Skipping a reward pays {pc(meta['per_level'] * level)} more runes"
        return text + (f" and +{embers} Ember." if embers else ".")
    return k


def level_text(power, i, spells, drawback):
    level = power["levels"][i]
    mods = {m["field"]: m["value"] for m in level["modifiers"]}
    for field in drawback:
        mods.pop(field, None)
    trigger = power.get("trigger", "passive")
    prefix = condition(level["activation"])
    texts = []
    if power.get("meta"):
        texts.append(meta_text(power["meta"], i + 1))
    if trigger == "on_kill":
        heal = []
        fp = []
        if mods.get("change_hp_rate"):
            heal.append(f"{num(-mods['change_hp_rate'])}% HP")
        if mods.get("change_hp_point"):
            heal.append(f"{num(-mods['change_hp_point'])} HP")
        if mods.get("change_mp_rate"):
            fp.append(f"{num(-mods['change_mp_rate'])}% FP")
        if mods.get("change_mp_point"):
            fp.append(f"{num(-mods['change_mp_point'])} FP")
        kill = []
        if heal:
            kill.append("heal " + " + ".join(heal))
        if fp:
            kill.append("regain " + " + ".join(fp))
        texts.append(sentence("Kills", kill))
    elif trigger == "on_kill_timed":
        texts.append(sentence(f"For {num(power.get('duration_seconds', 0))} s after a kill", effects(mods)))
    elif trigger == "on_momentum":
        texts.append(sentence(f"While you keep hitting (lasts {num(power.get('duration_seconds', 0))} s)", effects(mods)))
    elif trigger == "on_nearby_affliction":
        texts.append(sentence(f"For {num(power.get('duration_seconds', 0))} s after a status pops near you", effects(mods)))
    elif trigger == "on_critical_blessing":
        texts.append(sentence("After a riposte or backstab", effects(mods)))
    else:
        parts = effects(mods) + special(level)
        if parts:
            texts.append(sentence(prefix, parts))
    if level.get("proc"):
        text = spell_text(level["proc"], spells)
        if prefix and trigger == "passive" and not level["modifiers"]:
            text = f"{prefix}: {text[0].lower()}{text[1:]}"
        texts.append(text + ".")
    return " ".join(t for t in texts if t) or "No effect."


def drawback_fields(power):
    """Curse cards: modifiers that are bad and the same at every level."""
    if not power.get("curse"):
        return set()
    out = set()
    first = {m["field"]: m["value"] for m in power["levels"][0]["modifiers"]}
    for field, value in first.items():
        if field == "motion_interval" or neutral(field, value) or good(field, value):
            continue
        if all(any(m["field"] == field and abs(m["value"] - value) < 1e-6 for m in l["modifiers"]) for l in power["levels"]):
            out.add(field)
    return out


def curse_text(power, drawback):
    first = {m["field"]: m["value"] for m in power["levels"][0]["modifiers"] if m["field"] in drawback}
    parts = effects(first)
    if power["levels"][0].get("blood_price"):
        parts.append("spending FP costs HP")
    if not parts:
        return power.get("curse")
    text = ", ".join(parts)
    return text[0].upper() + text[1:] + "."


def synergy_text(s, spells):
    mods = {m["field"]: m["value"] for m in s.get("modifiers", [])}
    prefix = condition(s["activation"]) if s.get("activation") else ""
    parts = effects(mods)
    for proc, factor in s.get("proc_power", {}).items():
        parts.append(f"{spells.get(proc, proc)} hits {percent(factor).lstrip('+')} harder")
    return sentence(prefix, parts) or s.get("description", "")


def main():
    spells = {}
    for proc in json.loads(PROCS.read_text())["procs"]:
        name = proc.get("name", proc["id"])
        spells[proc["id"]] = name[0].upper() + name[1:]
    library = json.loads(POWERS.read_text())
    for power in library["powers"]:
        drawback = drawback_fields(power)
        for i, level in enumerate(power["levels"]):
            level["description"] = level_text(power, i, spells, drawback)
        power["description"] = power["levels"][0]["description"]
        if power.get("curse"):
            power["curse"] = curse_text(power, drawback)
    POWERS.write_text(json.dumps(library, ensure_ascii=False, indent=2) + "\n")
    book = json.loads(SYNERGIES.read_text())
    for s in book["synergies"]:
        s["description"] = synergy_text(s, spells)
    SYNERGIES.write_text(json.dumps(book, ensure_ascii=False, indent=2) + "\n")
    print(f"card texts: {len(library['powers'])} powers, {len(book['synergies'])} synergies")


if __name__ == "__main__":
    main()
