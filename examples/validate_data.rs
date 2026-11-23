//! Build step: load the shipped data with the DLL's own validation, so a bad
//! library fails the build instead of stopping the mod at game start.
//! Usage: cargo run --example validate_data -- <project root>
use ascension::{config::Config, powers::PowerLibrary, synergies::SynergyBook};
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    let check = || -> Result<String, String> {
        let library = PowerLibrary::from_json(
            &std::fs::read_to_string(root.join("config/powers.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("config/powers.json: {e}"))?;
        let book = SynergyBook::from_json(
            &std::fs::read_to_string(root.join("config/synergies.json")).map_err(|e| e.to_string())?,
            &library,
        )
        .map_err(|e| format!("config/synergies.json: {e}"))?;
        let (config, bosses, _) = Config::load(&root).map_err(|e| format!("config: {e}"))?;
        library
            .catalog(Default::default())
            .map_err(|e| format!("catalog: {e}"))?;
        let procs: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join("config/procs.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let ids: Vec<&str> = procs["procs"]
            .as_array()
            .ok_or("procs.json has no procs")?
            .iter()
            .filter_map(|p| p["id"].as_str())
            .collect();
        for power in &library.powers {
            for level in &power.levels {
                if let Some(proc) = &level.proc {
                    if !ids.contains(&proc.proc.as_str()) {
                        return Err(format!("{} uses unknown proc {}", power.id, proc.proc));
                    }
                }
            }
        }
        // Every capability a card needs must be one the DLL can advertise
        // (effects.rs capabilities()), or the card could never activate.
        const ADVERTISED: &[&str] = &[
            "meta_draw", "sampled_resources", "boss_combat", "action_flags", "elapsed_time",
            "sp_effect_params", "periodic_resources", "native_on_kill", "equip_load_snapshot",
            "weapon_status_impart", "multi_status_channels", "native_timed_on_kill",
            "native_weighted_momentum", "native_nearby_affliction", "native_critical_blessing",
            "event.hit_landed", "event.kill", "event.elite_kill", "event.perfect_dodge",
            "event.block", "event.guard_counter", "event.critical_hit", "event.poise_break",
            "event.status_proc", "event.parry", "event.backstab", "native_procs", "native_intervention",
            "native_blood_price", "native_item_speed", "native_attack_speed", "native_lifesteal",
            "native_arts", "native_allies", "event.execution", "event.rampage", "native_downed_strike",
        ];

        for power in &library.powers {
            if let Some(missing) = power.required().iter().find(|c| !ADVERTISED.contains(&c.as_str())) {
                return Err(format!("{} needs capability {missing}, which the DLL never advertises", power.id));
            }
        }
        // 2.0: signature cards name real bosses; Path bonuses name real
        // hidden cards; every visible card belongs to a Path.
        let mut signatures = 0;
        for power in &library.powers {
            for boss in &power.signature {
                if !bosses.iter().any(|b| &b.id == boss) {
                    return Err(format!("{} names unknown signature boss {boss}", power.id));
                }
            }
            if !power.signature.is_empty() {
                signatures += 1;
            }
            if !power.hidden && power.family.is_none() {
                return Err(format!("{} belongs to no Path", power.id));
            }
        }
        for path in ascension::paths::PATHS {
            for bonus in path.bonuses() {
                if !bonus.power.is_empty() && !library.powers.iter().any(|p| p.id == bonus.power && p.hidden) {
                    return Err(format!("Path bonus {} needs hidden power {}", bonus.name, bonus.power));
                }
            }
        }
        // 2.2 audit: every card level can switch on, does something, and no
        // value of it is lost to a trigger that ignores it or to a stacking cap.
        let audited = audit(&library, &book)?;
        attribute_gains()?;
        config.draw.validate().map_err(|e| e.to_string())?;
        // 0.7: every card has a picture in the atlases (tools/build_card_art.py).
        let art = ascension::art::ArtPack::load(&root).map_err(|e| format!("card art: {e}"))?;
        if let Some(missing) = library.powers.iter().find(|p| !p.hidden && !art.cards.contains_key(&p.id)) {
            return Err(format!("{} has no card art; run tools/build_card_art.py", missing.id));
        }
        Ok(format!(
            "{} powers ({signatures} signature), {} synergies, {} bosses, {} procs, {} card pictures, {audited}: valid",
            library.powers.len(),
            book.synergies.len(),
            bosses.len(),
            ids.len(),
            art.cards.len()
        ))

    };
    match check() {
        Ok(message) => println!("{message}"),
        Err(error) => {
            eprintln!("DATA INVALID: {error}");
            std::process::exit(1);
        }
    }
}

/// Every combat situation worth asking about, as permissive as the game allows.
fn scenarios(paths: [u8; 9]) -> Vec<ascension::powers::CombatSnapshot> {
    use ascension::powers::{AttributeValues, CombatEvent as E, CombatSnapshot, RecentEvent};
    let events = [
        E::HitLanded, E::Kill, E::PerfectDodge, E::StatusProc, E::CriticalHit, E::GuardCounter,
        E::PoiseBreak, E::EliteKill, E::Block, E::BleedProc, E::FrostbiteProc, E::PoisonProc,
        E::SleepProc, E::MadnessProc, E::Parry, E::Backstab, E::Execution, E::Rampage,
    ];
    let mut out = Vec::new();
    for hp in [0.05_f32, 0.2, 0.35, 0.5, 0.7, 0.9, 1.0] {
        for fp in [0.05_f32, 0.3, 0.6, 1.0] {
            for stamina in [0.05_f32, 0.3, 0.6, 1.0] {
                for flags in 0..32u8 {
                    let (dodging, guarding) = (flags & 1 != 0, flags & 2 != 0);
                    if dodging && guarding {
                        continue;
                    }
                    let (airborne, boss, heavy) = (flags & 4 != 0, flags & 8 != 0, flags & 16 != 0);
                    for hurt in [true, false] {
                        for dodged in [true, false] {
                            out.push(CombatSnapshot {
                                in_game: true,
                                hp_ratio: hp,
                                fp_ratio: fp,
                                stamina_ratio: stamina,
                                equip_load_ratio: Some(if heavy { 0.9 } else { 0.2 }),
                                seconds_since_hp_loss: Some(if hurt { 0.5 } else { 60.0 }),
                                boss_fight_active: boss,
                                dodging,
                                guarding,
                                airborne,
                                heavy_load: heavy,
                                seconds_since_dodge_end: dodged.then_some(0.3),
                                consecutive_hits: 50,
                                seconds_in_combat: 60.0,
                                attributes: AttributeValues {
                                    vigor: 99, mind: 99, endurance: 99, strength: 99,
                                    dexterity: 99, intelligence: 99, faith: 99, arcane: 99,
                                },
                                events: events
                                    .iter()
                                    .map(|e| (*e, RecentEvent { age_seconds: 0.2, stacks: 50 }))
                                    .collect(),
                                paths,
                                tide: 10,
                                allies_near: 1,
                            });
                        }
                    }
                }
            }
        }
    }
    out
}

/// What can never work is an error; the summary counts what was checked.
fn audit(
    library: &ascension::powers::PowerLibrary,
    book: &ascension::synergies::SynergyBook,
) -> Result<String, String> {
    use ascension::powers::{EffectField as F, EffectTrigger as T};
    // The most points each Path can reach: every visible card of it at V.
    let mut most = [0u8; 9];
    for power in library.powers.iter().filter(|p| !p.hidden) {
        if let Some(path) = power.family {
            most[path.index()] = most[path.index()].saturating_add(5).min(99);
        }
    }
    let world = scenarios(most);
    let mut problems = Vec::new();
    let mut levels_checked = 0;
    for power in &library.powers {
        // Only these fields reach the special-trigger engines (effects.rs).
        let allowed: Option<&[F]> = match power.trigger {
            T::Passive => None,
            T::OnKill => Some(&[F::ChangeHpRate, F::ChangeHpPoint, F::ChangeMpRate, F::ChangeMpPoint]),
            T::OnKillTimed => Some(&[F::SoulRate]),
            T::OnMomentum => Some(&[F::StaminaRecoverChangeSpeed, F::ConsumeStaminaRate]),
            T::OnNearbyAffliction => Some(&[F::MagicAttackRate, F::MagicConsumptionRate, F::MiracleConsumptionRate]),
            T::OnCriticalBlessing => Some(&[F::SaAttackPowerRate]),
        };
        for (index, level) in power.levels.iter().enumerate() {
            levels_checked += 1;
            let name = format!("{} {}", power.id, ["I", "II", "III", "IV", "V"][index]);
            if level.description.trim().is_empty() {
                problems.push(format!("{name}: no text"));
            }
            if !world.iter().any(|s| level.activation.active(s)) {
                problems.push(format!("{name}: its condition can never be met"));
            }
            let mut does_something = power.meta.is_some()
                || level.proc.is_some()
                || level.intervention.is_some()
                || level.elite_lure.is_some()
                || level.blood_price.is_some()
                || level.item_speed.is_some_and(|v| v > 0.0)
                || level.downed_strike.is_some_and(|d| d.window > 0.0)
                || level.arts().iter().any(|v| v.is_some_and(|v| v > 0.0));
            for m in &level.modifiers {
                let neutral = (m.value - m.field.neutral()).abs() < 1e-6;
                if !neutral && m.field != F::MotionInterval {
                    does_something = true;
                }
                if neutral {
                    continue;
                }
                if let Some(allowed) = allowed {
                    if !allowed.contains(&m.field) {
                        problems.push(format!("{name}: {:?} is ignored by its {:?} trigger", m.field, power.trigger));
                    }
                }
                let (lo, hi) = m.field.bounds();
                if m.field != F::MotionInterval && (m.value < lo - 1e-4 || m.value > hi + 1e-4) {
                    problems.push(format!("{name}: {:?} {} is cut to {lo}..{hi}", m.field, m.value));
                }
            }
            if !does_something {
                problems.push(format!("{name}: does nothing"));
            }
        }
        for (parent, needed) in &power.requires {
            match library.get(parent) {
                Some(p) if p.hidden => problems.push(format!("{} needs hidden card {parent}", power.id)),
                Some(_) if !(1..=5).contains(needed) => problems.push(format!("{} needs {parent} at level {needed}", power.id)),
                Some(_) => {}
                None => problems.push(format!("{} needs unknown card {parent}", power.id)),
            }
        }
    }
    for path in ascension::paths::PATHS {
        for bonus in path.bonuses() {
            if bonus.points > most[path.index()] {
                problems.push(format!("Path bonus {} needs {} points; {} can reach {}", bonus.name, bonus.points, path.name(), most[path.index()]));
            }
        }
    }
    for synergy in &book.synergies {
        if let Some(activation) = &synergy.activation {
            if !world.iter().any(|s| activation.active(s)) {
                problems.push(format!("synergy {}: its condition can never be met", synergy.id));
            }
        }
        if synergy.modifiers.iter().all(|m| (m.value - m.field.neutral()).abs() < 1e-6) && synergy.proc_power.is_empty() {
            problems.push(format!("synergy {}: does nothing", synergy.id));
        }
        for id in synergy.all.iter().map(|(id, _)| id).chain(synergy.any_of.iter().flat_map(|a| a.ids.iter())) {
            if library.get(id).is_none_or(|p| p.hidden) {
                problems.push(format!("synergy {} needs a card nobody can own: {id}", synergy.id));
            }
        }
    }
    // 3.1: every name a player reads is unique (cards, synergies, Path
    // bonuses); a hidden Path bonus card shares its bonus's name on purpose.
    let mut seen = std::collections::BTreeMap::<String, String>::new();
    let mut claim = |name: &str, what: String, problems: &mut Vec<String>| {
        if let Some(first) = seen.insert(name.to_lowercase(), what.clone()) {
            problems.push(format!("the name \"{name}\" is used twice: {first} and {what}"));
        }
    };
    for power in library.powers.iter().filter(|p| !p.hidden) {
        claim(&power.name, format!("card {}", power.id), &mut problems);
    }
    for synergy in &book.synergies {
        claim(&synergy.name, format!("synergy {}", synergy.id), &mut problems);
    }
    for path in ascension::paths::PATHS {
        for bonus in path.bonuses() {
            claim(bonus.name, format!("{} bonus", path.name()), &mut problems);
        }
    }
    if problems.is_empty() {
        Ok(format!("{levels_checked} card levels audited"))
    } else {
        Err(format!("audit found {} problems:\n  {}", problems.len(), problems.join("\n  ")))
    }
}

/// 2.2: the Level Up panel shows what each new point adds. Every attribute
/// must show a gain on at least one of its lines for one more point, at any
/// level from 1 to 98 (the curves have no hard cap).
fn attribute_gains() -> Result<(), String> {
    use ascension::attributes::{Part, format_gain, rows, same_shape};
    use ascension::powers::AttributeValues;
    for points in 1..=98u32 {
        let at = |p: u32| AttributeValues {
            vigor: p, mind: p, endurance: p, strength: p, dexterity: p, intelligence: p, faith: p, arcane: p,
        };
        let (before, after) = (rows(at(points), 1.0), rows(at(points + 1), 1.0));
        for (b, a) in before.iter().zip(&after) {
            let shows = b.lines.iter().zip(&a.lines).any(|(lb, la)| {
                !same_shape(lb, la)
                    || lb.iter().zip(la).any(|(x, y)| match (x, y) {
                        (Part::Num { value: v, unit, sign }, Part::Num { value: w, .. }) => format_gain(*v, *w, *unit, *sign).is_some(),
                        _ => false,
                    })
            });
            if !shows {
                return Err(format!("{} {points} -> {}: the Level Up panel would show no gain", b.name, points + 1));
            }
        }
    }
    Ok(())
}
