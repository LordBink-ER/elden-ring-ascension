use super::{BossTier, Card, Catalog, CoreError, DrawConfig, MetaEffect, Rarity, Result};
use std::collections::BTreeMap;

/// SplitMix64 has a persisted state and stable integer arithmetic. It is not a
/// cryptographic generator; the integration supplies a fresh OS-random seed.
fn uniform(state: &mut u64) -> f64 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^= z >> 31;
    (z >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0)
}

pub(crate) fn weights(
    catalog: &Catalog,
    levels: &BTreeMap<String, u8>,
    tier: BossTier,
    cfg: &DrawConfig,
) -> Result<Vec<(Card, f64)>> {
    cfg.validate()?;
    let mut counts = [0u32; 6];
    for p in catalog.iter().filter(|p| {
        p.category != PRISMATIC
            && catalog.is_enabled(p)
            && !cfg.excluded.contains(&p.id)
            && p.unlocked(levels)
            && levels.get(&p.id).copied().unwrap_or(0) < cfg.max_power_level
    }) {
        counts[p.base_rarity.index()] += 1;
    }
    let mut fortune = 0.0;
    let mut affinity = 0.0;
    let mut category_bias = BTreeMap::<String, f64>::new();
    for p in catalog.iter().filter(|p| catalog.is_enabled(p)) {
        let level = levels.get(&p.id).copied().unwrap_or(0) as f64;
        match &p.meta {
            Some(MetaEffect::Fortune { per_level }) => fortune += per_level * level,
            Some(MetaEffect::Affinity { per_level }) => affinity += per_level * level,
            Some(MetaEffect::CategoryBias {
                category,
                per_level,
            }) => *category_bias.entry(category.clone()).or_default() += per_level * level,
            _ => {}
        }
    }
    // Bounded by category, never exponential in the number of acquired cards.
    fortune = (fortune + cfg.luck).min(2.0);
    affinity = affinity.min(2.0);
    let mut output = Vec::new();
    let mut prismatic = Vec::new();
    for p in catalog.iter().filter(|p| catalog.is_enabled(p)) {
        let current = levels.get(&p.id).copied().unwrap_or(0);
        if current > 5 {
            return Err(CoreError::Invalid("owned power exceeds level V".into()));
        }
        if current >= cfg.max_power_level || cfg.excluded.contains(&p.id) || !p.unlocked(levels) {
            continue;
        }
        // 3.4: Prismatic cards get their own fixed share below.
        if p.category == PRISMATIC {
            prismatic.push((p, current));
            continue;
        }
        let rarity = p.rarity_at(current + 1)?;
        let pity = if rarity >= Rarity::Epic { cfg.pity } else { 1.0 };
        let weight = pity * cfg.tier_weights[tier.index()][p.base_rarity.index()]
            / f64::from(counts[p.base_rarity.index()])
            * p.weight
            * cfg.boosts.get(&p.id).copied().unwrap_or(1.0)
            * cfg.repeat_weights[current as usize]
            * (1.0 + fortune * rarity.index() as f64 / 5.0)
            * (if current > 0 { 1.0 + affinity } else { 1.0 })
            * (1.0
                + category_bias
                    .get(&p.category)
                    .copied()
                    .unwrap_or(0.0)
                    .min(2.0));
        if !weight.is_finite() || weight <= 0.0 {
            return Err(CoreError::Invalid("invalid computed draw weight".into()));
        }
        output.push((
            Card {
                power_id: p.id.clone(),
                previous_level: current,
                target_level: current + 1,
                base_rarity: p.base_rarity,
                display_rarity: rarity,
            },
            weight,
        ));
    }
    // 3.4 Prismatic: each card slot is one with chance q (by boss tier,
    // raised up to x1.5 by luck), shared by the Prismatic cards available;
    // a later level of one is rarer, as for every card.
    let q = (cfg.prismatic[tier.index()] * (1.0 + fortune.min(1.0) * 0.5)).min(0.1);
    let normal: f64 = output.iter().map(|(_, w)| *w).sum();
    if q > 0.0 && normal > 0.0 && !prismatic.is_empty() {
        let share = normal * q / (1.0 - q) / prismatic.len() as f64;
        for (p, current) in prismatic {
            let rarity = p.rarity_at(current + 1)?;
            let weight = share * cfg.repeat_weights[current as usize];
            if weight.is_finite() && weight > 0.0 {
                output.push((
                    Card {
                        power_id: p.id.clone(),
                        previous_level: current,
                        target_level: current + 1,
                        base_rarity: p.base_rarity,
                        display_rarity: rarity,
                    },
                    weight,
                ));
            }
        }
    }
    Ok(output)
}

/// 3.4: the category of the ultra-rare cards.
pub const PRISMATIC: &str = "prismatic";

pub(crate) fn draw(
    catalog: &Catalog,
    levels: &BTreeMap<String, u8>,
    tier: BossTier,
    cfg: &DrawConfig,
    rng: &mut u64,
    count: usize,
) -> Result<Vec<Card>> {
    let count = count.clamp(3, 4);
    let mut pool = weights(catalog, levels, tier, cfg)?;
    if pool.is_empty() {
        return Err(CoreError::PoolExhausted {
            available: pool.len(),
        });
    }
    // Preserve three genuine choices at the finite tail of the library. Show
    // every remaining distinct upgrade, then repeat only those real cards.
    // Choosing still commits one level once; no max-level card re-enters.
    let refill = if pool.len() < count {
        Some(pool.clone())
    } else {
        None
    };
    let mut result = Vec::with_capacity(count);
    for _ in 0..count {
        if pool.is_empty() {
            pool = refill
                .clone()
                .expect("only a finite pool can empty before three draws");
        }
        let total: f64 = pool.iter().map(|(_, w)| *w).sum();
        let mut cursor = uniform(rng) * total;
        let mut selected = pool.len() - 1;
        for (i, (_, weight)) in pool.iter().enumerate() {
            if cursor < *weight {
                selected = i;
                break;
            }
            cursor -= *weight;
        }
        result.push(pool.remove(selected).0);
    }
    Ok(result)
}

/// One card for `id` at its next level, if it may be offered now (locked
/// cards and guaranteed evolutions). Same eligibility as the weighted pool.
pub(crate) fn card_for(
    catalog: &Catalog,
    levels: &BTreeMap<String, u8>,
    cfg: &DrawConfig,
    id: &str,
) -> Option<Card> {
    let p = catalog.get(id)?;
    let current = levels.get(id).copied().unwrap_or(0);
    if !catalog.is_enabled(p)
        || cfg.excluded.contains(id)
        || !p.unlocked(levels)
        || current >= cfg.max_power_level.min(5)
    {
        return None;
    }
    Some(Card {
        power_id: p.id.clone(),
        previous_level: current,
        target_level: current + 1,
        base_rarity: p.base_rarity,
        display_rarity: p.rarity_at(current + 1).ok()?,
    })
}

/// One weighted card from the pool (used to replace a banished card).
pub(crate) fn draw_one(
    catalog: &Catalog,
    levels: &BTreeMap<String, u8>,
    tier: BossTier,
    cfg: &DrawConfig,
    rng: &mut u64,
) -> Result<Card> {
    let pool = weights(catalog, levels, tier, cfg)?;
    let total: f64 = pool.iter().map(|(_, w)| *w).sum();
    if pool.is_empty() || !(total > 0.0) {
        return Err(CoreError::PoolExhausted { available: 0 });
    }
    let mut cursor = uniform(rng) * total;
    for (card, weight) in &pool {
        if cursor < *weight {
            return Ok(card.clone());
        }
        cursor -= *weight;
    }
    Ok(pool[pool.len() - 1].0.clone())
}

pub(crate) fn reroll_budget(
    catalog: &Catalog,
    levels: &BTreeMap<String, u8>,
    cfg: &DrawConfig,
) -> u8 {
    let total: u32 = catalog
        .iter()
        .filter(|p| catalog.is_enabled(p))
        .map(|p| match p.meta {
            Some(MetaEffect::Rerolls { per_level }) => {
                u32::from(per_level) * u32::from(levels.get(&p.id).copied().unwrap_or(0))
            }
            _ => 0,
        })
        .sum();
    total.min(u32::from(cfg.max_rerolls_per_reward)) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{PowerDef, Rarity};
    use std::collections::BTreeSet;

    #[test]
    fn diminishing_weight_is_local_to_owned_power() {
        let defs = ["a", "b", "c", "d"]
            .map(|id| PowerDef::new(id, id, "test", Rarity::Common, [Rarity::Common; 5]));
        let c = Catalog::new(defs.into(), BTreeSet::new()).unwrap();
        let a = weights(
            &c,
            &BTreeMap::new(),
            BossTier::Minor,
            &DrawConfig::default(),
        )
        .unwrap();
        let b = weights(
            &c,
            &BTreeMap::from([("a".into(), 3)]),
            BossTier::Minor,
            &DrawConfig::default(),
        )
        .unwrap();
        assert_eq!(a[1].1, b[1].1);
        assert!((b[0].1 / a[0].1 - 0.25).abs() < 1e-12);
    }

    #[test]
    fn fortune_uses_evolved_rarity_affinity_uses_ownership() {
        let mut fortune = PowerDef::new(
            "fortune",
            "fixture",
            "meta",
            Rarity::Rare,
            [Rarity::Rare; 5],
        );
        fortune.meta = Some(MetaEffect::Fortune { per_level: 0.2 });
        let mut affinity = PowerDef::new(
            "affinity",
            "fixture",
            "meta",
            Rarity::Rare,
            [Rarity::Rare; 5],
        );
        affinity.meta = Some(MetaEffect::Affinity { per_level: 0.1 });
        let evolved = PowerDef::new(
            "evolved",
            "fixture",
            "test",
            Rarity::Common,
            [
                Rarity::Common,
                Rarity::Mythic,
                Rarity::Mythic,
                Rarity::Mythic,
                Rarity::Mythic,
            ],
        );
        let ordinary = PowerDef::new(
            "ordinary",
            "fixture",
            "test",
            Rarity::Common,
            [Rarity::Common; 5],
        );
        let c = Catalog::new(vec![fortune, affinity, evolved, ordinary], BTreeSet::new()).unwrap();
        let cfg = DrawConfig::default();
        let owned = BTreeMap::from([("evolved".into(), 1)]);
        let baseline = weights(&c, &owned, BossTier::Minor, &cfg).unwrap();
        let mut modified_owned = owned.clone();
        modified_owned.insert("fortune".into(), 1);
        modified_owned.insert("affinity".into(), 1);
        let modified = weights(&c, &modified_owned, BossTier::Minor, &cfg).unwrap();
        let find = |pool: &Vec<(Card, f64)>, id: &str| {
            pool.iter().find(|(p, _)| p.power_id == id).unwrap().1
        };
        assert!(
            (find(&modified, "evolved") / find(&baseline, "evolved") - 1.2 * 1.1).abs() < 1e-12
        );
        assert_eq!(find(&baseline, "ordinary"), find(&modified, "ordinary"));
    }
}
