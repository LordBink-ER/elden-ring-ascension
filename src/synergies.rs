//! Named synergies: owning certain powers together (sometimes with attribute
//! thresholds) grants extra real effects — SpEffect modifiers on the same
//! aggregate rows as cards, and damage multipliers for specific procs.
use crate::core::{CoreError, Rarity, Result};
use crate::powers::{Activation, Attribute, AttributeValues, CombatSnapshot, Modifier, PowerLibrary};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnyOf {
    pub count: u8,
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Synergy {
    pub id: String,
    pub name: String,
    pub rarity: Rarity,
    pub description: String,
    /// Every listed power at least at the given level.
    #[serde(default)]
    pub all: Vec<(String, u8)>,
    /// At least `count` of these powers (any level).
    #[serde(default)]
    pub any_of: Option<AnyOf>,
    #[serde(default)]
    pub attributes: BTreeMap<Attribute, u32>,
    #[serde(default)]
    pub activation: Option<Activation>,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
    #[serde(default)]
    pub proc_power: BTreeMap<String, f32>,
}

/// How far a character is from one synergy (for the collection screen).
#[derive(Debug, Clone)]
pub struct Progress {
    pub met: usize,
    pub needed: usize,
    pub missing: Vec<String>,
    pub complete: bool,
}

impl Synergy {
    pub fn progress(
        &self,
        levels: &BTreeMap<String, u8>,
        attributes: &AttributeValues,
        library: &PowerLibrary,
    ) -> Progress {
        let name = |id: &str| library.get(id).map(|p| p.name.clone()).unwrap_or_else(|| id.to_owned());
        let mut met = 0;
        let mut needed = 0;
        let mut missing = Vec::new();
        for (id, level) in &self.all {
            needed += 1;
            if levels.get(id).copied().unwrap_or(0) >= *level {
                met += 1;
            } else {
                missing.push(format!("{} {}", name(id), roman(*level)));
            }
        }
        if let Some(any) = &self.any_of {
            let owned = any.ids.iter().filter(|id| levels.contains_key(*id)).count();
            needed += usize::from(any.count);
            met += owned.min(usize::from(any.count));
            if owned < usize::from(any.count) {
                let rest: Vec<String> = any
                    .ids
                    .iter()
                    .filter(|id| !levels.contains_key(*id))
                    .map(|id| name(id))
                    .collect();
                missing.push(format!(
                    "{} more of: {}",
                    usize::from(any.count) - owned,
                    rest.join(", ")
                ));
            }
        }
        for (attribute, value) in &self.attributes {
            needed += 1;
            if attributes.get(*attribute) >= *value {
                met += 1;
            } else {
                missing.push(format!("{} {}", attribute_name(*attribute), value));
            }
        }
        Progress {
            met,
            needed,
            complete: met == needed && needed > 0,
            missing,
        }
    }
    /// Uses `power` (one of the cards it needs).
    pub fn involves(&self, power: &str) -> bool {
        self.all.iter().any(|(id, _)| id == power)
            || self.any_of.as_ref().is_some_and(|a| a.ids.iter().any(|id| id == power))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SynergyBook {
    pub schema: u32,
    pub synergies: Vec<Synergy>,
}

impl SynergyBook {
    pub fn from_json(source: &str, library: &PowerLibrary) -> Result<Self> {
        let book: Self = serde_json::from_str(source)?;
        book.validate(library)?;
        Ok(book)
    }
    pub fn shipped(library: &PowerLibrary) -> Result<Self> {
        Self::from_json(include_str!("../config/synergies.json"), library)
    }
    fn validate(&self, library: &PowerLibrary) -> Result<()> {
        if self.schema != 1 || self.synergies.len() > 500 {
            return Err(CoreError::Invalid("invalid synergy book".into()));
        }
        let mut ids = BTreeSet::new();
        for s in &self.synergies {
            let known = |id: &String| library.get(id).is_some();
            if !ids.insert(&s.id)
                || s.description.is_empty()
                || (s.all.is_empty() && s.any_of.is_none())
                || s.all.iter().any(|(id, level)| !known(id) || !(1..=5).contains(level))
                || s.any_of.as_ref().is_some_and(|a| {
                    a.count == 0 || usize::from(a.count) > a.ids.len() || !a.ids.iter().all(known)
                })
                || (s.modifiers.is_empty() && s.proc_power.is_empty())
                || s.modifiers.len() > 16
                || s
                    .modifiers
                    .iter()
                    .any(|m| !m.value.is_finite() || (!m.field.is_additive() && m.value <= 0.0))
                || s
                    .proc_power
                    .values()
                    .any(|f| !f.is_finite() || !(1.0..=3.0).contains(f))
            {
                return Err(CoreError::Invalid(format!("invalid synergy {}", s.id)));
            }
        }
        Ok(())
    }
    pub fn complete<'a>(
        &'a self,
        levels: &BTreeMap<String, u8>,
        attributes: &AttributeValues,
        library: &PowerLibrary,
    ) -> impl Iterator<Item = &'a Synergy> {
        let levels = levels.clone();
        let attributes = *attributes;
        let library_names: BTreeSet<String> = library.powers.iter().map(|p| p.id.clone()).collect();
        self.synergies.iter().filter(move |s| {
            s.all.iter().all(|(id, level)| {
                library_names.contains(id) && levels.get(id).copied().unwrap_or(0) >= *level
            }) && s.any_of.as_ref().is_none_or(|a| {
                a.ids.iter().filter(|id| levels.contains_key(*id)).count() >= usize::from(a.count)
            }) && s
                .attributes
                .iter()
                .all(|(attribute, value)| attributes.get(*attribute) >= *value)
        })
    }
    /// Modifiers of every complete synergy whose own condition holds now.
    pub fn active_modifiers(
        &self,
        levels: &BTreeMap<String, u8>,
        snapshot: &CombatSnapshot,
        library: &PowerLibrary,
    ) -> Vec<Modifier> {
        self.complete(levels, &snapshot.attributes, library)
            .filter(|s| s.activation.as_ref().is_none_or(|a| a.active(snapshot)))
            .flat_map(|s| s.modifiers.iter().copied())
            .collect()
    }
    /// Proc id -> damage multiplier from complete synergies (multiplicative, capped).
    pub fn proc_boosts(
        &self,
        levels: &BTreeMap<String, u8>,
        attributes: &AttributeValues,
        library: &PowerLibrary,
    ) -> BTreeMap<String, f32> {
        let mut boosts = BTreeMap::<String, f32>::new();
        for s in self.complete(levels, attributes, library) {
            for (proc, factor) in &s.proc_power {
                let entry = boosts.entry(proc.clone()).or_insert(1.0);
                *entry = (*entry * factor).min(3.0);
            }
        }
        boosts
    }
}

pub fn roman(level: u8) -> &'static str {
    match level {
        1 => "I",
        2 => "II",
        3 => "III",
        4 => "IV",
        5 => "V",
        _ => "-",
    }
}
pub fn attribute_name(attribute: Attribute) -> &'static str {
    match attribute {
        Attribute::Vigor => "Vigor",
        Attribute::Mind => "Mind",
        Attribute::Endurance => "Endurance",
        Attribute::Strength => "Strength",
        Attribute::Dexterity => "Dexterity",
        Attribute::Intelligence => "Intelligence",
        Attribute::Faith => "Faith",
        Attribute::Arcane => "Arcane",
    }
}
