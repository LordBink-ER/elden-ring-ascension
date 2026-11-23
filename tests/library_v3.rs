//! Offline checks of the 0.3 data: the shipped library validates, every proc
//! card names a catalogued proc, and journals written before 0.3 still open.
use ascension::core::{BossTier, Progression, Store};
use ascension::powers::PowerLibrary;
use std::collections::BTreeSet;

#[test]
fn shipped_library_validates_and_procs_exist() {
    let library = PowerLibrary::shipped().expect("powers.json validates");
    let procs: serde_json::Value =
        serde_json::from_str(include_str!("../config/procs.json")).unwrap();
    let ids: BTreeSet<String> = procs["procs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap().to_owned())
        .collect();
    let mut proc_cards = 0;
    for power in &library.powers {
        for level in &power.levels {
            if let Some(spec) = &level.proc {
                assert!(ids.contains(&spec.proc), "{} uses unknown proc {}", power.id, spec.proc);
                proc_cards += 1;
            }
        }
    }
    assert!(proc_cards >= 5 * 15, "expected at least 15 proc cards");
    let all: BTreeSet<String> = library.powers.iter().map(|p| p.id.clone()).collect();
    for old in ["amber_vessel", "steady_pulse", "rune_echo", "fortunes_favor", "perfect_reversal"] {
        assert!(all.contains(old), "0.2 power {old} must remain");
    }
    assert!(library.powers.len() >= 90);
    let book = ascension::synergies::SynergyBook::shipped(&library).expect("synergies validate");
    assert!(book.synergies.len() >= 15);
}

#[test]
fn new_state_field_is_omitted_while_zero() {
    let state = Progression::new("fixture-character", 7).unwrap();
    let text = serde_json::to_string(&state).unwrap();
    assert!(!text.contains("dry_offers"), "pre-0.3 journals must hash identically");
}

#[test]
fn pity_raises_rare_odds_only_after_dry_offers() {
    assert_eq!(ascension::core::pity_factor(0), 1.0);
    assert_eq!(ascension::core::pity_factor(3), 1.0);
    assert!(ascension::core::pity_factor(4) > 1.0);
    assert!(ascension::core::pity_factor(100) <= 3.0);
    let _ = BossTier::Minor;
}

/// ASCENSION_STATE_COPY=<copy of state/progression>: every character journal
/// in it must open (checksums unchanged by the 0.3 schema additions).
#[test]
fn existing_journals_still_open() {
    let Ok(root) = std::env::var("ASCENSION_STATE_COPY") else {
        return;
    };
    let mut opened = 0;
    for entry in std::fs::read_dir(&root).unwrap() {
        let dir = entry.unwrap().path();
        let mut latest = None;
        for file in std::fs::read_dir(&dir).unwrap() {
            let file = file.unwrap().path();
            if file.extension().is_some_and(|e| e == "json") {
                latest = latest.max(Some(file));
            }
        }
        let Some(latest) = latest else { continue };
        let envelope: serde_json::Value =
            serde_json::from_slice(&std::fs::read(latest).unwrap()).unwrap();
        let id = envelope["state"]["character_id"].as_str().unwrap().to_owned();
        let store = Store::open(std::path::Path::new(&root), &id).expect("journal opens");
        assert!(!store.state().levels().is_empty());
        opened += 1;
    }
    assert!(opened > 0);
}
