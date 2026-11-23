ELDEN RING: ASCENSION  -  version {VERSION}
Roguelite cards for the Lands Between  -  by Lord Bink
==========================================================================

Beat a boss for the first time and pick 1 of 3 cards. Every card is a
permanent power for that character: more damage, spells that fire on a
perfect dodge or a critical, statuses, allies, runes, survival. Pick the same
card again and it grows a level, up to V. Build your character around the
cards the bosses give you.

WHAT IS IN IT
-------------
- 261 cards from Common to Mythic, plus the ultra-rare Prismatic cards above
  Mythic, each with 5 levels and its own artwork.
- 9 Paths (Ferocity, Celerity, Malice, Resonance, Fortitude, Evasion,
  Thirst, Fortune, Command). Every card level is a Path point; 5, 10 and 15
  points unlock a Path bonus.
- 31 Signature cards: the great bosses (base game and Shadow of the
  Erdtree) always offer their own card, with their own spell.
- 18 Keystones, legendary cards that change how you play.
- 10 Prismatic cards: ultra rare (about two per playthrough), every colour
  at once, and a very loud MAX WIN when one turns over.
- Every attribute gets a second role on top of the normal game: Vigor
  survival, Mind FP and spirits, Endurance stamina and poise, Strength heavy
  and charged attacks, Dexterity attack speed and ripostes, Intelligence
  dodges and card spells, Faith protection and flasks, Arcane runes, luck and
  statuses. The panel next to Level Up shows exactly what each point adds.
- Embers: reroll an offer, lock a card for the next offer, banish a card
  forever, or take runes instead.
- A harder world: stronger enemies and bosses, more runes, and the Tide
  that rises with your cards. From Tide I, bosses can return as Ascended
  bosses with affixes and a better reward.
- Combat moments: STANCE BROKEN, RIPOSTE, BACKSTAB, PERFECT DODGE and more,
  with their own sounds, and the names of your cards that reacted.
- All in-game text is in English with short, simple numbers.

REQUIREMENTS
------------
- ELDEN RING on Steam, game version {APP_VERSION} exactly, on Windows or on
  Linux / Steam Deck with Proton. The title screen shows "App Ver. {APP_VERSION}".
  After a game update, wait for a new version of the mod: on any other game
  version its code switches itself off.
- Mod Engine 3 (me3), made with me3 {ME3_VERSION}:
  https://github.com/garyttierney/me3/releases
- The game must have been started at least once (so its save folder exists).
- Shadow of the Erdtree is not required, but the mod fully covers it: all
  42 DLC bosses give cards, and 11 of the 31 Signature cards come from DLC
  bosses, including Messmer, Rellana, Bayle and Promised Consort Radahn.
  Without the DLC you get everything from the base game.

INSTALL ON WINDOWS
------------------
1. Install Mod Engine 3 (the me3 installer from the link above).
2. Extract this zip to a normal folder you can write to, for example
   Documents\Ascension. Not Program Files and not the game folder.
3. Start the game with ONE of these:
   - double-click "Launch Ascension.bat", or
   - double-click ascension.me3 (the me3 installer links .me3 files), or
   - in a terminal:  me3 launch --game eldenring --profile ascension.me3
4. On the title screen you see "ASCENSION" and the version in the
   bottom-left corner. That means the mod is running.

INSTALL ON LINUX / STEAM DECK
-----------------------------
1. Install Mod Engine 3 for Linux (same link). Steam must be running and
   ELDEN RING must use Proton.
2. Extract this zip to a folder in your home directory.
3. Run ./launch-ascension-linux.sh
   (or: me3 launch --game eldenring --profile /path/to/ascension.me3)

FIRST STEPS
-----------
- The mod uses its OWN save file (ER0000.ascension.sl2, next to your normal
  save). Your normal save is never touched. Start a new character.
- Play offline. me3 starts the game without Easy Anti-Cheat; never take a
  modded game online.
- F7 opens the collection: Cards, Paths, Codex, Combos, You, Guide and
  Settings. The Guide tab explains everything in short lines.

CONTROLS
--------
                         Keyboard / mouse        Controller
  Choose a card          1/2/3, left/right       D-pad left/right
  Confirm                Enter                   A / bottom button
  Reroll (if free/Embers) R                      Y / top button
  Banish a card          X (press twice)         X / left button
  Lock a card            L                       LB
  Take runes instead     S (press twice)         RB
  Put off / close        Esc                     B / right button
  Collection / resume    F7                      Back / View
  Combat HUD             F6                      -
  Reveal all cards       any key or click        A
  Diagnostics panel      F8                      -

UPDATING
--------
Replace the files with the new version but KEEP these:
- the state folder (your cards and progress for every character),
- the local folder,
- your save ER0000.ascension.sl2 (it stays in the game's save folder anyway).
Your card progress lives in "state", next to the mod. Copying only the
.sl2 save does not carry the cards. Never delete "state" to "fix" a problem.

UNINSTALL
---------
Delete the mod folder. Your normal save was never changed. The Ascension
save stays in the game's save folder (on Windows
%APPDATA%\EldenRing\<your Steam ID>\ER0000.ascension.sl2) until you
delete it yourself.

COMPATIBILITY
-------------
- Not compatible with other mods that replace regulation.bin (Convergence,
  Reforged, randomizers and other overhaul mods) or with Seamless Co-op.
- If an old dinput8.dll or winhttp.dll mod loader sits in the game folder
  (Mod Engine 2, Elden Mod Loader), the mod switches itself off. Move that
  file out of the game folder.
- Offline only.

SETTINGS
--------
config\ascension.toml holds the balance: boss HP and damage, enemy
strength, rarity weights, rerolls and more. The Settings tab (F7) has the
common options. Changes in the file are read when the game starts.

TROUBLESHOOTING
---------------
- No "ASCENSION" on the title screen: the mod did not start. Open
  logs\runtime.jsonl in the mod folder; the last lines say why (wrong game
  version, a dinput8.dll in the game folder, a missing file).
- "launch context unavailable": start ELDEN RING once normally, quit, then
  start it with the mod again.
- Each character's progress is backed up on every load to state\backups.
- When you report a bug, please attach logs\runtime.jsonl.

This is an early public release, so please report anything odd. It has been
played only on Lord Bink's own Linux (Proton) setup. Setups without
Shadow of the Erdtree have not been tried.

CREDITS
-------
- Mod Engine 3 (me3) by garyttierney and contributors - loads the mod.
- fromsoftware-rs (Vincent Swarte and contributors) - Elden Ring structures
  for Rust. MIT / Apache 2.0 NON-AI licenses: this code may not be used to
  train AI models.
- hudhook and Dear ImGui (Omar Cornut) - the in-game overlay.
- pelite, and many other Rust libraries; see THIRD-PARTY-NOTICES.txt.
- EROverlay by Soar Qin (MIT) - boss names and flags.
- Fonts: Noto Serif Display (SIL Open Font License), DejaVu Sans.
- Card artwork: painted procedurally by the mod's own scripts.
  Sounds: synthesized by the mod's own scripts. No game assets reused.

AI DISCLOSURE: this mod was built with AI. The code, the card designs, the
card art scripts and the sound scripts were written by AI coding agents,
Claude (Anthropic, in Claude Code) and Codex (OpenAI), directed and played by
Lord Bink.

LICENSE
-------
The mod's own code and data: MIT (LICENSE.txt). Third-party licenses are in
the "licenses" folder. ELDEN RING belongs to FromSoftware / Bandai Namco;
this is a fan-made mod and is not affiliated with them.
