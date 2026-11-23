//! Attribute Overhaul (2.1). The base game's attributes, levelled at Sites of
//! Grace as always, each get one clear job on top of vanilla, so every level
//! up shapes the build:
//!
//! - Vigor, survival: status resistance, healing when safe, a last stand,
//!   and a little lifesteal.
//! - Mind, focus and spirits: cheaper FP, slow FP regeneration, stronger
//!   spirit summons and allies.
//! - Endurance, staying power: stamina, poise, cheaper blocking.
//! - Strength, heavy blows: stance damage, charged and jumping attacks.
//! - Dexterity, speed and precision: attack and item speed, ripostes and
//!   backstabs, a strike after a dodge, lighter falls.
//! - Intelligence, arcane timing: longer dodge invincibility, card spells
//!   recharge faster.
//! - Faith, protection: less damage taken, better flasks, divine intervention
//!   from 30.
//! - Arcane, fate and blood: runes, item discovery, card status buildup and
//!   status damage, rarer cards.
//!
//! (0.8's separate Ascension stats were removed in 2.0; their effects live
//! here now, on the attribute that fits them.) Every value follows
//! `curve(points, knee)`: each point helps, with diminishing returns and no
//! hard cap. Returned modifiers go to dedicated Ascension rows; the caller
//! owns row lifetime and the separate backends (roll invulnerability, attack
//! speed, talisman copies, allies, lifesteal, card spells, status marks).
use crate::powers::{
    AttributeValues, CombatSnapshot, EffectField, InterventionSpec, Modifier, aggregate_modifiers,
};

/// Knee of each attribute's curve (points where half the maximum is reached).
const KNEE: [f32; 8] = [45.0, 50.0, 45.0, 50.0, 55.0, 60.0, 60.0, 60.0];
// Maximums (at the far end of the curve; 60 points give about half to 57%).
const VIGOR_POISON_ROT: f32 = 0.14;
const VIGOR_BLEED: f32 = 0.10;
const VIGOR_REGEN: f32 = 0.24;
const VIGOR_LAST_STAND: f32 = 0.14;
const VIGOR_LIFESTEAL: f32 = 0.03;
const MIND_FP_COST: f32 = 0.12;
const MIND_FP_REGEN: f32 = 0.4;
const MIND_ALLIES: f32 = 0.40;
const END_REGEN: f32 = 15.0;
const END_STAMINA_COST: f32 = 0.08;
const END_POISE: f32 = 0.25;
const END_GUARD: f32 = 0.20;
const STR_STANCE: f32 = 0.18;
const STR_STAMINA_DAMAGE: f32 = 0.12;
const STR_CHARGED: f32 = 0.30;
const DEX_ATTACK_SPEED: f32 = 0.15;
const DEX_ITEM_SPEED: f32 = 0.18;
const DEX_CRITICAL: f32 = 0.30;
const DEX_AFTER_DODGE: f32 = 0.10;
const DEX_FALL: f32 = 0.25;
const INT_IFRAMES: f32 = 0.16;
const INT_SPELL_COOLDOWN: f32 = 0.35;
const FAITH_PHYSICAL: f32 = 0.055;
const FAITH_ELEMENTAL: f32 = 0.08;
const FAITH_FLASK: f32 = 0.10;
const ARC_RUNES: f32 = 0.40;
const ARC_DISCOVERY: f32 = 35.0;
const ARC_POTENCY: f32 = 0.5;
const ARC_FLAT: f32 = 12.0;
const ARC_STATUS_DAMAGE: f32 = 0.40;

#[derive(Debug, Clone, Default)]
pub struct AttributeEvolution {
    pub modifiers: Vec<Modifier>,
    /// A requested extension, not a direct write or a claim of active i-frames.
    /// Apply only through the dedicated roll_extension capability.
    pub roll_extension_seconds: f32,
    /// Card status potency from Arcane (buildup multiplier, 1.0..1.5).
    pub status_potency: f32,
    /// Capacity for the weapon-status backend; never applied as a fake SpEffect.
    pub status_channels: u8,
    pub status_flat_bonus: i32,
    pub casting_dex_bonus: i8,
    /// Faith 30+: divine intervention (survive one lethal blow per cooldown).
    pub intervention: Option<InterventionSpec>,
    /// Arcane: extra luck for Ascension card draws (0..0.6).
    pub luck: f64,
    /// Dexterity: flask/item use and attack animations this much faster.
    pub item_speed: f32,
    pub attack_speed: f32,
    /// Strength: charged and jumping attacks; Dexterity: ripostes and
    /// backstabs (fractions, through the talisman copies).
    pub charge_damage: f32,
    pub jump_damage: f32,
    pub critical_damage: f32,
    /// Vigor: share of the damage you deal that heals you.
    pub lifesteal: f32,
    /// Mind: allies' (spirit summons, Torrent, friendly NPCs) damage and HP.
    pub ally_damage: f32,
    pub ally_hp: f32,
    /// Intelligence: card spells recharge this much faster (fraction).
    pub spell_cooldown: f32,
    /// Arcane: status damage on enemies you fight (fraction, Malice mark).
    pub status_damage: f32,
}

fn scale_of(strength: f32) -> f32 {
    if strength.is_finite() { strength.clamp(0.0, 2.0) } else { 0.0 }
}
/// Dexterity's nimble hands: item use (flasks included) up to +18% faster.
pub fn dexterity_item_speed(dexterity: u32, strength: f32) -> f32 {
    (DEX_ITEM_SPEED * curve(dexterity, KNEE[4]) * scale_of(strength)).clamp(0.0, 0.36)
}

/// Arcane's luck for the card draw; also used by the storage worker, which has
/// only the latest snapshot's attributes.
pub fn arcane_luck(arcane: u32, strength: f32) -> f64 {
    f64::from(0.6 * curve(arcane, KNEE[7]) * scale_of(strength)).clamp(0.0, 0.6)
}

/// Faith's divine intervention: from 30 Faith, a lethal blow leaves 1 HP, then
/// part of max HP returns under a short ward. Cooldown 8 min at 30 Faith,
/// about 4.7 min at 99. Cards with the same mechanic combine (best of each).
pub fn faith_intervention(faith: u32, strength: f32) -> Option<InterventionSpec> {
    if faith < 30 || !strength.is_finite() || strength <= 0.0 {
        return None;
    }
    let f = curve(faith, KNEE[6]) * strength.clamp(0.0, 2.0);
    Some(InterventionSpec {
        cooldown: (720.0 - 700.0 * f).clamp(180.0, 720.0),
        heal_share: (0.15 + 0.35 * f).clamp(0.15, 0.6),
        ward_seconds: 2.0,
        ward_cut: (0.6 - 0.3 * f).clamp(0.2, 0.6),
    })
}

/// Every point, including points beyond soft caps, increases a bounded curve.
/// The denominator differs by mechanic. No value can diverge at high attributes.
pub fn curve(value: u32, knee: f32) -> f32 {
    let value = value.min(1_000_000) as f32;
    value / (value + knee.max(1.0))
}

/// The eight curves (VIG, MND, END, STR, DEX, INT, FTH, ARC) at `strength`.
fn curves(a: AttributeValues, strength: f32) -> [f32; 8] {
    let s = scale_of(strength);
    let v = [a.vigor, a.mind, a.endurance, a.strength, a.dexterity, a.intelligence, a.faith, a.arcane];
    std::array::from_fn(|i| curve(v[i], KNEE[i]) * s)
}

pub fn evaluate(attributes: AttributeValues, snapshot: &CombatSnapshot, strength: f32) -> AttributeEvolution {
    if !snapshot.in_game || snapshot.hp_ratio <= 0.0 || !snapshot.hp_ratio.is_finite() {
        return AttributeEvolution::default();
    }
    let [vigor, mind, endurance, power, dex, intelligence, faith, arcane] = curves(attributes, strength);
    let mut modifiers = Vec::with_capacity(32);
    let mut add = |field, value| modifiers.push(Modifier { field, value });
    use EffectField::*;

    // Vigor: status resilience, slow recovery after eight seconds without
    // HP loss (stops at 60% HP, cannot rescue lethal damage), a last stand.
    add(RegistPoizonChangeRate, 1.0 + VIGOR_POISON_ROT * vigor);
    add(RegistDiseaseChangeRate, 1.0 + VIGOR_POISON_ROT * vigor);
    add(RegistBloodChangeRate, 1.0 + VIGOR_BLEED * vigor);
    let regenerating = snapshot.hp_ratio < 0.60
        && snapshot.seconds_since_hp_loss.is_some_and(|s| s.is_finite() && s >= 8.0);
    if regenerating {
        let combat_factor = if snapshot.boss_fight_active { 0.35 } else { 1.0 };
        add(ChangeHpRate, -VIGOR_REGEN * vigor * combat_factor);
    }
    if snapshot.hp_ratio < 0.25 {
        for f in [
            SlashDamageCutRate,
            BlowDamageCutRate,
            ThrustDamageCutRate,
            NeutralDamageCutRate,
            MagicDamageCutRate,
            FireDamageCutRate,
            ThunderDamageCutRate,
            DarkDamageCutRate,
        ] {
            add(f, 1.0 - VIGOR_LAST_STAND * vigor);
        }
    }
    // Mind: every FP discipline is cheaper, FP trickles back.
    add(ArtsConsumptionRate, 1.0 - MIND_FP_COST * mind);
    add(MagicConsumptionRate, 1.0 - MIND_FP_COST * mind);
    add(MiracleConsumptionRate, 1.0 - MIND_FP_COST * mind);
    if mind > 0.0 {
        add(ChangeMpRate, -MIND_FP_REGEN * mind);
    }
    if regenerating || mind > 0.0 {
        add(MotionInterval, 1.0);
    }
    // Endurance: stamina, poise and blocking.
    add(StaminaRecoverChangeSpeed, (END_REGEN * endurance).round());
    add(ConsumeStaminaRate, 1.0 - END_STAMINA_COST * endurance);
    add(ToughnessDamageCutRate, 1.0 - END_POISE * endurance);
    add(GuardStaminaMult, 1.0 - END_GUARD * endurance);
    // Strength: posture and stamina damage (charged/jump attacks below).
    add(SaAttackPowerRate, 1.0 + STR_STANCE * power);
    add(StaminaAttackRate, 1.0 + STR_STAMINA_DAMAGE * power);
    // Dexterity: a strike right after a dodge, lighter falls (speed below).
    if snapshot
        .seconds_since_dodge_end
        .is_some_and(|s| s.is_finite() && (0.0..1.5).contains(&s))
    {
        add(PhysicsAttackRate, 1.0 + DEX_AFTER_DODGE * dex);
    }
    add(FallDamageRate, 1.0 - DEX_FALL * dex);
    // Faith: a modest ward and better flasks.
    for f in [SlashDamageCutRate, BlowDamageCutRate, ThrustDamageCutRate, NeutralDamageCutRate] {
        add(f, 1.0 - FAITH_PHYSICAL * faith);
    }
    for f in [MagicDamageCutRate, FireDamageCutRate, ThunderDamageCutRate, DarkDamageCutRate] {
        add(f, 1.0 - FAITH_ELEMENTAL * faith);
    }
    add(ChangeHpEstusFlaskCorrectRate, 1.0 + FAITH_FLASK * faith);
    // Arcane uses the engine's kill-rune field, preserving Gold Scarab and
    // consumable multiplication.
    add(SoulRate, 1.0 + ARC_RUNES * arcane);
    add(ItemDropRate, ARC_DISCOVERY * arcane);
    AttributeEvolution {
        modifiers: aggregate_modifiers(modifiers.iter()),
        // Up to 0.10 s more roll invulnerability, never unbounded.
        roll_extension_seconds: (INT_IFRAMES * intelligence).min(0.10),
        status_channels: 7,
        status_potency: 1.0 + ARC_POTENCY * arcane,
        status_flat_bonus: (ARC_FLAT * arcane).round().clamp(0.0, 24.0) as i32,
        casting_dex_bonus: 0,
        intervention: faith_intervention(attributes.faith, strength),
        luck: arcane_luck(attributes.arcane, strength),
        item_speed: dexterity_item_speed(attributes.dexterity, strength),
        attack_speed: DEX_ATTACK_SPEED * dex,
        charge_damage: STR_CHARGED * power,
        jump_damage: STR_CHARGED * power,
        critical_damage: DEX_CRITICAL * dex,
        lifesteal: VIGOR_LIFESTEAL * vigor,
        ally_damage: MIND_ALLIES * mind,
        ally_hp: MIND_ALLIES * mind,
        spell_cooldown: INT_SPELL_COOLDOWN * intelligence,
        status_damage: ARC_STATUS_DAMAGE * arcane,
    }
}

/// 2.2: how a number in an attribute line is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// A fraction shown as a percentage (0.084 -> "8.4%").
    Percent,
    /// Already in percent per second (0.06 -> "0.06%").
    PercentPerSecond,
    /// A whole number the game applies rounded (stamina regen).
    Whole,
    /// A plain number with one decimal (item discovery).
    Flat,
    Seconds,
    Minutes,
}

/// 2.2: a piece of an attribute line: text, or a number with its sign.
#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Text(&'static str),
    Num { value: f32, unit: Unit, sign: Option<char> },
}

/// 2.2: one attribute: name, job, points and its lines.
#[derive(Debug, Clone)]
pub struct AttributeRow {
    pub name: &'static str,
    pub job: &'static str,
    pub points: u32,
    pub lines: Vec<Vec<Part>>,
}

fn trim(text: String) -> String {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

/// A number as the panel shows it (without its sign).
pub fn format_value(value: f32, unit: Unit) -> String {
    let v = value.abs();
    match unit {
        Unit::Percent => {
            let p = v * 100.0;
            if p < 10.0 { trim(format!("{p:.1}")) + "%" } else { format!("{p:.0}%") }
        }
        Unit::PercentPerSecond => trim(format!("{v:.2}")) + "%",
        Unit::Whole => format!("{:.0}", v.round()),
        Unit::Flat => trim(format!("{v:.1}")),
        Unit::Seconds => trim(format!("{v:.2}")) + " s",
        Unit::Minutes => trim(format!("{v:.1}")) + " min",
    }
}

/// What going from `before` to `after` adds, e.g. "+0.6%"; None when it
/// would show as nothing. Smaller steps get more decimals, so a single point
/// always shows what it really adds.
pub fn format_gain(before: f32, after: f32, unit: Unit, sign: Option<char>) -> Option<String> {
    let (b, a) = match unit {
        Unit::Whole => (before.abs().round(), after.abs().round()),
        _ => (before.abs(), after.abs()),
    };
    let delta = a - b;
    let scale = match unit {
        Unit::Percent => 100.0,
        _ => 1.0,
    };
    let d = delta.abs() * scale;
    let text = match unit {
        Unit::Whole => format!("{d:.0}"),
        _ if d >= 9.95 => format!("{d:.0}"),
        _ if d >= 0.095 => trim(format!("{d:.1}")),
        _ if d >= 0.0095 => trim(format!("{d:.2}")),
        _ => trim(format!("{d:.3}")),
    };
    if text == "0" || d < 0.0005 {
        return None;
    }
    let grew = delta > 0.0;
    let symbol = match sign {
        Some('-') => if grew { '-' } else { '+' },
        Some(_) => if grew { '+' } else { '-' },
        // No sign shown: the plain direction of the number (minutes).
        None => if (after - before) >= 0.0 { '+' } else { '-' },
    };
    let suffix = match unit {
        Unit::Percent | Unit::PercentPerSecond => "%",
        Unit::Seconds => " s",
        Unit::Minutes => " min",
        _ => "",
    };
    Some(format!("{symbol}{text}{suffix}"))
}

impl Part {
    /// The part as text (numbers with their sign and unit).
    pub fn show(&self) -> String {
        match self {
            Part::Text(text) => (*text).to_owned(),
            Part::Num { value, unit, sign } => {
                let mut out = sign.map(String::from).unwrap_or_default();
                out.push_str(&format_value(*value, *unit));
                out
            }
        }
    }
}

/// Two lines with the same words and units (only the numbers may differ).
pub fn same_shape(a: &[Part], b: &[Part]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| match (x, y) {
            (Part::Text(p), Part::Text(q)) => p == q,
            (Part::Num { unit: u, sign: s, .. }, Part::Num { unit: v, sign: t, .. }) => u == v && s == t,
            _ => false,
        })
}

/// What each attribute does on top of the base game, line by line, from the
/// same constants as `evaluate`.
pub fn rows(a: AttributeValues, strength: f32) -> Vec<AttributeRow> {
    use Part::Text as T;
    let [vig, mnd, end, str_, dex, int, fai, arc] = curves(a, strength);
    let n = |value: f32, unit: Unit, sign: char| Part::Num { value, unit, sign: Some(sign) };
    let pct = |value: f32, sign: char| n(value, Unit::Percent, sign);
    let faith_last = match faith_intervention(a.faith, strength) {
        Some(spec) => vec![
            T("Survive a deadly blow every "),
            Part::Num { value: spec.cooldown / 60.0, unit: Unit::Minutes, sign: None },
        ],
        None => vec![T("At 30: survive a deadly blow now and then")],
    };
    let row = |name, job, points, lines| AttributeRow { name, job, points, lines };
    vec![
        row("Vigor", "survival", a.vigor, vec![
            vec![pct(VIGOR_POISON_ROT * vig, '+'), T(" poison and rot resistance")],
            vec![pct(VIGOR_BLEED * vig, '+'), T(" bleed resistance")],
            vec![T("Heal "), Part::Num { value: VIGOR_REGEN * vig, unit: Unit::PercentPerSecond, sign: None }, T(" HP/s when hurt and safe for 8 s")],
            vec![pct(VIGOR_LAST_STAND * vig, '-'), T(" damage taken below 25% HP")],
            vec![T("Heal "), Part::Num { value: VIGOR_LIFESTEAL * vig, unit: Unit::Percent, sign: None }, T(" of the damage you deal")],
        ]),
        row("Mind", "focus and spirits", a.mind, vec![
            vec![pct(MIND_FP_COST * mnd, '-'), T(" FP cost of spells and skills")],
            vec![T("Regain "), Part::Num { value: MIND_FP_REGEN * mnd, unit: Unit::PercentPerSecond, sign: None }, T(" FP per second")],
            vec![T("Spirit summons and allies "), pct(MIND_ALLIES * mnd, '+'), T(" damage and HP")],
        ]),
        row("Endurance", "staying power", a.endurance, vec![
            vec![n(END_REGEN * end, Unit::Whole, '+'), T(" stamina regen")],
            vec![pct(END_STAMINA_COST * end, '-'), T(" stamina costs")],
            vec![pct(END_POISE * end, '+'), T(" poise")],
            vec![pct(END_GUARD * end, '-'), T(" stamina lost when blocking")],
        ]),
        row("Strength", "heavy blows", a.strength, vec![
            vec![pct(STR_STANCE * str_, '+'), T(" stance damage")],
            vec![pct(STR_CHARGED * str_, '+'), T(" charged and jump attack damage")],
        ]),
        row("Dexterity", "speed and precision", a.dexterity, vec![
            vec![T("Attacks "), pct(DEX_ATTACK_SPEED * dex, '+'), T(" faster")],
            vec![T("Flasks and items "), pct(dexterity_item_speed(a.dexterity, strength), '+'), T(" faster")],
            vec![pct(DEX_CRITICAL * dex, '+'), T(" critical hit damage")],
            vec![pct(DEX_AFTER_DODGE * dex, '+'), T(" damage right after a dodge")],
            vec![pct(DEX_FALL * dex, '-'), T(" fall damage")],
        ]),
        row("Intelligence", "arcane timing", a.intelligence, vec![
            vec![T("Dodge invincibility "), n((INT_IFRAMES * int).min(0.10), Unit::Seconds, '+')],
            vec![T("Card spells recharge "), pct(INT_SPELL_COOLDOWN * int, '+'), T(" faster")],
        ]),
        row("Faith", "protection", a.faith, vec![
            vec![pct(FAITH_PHYSICAL * fai, '-'), T(" physical damage taken")],
            vec![pct(FAITH_ELEMENTAL * fai, '-'), T(" elemental damage taken")],
            vec![pct(FAITH_FLASK * fai, '+'), T(" flask healing")],
            faith_last,
        ]),
        row("Arcane", "fate and blood", a.arcane, vec![
            vec![pct(ARC_RUNES * arc, '+'), T(" runes")],
            vec![n(ARC_DISCOVERY * arc, Unit::Flat, '+'), T(" item discovery")],
            vec![pct(ARC_POTENCY * arc, '+'), T(" status buildup from cards")],
            vec![T("Statuses hit "), pct(ARC_STATUS_DAMAGE * arc, '+'), T(" harder")],
            vec![T("Rarer cards "), pct(arcane_luck(a.arcane, strength) as f32, '+')],
        ]),
    ]
}

/// The same lines as plain text: (name, job, points, lines).
pub fn describe(a: AttributeValues, strength: f32) -> Vec<(&'static str, &'static str, u32, Vec<String>)> {
    rows(a, strength)
        .into_iter()
        .map(|row| {
            let lines = row
                .lines
                .iter()
                .map(|line| line.iter().map(Part::show).collect::<String>())
                .collect();
            (row.name, row.job, row.points, lines)
        })
        .collect()
}
