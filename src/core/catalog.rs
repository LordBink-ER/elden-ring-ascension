use super::{CoreError, Result, valid_id};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    Epic,
    Legendary,
    Mythic,
}
impl Rarity {
    pub fn index(self) -> usize {
        self as usize
    }
    pub const ALL: [Self; 6] = [
        Self::Common,
        Self::Uncommon,
        Self::Rare,
        Self::Epic,
        Self::Legendary,
        Self::Mythic,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum BossTier {
    Minor,
    Greater,
    Major,
    Remembrance,
}
impl BossTier {
    pub fn index(self) -> usize {
        self as usize
    }
}

/// All values are per acquired level. These alter the real draw algorithm,
/// not the game's combat system. Reroll allowance is captured per reward.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MetaEffect {
    Fortune { per_level: f64 },
    Affinity { per_level: f64 },
    Rerolls { per_level: u8 },
    CategoryBias { category: String, per_level: f64 },
    /// 0.6 Thread of Fate: Lock costs 1 Ember (free at level V).
    LockDiscount,
    /// 0.6 Fate's Ledger: skipped rewards pay `per_level` x level more runes,
    /// plus 1 Ember from level III and 2 at level V.
    SkipBonus { per_level: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PowerDef {
    pub id: String,
    pub name: String,
    pub category: String,
    pub base_rarity: Rarity,
    pub level_rarities: [Rarity; 5],
    pub weight: f64,
    #[serde(default)]
    pub required_capabilities: BTreeSet<String>,
    #[serde(default)]
    pub meta: Option<MetaEffect>,
    /// 0.5 evolutions: drawable only once every listed power has this level.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<(String, u8)>,
    /// 0.5 curse cards: a strong power with a permanent drawback.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cursed: bool,
    /// 2.0 signature cards: the bosses (ids) whose first defeat always offers
    /// this card. It joins the ordinary pool once one of them is defeated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signature: Vec<String>,
}
impl PowerDef {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        category: impl Into<String>,
        base_rarity: Rarity,
        level_rarities: [Rarity; 5],
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            category: category.into(),
            base_rarity,
            level_rarities,
            weight: 1.0,
            required_capabilities: BTreeSet::new(),
            meta: None,
            requires: Vec::new(),
            cursed: false,
            signature: Vec::new(),
        }
    }
    /// An evolution (it names parent powers).
    pub fn is_evolution(&self) -> bool {
        !self.requires.is_empty()
    }
    /// Every parent power owned at its required level.
    pub fn unlocked(&self, levels: &BTreeMap<String, u8>) -> bool {
        self.requires
            .iter()
            .all(|(id, level)| levels.get(id).copied().unwrap_or(0) >= *level)
    }
    pub fn rarity_at(&self, level: u8) -> Result<Rarity> {
        self.level_rarities
            .get(level.wrapping_sub(1) as usize)
            .copied()
            .ok_or_else(|| CoreError::Invalid("power level must be I–V".into()))
    }
    fn validate(&self) -> Result<()> {
        if !valid_id(&self.id)
            || !valid_id(&self.name)
            || !valid_id(&self.category)
            || !self.weight.is_finite()
            || !(0.000_001..=1_000_000.0).contains(&self.weight)
        {
            return Err(CoreError::Invalid(format!(
                "invalid power definition {}",
                self.id
            )));
        }
        if self.level_rarities[0] != self.base_rarity
            || self.level_rarities.windows(2).any(|r| r[1] < r[0])
            || self.required_capabilities.iter().any(|s| !valid_id(s))
        {
            return Err(CoreError::Invalid(format!(
                "invalid rarity/capability progression for {}",
                self.id
            )));
        }
        if self.signature.len() > 8 || self.signature.iter().any(|id| !valid_id(id)) {
            return Err(CoreError::Invalid(format!("invalid signature bosses for {}", self.id)));
        }
        if self.requires.len() > 4
            || self
                .requires
                .iter()
                .any(|(id, level)| !valid_id(id) || id == &self.id || !(1..=5).contains(level))
        {
            return Err(CoreError::Invalid(format!(
                "invalid evolution requirement for {}",
                self.id
            )));
        }
        match &self.meta {
            Some(
                MetaEffect::Fortune { per_level }
                | MetaEffect::Affinity { per_level }
                | MetaEffect::SkipBonus { per_level }
                | MetaEffect::CategoryBias { per_level, .. },
            ) if !per_level.is_finite() || !(0.0..=1.0).contains(per_level) => {
                return Err(CoreError::Invalid(
                    "meta draw coefficient must be finite and 0..1".into(),
                ));
            }
            Some(MetaEffect::CategoryBias { category, .. }) if !valid_id(category) => {
                return Err(CoreError::Invalid("invalid preferred category".into()));
            }
            Some(MetaEffect::Rerolls { per_level }) if *per_level > 2 => {
                return Err(CoreError::Invalid(
                    "at most two rerolls per power level".into(),
                ));
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Catalog {
    pub(crate) powers: BTreeMap<String, PowerDef>,
    pub(crate) capabilities: BTreeSet<String>,
}
impl Catalog {
    pub fn new(powers: Vec<PowerDef>, capabilities: BTreeSet<String>) -> Result<Self> {
        if powers.len() > 10_000 || capabilities.iter().any(|s| !valid_id(s)) {
            return Err(CoreError::Invalid(
                "catalog exceeds limits or has invalid capabilities".into(),
            ));
        }
        let mut map = BTreeMap::new();
        for p in powers {
            p.validate()?;
            if map.insert(p.id.clone(), p).is_some() {
                return Err(CoreError::Invalid("duplicate power id".into()));
            }
        }
        // Evolution parents must exist and may not be evolutions themselves.
        for p in map.values() {
            for (parent, _) in &p.requires {
                if map.get(parent).is_none_or(PowerDef::is_evolution) {
                    return Err(CoreError::Invalid(format!(
                        "evolution {} needs an unknown or evolved parent {parent}",
                        p.id
                    )));
                }
            }
        }
        Ok(Self {
            powers: map,
            capabilities,
        })
    }
    pub fn get(&self, id: &str) -> Option<&PowerDef> {
        self.powers.get(id)
    }
    pub fn iter(&self) -> impl Iterator<Item = &PowerDef> {
        self.powers.values()
    }
    pub fn is_enabled(&self, p: &PowerDef) -> bool {
        p.required_capabilities.is_subset(&self.capabilities)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrawConfig {
    /// Future offers stop at this configured ceiling; persisted ownership and
    /// already prepared offers retain the absolute I–V domain.
    #[serde(default = "default_max_power_level")]
    pub max_power_level: u8,
    /// Relative rarity probabilities per boss tier before per-power modifiers.
    pub tier_weights: [[f64; 6]; 4],
    /// Indexed by current level, including level 0; V must have zero weight.
    pub repeat_weights: [f64; 6],
    pub max_rerolls_per_reward: u8,
    /// Runtime only (never read from the profile): Arcane's draw luck, added
    /// to Fortune's bonus before its cap.
    #[serde(skip)]
    pub luck: f64,
    /// Runtime only: bad-luck protection factor for Epic and rarer cards.
    #[serde(skip, default = "neutral_factor")]
    pub pity: f64,
    /// Runtime only: powers that may not be drawn (banished, already shown).
    #[serde(skip)]
    pub excluded: BTreeSet<String>,
    /// Runtime only (0.5 [fate]): rank rewards (extra rerolls, fourth card).
    #[serde(skip, default = "yes")]
    pub rank_rewards: bool,
    /// Runtime only (0.5 [fate]): an unlocked evolution always joins the
    /// next offer until it is owned.
    #[serde(skip, default = "yes")]
    pub guarantee_evolutions: bool,
    /// Runtime only (2.0 Fortune path 10): cards added to every offer (max 4 total).
    #[serde(skip)]
    pub extra_cards: usize,
    /// Runtime only (2.0): weight multipliers per power (cards of the Paths
    /// you invest in show up more often), each 1.0..=2.0.
    #[serde(skip)]
    pub boosts: BTreeMap<String, f64>,
    /// 3.4: chance per card slot of a Prismatic card (category "prismatic"),
    /// per boss tier (minor, greater, major, remembrance), before luck.
    #[serde(default = "default_prismatic")]
    pub prismatic: [f64; 4],
}

fn default_prismatic() -> [f64; 4] {
    [0.0015, 0.003, 0.006, 0.012]
}
fn yes() -> bool {
    true
}
fn default_max_power_level() -> u8 {
    5
}
fn neutral_factor() -> f64 {
    1.0
}
impl Default for DrawConfig {
    fn default() -> Self {
        Self {
            max_power_level: default_max_power_level(),
            tier_weights: [
                [58.0, 26.0, 11.0, 3.7, 1.1, 0.2],
                [39.0, 30.0, 19.0, 8.0, 3.4, 0.6],
                [24.0, 27.0, 25.0, 15.0, 7.0, 2.0],
                [12.0, 19.0, 27.0, 24.0, 13.0, 5.0],
            ],
            repeat_weights: [1.0, 0.62, 0.40, 0.25, 0.16, 0.0],
            max_rerolls_per_reward: 10,
            luck: 0.0,
            pity: 1.0,
            excluded: BTreeSet::new(),
            rank_rewards: true,
            guarantee_evolutions: true,
            extra_cards: 0,
            boosts: BTreeMap::new(),
            prismatic: default_prismatic(),
        }
    }
}
impl DrawConfig {
    pub fn validate(&self) -> Result<()> {
        if self.boosts.values().any(|b| !b.is_finite() || !(1.0..=2.0).contains(b)) {
            return Err(CoreError::Invalid("draw boosts must be 1..2".into()));
        }
        if !(1..=5).contains(&self.max_power_level) {
            return Err(CoreError::Invalid(
                "configured maximum power level must remain within I–V".into(),
            ));
        }
        if self
            .tier_weights
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || !(0.000_001..=1_000_000.0).contains(v))
        {
            return Err(CoreError::Invalid(
                "every rarity must retain a finite nonzero chance in each boss tier".into(),
            ));
        }
        if self.repeat_weights[0] != 1.0
            || self.repeat_weights[5] != 0.0
            || self.repeat_weights[..5]
                .iter()
                .any(|v| !v.is_finite() || !(0.000_001..=1.0).contains(v))
            || self.repeat_weights[..5].windows(2).any(|w| w[1] >= w[0])
        {
            return Err(CoreError::Invalid(
                "repeat weights must strictly decrease from 1 through IV, with V excluded".into(),
            ));
        }
        if self.prismatic.iter().any(|v| !v.is_finite() || !(0.0..=0.1).contains(v)) {
            return Err(CoreError::Invalid("prismatic chance must be 0..0.1 per slot".into()));
        }
        if self.max_rerolls_per_reward > 10 {
            return Err(CoreError::Invalid("reroll safety cap is 10".into()));
        }
        if !self.luck.is_finite()
            || !(0.0..=1.0).contains(&self.luck)
            || !self.pity.is_finite()
            || !(1.0..=4.0).contains(&self.pity)
        {
            return Err(CoreError::Invalid("runtime draw luck/pity out of range".into()));
        }
        Ok(())
    }
}
