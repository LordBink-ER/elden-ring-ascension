use super::{BossTier, Catalog, CoreError, DrawConfig, Rarity, Result, draw, valid_id};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RewardKey {
    pub cycle: u32,
    pub boss_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Victory {
    pub key: RewardKey,
    pub tier: BossTier,
    /// 3.0: the boss was Ascended with this many affixes of its own (rarer
    /// cards, +1 card from 2). Omitted at 0, so older journals still match.
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub ascended: u8,
}
fn is_zero_u8(value: &u8) -> bool {
    *value == 0
}
/// 3.0: an Ascended victory's luck (+0.12 per affix, at most three).
fn ascended_luck(victory: &Victory) -> f64 {
    0.12 * f64::from(victory.ascended.min(3))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Card {
    pub power_id: String,
    pub previous_level: u8,
    pub target_level: u8,
    pub base_rarity: Rarity,
    pub display_rarity: Rarity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Offer {
    pub id: u64,
    pub victory: Victory,
    pub cards: Vec<Card>,
    pub rerolls_remaining: u8,
}
impl Offer {
    /// Fewer than three means the final valid upgrades are repeated in slots,
    /// not that selecting multiple copies grants more than one level.
    pub fn distinct_power_count(&self) -> usize {
        self.cards
            .iter()
            .map(|c| c.power_id.as_str())
            .collect::<BTreeSet<_>>()
            .len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceRecord {
    pub offer_id: u64,
    pub victory: Victory,
    pub power_id: String,
    pub level: u8,
}

/// 0.5: a card kept through rerolls and carried into the offer after the one
/// where it was locked (`origin`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedCard {
    pub power_id: String,
    pub origin: RewardKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Progression {
    character_id: String,
    cycle: Option<u32>,
    levels: BTreeMap<String, u8>,
    seen_bosses: BTreeSet<RewardKey>,
    pending: VecDeque<Victory>,
    offer: Option<Offer>,
    choices: Vec<ChoiceRecord>,
    rng: u64,
    next_offer_id: u64,
    /// Offers in a row without an Epic or rarer card (bad-luck protection).
    /// Omitted while zero, so journals written before 0.3 hash identically.
    #[serde(default, skip_serializing_if = "is_zero")]
    dry_offers: u32,
    /// Ascension Embers (0.4): earned from bosses and elites, spent on extra
    /// rerolls and banishing cards. Omitted while zero (journal checksums).
    #[serde(default, skip_serializing_if = "is_zero")]
    embers: u32,
    /// Powers this character never wants offered again. Owned levels stay.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    banished: BTreeSet<String>,
    /// 0.5 Fate: the locked card (Embers), omitted while none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    locked: Option<LockedCard>,
    /// 0.5 Fate: victories whose card reward was skipped for runes.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    skipped: BTreeSet<RewardKey>,
    /// 0.5 Fate: runes owed for skipped rewards; the game task pays them into
    /// the character and acknowledges. Omitted while zero.
    #[serde(default, skip_serializing_if = "is_zero")]
    rune_debt: u32,
}
fn is_zero(value: &u32) -> bool {
    *value == 0
}
/// Embers to lock a card: it survives rerolls and returns in the next offer.
pub const LOCK_COST: u32 = 2;
/// 0.6 Fate cards: the Lock cost with the owned cards (Thread of Fate: 1, free at V).
pub fn lock_cost(catalog: &Catalog, levels: &BTreeMap<String, u8>) -> u32 {
    catalog
        .iter()
        .filter(|p| matches!(p.meta, Some(crate::core::MetaEffect::LockDiscount)))
        .filter_map(|p| levels.get(&p.id).copied().filter(|l| *l > 0))
        .map(|level| if level >= 5 { 0 } else { 1 })
        .min()
        .unwrap_or(LOCK_COST)
}
/// 0.6 Fate cards: (rune multiplier, extra embers) for skipping a reward.
pub fn skip_bonus(catalog: &Catalog, levels: &BTreeMap<String, u8>) -> (f64, u32) {
    let mut rate = 1.0;
    let mut embers = 0;
    for p in catalog.iter() {
        if let Some(crate::core::MetaEffect::SkipBonus { per_level }) = &p.meta {
            let level = levels.get(&p.id).copied().unwrap_or(0);
            if level > 0 {
                rate += per_level * f64::from(level);
                embers += u32::from(level >= 3) + u32::from(level >= 5);
            }
        }
    }
    (rate.clamp(1.0, 4.0), embers.min(4))
}
/// Embers granted (with runes) for skipping a card reward.
pub const SKIP_EMBERS: u32 = 1;
const MAX_RUNE_DEBT: u32 = 999_999_999;
const MAX_SKIP_RUNES: u32 = 10_000_000;

/// Ascension ranks by total owned power levels.
pub const RANKS: [(u32, &str); 7] = [
    (0, "Tarnished"),
    (5, "Ascendant"),
    (12, "Champion"),
    (22, "Demigod"),
    (36, "Lord"),
    (56, "Elden Lord"),
    (91, "God of the Ring"),
];
pub fn total_levels(levels: &BTreeMap<String, u8>) -> u32 {
    levels.values().map(|l| u32::from(*l)).sum()
}
pub fn rank_name(total: u32) -> &'static str {
    RANKS
        .iter()
        .rev()
        .find(|(from, _)| total >= *from)
        .map(|(_, name)| *name)
        .unwrap_or("Tarnished")
}
/// The next rank and the levels it needs, if any.
pub fn next_rank(total: u32) -> Option<(&'static str, u32)> {
    RANKS
        .iter()
        .find(|(from, _)| *from > total)
        .map(|(from, name)| (*name, *from))
}
/// Rank reward: one extra free reroll per offer for every 10 power levels.
pub fn rank_rerolls(total: u32) -> u8 {
    (total / 10).min(5) as u8
}
/// Rank reward: Elden Lord sees four cards after Remembrance bosses, God of
/// the Ring after Major and Remembrance bosses.
pub fn offer_size(total: u32, tier: BossTier) -> usize {
    if (total >= 91 && tier.index() >= BossTier::Major.index())
        || (total >= 56 && tier == BossTier::Remembrance)
    {
        4
    } else {
        3
    }
}
/// Runes for leaving level `level` (Elden Ring's level-up cost curve).
pub fn level_up_cost(level: u32) -> u32 {
    let l = f64::from(level.clamp(1, 712) + 81);
    let x = ((l - 92.0) * 0.02).max(0.0);
    ((x + 0.1) * l * l + 1.0).floor() as u32
}
/// Runes for skipping a reward: part of a level-up, more for greater bosses
/// and in NG+.
pub fn skip_runes(level: u32, tier: BossTier, ng_plus: u32) -> u32 {
    let share = match tier {
        BossTier::Minor => 0.5,
        BossTier::Greater => 0.8,
        BossTier::Major => 1.2,
        BossTier::Remembrance => 2.0,
    };
    let ng = 1.0 + 0.25 * f64::from(ng_plus.min(7));
    (f64::from(level_up_cost(level)) * share * ng).round() as u32
}
/// Embers for a reroll once the free rerolls of an offer are used up.
pub const EMBER_REROLL_COST: u32 = 3;
/// Embers to banish one card of an offer (it is replaced by a new draw).
pub const BANISH_COST: u32 = 2;
const MAX_EMBERS: u32 = 1_000_000;
/// Embers for a first victory over a boss of this tier.
pub fn tier_embers(tier: BossTier) -> u32 {
    match tier {
        BossTier::Minor => 1,
        BossTier::Greater => 2,
        BossTier::Major => 3,
        BossTier::Remembrance => 5,
    }
}
/// From the fourth dry offer on, Epic and rarer cards weigh 1.5x, 2x .. 3x.
pub fn pity_factor(dry_offers: u32) -> f64 {
    if dry_offers < 4 {
        1.0
    } else {
        (1.0 + 0.5 * f64::from(dry_offers - 3)).min(3.0)
    }
}

impl Progression {
    /// `character_id` must be an adapter-verified immutable identity, including
    /// the account, save namespace, slot, and a creation discriminator. A name,
    /// slot number, play time, mutable stats or current NG+ is insufficient.
    pub fn new(character_id: impl Into<String>, seed: u64) -> Result<Self> {
        let character_id = character_id.into();
        if !valid_id(&character_id) {
            return Err(CoreError::Invalid("invalid character identity".into()));
        }
        Ok(Self {
            character_id,
            cycle: None,
            levels: BTreeMap::new(),
            seen_bosses: BTreeSet::new(),
            pending: VecDeque::new(),
            offer: None,
            choices: Vec::new(),
            rng: seed,
            next_offer_id: 1,
            dry_offers: 0,
            embers: 0,
            banished: BTreeSet::new(),
            locked: None,
            skipped: BTreeSet::new(),
            rune_debt: 0,
        })
    }
    pub fn character_id(&self) -> &str {
        &self.character_id
    }
    pub fn cycle(&self) -> Option<u32> {
        self.cycle
    }
    pub fn levels(&self) -> &BTreeMap<String, u8> {
        &self.levels
    }
    pub fn level(&self, id: &str) -> u8 {
        self.levels.get(id).copied().unwrap_or(0)
    }
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
    pub fn current_offer(&self) -> Option<&Offer> {
        self.offer.as_ref()
    }
    pub fn dry_offers(&self) -> u32 {
        self.dry_offers
    }
    pub fn embers(&self) -> u32 {
        self.embers
    }
    pub fn banished(&self) -> &BTreeSet<String> {
        &self.banished
    }
    pub fn locked(&self) -> Option<&LockedCard> {
        self.locked.as_ref()
    }
    pub fn skipped_count(&self) -> usize {
        self.skipped.len()
    }
    pub fn rune_debt(&self) -> u32 {
        self.rune_debt
    }
    pub fn total_levels(&self) -> u32 {
        total_levels(&self.levels)
    }
    fn size_for(&self, cfg: &DrawConfig, tier: BossTier) -> usize {
        let base = if cfg.rank_rewards {
            offer_size(self.total_levels(), tier)
        } else {
            3
        };
        (base + cfg.extra_cards).min(4)
    }

    fn reroll_allowance(&self, catalog: &Catalog, cfg: &DrawConfig) -> u8 {
        let rank = if cfg.rank_rewards {
            rank_rerolls(self.total_levels())
        } else {
            0
        };
        draw::reroll_budget(catalog, &self.levels, cfg)
            .saturating_add(rank)
            .min(cfg.max_rerolls_per_reward)
    }
    /// 2.0: signature cards stay out of the pool until one of their bosses
    /// has been defeated by this character (any cycle, baseline included).
    fn hold_back_signatures(&self, catalog: &Catalog, cfg: &mut DrawConfig) {
        for p in catalog.iter().filter(|p| !p.signature.is_empty() && self.level(&p.id) == 0) {
            let met = p
                .signature
                .iter()
                .any(|boss| self.seen_bosses.iter().any(|key| &key.boss_id == boss));
            if !met {
                cfg.excluded.insert(p.id.clone());
            }
        }
    }
    /// The boss's signature card, the locked card and a newly unlocked
    /// evolution always join an offer, in that order from the first slot.
    /// A card already drawn is moved there.
    fn apply_guarantees(&self, cards: &mut [Card], catalog: &Catalog, cfg: &DrawConfig, victory: &Victory) {
        let mut forced: Vec<Card> = Vec::new();
        if let Some(card) = catalog
            .iter()
            .filter(|p| p.signature.iter().any(|boss| boss == &victory.key.boss_id))
            .find_map(|p| draw::card_for(catalog, &self.levels, cfg, &p.id))
        {
            forced.push(card);
        }
        if let Some(lock) = &self.locked {
            if let Some(card) = draw::card_for(catalog, &self.levels, cfg, &lock.power_id) {
                if !forced.iter().any(|c| c.power_id == card.power_id) {
                    forced.push(card);
                }
            }
        }
        if cfg.guarantee_evolutions {
            if let Some(card) = catalog
                .iter()
                .filter(|p| p.is_evolution() && self.level(&p.id) == 0)
                .find_map(|p| draw::card_for(catalog, &self.levels, cfg, &p.id))
            {
                if !forced.iter().any(|c| c.power_id == card.power_id) {
                    forced.push(card);
                }
            }
        }
        for (slot, card) in forced.into_iter().enumerate() {
            if slot >= cards.len() {
                break;
            }
            if let Some(existing) = cards.iter().position(|c| c.power_id == card.power_id) {
                cards.swap(existing, slot);
            } else {
                cards[slot] = card;
            }
        }
    }
    /// Spend embers to lock the card in `slot`: it stays through rerolls and
    /// returns in the next offer even if another card is chosen now.
    pub fn lock(&mut self, offer_id: u64, slot: usize) -> Result<&Offer> {
        self.lock_with(offer_id, slot, LOCK_COST)
    }
    /// `lock` at a given Ember cost (0.6 Fate cards).
    pub fn lock_with(&mut self, offer_id: u64, slot: usize, cost: u32) -> Result<&Offer> {
        let offer = self.offer.as_ref().ok_or(CoreError::NoOffer)?;
        if offer.id != offer_id {
            return Err(CoreError::StaleOffer);
        }
        let card = offer
            .cards
            .get(slot)
            .ok_or_else(|| CoreError::Invalid("no such card".into()))?
            .clone();
        if self
            .locked
            .as_ref()
            .is_some_and(|l| l.power_id == card.power_id && l.origin == offer.victory.key)
        {
            return Err(CoreError::Invalid("this card is already locked".into()));
        }
        if self.embers < cost {
            return Err(CoreError::NotEnoughEmbers {
                have: self.embers,
                need: cost,
            });
        }
        let next = self
            .next_offer_id
            .checked_add(1)
            .ok_or_else(|| CoreError::Invalid("offer id overflow".into()))?;
        let mut cards = offer.cards.clone();
        cards.swap(0, slot);
        self.locked = Some(LockedCard {
            power_id: card.power_id,
            origin: offer.victory.key.clone(),
        });
        self.offer = Some(Offer {
            id: self.next_offer_id,
            victory: offer.victory.clone(),
            cards,
            rerolls_remaining: offer.rerolls_remaining,
        });
        self.embers -= cost;
        self.next_offer_id = next;
        Ok(self.offer.as_ref().expect("offer was assigned above"))
    }
    /// Leave this reward's cards for runes (owed until the game task pays
    /// them) and one Ember. The victory can never be rewarded again.
    pub fn skip(&mut self, offer_id: u64, runes: u32) -> Result<u32> {
        self.skip_with(offer_id, runes, SKIP_EMBERS)
    }
    /// `skip` granting `embers` (0.6 Fate cards add to the base Ember).
    pub fn skip_with(&mut self, offer_id: u64, runes: u32, embers: u32) -> Result<u32> {
        let offer = self.offer.as_ref().ok_or(CoreError::NoOffer)?;
        if offer.id != offer_id {
            return Err(CoreError::StaleOffer);
        }
        let key = offer.victory.key.clone();
        let runes = runes.min(MAX_SKIP_RUNES);
        self.pending.pop_front();
        self.offer = None;
        self.skipped.insert(key.clone());
        self.rune_debt = self.rune_debt.saturating_add(runes).min(MAX_RUNE_DEBT);
        self.embers = self.embers.saturating_add(embers.min(10)).min(MAX_EMBERS);
        if self.locked.as_ref().is_some_and(|l| l.origin != key) {
            self.locked = None;
        }
        Ok(runes)
    }
    /// The game task paid `amount` owed runes into the character.
    pub fn runes_paid(&mut self, amount: u32) -> Result<()> {
        self.rune_debt = self.rune_debt.saturating_sub(amount);
        Ok(())
    }
    /// Bounded per call (an elite is worth a few embers at most).
    pub fn add_embers(&mut self, amount: u32) -> Result<()> {
        if amount == 0 || amount > 100 {
            return Err(CoreError::Invalid("ember grant out of range".into()));
        }
        self.embers = self.embers.saturating_add(amount).min(MAX_EMBERS);
        Ok(())
    }
    /// Spend embers to banish the card in `slot`: it never appears again for
    /// this character and is replaced in this offer by a fresh draw.
    pub fn banish(&mut self, offer_id: u64, slot: usize, catalog: &Catalog, cfg: &DrawConfig) -> Result<&Offer> {
        let offer = self.offer.as_ref().ok_or(CoreError::NoOffer)?;
        if offer.id != offer_id {
            return Err(CoreError::StaleOffer);
        }
        let card = offer
            .cards
            .get(slot)
            .ok_or_else(|| CoreError::Invalid("no such card".into()))?
            .clone();
        if self.embers < BANISH_COST {
            return Err(CoreError::NotEnoughEmbers {
                have: self.embers,
                need: BANISH_COST,
            });
        }
        let next = self
            .next_offer_id
            .checked_add(1)
            .ok_or_else(|| CoreError::Invalid("offer id overflow".into()))?;
        let mut cfg = cfg.clone();
        cfg.pity = pity_factor(self.dry_offers);
        cfg.luck = (cfg.luck + ascended_luck(&offer.victory)).min(1.0);
        cfg.excluded.extend(self.banished.iter().cloned());
        self.hold_back_signatures(catalog, &mut cfg);
        cfg.excluded.insert(card.power_id.clone());
        for other in &offer.cards {
            cfg.excluded.insert(other.power_id.clone());
        }
        let mut next_rng = self.rng;
        let replacement = draw::draw_one(catalog, &self.levels, offer.victory.tier, &cfg, &mut next_rng)?;
        let mut cards = offer.cards.clone();
        cards[slot] = replacement;
        self.offer = Some(Offer {
            id: self.next_offer_id,
            victory: offer.victory.clone(),
            cards,
            rerolls_remaining: offer.rerolls_remaining,
        });
        self.embers -= BANISH_COST;
        if self.locked.as_ref().is_some_and(|l| l.power_id == card.power_id) {
            self.locked = None;
        }
        self.banished.insert(card.power_id);
        self.next_offer_id = next;
        self.rng = next_rng;
        Ok(self.offer.as_ref().expect("offer was assigned above"))
    }
    pub fn choices(&self) -> &[ChoiceRecord] {
        &self.choices
    }
    pub fn has_seen(&self, cycle: u32, boss_id: &str) -> bool {
        self.seen_bosses.contains(&RewardKey {
            cycle,
            boss_id: boss_id.into(),
        })
    }

    /// A baseline is deliberately not a reward. This distinguishes a queued or
    /// already chosen victory when applying an externally verified recovery.
    pub fn has_reward(&self, cycle: u32, boss_id: &str) -> bool {
        let key = RewardKey {
            cycle,
            boss_id: boss_id.into(),
        };
        self.pending.iter().any(|victory| victory.key == key)
            || self.choices.iter().any(|choice| choice.victory.key == key)
            || self.skipped.contains(&key)
    }

    /// First attachment and each forward NG+ transition need one complete,
    /// trustworthy boss-flag snapshot. Existing victories are baselined without
    /// rewards. Repeating baseline in the same cycle is rejected: it could erase
    /// a just-earned reward. Save rollback requires explicit reconciliation.
    pub fn baseline(&mut self, cycle: u32, already_defeated: &[String]) -> Result<()> {
        if already_defeated.len() > 10_000 || already_defeated.iter().any(|s| !valid_id(s)) {
            return Err(CoreError::Invalid("invalid boss baseline".into()));
        }
        if let Some(stored) = self.cycle {
            if cycle <= stored {
                return Err(CoreError::CycleMismatch {
                    stored,
                    observed: cycle,
                });
            }
        }
        self.cycle = Some(cycle);
        self.seen_bosses
            .extend(already_defeated.iter().map(|id| RewardKey {
                cycle,
                boss_id: id.clone(),
            }));
        Ok(())
    }

    /// Records a final encounter victory, not individual enemy deaths/phases.
    /// The adapter owns verified event-flag mapping. False positives cannot be
    /// corrected by this algorithm; never call it from a health==0 heuristic.
    pub fn observe_boss(
        &mut self,
        cycle: u32,
        boss_id: impl Into<String>,
        tier: BossTier,
    ) -> Result<bool> {
        self.observe_boss_with(cycle, boss_id, tier, 0)
    }

    /// `observe_boss` for a boss that rose Ascended with `ascended` affixes of
    /// its own (3.0): +1 Ember per affix now, a better offer later.
    pub fn observe_boss_with(
        &mut self,
        cycle: u32,
        boss_id: impl Into<String>,
        tier: BossTier,
        ascended: u8,
    ) -> Result<bool> {
        let ascended = ascended.min(3);
        let stored = self.cycle.ok_or(CoreError::NotBaselined)?;
        if cycle != stored {
            return Err(CoreError::CycleMismatch {
                stored,
                observed: cycle,
            });
        }
        let boss_id = boss_id.into();
        if !valid_id(&boss_id) {
            return Err(CoreError::Invalid("invalid boss id".into()));
        }
        let key = RewardKey { cycle, boss_id };
        if self.seen_bosses.contains(&key) {
            return Ok(false);
        }
        if self.pending.len() >= 10_000 {
            return Err(CoreError::Invalid(
                "pending reward safety limit exceeded".into(),
            ));
        }
        self.seen_bosses.insert(key.clone());
        self.pending.push_back(Victory { key, tier, ascended });
        self.embers = self
            .embers
            .saturating_add(tier_embers(tier) + u32::from(ascended))
            .min(MAX_EMBERS);
        Ok(true)
    }

    /// Restore one independently proven victory that was observed before a
    /// delayed first baseline. The adapter must authenticate both the evidence
    /// and the character's immutable identity; a defeated flag alone does not
    /// authorize this operation. Call inside Store::transaction.
    ///
    /// This never removes a baseline, changes a choice, or replaces an offer.
    /// The same boss+cycle in pending/choices is an idempotent no-op, including
    /// after a crash between durable commit and acknowledgement.
    pub fn recover_observed_victory(
        &mut self,
        cycle: u32,
        boss_id: impl Into<String>,
        tier: BossTier,
    ) -> Result<bool> {
        let stored = self.cycle.ok_or(CoreError::NotBaselined)?;
        if cycle != stored {
            return Err(CoreError::CycleMismatch {
                stored,
                observed: cycle,
            });
        }
        let boss_id = boss_id.into();
        if !valid_id(&boss_id) {
            return Err(CoreError::Invalid("invalid recovered boss id".into()));
        }
        if self.has_reward(cycle, &boss_id) {
            return Ok(false);
        }
        if self.pending.len() >= 10_000 {
            return Err(CoreError::Invalid(
                "pending reward safety limit exceeded".into(),
            ));
        }
        let key = RewardKey { cycle, boss_id };
        self.seen_bosses.insert(key.clone());
        self.pending.push_back(Victory { key, tier, ascended: 0 });
        Ok(true)
    }

    /// Prepare the first queued victory using current owned levels. Later
    /// victories are queued instead of generating stale duplicate upgrade cards.
    pub fn prepare_offer(&mut self, catalog: &Catalog, cfg: &DrawConfig) -> Result<Option<&Offer>> {
        if self.offer.is_some() {
            return Ok(self.offer.as_ref());
        }
        let Some(victory) = self.pending.front().cloned() else {
            return Ok(None);
        };
        let next = self
            .next_offer_id
            .checked_add(1)
            .ok_or_else(|| CoreError::Invalid("offer id overflow".into()))?;
        let mut next_rng = self.rng;
        let mut cfg = cfg.clone();
        cfg.pity = pity_factor(self.dry_offers);
        // 3.0 Ascended victory: rarer cards, one more from two affixes.
        cfg.luck = (cfg.luck + ascended_luck(&victory)).min(1.0);
        cfg.extra_cards += usize::from(victory.ascended >= 2);
        cfg.excluded.extend(self.banished.iter().cloned());
        self.hold_back_signatures(catalog, &mut cfg);
        let size = self.size_for(&cfg, victory.tier);
        let mut cards = draw::draw(catalog, &self.levels, victory.tier, &cfg, &mut next_rng, size)?;
        self.apply_guarantees(&mut cards, catalog, &cfg, &victory);
        if cards.iter().any(|card| card.display_rarity >= Rarity::Epic) {
            self.dry_offers = 0;
        } else {
            self.dry_offers = self.dry_offers.saturating_add(1).min(1000);
        }
        self.offer = Some(Offer {
            id: self.next_offer_id,
            victory,
            cards,
            rerolls_remaining: self.reroll_allowance(catalog, &cfg),
        });
        self.next_offer_id = next;
        self.rng = next_rng;
        Ok(self.offer.as_ref())
    }

    /// Returns the newly acquired level. Call inside Store::transaction and
    /// apply combat effects only after that transaction has durably committed.
    pub fn choose(&mut self, offer_id: u64, power_id: &str, catalog: &Catalog) -> Result<u8> {
        let offer = self.offer.as_ref().ok_or(CoreError::NoOffer)?;
        if offer.id != offer_id {
            return Err(CoreError::StaleOffer);
        }
        let card = offer
            .cards
            .iter()
            .find(|c| c.power_id == power_id)
            .ok_or(CoreError::StaleOffer)?;
        let p = catalog
            .get(power_id)
            .filter(|p| catalog.is_enabled(p))
            .ok_or_else(|| {
                CoreError::Invalid("selected power is no longer supported by this adapter".into())
            })?;
        if self.level(power_id) != card.previous_level
            || card.target_level != card.previous_level + 1
            || card.target_level > 5
            || p.base_rarity != card.base_rarity
            || p.rarity_at(card.target_level)? != card.display_rarity
        {
            return Err(CoreError::StaleOffer);
        }
        let level = card.target_level;
        let origin = offer.victory.key.clone();
        if self
            .locked
            .as_ref()
            .is_some_and(|l| l.power_id == power_id || l.origin != origin)
        {
            self.locked = None;
        }
        let offer = self.offer.as_ref().ok_or(CoreError::NoOffer)?;
        self.levels.insert(power_id.into(), level);
        self.choices.push(ChoiceRecord {
            offer_id,
            victory: offer.victory.clone(),
            power_id: power_id.into(),
            level,
        });
        self.pending.pop_front();
        self.offer = None;
        Ok(level)
    }

    /// The old offer ID becomes invalid. No new victory or power is generated.
    /// RNG, three replacements and consumed budget commit together on disk.
    pub fn reroll(&mut self, offer_id: u64, catalog: &Catalog, cfg: &DrawConfig) -> Result<&Offer> {
        let offer = self.offer.as_ref().ok_or(CoreError::NoOffer)?;
        if offer.id != offer_id {
            return Err(CoreError::StaleOffer);
        }
        // Free rerolls first; then embers pay for more.
        let paid = offer.rerolls_remaining == 0;
        if paid && self.embers < EMBER_REROLL_COST {
            return Err(CoreError::NotEnoughEmbers {
                have: self.embers,
                need: EMBER_REROLL_COST,
            });
        }
        let next = self
            .next_offer_id
            .checked_add(1)
            .ok_or_else(|| CoreError::Invalid("offer id overflow".into()))?;
        let mut next_rng = self.rng;
        let mut cfg = cfg.clone();
        cfg.pity = pity_factor(self.dry_offers);
        cfg.luck = (cfg.luck + ascended_luck(&offer.victory)).min(1.0);
        cfg.excluded.extend(self.banished.iter().cloned());
        self.hold_back_signatures(catalog, &mut cfg);
        let mut cards = draw::draw(
            catalog,
            &self.levels,
            offer.victory.tier,
            &cfg,
            &mut next_rng,
            offer.cards.len(),
        )?;
        let victory = offer.victory.clone();
        self.apply_guarantees(&mut cards, catalog, &cfg, &victory);
        self.offer = Some(Offer {
            id: self.next_offer_id,
            victory: offer.victory.clone(),
            cards,
            rerolls_remaining: offer.rerolls_remaining.saturating_sub(1),
        });
        if paid {
            self.embers -= EMBER_REROLL_COST;
        }
        self.next_offer_id = next;
        self.rng = next_rng;
        Ok(self.offer.as_ref().expect("offer was assigned above"))
    }

    pub fn validate(&self) -> Result<()> {
        if !valid_id(&self.character_id)
            || self.embers > MAX_EMBERS
            || self.banished.len() > 10_000
            || self.banished.iter().any(|id| !valid_id(id))
            || self.rune_debt > MAX_RUNE_DEBT
            || self.skipped.len() > 1_000_000

            || self
                .locked
                .as_ref()
                .is_some_and(|l| !valid_id(&l.power_id) || !self.seen_bosses.contains(&l.origin))
            || self.next_offer_id == 0
            || self.levels.len() > 10_000
            || self.seen_bosses.len() > 1_000_000
            || self.pending.len() > 10_000
            || self.choices.len() > 50_000
        {
            return Err(CoreError::Corrupt(
                "invalid state dimensions/identity".into(),
            ));
        }
        if self
            .levels
            .iter()
            .any(|(id, l)| !valid_id(id) || !(1..=5).contains(l))
        {
            return Err(CoreError::Corrupt("invalid owned power level".into()));
        }
        let mut pending_keys = BTreeSet::new();
        let mut chosen_keys = BTreeSet::new();
        let mut chosen_ids = BTreeSet::new();
        let mut reconstructed = BTreeMap::<String, u8>::new();
        for key in &self.seen_bosses {
            if !valid_id(&key.boss_id) || self.cycle.is_none_or(|c| key.cycle > c) {
                return Err(CoreError::Corrupt(
                    "boss history has an invalid cycle/id".into(),
                ));
            }
        }
        for victory in &self.pending {
            if !self.seen_bosses.contains(&victory.key) || !pending_keys.insert(&victory.key) {
                return Err(CoreError::Corrupt(
                    "pending reward is duplicated or not recorded".into(),
                ));
            }
        }
        for choice in &self.choices {
            let prev = reconstructed.get(&choice.power_id).copied().unwrap_or(0);
            if !valid_id(&choice.power_id)
                || choice.level != prev + 1
                || choice.level > 5
                || choice.offer_id == 0
                || choice.offer_id >= self.next_offer_id
                || !chosen_ids.insert(choice.offer_id)
                || !chosen_keys.insert(&choice.victory.key)
                || pending_keys.contains(&choice.victory.key)
                || !self.seen_bosses.contains(&choice.victory.key)
            {
                return Err(CoreError::Corrupt(
                    "reward choice history is inconsistent".into(),
                ));
            }
            reconstructed.insert(choice.power_id.clone(), choice.level);
        }
        for key in &self.skipped {
            if !self.seen_bosses.contains(key) || pending_keys.contains(key) || chosen_keys.contains(key) {
                return Err(CoreError::Corrupt("skipped reward history is inconsistent".into()));
            }
        }
        if reconstructed != self.levels {
            return Err(CoreError::Corrupt(
                "owned powers do not match committed choices".into(),
            ));
        }
        if let Some(offer) = &self.offer {
            if offer.id == 0
                || offer.id >= self.next_offer_id
                || chosen_ids.contains(&offer.id)
                || self.pending.front() != Some(&offer.victory)
                || !(3..=4).contains(&offer.cards.len())
                || offer.rerolls_remaining > 10
            {
                return Err(CoreError::Corrupt("prepared reward is inconsistent".into()));
            }
            let mut ids = BTreeMap::<&str, &Card>::new();
            for c in &offer.cards {
                if !valid_id(&c.power_id)
                    || c.previous_level > 4
                    || c.target_level != c.previous_level + 1
                    || self.level(&c.power_id) != c.previous_level
                    || c.display_rarity < c.base_rarity
                {
                    return Err(CoreError::Corrupt("prepared cards are inconsistent".into()));
                }
                if let Some(previous) = ids.insert(c.power_id.as_str(), c) {
                    if previous != c {
                        return Err(CoreError::Corrupt(
                            "repeated final upgrades disagree".into(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}
