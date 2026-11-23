#!/usr/bin/env python3
"""Write docs/POWERS.md from config/powers.json, procs.json and synergies.json."""
from __future__ import annotations
import json
from pathlib import Path

PROJECT = Path(__file__).resolve().parents[1]
ROMAN = ["I", "II", "III", "IV", "V"]
CATEGORY = {
    "arcane_arsenal": "Arcane Arsenal (automatyczne czary)", "offense": "Offense", "defense": "Defense",
    "recovery": "Recovery", "endurance": "Endurance", "magic": "Magic", "mobility": "Mobility",
    "exploration": "Exploration", "status": "Status", "elemental": "Elemental", "economy": "Economy",
    "roguelite": "Fate & Ascension (losowanie)", "attribute_synergy": "Attribute Synergy",
    "experimental": "Experimental",
    "evolution": "Ewolucje (Mythic, 0.5–0.7)", "cursed": "Karty przeklęte (0.5)",
    "reflex": "Odruchy walki (0.6–0.9)",
    "command": "Command — sojusznicy (0.9)",
    "signature": "Karty bossów — Signature (2.0)",
}

INTRO = """# Biblioteka mocy, ścieżki, synergie i Attribute Evolution (2.0)

Plik generowany przez `tools/doc_powers.py` z `config/powers.json`, `config/procs.json`
i `config/synergies.json`. Liczby poniżej są dokładnie tymi, które czyta DLL.

Razem {total} kart. Opisy kart w grze są po angielsku i generuje je `tools/card_text.py`
z prawdziwych liczb (jedna krótka linia na poziom). W 2.0 każda karta należy do jednej z
dziewięciu **ścieżek** (Paths); każdy poziom karty to 1 punkt ścieżki, a 5, 10 i 15 punktów
odblokowuje premię (lista niżej). Wielcy bossowie mają własne **karty Signature**: pierwsze
pokonanie bossa zawsze ją oferuje, a potem może trafić się w każdej nagrodzie.

## Ścieżki (2.0)

| Ścieżka | 5 punktów | 10 punktów | 15 punktów |
|---|---|---|---|
| Ferocity (ciężkie ciosy) | Bonebreaker: riposta/cios w plecy leczy 6% HP w 3 s | Cataclysm: złamanie postawy = meteoryt | Titan's Wrath: +25% obrażeń 8 s po złamaniu postawy |
| Celerity (szybkość) | Momentum: +10% obrażeń przy serii 4 trafień | Blur: ataki 10% szybsze 4 s po zabójstwie | Tempest: seria 8 trafień = piorun |
| Malice (statusy) | Lingering Curse: +10% obrażeń 8 s po wybuchu statusu | Plague Bearer: status przechodzi na wrogów w 5 m | Death's Herald: ciosy budują Death Blight |
| Resonance (czary kart) | Echo: +1 pocisk | Chorus: +1 pocisk | Arcane Storm: pełnia co 10 s w walce |
| Fortitude (obrona) | Iron Will: -15% obrażeń poniżej 30% HP | Undying: przeżycie śmiertelnego ciosu co 4 min | Juggernaut: 2 s po trafieniu +40% postury, -10% obrażeń |
| Evasion (uniki) | Quickened Breath: stamina po perfekcyjnym uniku | Counterstrike: +35% obrażeń 3 s po perfekcyjnym uniku | Phantom: widmowa strzała po perfekcyjnym uniku |
| Thirst (leczenie) | Feast: zabójstwo leczy 3% HP | Crimson Tide: zabójstwo daje 5 FP, czary kart leczą | Sanguine Lord: lifesteal ×2 |
| Fortune (runy, szczęście) | Ember Sense: więcej elit, +1 Ember | Golden Destiny: +1 karta w nagrodzie | Midas: +10% run, pominięcie płaci podwójnie |
| Command (sojusznicy) | Rally: sojusznicy leczą 0.5% HP/s | Phalanx: sojusznicy -25% obrażeń, +30% postury | Warlord: z sojusznikiem +15% obrażeń, -10% otrzymywanych |

## Jak działają karty-czary (Arcane Arsenal i proci)

Każdy proc klonuje łańcuch pocisków gracza z gry (np. Glintstone Pebble, Lightning Spear,
Bloodflame Talons) do własnych wierszy Bullet z własnymi wierszami AtkParam o stałych
obrażeniach. Przed każdym wystrzałem DLL wpisuje obrażenia:

`moc karty × (90 + 4.2 × poziom postaci) × (0.75 + atrybuty/80) × (1 + 0.15 × cykl NG+)`

Atrybuty to ważona suma podanych przy procu statystyk (np. Intelligence dla kamieni blasku).
Wynik przechodzi przez normalną obronę i odporności przeciwnika, a premie SpEffect gracza
(np. obrażenia magiczne ×) też działają. Automatyczne czary strzelają tylko w walce, w
przeciwników, z którymi walczysz (lub nieprzyjaznych w promieniu 10 m) — nie zaczynają walk
same. Obrażenia procy nie wyzwalają innych procy „po trafieniu" (ochrona przed pętlami),
a wszystkie procy mają czasy odnowienia i wspólny limit 12 pocisków na sekundę.

## Atrybuty gry — Attribute Overhaul (2.1)

Poziomy w miejscach łaski działają jak w grze podstawowej, a każdy atrybut ma dodatkowo
własną rolę. Krzywa `punkty / (punkty + kolano)` rośnie z każdym punktem (60 punktów daje
około połowy maksimum). W nawiasie maksimum.

| Atrybut | Rola | Działanie |
|---|---|---|
| Vigor | przetrwanie | odporność na truciznę/zgniliznę (+14%) i krwawienie (+10%); leczenie poniżej 60% HP po 8 s bez obrażeń (0.24%/s, w walce z bossem 35%); poniżej 25% HP mniej obrażeń (-14%); lifesteal (3% zadanych obrażeń) |
| Mind | skupienie i duchy | koszt FP czarów i umiejętności (-12%); regeneracja FP (0.4%/s); przywołane duchy i sojusznicy +obrażenia i HP (+40%) |
| Endurance | wytrzymałość | regeneracja staminy (+15), koszt staminy (-8%), postura (+25%), stamina przy bloku (-20%) |
| Strength | ciężkie ciosy | obrażenia postury (+18%), obrażenia staminy wroga (+12%), ataki ładowane i z wyskoku (+30%) |
| Dexterity | szybkość i precyzja | szybkość ataków (+15%), flakony i przedmioty (+18%), riposta i cios w plecy (+30%), 1.5 s po uniku +10% obrażeń, upadki (-25%) |
| Intelligence | wyczucie | dłuższa nietykalność uniku (do 0.10 s), czary kart odnawiają się szybciej (-35% czasu) |
| Faith | ochrona | otrzymywane obrażenia fizyczne (-5.5%) i żywiołowe/święte (-8%), flakony (+10%), od 30 Faith boska interwencja |
| Arcane | los i krew | runy (+40%), odkrywanie (+35), buildup statusów z kart (+50%, do +12 płasko), statusy ranią mocniej (+40%), rzadsze karty |

Panel **ATTRIBUTE BONUSES** pojawia się obok strony Level Up w miejscu łaski (tylko do
czytania), a kolekcja (F7) → *You* pokazuje te same wartości. Dawne statystyki Ascension
z 0.8 zostały usunięte w 2.0; ich efekty żyją teraz w atrybutach, które do nich pasują.

## Fate (0.5): ewolucje, klątwy, blokada, pominięcie, rangi

- **Ewolucje:** gdy obie karty bazowe mają poziom V, odblokowuje się karta Mythic z nową
  mechaniką. Pojawia się w najbliższej ofercie (także po rerollu) aż do jej wybrania; kolejne
  poziomy losują się normalnie, z większą wagą. Postęp: kolekcja (F7) → *Combos*.
- **Karty przeklęte:** duża moc ze stałą wadą (czerwona ramka, napis CURSED, opis klątwy).
  Wada nie rośnie z poziomem, rośnie tylko moc. Wyłączane w `[fate] curses = false`.
- **Blokada (2 Embers):** zablokowana karta zostaje przy rerollach i wraca w następnej
  ofercie, nawet jeśli teraz wybierzesz inną.
- **Pominięcie:** zamiast karty bierzesz runy (część kosztu awansu poziomu: Minor 0.5,
  Greater 0.8, Major 1.2, Remembrance 2.0; +25% za każdy cykl NG+) i 1 Ember.
- **Nagrody rangi:** co 10 poziomów mocy +1 darmowy reroll na ofertę (do +5); ranga Elden Lord
  (56 poziomów) daje 4 karty po bossach Remembrance, God of the Ring (91) także po Major.
- **Afiksy bossów w NG+:** każdy boss cyklu NG+ ma afiks (od NG+3 dwa): Relentless (postura),
  Ironclad (fizyczne), Spellward (żywioły), Frenzied (+12% obrażeń), Regenerating (0.25% HP/s).
  Kolejność jest stała dla postaci; kolejne cykle nie powtarzają pierwszego afiksu.

## Odruchy walki (0.6)

Skaner świata (10 razy na sekundę) rozpoznaje nowe zdarzenia walki. Karty kategorii
*Odruchy walki* reagują na nie czarami (procy) albo krótkimi premiami:

- **Perfekcyjny unik:** unik, w czasie którego (w nietykalności przewrotu lub w pierwszych
  0.45 s) wróg w zasięgu ciosu i zwrócony do ciebie był w aktywnej części animacji ataku,
  albo atak dosięgnął cię w nietykalności, a unik skończył się bez utraty HP.
- **Blok:** cios przyjęty na gardę (zmiana ostatnio otrzymanego ataku lub spadek staminy
  przy atakującym wrogu) bez dużej utraty HP. **Kontra po gardzie:** twoje trafienie do 1.5 s
  po bloku.
- **Krytyk:** riposta, cios w plecy i inne krytyczne chwyty (stan rzutu gracza).
- **Złamanie postawy:** postawa zaangażowanego wroga pęka (moduł super armor).
- **Statusy na wrogu:** krwotok, odmrożenie, sen, szaleństwo, trucizna/zgnilizna, Death Blight
  wykryte w module odporności wroga (pełny pasek lub nagły reset).

Pierwsze wykrycia każdego rodzaju i podsumowanie co minutę trafiają do `logs/runtime.jsonl`.
**Dexterity** (i karta Swift Draught) przyspiesza animacje picia z flakonów i użycia
przedmiotów (do +18% z atrybutu; razem z kartami najwyżej +50%).

**Fate 0.6:** Thread of Fate (blokada za 1 Ember, na V darmowa), Fate's Ledger (więcej run
i Embers za pominięcie), ewolucje z trzech kart (wszystkie trzy na V), synergie dla kart
przeklętych, afiks bossów **Adaptive** (odporność ×0.75 na główny typ obrażeń twojej broni).

## Statusy z kart (2.0)

- **Wszystkie naraz:** każdy status z posiadanych kart jest na broni jednocześnie. Najpierw
  trafia do wolnych slotów pasywnych broni (widoczny w menu broni), reszta przez niewidoczny
  nośnik jak smar — działa tak samo.
- **Moc:** buildup karty × (1 + 0.5 × krzywa Arcane) × (1 + 0.006 × poziom postaci, maks. ×2.2)
  × (1 + 0.15 × NG+), plus płaska premia Arcane. Odporni wrogowie zostają odporni.
- Ścieżka Malice: Lingering Curse (5), Plague Bearer (10), Death's Herald (15).

## Parowanie i cios w plecy (0.7)

- **Parowanie:** atak wroga przerwany przed 80% animacji (z co najmniej 0.3 s do końca)
  tuż po jego oknie parowania, wróg do 4.5 m przed tobą bez utraty HP, ty bez uniku i bez
  dużej utraty HP. Riposta (krytyk na wrogu zwróconym do ciebie) bez złamanej postawy
  w ostatnich 6 s też liczy się jako parowanie (raz).
- **Cios w plecy:** krytyk na wrogu odwróconym od ciebie.
- Każda karta ma ilustrację (`assets/cards`), a kolekcja (F7) zakładkę **You** z sumą
  premii z kart, synergii i atrybutów.

## Ochrona przed pechem

Po czterech kolejnych ofertach bez karty Epic lub rzadszej, karty Epic+ ważą ×1.5, potem ×2,
×2.5, do ×3, aż taka karta się pojawi. Licznik jest zapisany w dzienniku postaci.

"""


BOSSES = {}


def main():
    powers = [p for p in json.loads((PROJECT / "config/powers.json").read_text())["powers"] if not p.get("hidden")]
    procs = {p["id"]: p for p in json.loads((PROJECT / "config/procs.json").read_text())["procs"]}
    synergies = json.loads((PROJECT / "config/synergies.json").read_text())["synergies"]
    names = {p["id"]: p["name"] for p in powers}
    global BOSSES
    BOSSES = {b["id"]: b["name"] for b in json.loads((PROJECT / "config/bosses.json").read_text())}
    new = sum(1 for p in powers if p["effect_row_base"] >= 95000464)
    out = [INTRO.format(new=new, total=len(powers))]
    out.append("## Synergie\n\nSynergia działa automatycznie, gdy posiadasz wszystkie jej karty.\n")
    for s in synergies:
        need = [f"{names[i]} {ROMAN[l - 1]}" for i, l in s.get("all", [])]
        if "any_of" in s:
            need.append(f"dowolne {s['any_of']['count']} z: " + ", ".join(names[i] for i in s["any_of"]["ids"]))
        need += [f"{a.capitalize()} {v}+" for a, v in s.get("attributes", {}).items()]
        bonus = [f"`{m['field']}` {m['value']}" for m in s.get("modifiers", []) if m["field"] != "motion_interval"]
        bonus += [f"{procs[k]['name']} ×{v}" for k, v in s.get("proc_power", {}).items()]
        out.append(f"### {s['name']} ({s['rarity']})\n\n{s['description']}\n\n- Wymaga: {' + '.join(need)}\n- Premia: {', '.join(bonus)}\n")
    by_cat = {}
    for p in powers:
        by_cat.setdefault(p["category"], []).append(p)
    out.append("## Karty\n")
    for cat in sorted(by_cat, key=lambda c: list(CATEGORY).index(c) if c in CATEGORY else 99):
        out.append(f"## {CATEGORY.get(cat, cat)}\n")
        for p in by_cat[cat]:
            rarities = " → ".join(f"{r.capitalize()} {ROMAN[i]}" for i, r in enumerate(p["level_rarities"]))
            path = f" · ścieżka {p['family'].capitalize()}" if p.get("family") else ""
            out.append(f"### {p['name']}\n\n`{p['id']}` · {rarities}{path}\n")
            if p.get("signature"):
                out.append("Karta Signature bossa: " + ", ".join(BOSSES.get(b, b) for b in p["signature"]) + "\n")
            if p.get("requires"):
                out.append("Ewolucja z: " + " + ".join(f"{names[i]} {ROMAN[l - 1]}" for i, l in p["requires"]) + "\n")
            if p["levels"][0].get("item_speed") is not None:
                out.append("Szybkość użycia przedmiotów: " + ", ".join(f"+{l['item_speed'] * 100:.0f}%" for l in p["levels"]) + "\n")
            if p.get("curse"):
                out.append(f"**Klątwa:** {p['curse']}\n")
            proc = p["levels"][0].get("proc")
            if proc:
                d = procs[proc["proc"]]
                scaling = ", ".join(f"{k} {v:g}" for k, v in d["scaling"].items())
                element = ", ".join(f"{k} {v:g}" for k, v in d["element"].items())
                out.append(f"Czar: {d['name']} · obrażenia: {element} · skalowanie: {scaling}\n")
            for i, level in enumerate(p["levels"]):
                out.append(f"- **{ROMAN[i]}:** {level['description']}")
            out.append("")
    (PROJECT / "docs/POWERS.md").write_text("\n".join(out) + "\n")
    print(f"docs/POWERS.md: {len(powers)} powers, {len(synergies)} synergies")


if __name__ == "__main__":
    main()
