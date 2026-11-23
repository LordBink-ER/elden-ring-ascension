use ascension::core::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

fn catalog() -> Catalog {
    let rarities = Rarity::ALL;
    Catalog::new(
        (0..12)
            .map(|n| {
                let base = rarities[n % 6];
                PowerDef::new(
                    format!("fixture_{n}"),
                    format!("Test fixture {n}"),
                    "test",
                    base,
                    [base; 5],
                )
            })
            .collect(),
        BTreeSet::new(),
    )
    .unwrap()
}
fn initialized(seed: u64) -> Progression {
    let mut s =
        Progression::new("verified-account:isolated-save:slot0:creation-123", seed).unwrap();
    s.baseline(0, &["preexisting_boss".into()]).unwrap();
    s
}
fn enqueue(s: &mut Progression, id: &str) {
    s.observe_boss(s.cycle().unwrap(), id, BossTier::Minor)
        .unwrap();
}

#[test]
fn baseline_and_phase_duplicates_do_not_mint_rewards() {
    let mut s = Progression::new("fixture", 1).unwrap();
    assert!(matches!(
        s.observe_boss(0, "a", BossTier::Minor),
        Err(CoreError::NotBaselined)
    ));
    s.baseline(0, &["old".into()]).unwrap();
    assert!(!s.observe_boss(0, "old", BossTier::Minor).unwrap());
    for _ in 0..20 {
        s.observe_boss(0, "new", BossTier::Remembrance).unwrap();
    }
    assert_eq!(s.pending_count(), 1);
    assert!(s.baseline(0, &[]).is_err());
    assert!(s.observe_boss(1, "new", BossTier::Minor).is_err());
    s.validate().unwrap();
}

#[test]
fn exactly_three_unique_cards_and_exactly_one_choice_survive_reload() {
    let c = catalog();
    let cfg = DrawConfig::default();
    let mut s = initialized(7);
    enqueue(&mut s, "one");
    enqueue(&mut s, "two");
    let offer = s.prepare_offer(&c, &cfg).unwrap().unwrap().clone();
    assert_eq!(offer.cards.len(), 3);
    assert_eq!(
        offer
            .cards
            .iter()
            .map(|p| &p.power_id)
            .collect::<BTreeSet<_>>()
            .len(),
        3
    );
    let mut loaded: Progression = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
    assert_eq!(loaded.prepare_offer(&c, &cfg).unwrap().unwrap(), &offer);
    let selected = &offer.cards[0].power_id;
    assert_eq!(loaded.choose(offer.id, selected, &c).unwrap(), 1);
    assert!(loaded.choose(offer.id, selected, &c).is_err());
    assert_eq!(loaded.pending_count(), 1);
    assert_eq!(loaded.levels().values().copied().sum::<u8>(), 1);
    let next = loaded.prepare_offer(&c, &cfg).unwrap().unwrap();
    for card in &next.cards {
        if &card.power_id == selected {
            assert_eq!(card.previous_level, 1);
            assert_eq!(card.target_level, 2);
        }
    }
    loaded.validate().unwrap();
}

#[test]
fn deterministic_rng_is_persisted_not_reseeded_on_menu_reopen() {
    let c = catalog();
    let cfg = DrawConfig::default();
    let mut a = initialized(1234);
    let mut b = initialized(1234);
    for n in 0..30 {
        for state in [&mut a, &mut b] {
            enqueue(state, &format!("boss_{n}"));
            state.prepare_offer(&c, &cfg).unwrap();
        }
        assert_eq!(a.current_offer(), b.current_offer());
        let offer = a.current_offer().unwrap().clone();
        a.choose(offer.id, &offer.cards[0].power_id, &c).unwrap();
        b.choose(offer.id, &offer.cards[0].power_id, &c).unwrap();
        b = serde_json::from_slice(&serde_json::to_vec(&b).unwrap()).unwrap();
    }
    assert_eq!(a, b);
}

#[test]
fn level_v_is_possible_in_one_cycle_and_removed_from_draws() {
    let c = Catalog::new(
        (0..4)
            .map(|n| {
                PowerDef::new(
                    format!("p{n}"),
                    "fixture",
                    "test",
                    Rarity::Common,
                    [
                        Rarity::Common,
                        Rarity::Uncommon,
                        Rarity::Rare,
                        Rarity::Epic,
                        Rarity::Legendary,
                    ],
                )
            })
            .collect(),
        BTreeSet::new(),
    )
    .unwrap();
    let cfg = DrawConfig::default();
    let mut s = initialized(15);
    for n in 0..200 {
        enqueue(&mut s, &format!("boss_{n}"));
        let Some(offer) = s.prepare_offer(&c, &cfg).ok().flatten().cloned() else {
            break;
        };
        let pick = offer
            .cards
            .iter()
            .find(|p| p.power_id == "p0")
            .unwrap_or(&offer.cards[0]);
        assert_eq!(pick.base_rarity, Rarity::Common);
        if pick.target_level == 5 {
            assert_eq!(pick.display_rarity, Rarity::Legendary);
        }
        s.choose(offer.id, &pick.power_id, &c).unwrap();
        if s.level("p0") == 5 {
            break;
        }
    }
    assert_eq!(s.cycle(), Some(0));
    assert_eq!(s.level("p0"), 5);
    enqueue(&mut s, "after_v");
    let offer = s.prepare_offer(&c, &cfg).unwrap().unwrap();
    assert!(offer.cards.iter().all(|p| p.power_id != "p0"));
    s.validate().unwrap();
}

#[test]
fn ng_plus_retains_powers_but_each_boss_can_reward_again() {
    let c = catalog();
    let cfg = DrawConfig::default();
    let mut s = initialized(56);
    enqueue(&mut s, "boss");
    let offer = s.prepare_offer(&c, &cfg).unwrap().unwrap().clone();
    s.choose(offer.id, &offer.cards[0].power_id, &c).unwrap();
    let owned = s.levels().clone();
    s.baseline(1, &[]).unwrap();
    assert_eq!(s.levels(), &owned);
    assert!(s.observe_boss(1, "boss", BossTier::Minor).unwrap());
    assert!(!s.observe_boss(1, "boss", BossTier::Minor).unwrap());
    assert!(s.baseline(0, &[]).is_err());
    s.validate().unwrap();
}

#[test]
fn unsupported_and_exhausted_pools_keep_victory_queued() {
    let mut p = PowerDef::new(
        "blocked",
        "fixture",
        "test",
        Rarity::Mythic,
        [Rarity::Mythic; 5],
    );
    p.required_capabilities.insert("not_implemented".into());
    let c = Catalog::new(vec![p], BTreeSet::new()).unwrap();
    let mut s = initialized(1);
    enqueue(&mut s, "victory");
    let before = s.clone();
    assert!(matches!(
        s.prepare_offer(&c, &DrawConfig::default()),
        Err(CoreError::PoolExhausted { available: 0 })
    ));
    assert_eq!(s, before);
    assert_eq!(s.pending_count(), 1);
    assert!(s.current_offer().is_none());
}

#[test]
fn rerolls_are_bounded_and_reject_old_ui_requests() {
    let mut defs: Vec<_> = (0..3)
        .map(|n| {
            PowerDef::new(
                format!("p{n}"),
                "fixture",
                "test",
                Rarity::Common,
                [Rarity::Common; 5],
            )
        })
        .collect();
    defs[0].meta = Some(MetaEffect::Rerolls { per_level: 1 });
    let c = Catalog::new(defs, BTreeSet::new()).unwrap();
    let cfg = DrawConfig::default();
    let mut s = initialized(14);
    enqueue(&mut s, "first");
    let id = s.prepare_offer(&c, &cfg).unwrap().unwrap().id;
    // 0.4: past the free rerolls a reroll costs embers (one minor boss = 1).
    assert!(matches!(
        s.reroll(id, &c, &cfg),
        Err(CoreError::NotEnoughEmbers { have: 1, need: 3 })
    ));
    s.choose(id, "p0", &c).unwrap();
    enqueue(&mut s, "second");
    let id = s.prepare_offer(&c, &cfg).unwrap().unwrap().id;
    assert_eq!(s.current_offer().unwrap().rerolls_remaining, 1);
    let updated = s.reroll(id, &c, &cfg).unwrap().clone();
    assert_ne!(updated.id, id);
    assert_eq!(updated.rerolls_remaining, 0);
    assert!(matches!(s.choose(id, "p0", &c), Err(CoreError::StaleOffer)));
    assert!(matches!(
        s.reroll(updated.id, &c, &cfg),
        Err(CoreError::NotEnoughEmbers { have: 2, need: 3 })
    ));
    assert_eq!(s.levels().get("p0"), Some(&1));
    s.validate().unwrap();
}

#[test]
fn rarity_distribution_matches_first_slot_and_minor_retains_mythic() {
    let c = Catalog::new(
        Rarity::ALL
            .into_iter()
            .map(|r| PowerDef::new(format!("r{}", r.index()), "fixture", "test", r, [r; 5]))
            .collect(),
        BTreeSet::new(),
    )
    .unwrap();
    let cfg = DrawConfig::default();
    for tier in [BossTier::Minor, BossTier::Remembrance] {
        let mut counts = [0u32; 6];
        for seed in 0..30_000u64 {
            let mut s = initialized(seed);
            s.observe_boss(0, "boss", tier).unwrap();
            let rarity = s.prepare_offer(&c, &cfg).unwrap().unwrap().cards[0].base_rarity;
            counts[rarity.index()] += 1;
        }
        let total: f64 = cfg.tier_weights[tier.index()].iter().sum();
        for r in Rarity::ALL {
            let expected = cfg.tier_weights[tier.index()][r.index()] / total;
            let actual = counts[r.index()] as f64 / 30_000.0;
            assert!(
                (actual - expected).abs() < 0.012,
                "{tier:?} {r:?}: {actual} vs {expected}"
            );
        }
        assert!(counts[Rarity::Mythic.index()] > 0);
    }
}

#[test]
fn invalid_probabilities_and_duplicate_definitions_are_rejected() {
    let mut cfg = DrawConfig::default();
    cfg.repeat_weights[2] = cfg.repeat_weights[1];
    assert!(cfg.validate().is_err());
    cfg = DrawConfig::default();
    cfg.tier_weights[0][5] = 0.0;
    assert!(cfg.validate().is_err());
    let p = PowerDef::new("x", "fixture", "test", Rarity::Common, [Rarity::Common; 5]);
    assert!(Catalog::new(vec![p.clone(), p], BTreeSet::new()).is_err());
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ascension-core-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn journal_reloads_pending_offer_and_prevents_parallel_writers() {
    let temp = Temp::new();
    let c = catalog();
    let cfg = DrawConfig::default();
    let s = initialized(42);
    let identity = s.character_id().to_owned();
    let mut store = Store::create(&temp.0, s).unwrap();
    assert!(matches!(
        Store::open(&temp.0, &identity),
        Err(CoreError::Locked)
    ));
    store
        .transaction(|s| {
            enqueue(s, "boss");
            s.prepare_offer(&c, &cfg)?;
            Ok(())
        })
        .unwrap();
    let expected = store.state().clone();
    let offer = expected.current_offer().unwrap().clone();
    std::fs::write(
        store.directory().join("uncommitted.tmp"),
        b"interrupted write",
    )
    .unwrap();
    drop(store);
    let mut loaded = Store::open(&temp.0, &identity).unwrap();
    assert_eq!(loaded.state(), &expected);
    loaded
        .transaction(|s| {
            s.choose(offer.id, &offer.cards[0].power_id, &c)?;
            Ok(())
        })
        .unwrap();
    drop(loaded);
    let loaded = Store::open(&temp.0, &identity).unwrap();
    assert_eq!(loaded.state().choices().len(), 1);
    assert_eq!(loaded.state().pending_count(), 0);
}

#[test]
fn corruption_never_resets_or_silently_rolls_back_progression() {
    let temp = Temp::new();
    let mut store = Store::create(&temp.0, initialized(8)).unwrap();
    let identity = store.state().character_id().to_owned();
    store
        .transaction(|s| s.observe_boss(0, "new", BossTier::Minor))
        .unwrap();
    let latest = store
        .directory()
        .join(format!("{:020}.json", store.generation()));
    drop(store);
    std::fs::write(latest, b"{ interrupted/corrupt }").unwrap();
    assert!(matches!(
        Store::open(&temp.0, &identity),
        Err(CoreError::Corrupt(_))
    ));
    assert!(matches!(
        Store::create(&temp.0, initialized(9)),
        Err(CoreError::AlreadyExists)
    ));
}

#[test]
fn syntactically_valid_tampering_is_detected_by_checksum() {
    let temp = Temp::new();
    let store = Store::create(&temp.0, initialized(8)).unwrap();
    let identity = store.state().character_id().to_owned();
    let latest = store
        .directory()
        .join(format!("{:020}.json", store.generation()));
    drop(store);
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&latest).unwrap()).unwrap();
    json["state"]["rng"] = serde_json::json!(999);
    std::fs::write(latest, serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(matches!(
        Store::open(&temp.0, &identity),
        Err(CoreError::Corrupt(_))
    ));
}

#[test]
fn journal_retention_is_bounded_and_noops_do_not_write() {
    let temp = Temp::new();
    let mut store = Store::create(&temp.0, initialized(8)).unwrap();
    for n in 0..30 {
        store
            .transaction(|s| s.observe_boss(0, format!("boss_{n}"), BossTier::Minor))
            .unwrap();
    }
    let generation = store.generation();
    store
        .transaction(|s| s.observe_boss(0, "boss_0", BossTier::Minor))
        .unwrap();
    assert_eq!(store.generation(), generation);
    let count = std::fs::read_dir(store.directory())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|e| e == "json"))
        .count();
    assert_eq!(count, 16);
    let identity = store.state().character_id().to_owned();
    drop(store);
    assert_eq!(
        Store::open(&temp.0, &identity)
            .unwrap()
            .state()
            .pending_count(),
        30
    );
}

#[test]
fn failed_transactions_do_not_change_memory_or_disk_and_characters_are_isolated() {
    let temp = Temp::new();
    let mut store = Store::create(&temp.0, initialized(8)).unwrap();
    let before = store.state().clone();
    let generation = store.generation();
    let failure: Result<()> = store.transaction(|s| {
        s.observe_boss(0, "new", BossTier::Minor)?;
        Err(CoreError::Invalid("injected operation failure".into()))
    });
    assert!(failure.is_err());
    assert_eq!(store.state(), &before);
    assert_eq!(store.generation(), generation);
    let mut other = Progression::new("different-immutable-character", 8).unwrap();
    other.baseline(0, &[]).unwrap();
    let other = Store::create(&temp.0, other).unwrap();
    assert_ne!(store.directory(), other.directory());
    assert_eq!(other.state().levels(), &BTreeMap::new());
}

#[test]
fn disk_write_failure_does_not_adopt_a_pending_victory() {
    let temp = Temp::new();
    let mut store = Store::create(&temp.0, initialized(8)).unwrap();
    let before = store.state().clone();
    let original = store.directory().to_owned();
    let moved = temp.0.join("temporarily-moved");
    std::fs::rename(&original, &moved).unwrap();
    assert!(
        store
            .transaction(|s| s.observe_boss(0, "new", BossTier::Minor))
            .is_err()
    );
    assert_eq!(store.state(), &before);
    std::fs::rename(moved, original).unwrap();
}
