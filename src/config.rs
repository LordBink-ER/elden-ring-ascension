use crate::core::{BossTier, DrawConfig};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BossDefinition {
    pub id: String,
    pub name: String,
    pub region: String,
    pub flag: u32,
    pub tier: BossTier,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema: u32,
    pub profile_namespace: String,
    pub diagnostics: Diagnostics,
    pub balance: Balance,
    #[serde(default)]
    pub status: StatusConfig,
    #[serde(default)]
    pub draw: DrawConfig,
    #[serde(default)]
    pub elites: EliteSettings,
    #[serde(default)]
    pub fate: FateSettings,
    #[serde(default)]
    pub ui: UiSettings,
    #[serde(default)]
    pub world: WorldSettings,
}

/// 0.8: the world grows with you (regular enemies, runes, the Tide).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldSettings {
    /// Regular enemies get the Tide rows (bosses keep [balance]).
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "d_enemy_hp")]
    pub enemy_hp: f32,
    #[serde(default = "d_enemy_damage")]
    pub enemy_damage: f32,
    #[serde(default = "d_enemy_poise")]
    pub enemy_poise: f32,
    #[serde(default = "d_boss_poise")]
    pub boss_poise: f32,
    /// Runes from everything.
    #[serde(default = "d_runes")]
    pub rune_multiplier: f32,
    /// One Tide level per this many card levels, up to X; each level adds
    /// the four values below.
    #[serde(default = "d_tide_step")]
    pub tide_step: u32,
    #[serde(default = "d_tide_hp")]
    pub tide_hp: f32,
    #[serde(default = "d_tide_damage")]
    pub tide_damage: f32,
    #[serde(default = "d_tide_poise")]
    pub tide_poise: f32,
    #[serde(default = "d_tide_runes")]
    pub tide_runes: f32,
}
fn d_enemy_hp() -> f32 { 1.3 }
fn d_enemy_damage() -> f32 { 1.15 }
fn d_enemy_poise() -> f32 { 1.25 }
fn d_boss_poise() -> f32 { 1.2 }
fn d_runes() -> f32 { 1.5 }
fn d_tide_step() -> u32 { 20 }
fn d_tide_hp() -> f32 { 0.05 }
fn d_tide_damage() -> f32 { 0.03 }
fn d_tide_poise() -> f32 { 0.03 }
fn d_tide_runes() -> f32 { 0.08 }
impl Default for WorldSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            enemy_hp: d_enemy_hp(),
            enemy_damage: d_enemy_damage(),
            enemy_poise: d_enemy_poise(),
            boss_poise: d_boss_poise(),
            rune_multiplier: d_runes(),
            tide_step: d_tide_step(),
            tide_hp: d_tide_hp(),
            tide_damage: d_tide_damage(),
            tide_poise: d_tide_poise(),
            tide_runes: d_tide_runes(),
        }
    }
}

/// 0.7 "Wygląd": card art, card animations and sounds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiSettings {
    /// Card illustrations (assets/cards) on cards, tiles and the combat tracker.
    #[serde(default = "default_true")]
    pub card_art: bool,
    /// Cards are dealt face down and flip over one by one.
    #[serde(default = "default_true")]
    pub card_animations: bool,
    /// Interface sounds (deal, rarity reveals, choice, evolution).
    #[serde(default = "default_true")]
    pub sounds: bool,
    /// 0.0-1.0; PlaySound ignores the game's own volume sliders.
    #[serde(default = "default_volume")]
    pub sound_volume: f32,
    /// Short sounds for perfect dodges, parries and criticals.
    #[serde(default = "default_true")]
    pub reflex_sounds: bool,
}
fn default_volume() -> f32 {
    0.6
}
impl Default for UiSettings {
    fn default() -> Self {
        Self {
            card_art: true,
            card_animations: true,
            sounds: true,
            sound_volume: default_volume(),
            reflex_sounds: true,
        }
    }
}

/// 0.5 "Fate": choices at the reward screen and NG+ boss affixes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FateSettings {
    /// Curse cards (big power, permanent drawback) may be offered.
    #[serde(default = "default_true")]
    pub curses: bool,
    /// An unlocked evolution (two parents at V) joins the next offer.
    #[serde(default = "default_true")]
    pub evolutions: bool,
    /// +1 free reroll per 10 power levels; a fourth card at Elden Lord.
    #[serde(default = "default_true")]
    pub rank_rewards: bool,
    /// A reward may be skipped for runes and one Ember.
    #[serde(default = "default_true")]
    pub skip_rewards: bool,
    /// Bosses in NG+ gain affixes (one per cycle, two from NG+3).
    #[serde(default = "default_true")]
    pub boss_affixes: bool,
    /// First NG+ cycle with boss affixes (1 = NG+1).
    #[serde(default = "default_affix_cycle")]
    pub boss_affix_from_cycle: u32,
    /// 3.0 Ascended Bosses: from Tide I a boss may rise with its own
    /// affixes; beating it pays rarer cards and Embers.
    #[serde(default = "default_true")]
    pub ascended_bosses: bool,
}
fn default_affix_cycle() -> u32 {
    1
}
impl Default for FateSettings {
    fn default() -> Self {
        Self {
            curses: true,
            evolutions: true,
            rank_rewards: true,
            skip_rewards: true,
            boss_affixes: true,
            boss_affix_from_cycle: default_affix_cycle(),
            ascended_bosses: true,
        }
    }
}

/// Ascended Elites (0.4): empowered regular enemies with affixes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EliteSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Chance that a newly met regular enemy is an elite.
    #[serde(default = "default_elite_chance")]
    pub chance: f64,
    /// Chance of a second affix (+0.15 per NG+ cycle).
    #[serde(default = "default_double_affix")]
    pub double_affix_chance: f64,
    /// At most this many living elites within 60 m.
    #[serde(default = "default_max_nearby")]
    pub max_nearby: u8,
    /// Runes an elite drops, as a multiple of a normal enemy of its kind.
    #[serde(default = "default_elite_runes")]
    pub rune_multiplier: f32,
}
fn default_true() -> bool {
    true
}
fn default_elite_chance() -> f64 {
    0.06
}
fn default_double_affix() -> f64 {
    0.2
}
fn default_max_nearby() -> u8 {
    3
}
fn default_elite_runes() -> f32 {
    3.0
}
impl Default for EliteSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            chance: default_elite_chance(),
            double_affix_chance: default_double_affix(),
            max_nearby: default_max_nearby(),
            rune_multiplier: default_elite_runes(),
        }
    }
}
impl EliteSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !self.chance.is_finite()
            || !(0.0..=0.5).contains(&self.chance)
            || !self.double_affix_chance.is_finite()
            || !(0.0..=1.0).contains(&self.double_affix_chance)
            || self.max_nearby > 10
            || !self.rune_multiplier.is_finite()
            || !(1.0..=10.0).contains(&self.rune_multiplier)
        {
            return Err("invalid [elites] settings".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostics {
    pub overlay: bool,
    pub snapshot_interval_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Balance {
    pub boss_hp_multiplier: f64,
    pub boss_damage_multiplier: f64,
    #[serde(default = "default_attribute_strength")]
    pub attribute_strength: f32,
    /// 0.4: per boss tier (minor, greater, major, remembrance). Absent = the
    /// flat multipliers above for every tier.
    #[serde(default)]
    pub tier_hp_multipliers: Option<[f64; 4]>,
    #[serde(default)]
    pub tier_damage_multipliers: Option<[f64; 4]>,
}
impl Balance {
    pub fn tiers(&self) -> [(f32, f32); 4] {
        std::array::from_fn(|i| {
            (
                self.tier_hp_multipliers.map(|t| t[i]).unwrap_or(self.boss_hp_multiplier) as f32,
                self.tier_damage_multipliers.map(|t| t[i]).unwrap_or(self.boss_damage_multiplier) as f32,
            )
        })
    }
}
fn default_attribute_strength() -> f32 {
    1.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusChannel {
    Bleed,
    Frostbite,
    Poison,
    ScarletRot,
    Sleep,
    Madness,
}
impl StatusChannel {
    pub fn field(self) -> crate::powers::EffectField {
        use crate::powers::EffectField::*;
        match self {
            Self::Bleed => BloodAttackPower,
            Self::Frostbite => FreezeAttackPower,
            Self::Poison => PoizonAttackPower,
            Self::ScarletRot => DiseaseAttackPower,
            Self::Sleep => SleepAttackPower,
            Self::Madness => MadnessAttackPower,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusConfig {
    pub priority: Vec<StatusChannel>,
}
impl Default for StatusConfig {
    fn default() -> Self {
        Self {
            priority: vec![
                StatusChannel::Bleed,
                StatusChannel::Frostbite,
                StatusChannel::Poison,
                StatusChannel::ScarletRot,
                StatusChannel::Sleep,
                StatusChannel::Madness,
            ],
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Compatibility {
    pub product_version: String,
    pub sha256: Vec<String>,
}

impl Config {
    pub fn load(root: &Path) -> Result<(Self, Vec<BossDefinition>, Compatibility), String> {
        let config: Self = toml::from_str(
            &fs::read_to_string(root.join("config/ascension.toml")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        config.validate()?;
        let bosses: Vec<BossDefinition> = serde_json::from_slice(
            &fs::read(root.join("config/bosses.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        validate_bosses(&bosses)?;
        let compatibility: Compatibility = serde_json::from_slice(
            &fs::read(root.join("config/compatibility.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if compatibility.product_version != "2.7.2.0"
            || compatibility.sha256.is_empty()
            || compatibility
                .sha256
                .iter()
                .any(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err("invalid executable compatibility gate".into());
        }
        Ok((config, bosses, compatibility))
    }

    pub fn validate(&self) -> Result<(), String> {
        self.elites.validate()?;
        if self.fate.boss_affix_from_cycle > 10 {
            return Err("[fate] boss_affix_from_cycle must be 0..10".into());
        }
        if !self.ui.sound_volume.is_finite() || !(0.0..=1.0).contains(&self.ui.sound_volume) {
            return Err("[ui] sound_volume must be 0.0..1.0".into());
        }
        let w = &self.world;
        for (name, value, lo, hi) in [
            ("enemy_hp", w.enemy_hp, 0.5, 4.0),
            ("enemy_damage", w.enemy_damage, 0.5, 3.0),
            ("enemy_poise", w.enemy_poise, 0.5, 3.0),
            ("boss_poise", w.boss_poise, 0.5, 3.0),
            ("rune_multiplier", w.rune_multiplier, 0.1, 5.0),
            ("tide_hp", w.tide_hp, 0.0, 0.3),
            ("tide_damage", w.tide_damage, 0.0, 0.3),
            ("tide_poise", w.tide_poise, 0.0, 0.3),
            ("tide_runes", w.tide_runes, 0.0, 0.5),
        ] {
            if !value.is_finite() || !(lo..=hi).contains(&value) {
                return Err(format!("[world] {name} must be {lo}..{hi}"));
            }
        }
        if !(1..=1000).contains(&w.tide_step) {
            return Err("[world] tide_step must be 1..1000".into());
        }
        if self.schema != 1
            || self.profile_namespace.is_empty()
            || self.profile_namespace.len() > 128
            || self.profile_namespace.chars().any(char::is_control)
        {
            return Err("invalid schema or profile namespace".into());
        }
        if !(250..=60_000).contains(&self.diagnostics.snapshot_interval_ms) {
            return Err("diagnostic interval must be 250..60000 ms".into());
        }
        let tiers: Vec<f64> = self
            .balance
            .tier_hp_multipliers
            .into_iter()
            .chain(self.balance.tier_damage_multipliers)
            .flatten()
            .collect();
        for value in [
            self.balance.boss_hp_multiplier,
            self.balance.boss_damage_multiplier,
        ]
        .into_iter()
        .chain(tiers)
        {
            if !value.is_finite() || !(0.1..=10.0).contains(&value) {
                return Err("boss multiplier must be finite and 0.1..10".into());
            }
        }
        if !self.balance.attribute_strength.is_finite()
            || !(0.0..=2.0).contains(&self.balance.attribute_strength)
        {
            return Err("attribute strength must be finite and 0..2".into());
        }
        if self.status.priority.len() != 6
            || self.status.priority.iter().collect::<BTreeSet<_>>().len() != 6
        {
            return Err("status priority must contain all six channels exactly once".into());
        }
        self.draw.validate().map_err(|e| e.to_string())
    }
}

pub fn validate_bosses(bosses: &[BossDefinition]) -> Result<(), String> {
    if bosses.len() > 1024 {
        return Err("too many boss definitions".into());
    }
    let mut ids = BTreeSet::new();
    let mut flags = BTreeSet::new();
    for boss in bosses {
        if boss.id.is_empty()
            || boss.id.len() > 128
            || !boss
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || boss.name.is_empty()
            || boss.flag == 0
            || !ids.insert(&boss.id)
            || !flags.insert(boss.flag)
        {
            return Err(format!("invalid or duplicate boss definition: {}", boss.id));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shipped_config_is_valid() {
        let (_, bosses, _) = Config::load(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        assert!(bosses.len() >= 200);
        assert!(
            bosses
                .iter()
                .any(|b| b.region.contains("Shadow") || b.region.contains("Gravesite"))
        );
    }
}

/// 1.0: write the [ui] settings back into config/ascension.toml (in-game
/// Settings tab). Only the five [ui] keys change; every comment and other
/// section stays as it is. Written to a temporary file, then renamed.
pub fn save_ui_settings(root: &Path, ui: &UiSettings) -> Result<(), String> {
    let path = root.join("config/ascension.toml");
    let text = fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let values = [
        ("card_art", ui.card_art.to_string()),
        ("card_animations", ui.card_animations.to_string()),
        ("sounds", ui.sounds.to_string()),
        ("sound_volume", format!("{:.2}", ui.sound_volume.clamp(0.0, 1.0))),
        ("reflex_sounds", ui.reflex_sounds.to_string()),
    ];
    let mut out: Vec<String> = Vec::new();
    let mut in_ui = false;
    let mut seen = [false; 5];
    let mut had_ui = false;
    let flush_missing = |out: &mut Vec<String>, seen: &[bool; 5]| {
        for (i, (key, value)) in values.iter().enumerate() {
            if !seen[i] {
                out.push(format!("{key} = {value}"));
            }
        }
    };
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if in_ui {
                flush_missing(&mut out, &seen);
                seen = [true; 5];
            }
            in_ui = trimmed == "[ui]";
            had_ui |= in_ui;
        }
        if in_ui {
            if let Some((i, (key, value))) = values
                .iter()
                .enumerate()
                .find(|(_, (key, _))| trimmed.split('=').next().is_some_and(|k| k.trim() == *key))
            {
                out.push(format!("{key} = {value}"));
                seen[i] = true;
                continue;
            }
        }
        out.push(line.to_owned());
    }
    if in_ui {
        flush_missing(&mut out, &seen);
    }
    if !had_ui {
        out.push(String::new());
        out.push("[ui]".into());
        flush_missing(&mut out, &[false; 5]);
    }
    let mut result = out.join("\n");
    result.push('\n');
    // The file must still load before it replaces the old one.
    toml::from_str::<Config>(&result).map_err(|e| format!("settings not saved: {e}"))?;
    let temporary = path.with_extension("toml.new");
    fs::write(&temporary, &result).map_err(|e| format!("cannot write {}: {e}", temporary.display()))?;
    fs::rename(&temporary, &path).map_err(|e| format!("cannot replace {}: {e}", path.display()))?;
    Ok(())
}
