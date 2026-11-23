//! 2.0 Paths. Every card belongs to one of nine Paths. A Path's points are the
//! levels of your cards in it (a card at III gives 3). At 5, 10 and 15 points
//! the Path unlocks a bonus. Nothing to buy, no menu: the cards you choose
//! decide. Most bonuses are hidden powers in config/powers.json ("perk_*");
//! the rest are computed here.
//!
//! Also here: the vanilla attributes' overhaul (attributes.rs) and the
//! world's runes, merged into one `Boons` value for the effect engine.
//! The 0.8 stat system (points bought with runes, Bonds, Oaths) is gone.
use crate::powers::{CombatEvent, CombatSnapshot, EffectField, InterventionSpec, Modifier, PowerLibrary, aggregate_modifiers};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Path {
    Ferocity,
    Celerity,
    Malice,
    Resonance,
    Fortitude,
    Evasion,
    Thirst,
    Fortune,
    Command,
}
pub const PATHS: [Path; 9] = [
    Path::Ferocity,
    Path::Celerity,
    Path::Malice,
    Path::Resonance,
    Path::Fortitude,
    Path::Evasion,
    Path::Thirst,
    Path::Fortune,
    Path::Command,
];
/// Points that unlock a Path's three bonuses.
pub const TIERS: [u8; 3] = [5, 10, 15];
/// 3.0: a Path's two Keystone cards ("keystone" category) can be offered
/// once it has this many points.
pub const KEYSTONE_GATE: u8 = 6;
/// Points of every Path, in `PATHS` order (capped at 99).
pub type PathPoints = [u8; 9];

/// One bonus of a Path: (points, hidden power id or "", name, what it does).
pub struct Bonus {
    pub points: u8,
    pub power: &'static str,
    pub name: &'static str,
    pub text: &'static str,
}

impl Path {
    pub fn index(self) -> usize {
        PATHS.iter().position(|s| *s == self).unwrap_or(0)
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::Ferocity => "ferocity",
            Self::Celerity => "celerity",
            Self::Malice => "malice",
            Self::Resonance => "resonance",
            Self::Fortitude => "fortitude",
            Self::Evasion => "evasion",
            Self::Thirst => "thirst",
            Self::Fortune => "fortune",
            Self::Command => "command",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        PATHS.iter().copied().find(|s| s.id() == id)
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Ferocity => "Ferocity",
            Self::Celerity => "Celerity",
            Self::Malice => "Malice",
            Self::Resonance => "Resonance",
            Self::Fortitude => "Fortitude",
            Self::Evasion => "Evasion",
            Self::Thirst => "Thirst",
            Self::Fortune => "Fortune",
            Self::Command => "Command",
        }
    }
    /// What the Path is about, in two or three words.
    pub fn theme(self) -> &'static str {
        match self {
            Self::Ferocity => "heavy blows",
            Self::Celerity => "speed",
            Self::Malice => "status effects",
            Self::Resonance => "card spells",
            Self::Fortitude => "defense",
            Self::Evasion => "dodging",
            Self::Thirst => "healing",
            Self::Fortune => "runes and luck",
            Self::Command => "allies",
        }
    }
    /// Path colour (RGBA) for the interface.
    pub fn color(self) -> [f32; 4] {
        match self {
            Self::Ferocity => [0.90, 0.42, 0.28, 1.0],
            Self::Celerity => [0.98, 0.84, 0.38, 1.0],
            Self::Malice => [0.62, 0.86, 0.36, 1.0],
            Self::Resonance => [0.48, 0.70, 1.00, 1.0],
            Self::Fortitude => [0.78, 0.74, 0.66, 1.0],
            Self::Evasion => [0.52, 0.90, 0.88, 1.0],
            Self::Thirst => [0.92, 0.30, 0.38, 1.0],
            Self::Fortune => [0.96, 0.70, 0.22, 1.0],
            Self::Command => [0.74, 0.58, 0.96, 1.0],
        }
    }
    pub fn bonuses(self) -> [Bonus; 3] {
        let b = |points, power, name, text| Bonus { points, power, name, text };
        let [one, two, three] = TIERS;
        match self {
            Self::Ferocity => [
                b(one, "perk_bonebreaker", "Bonebreaker", "Ripostes and backstabs heal 6% HP over 3 s."),
                b(two, "perk_cataclysm", "Cataclysm", "Breaking a stance calls down a meteorite."),
                b(three, "perk_titans_wrath", "Titan's Wrath", "+25% damage for 8 s after you break a stance."),
            ],
            Self::Celerity => [
                b(one, "perk_momentum", "Momentum", "+10% damage while you keep a 4-hit streak."),
                b(two, "", "Blur", "Attacks 10% faster for 4 s after a kill."),
                b(three, "perk_tempest", "Tempest", "Every 8-hit streak calls lightning on your target."),
            ],
            Self::Malice => [
                b(one, "", "Lingering Curse", "When a status pops, the enemy takes +10% damage for 8 s."),
                b(two, "", "Plague Bearer", "A status that pops spreads to enemies within 5 m."),
                b(three, "", "Death's Herald", "Your hits also build up Death Blight."),
            ],
            Self::Resonance => [
                b(one, "", "Echo", "Card spells fire 1 extra projectile."),
                b(two, "", "Chorus", "Card spells fire 1 more extra projectile."),
                b(three, "perk_arcane_storm", "Arcane Storm", "In combat, a Full Moon falls on your foe every 10 s."),
            ],
            Self::Fortitude => [
                b(one, "perk_iron_will", "Iron Will", "-15% damage taken below 30% HP."),
                b(two, "", "Undying", "Survive one deadly blow every 4 minutes."),
                b(three, "perk_juggernaut", "Juggernaut", "For 2 s after each hit: +40% poise, -10% damage taken."),
            ],
            Self::Evasion => [
                b(one, "perk_second_wind", "Quickened Breath", "A perfect dodge refills stamina fast for 2 s."),
                b(two, "perk_counterstrike", "Counterstrike", "+35% damage for 3 s after a perfect dodge."),
                b(three, "perk_phantom", "Phantom", "A perfect dodge fires a phantom arrow at the attacker."),
            ],
            Self::Thirst => [
                b(one, "perk_feast", "Feast", "Kills heal 3% HP."),
                b(two, "perk_crimson_tide", "Crimson Tide", "Kills restore 5 FP; card spells heal you too."),
                b(three, "", "Sanguine Lord", "All lifesteal is doubled."),
            ],
            Self::Fortune => [
                b(one, "", "Ember Sense", "More Ascended Elites, +1 Ember each."),
                b(two, "", "Golden Destiny", "Every boss reward shows 1 more card."),
                b(three, "perk_midas", "Midas", "+10% runes; skipping a reward pays double."),
            ],
            Self::Command => [
                b(one, "", "Rally", "Allies heal 0.5% HP per second."),
                b(two, "", "Phalanx", "Allies take 25% less damage, +30% poise."),
                b(three, "perk_warlord", "Warlord", "With an ally near: +15% damage, -10% damage taken."),
            ],
        }
    }
}

/// Points per Path: the levels of every owned card in it (hidden powers and
/// unknown ids do not count).
pub fn points(levels: &BTreeMap<String, u8>, library: &PowerLibrary) -> PathPoints {
    let mut out = [0u8; 9];
    for (id, level) in levels {
        if let Some(power) = library.get(id).filter(|p| !p.hidden) {
            if let Some(path) = power.family {
                let slot = &mut out[path.index()];
                *slot = slot.saturating_add((*level).min(5)).min(99);
            }
        }
    }
    out
}
/// Bonuses unlocked by `points` (0..=3).
pub fn tier(points: u8) -> usize {
    TIERS.iter().filter(|t| points >= **t).count()
}
/// The Tide: every `step` card levels raise it by one, up to X.
pub fn tide(card_levels: u32, step: u32) -> u8 {
    (card_levels / step.max(1)).min(10) as u8
}
/// Hidden powers the Paths switch on (level 1).
pub fn hidden_powers(points: &PathPoints) -> Vec<&'static str> {
    let mut out = Vec::new();
    for path in PATHS {
        for bonus in path.bonuses() {
            if points[path.index()] >= bonus.points && !bonus.power.is_empty() {
                out.push(bonus.power);
            }
        }
    }
    out
}

/// The world's settings ([world]).
#[derive(Debug, Clone, Copy)]
pub struct World {
    pub rune_multiplier: f32,
    pub tide_runes: f32,
}

/// Everything the Paths, the vanilla attributes and the world do this tick.
#[derive(Debug, Clone, Default)]
pub struct Boons {
    /// To the player's own Ascension rows.
    pub modifiers: Vec<Modifier>,
    pub roll_extension_seconds: f32,
    pub item_speed: f32,
    pub attack_speed: f32,
    /// Card status buildup multiplier (Arcane) and a flat bonus (Arcane).
    pub status_potency: f32,
    pub status_flat: i32,
    pub death_blight: f32,
    pub lingering_curse: bool,
    pub plague_bearer: bool,
    /// Card spells: extra projectiles (Resonance path) and how much faster
    /// they recharge (Intelligence).
    pub spell_extra: u8,
    pub spell_cooldown: f32,
    /// Lifesteal from Vigor, its multiplier (Thirst 15 doubles all lifesteal)
    /// and card spells healing too (Thirst 10).
    pub lifesteal: f32,
    pub lifesteal_mult: f32,
    pub lifesteal_spells: bool,
    /// Card draw luck (Arcane).
    pub luck: f64,
    pub intervention: Option<InterventionSpec>,
    pub elite_chance: f64,
    pub elite_embers: u32,
    pub extra_card: bool,
    pub skip_double: bool,
    /// Charged / jumping attacks (Strength) and ripostes / backstabs
    /// (Dexterity), as fractions.
    pub charge_damage: f32,
    pub jump_damage: f32,
    pub critical_damage: f32,
    /// Status damage on enemies you fight (Arcane), fraction.
    pub status_damage: f32,
    /// Allies: damage and HP (Mind), regeneration (% max HP per second),
    /// damage taken reduction and a poise multiplier (Command path).
    pub ally_damage: f32,
    pub ally_hp: f32,
    pub ally_regen: f32,
    pub ally_defense: f32,
    pub ally_poise: f32,
}

fn recent(snapshot: &CombatSnapshot, event: CombatEvent, seconds: f32) -> bool {
    snapshot
        .events
        .get(&event)
        .is_some_and(|e| e.age_seconds.is_finite() && e.age_seconds <= seconds)
}

/// This tick's boons. `strength` is [balance] attribute_strength.
pub fn evaluate(snapshot: &CombatSnapshot, world: World, strength: f32) -> Boons {
    let points = snapshot.paths;
    let at = |path: Path| points[path.index()];
    let attributes = crate::attributes::evaluate(snapshot.attributes, snapshot, strength);
    let mut out = Boons {
        status_potency: attributes.status_potency.max(1.0),
        status_flat: attributes.status_flat_bonus,
        roll_extension_seconds: attributes.roll_extension_seconds,
        item_speed: attributes.item_speed,
        attack_speed: attributes.attack_speed,
        luck: attributes.luck,
        intervention: attributes.intervention,
        lifesteal: attributes.lifesteal,
        lifesteal_mult: 1.0,
        spell_cooldown: attributes.spell_cooldown,
        charge_damage: attributes.charge_damage,
        jump_damage: attributes.jump_damage,
        critical_damage: attributes.critical_damage,
        status_damage: attributes.status_damage,
        ally_damage: attributes.ally_damage,
        ally_hp: attributes.ally_hp,
        ally_poise: 1.0,
        ..Default::default()
    };
    let in_game = snapshot.in_game && snapshot.hp_ratio.is_finite() && snapshot.hp_ratio > 0.0;
    let mut modifiers: Vec<Modifier> = attributes.modifiers.clone();
    // Celerity 10, Blur.
    if at(Path::Celerity) >= TIERS[1] && recent(snapshot, CombatEvent::Kill, 4.0) {
        out.attack_speed += 0.10;
    }
    // Malice.
    out.lingering_curse = at(Path::Malice) >= TIERS[0];
    out.plague_bearer = at(Path::Malice) >= TIERS[1];
    if at(Path::Malice) >= TIERS[2] {
        out.death_blight = 40.0;
    }
    // Resonance: Echo and Chorus.
    out.spell_extra = u8::from(at(Path::Resonance) >= TIERS[0]) + u8::from(at(Path::Resonance) >= TIERS[1]);
    // Fortitude 10, Undying (combines with Faith's intervention: best of each).
    if at(Path::Fortitude) >= TIERS[1] {
        let undying = InterventionSpec {
            cooldown: 240.0,
            heal_share: 0.3,
            ward_seconds: 2.5,
            ward_cut: 0.4,
        };
        out.intervention = Some(match out.intervention {
            Some(faith) => InterventionSpec::combine(faith, undying),
            None => undying,
        });
    }
    // Thirst.
    out.lifesteal_spells = at(Path::Thirst) >= TIERS[1];
    if at(Path::Thirst) >= TIERS[2] {
        out.lifesteal_mult = 2.0;
    }
    // Fortune.
    if at(Path::Fortune) >= TIERS[0] {
        out.elite_chance = 0.3;
        out.elite_embers = 1;
    }
    out.extra_card = at(Path::Fortune) >= TIERS[1];
    out.skip_double = at(Path::Fortune) >= TIERS[2];
    // Command.
    if at(Path::Command) >= TIERS[0] {
        out.ally_regen = 0.5;
    }
    if at(Path::Command) >= TIERS[1] {
        out.ally_defense = 0.25;
        out.ally_poise = 1.3;
    }
    // The world pays more, and more with every Tide.
    let runes = world.rune_multiplier.max(0.1) * (1.0 + world.tide_runes * f32::from(snapshot.tide.min(10)));
    if (runes - 1.0).abs() > 1e-4 {
        modifiers.push(Modifier { field: EffectField::SoulRate, value: runes });
    }
    if !in_game {
        return Boons {
            status_potency: out.status_potency,
            status_flat: out.status_flat,
            luck: out.luck,
            lifesteal_mult: 1.0,
            ally_poise: 1.0,
            ..Default::default()
        };
    }
    out.modifiers = aggregate_modifiers(modifiers.iter());
    out
}
