//! 0.6 "Reflexes": combat events read from the world scan each tick (10 Hz).
//!
//! - Perfect dodge: during a dodge's invulnerability (or its first 0.45 s) an
//!   enemy attack aimed at the player was active within reach, or the player's
//!   last-received attack changed, and the dodge ended without HP loss.
//! - Block: while guarding, an attack landed on the guard (the received attack
//!   changed, or stamina dropped with an attacker in reach) without a big HP loss.
//! - Guard counter: the player's next confirmed hit within 1.5 s of a block.
//! - Critical: the player's throw node entered the attacker state (riposte,
//!   backstab and other critical throws).
//! - Stance breaks and status procs come straight from the world scan.
//! - 0.7 parry: an enemy attack broke off early right after its parryable
//!   window, close in front of a player who was not rolling and took no hit
//!   (world.rs candidates). Fallback: a frontal critical (riposte) on an enemy
//!   whose stance did not just break also counts as a parry, once.
//! - 0.7 backstab: a critical with the victim facing away from the player.
//!
//! Plain values only between ticks. Source-derived; not verified in game
//! (AGENTS.md: no testing by the agent). The first detections are logged.
use crate::{
    powers::{CombatEvent, RecentEvent, StatusKind},
    world::{CriticalKind, EventTarget, ReflexEvents, WorldScan},
};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

const DODGE_WINDOW: Duration = Duration::from_millis(450);
const DODGE_CONFIRM: Duration = Duration::from_millis(350);
const PERFECT_COOLDOWN: Duration = Duration::from_millis(1200);
const BLOCK_COOLDOWN: Duration = Duration::from_millis(350);
const COUNTER_WINDOW: Duration = Duration::from_millis(1500);
const CRITICAL_COOLDOWN: Duration = Duration::from_millis(1000);
const PARRY_COOLDOWN: Duration = Duration::from_millis(800);
/// A riposte this soon after a detected parry belongs to it.
const RIPOSTE_AFTER_PARRY: Duration = Duration::from_secs(3);
/// A frontal critical this soon after a stance break is a stance critical.
const STANCE_CRITICAL: Duration = Duration::from_secs(6);
/// Events count as stacks within this window (AfterEvent minimum_stacks).
const STACK_WINDOW: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Default)]
pub struct ReflexTotals {
    pub perfect_dodges: u32,
    pub blocks: u32,
    pub guard_counters: u32,
    pub criticals: u32,
    pub stance_breaks: u32,
    pub status_procs: u32,
    /// 0.7
    pub parries: u32,
    pub backstabs: u32,
    /// 0.9
    pub executions: u32,
    pub rampages: u32,
}

#[derive(Default)]
pub struct ReflexTracker {
    last_received: Option<u32>,
    last_hp: Option<i32>,
    last_stamina: Option<i32>,
    last_throw: u32,
    /// 3.2: in a critical last tick (throw state or the attacker flag).
    was_throwing: bool,
    /// 3.2: the critical's victim while it is down: (handle, critical start,
    /// critical end) and the last No Quarter strike on it.
    downed: Option<(eldenring::cs::FieldInsHandle, Instant, Option<Instant>, bool)>,
    strike: Option<(eldenring::cs::FieldInsHandle, Instant)>,
    was_dodging: bool,
    was_guarding: bool,
    dodge_start: Option<Instant>,
    dodge_hp: i32,
    pending: Option<(EventTarget, Instant, String)>,
    last_perfect: Option<Instant>,
    last_block: Option<Instant>,
    counter_until: Option<Instant>,
    last_critical: Option<Instant>,
    last_parry: Option<Instant>,
    /// 0.9: the last critical's victim (Execution) and recent kills (Rampage).
    critical_victim: Option<(eldenring::cs::FieldInsHandle, Instant)>,
    kill_times: Vec<Instant>,
    last_rampage: Option<Instant>,
    /// Recent stance breaks per enemy (riposte vs stance critical).
    broken: Vec<(eldenring::cs::FieldInsHandle, Instant)>,
    /// Recent times of every event kind (for AfterEvent and the HUD).
    history: BTreeMap<CombatEvent, Vec<Instant>>,
    pub totals: ReflexTotals,
    /// Explained first detections (drained by the runtime into the log).
    pub notes: Vec<String>,
    noted: BTreeMap<&'static str, u32>,
}

impl ReflexTracker {
    /// Character change or world unload.
    pub fn reset(&mut self) {
        let totals = self.totals;
        let noted = std::mem::take(&mut self.noted);
        *self = Self::default();
        self.totals = totals;
        self.noted = noted;
    }

    fn note(&mut self, kind: &'static str, line: String) {
        let count = self.noted.entry(kind).or_insert(0);
        if *count < 6 {
            *count += 1;
            self.notes.push(line);
        }
    }

    fn record(&mut self, event: CombatEvent, now: Instant) {
        let times = self.history.entry(event).or_default();
        times.push(now);
        times.retain(|t| now.duration_since(*t) <= STACK_WINDOW);
    }

    /// Detect this tick's reflexes and store them in `scan.reflex`.
    pub fn observe(&mut self, scan: &mut WorldScan, now: Instant) {
        let mut out = ReflexEvents::default();
        if !scan.ready {
            scan.reflex = out;
            self.reset();
            return;
        }
        let p = scan.player_state;
        let hp_loss = self.last_hp.map(|hp| hp - p.hp).unwrap_or(0).max(0);
        let stamina_drop = self.last_stamina.map(|s| s - p.stamina).unwrap_or(0).max(0);
        let received_changed = self.last_received.is_some_and(|r| r != p.received) && p.received != 0;
        let nearest_threat = scan.threats.first().copied();
        let target_of = |threat: Option<crate::world::Threat>| -> EventTarget {
            match threat {
                Some(t) => EventTarget { position: t.position, handle: Some(t.handle) },
                None => match scan.foes.iter().find(|f| f.engaged && f.distance <= 8.0) {
                    Some(f) => EventTarget { position: f.position, handle: Some(f.handle) },
                    None => EventTarget { position: scan.player_position, handle: None },
                },
            }
        };
        let significant = |loss: i32| loss >= 10.max(p.max_hp / 50);

        // Perfect dodge.
        if p.dodging && !self.was_dodging {
            self.dodge_start = Some(now);
            self.dodge_hp = p.hp;
            self.pending = None;
        }
        if p.dodging {
            let early = self.dodge_start.is_some_and(|s| now.duration_since(s) <= DODGE_WINDOW);
            let cooled = self.last_perfect.is_none_or(|t| now.duration_since(t) >= PERFECT_COOLDOWN);
            if (p.iframes || early) && self.pending.is_none() && cooled {
                if let Some(threat) = nearest_threat {
                    self.pending = Some((
                        target_of(Some(threat)),
                        now,
                        format!(
                            "attack animation {} of c{:04} at {:.1} m",
                            threat.anim, threat.character_id, threat.distance
                        ),
                    ));
                } else if received_changed && !significant(self.dodge_hp - p.hp) {
                    self.pending = Some((
                        target_of(None),
                        now,
                        format!("attack {} reached the player during invulnerability", p.received),
                    ));
                }
            }
        }
        if let Some((target, at, why)) = self.pending.take() {
            if significant(self.dodge_hp - p.hp) {
                self.note("dodge_failed", format!("Reflex: dodge was not perfect, {} HP lost ({why})", self.dodge_hp - p.hp));
            } else if !p.dodging || now.duration_since(at) >= DODGE_CONFIRM {
                out.perfect_dodge = Some(target);
                self.last_perfect = Some(now);
                self.totals.perfect_dodges += 1;
                self.record(CombatEvent::PerfectDodge, now);
                self.note("perfect_dodge", format!("Reflex: perfect dodge ({why})"));
            } else {
                self.pending = Some((target, at, why));
            }
        }
        if !p.dodging && self.was_dodging {
            self.dodge_start = None;
        }

        // Block and guard counter.
        let guarding = p.guarding || self.was_guarding;
        let block_cooled = self.last_block.is_none_or(|t| now.duration_since(t) >= BLOCK_COOLDOWN);
        if guarding && !p.dodging && block_cooled && p.max_hp > 0 && hp_loss * 4 < p.max_hp {
            let attacker_near = scan.threats.iter().find(|t| t.distance <= 6.0).copied();
            // 2.2: any engaged enemy next to you counts too (not every attack
            // animation is in the range the threat scan knows).
            let foe_close = scan.foes.iter().any(|f| f.engaged && f.distance <= 4.0);
            let why = if received_changed {
                Some(format!("attack {} hit the guard", p.received))
            } else if stamina_drop >= 5 && (attacker_near.is_some() || foe_close) {
                Some(format!("stamina -{stamina_drop} while guarding"))
            } else {
                None
            };
            if let Some(why) = why {
                out.block = Some(target_of(attacker_near.or(nearest_threat)));
                self.last_block = Some(now);
                self.counter_until = Some(now + COUNTER_WINDOW);
                self.totals.blocks += 1;
                self.record(CombatEvent::Block, now);
                self.note("block", format!("Reflex: block ({why}, {hp_loss} HP through the guard)"));
            }
        }
        if scan.hits > 0 && self.counter_until.is_some_and(|until| now <= until) {
            self.counter_until = None;
            let target = scan
                .last_hit_target
                .and_then(|h| scan.foes.iter().find(|f| f.handle == h))
                .map(|f| EventTarget { position: f.position, handle: Some(f.handle) })
                .unwrap_or_else(|| target_of(None));
            out.guard_counter = Some(target);
            self.totals.guard_counters += 1;
            self.record(CombatEvent::GuardCounter, now);
            self.note("guard_counter", "Reflex: guard counter (a hit within 1.5 s of a block)".into());
        }

        // Critical throws (riposte, backstab, stance-break critical). 3.2: the
        // throw node state (3/5) or the attacker's throw invincibility flag
        // (TAE 68), which big bosses' stance criticals set too; until 3.2
        // those were missed and critical cards never fired on bosses.
        let throwing = p.in_critical();
        if throwing
            && !self.was_throwing
            && self.last_critical.is_none_or(|t| now.duration_since(t) >= CRITICAL_COOLDOWN)
        {
            // The victim: a foe whose stance just broke (big bosses stand
            // several metres away), else the nearest within 4 m, else the
            // nearest engaged foe within 10 m (a boss first).
            let broken_foe = scan
                .foes
                .iter()
                .filter(|f| {
                    f.distance <= 12.0
                        && self.broken.iter().any(|(h, at)| *h == f.handle && now.duration_since(*at) <= STANCE_CRITICAL)
                })
                .min_by(|a, b| a.distance.total_cmp(&b.distance))
                .copied();
            let close = scan
                .foes
                .iter()
                .filter(|f| f.distance <= 4.0 && !f.protected)
                .min_by(|a, b| a.distance.total_cmp(&b.distance))
                .copied();
            let engaged = scan
                .foes
                .iter()
                .filter(|f| f.engaged && !f.protected && f.distance <= 10.0)
                .min_by(|a, b| (!a.boss, a.distance).partial_cmp(&(!b.boss, b.distance)).unwrap_or(std::cmp::Ordering::Equal))
                .copied();
            let foe = broken_foe.or(close).or(engaged);
            let victim = foe
                .map(|f| EventTarget { position: f.position, handle: Some(f.handle) })
                .unwrap_or_else(|| target_of(None));
            out.critical = Some(victim);
            self.last_critical = Some(now);
            if let Some(f) = foe {
                self.critical_victim = Some((f.handle, now));
                self.downed = Some((f.handle, now, None, false));
            }
            self.totals.criticals += 1;
            self.record(CombatEvent::CriticalHit, now);
            // 0.7: from behind = backstab. 2.3: from the front, a critical
            // on an enemy whose stance broke, or that your blows were wearing
            // down while it attacked nothing parryable, is a stance critical
            // (counted as a stance break too, if the gauges missed it);
            // otherwise it follows a parry (riposte). Until 2.3 every frontal
            // critical without a detected stance break counted as a parry and
            // showed PARRY.
            let facing = foe.map(|f| f.facing).unwrap_or(0.0);
            let kind = if facing <= -0.2 {
                out.backstab = Some(victim);
                self.totals.backstabs += 1;
                self.record(CombatEvent::Backstab, now);
                out.critical_kind = Some(CriticalKind::Backstab);
                "backstab".to_owned()
            } else if facing >= 0.2 {
                let detected = foe.is_some_and(|f| {
                    self.broken.iter().any(|(h, at)| *h == f.handle && now.duration_since(*at) <= STANCE_CRITICAL)
                });
                let parried_attack = foe.is_some_and(|f| f.parryable_age.is_some_and(|age| age <= 3.0));
                let worn_down = foe.is_some_and(|f| f.hit_age.is_some_and(|age| age <= 3.0)) && !parried_attack;
                let parried = self.last_parry.is_some_and(|t| now.duration_since(t) <= RIPOSTE_AFTER_PARRY);
                let anim = foe.map(|f| f.anim).unwrap_or(0);
                if detected || (worn_down && !parried) {
                    if !detected {
                        // The gauges missed this break: count it now.
                        out.stance_breaks.push(victim);
                        if let Some(f) = foe {
                            self.broken.push((f.handle, now));
                        }
                    }
                    out.critical_kind = Some(CriticalKind::StanceCritical);
                    format!(
                        "stance critical ({}, victim animation {anim})",
                        if detected { "stance break seen" } else { "your hits wore it down" }
                    )
                } else if parried {
                    out.critical_kind = Some(CriticalKind::Riposte);
                    format!("riposte after the parry (victim animation {anim})")
                } else {
                    out.parry = Some(victim);
                    self.last_parry = Some(now);
                    self.totals.parries += 1;
                    self.record(CombatEvent::Parry, now);
                    out.critical_kind = Some(CriticalKind::Riposte);
                    format!(
                        "riposte, the parry is counted now (its attack was parryable {}, victim animation {anim})",
                        match foe.and_then(|f| f.parryable_age) {
                            Some(age) => format!("{age:.1} s ago"),
                            None => "never seen".to_owned(),
                        }
                    )
                }
            } else {
                out.critical_kind = Some(CriticalKind::Critical);
                "critical from the side".to_owned()
            };
            self.note(
                "critical",
                format!(
                    "Reflex: critical (throw state {}, attacker flag {}, player animation {}, victim facing {facing:.2} at {:.1} m: {kind})",
                    p.throw_state,
                    p.throw_attacker,
                    p.anim,
                    foe.map(|f| f.distance).unwrap_or(-1.0)
                ),
            );
        }

        // 0.7 parry from an enemy attack that broke off early.
        let parry_cooled = self.last_parry.is_none_or(|t| now.duration_since(t) >= PARRY_COOLDOWN);
        if out.parry.is_none() && parry_cooled && !p.dodging && !significant(hp_loss) {
            let candidate = scan.parry_candidates.iter().copied().find(|c| {
                let handle = c.target.handle;
                let in_front = {
                    let d = [c.target.position[0] - scan.player_position[0], c.target.position[2] - scan.player_position[2]];
                    let len = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-3);
                    (scan.player_forward[0] * d[0] + scan.player_forward[2] * d[1]) / len >= 0.3
                };
                in_front
                    && !scan.stance_breaks.iter().any(|s| s.handle == handle)
                    && !scan.status_procs.iter().any(|(_, s)| s.handle == handle)
            });
            if let Some(c) = candidate {
                out.parry = Some(c.target);
                self.last_parry = Some(now);
                self.totals.parries += 1;
                self.record(CombatEvent::Parry, now);
                self.note(
                    "parry",
                    format!(
                        "Reflex: parry (c{:04} attack {} broke off at {:.0}% into animation {}, {:.1} m; player animation {})",
                        c.character_id,
                        c.from,
                        c.progress * 100.0,
                        c.to,
                        c.distance,
                        p.anim
                    ),
                );
            }
        }

        // 0.9 Execution (a kill within 3 s of your critical on that enemy)
        // and Rampage (three kills within 6 s).
        // 2.2: a victim that died while engaged counts too (its last blow was
        // not credited to the player), and 5 s covers long riposte animations.
        let deaths = scan
            .killed
            .iter()
            .enumerate()
            .map(|(index, handle)| (*handle, scan.kills.get(index).copied().unwrap_or(scan.player_position), true))
            .chain(scan.engaged_deaths.iter().map(|handle| (*handle, scan.player_position, false)));
        for (handle, position, credited) in deaths {
            if self
                .critical_victim
                .is_some_and(|(victim, at)| victim == handle && now.duration_since(at) <= Duration::from_secs(5))
            {
                self.critical_victim = None;
                out.execution = Some(EventTarget { position, handle: None });
                self.totals.executions += 1;
                self.record(CombatEvent::Execution, now);
                self.note("execution", "Reflex: execution (a kill right after your critical)".into());
            }
            if credited {
                self.kill_times.push(now);
            }
        }
        self.kill_times.retain(|t| now.duration_since(*t) <= Duration::from_secs(6));
        if self.kill_times.len() >= 3
            && self.last_rampage.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(6))
        {
            let position = scan.kills.last().copied().unwrap_or(scan.player_position);
            out.rampage = Some(EventTarget { position, handle: None });
            out.rampage_kills = self.kill_times.len().min(u16::MAX as usize) as u16;
            self.last_rampage = Some(now);
            self.totals.rampages += 1;
            self.record(CombatEvent::Rampage, now);
            self.note("rampage", format!("Reflex: rampage ({} kills within 6 s)", self.kill_times.len()));
        }

        // Stance breaks and status procs from the world scan.
        for target in &scan.stance_breaks {

            if let Some(handle) = target.handle {
                self.broken.push((handle, now));
            }
        }
        self.broken.retain(|(_, at)| now.duration_since(*at) <= STANCE_CRITICAL);
        // (2.3: a stance critical above may already have added one.)
        out.stance_breaks.extend(scan.stance_breaks.iter().copied());

        for _ in 0..out.stance_breaks.len() {
            self.totals.stance_breaks += 1;
            self.record(CombatEvent::PoiseBreak, now);
        }
        out.status_procs = scan.status_procs.clone();
        let kinds: Vec<StatusKind> = out.status_procs.iter().map(|(k, _)| *k).collect();
        for kind in kinds {
            self.totals.status_procs += 1;
            self.record(CombatEvent::StatusProc, now);
            if let Some(event) = kind.event() {
                self.record(event, now);
            }
        }

        self.last_received = Some(p.received);
        self.last_hp = Some(p.hp);
        self.last_stamina = Some(p.stamina);
        self.last_throw = p.throw_state;
        // 3.2: the downed victim. Its critical ends, then it gets up. 3.3: a
        // real hit of yours (not a No Quarter strike) marks it hittable but
        // keeps the state (No Quarter's window goes on); it ends when it
        // attacks, dies or 6 s after the critical ended.
        if let Some((handle, started, ended, landed)) = self.downed {
            let foe = scan.foes.iter().find(|f| f.handle == handle);
            let ended = match ended {
                None if !throwing => Some(now),
                other => other,
            };
            let struck = self.strike.is_some_and(|(h, at)| h == handle && now.duration_since(at) <= Duration::from_millis(600));
            let real_hit = ended.is_some_and(|end| now.duration_since(end) >= Duration::from_millis(300))
                && scan.hits > 0
                && scan.last_hit_target == Some(handle)
                && !struck;
            let attacking = foe.is_some_and(|f| (3000..=3999).contains(&f.anim.rem_euclid(1_000_000)));
            let landed = landed || real_hit;
            let over = foe.is_none()
                || (attacking && ended.is_some())
                || ended.is_some_and(|end| now.duration_since(end) >= Duration::from_secs(6))
                || now.duration_since(started) >= Duration::from_secs(14);
            if over {
                if let Some(end) = ended {
                    let reason = if foe.is_none() {
                        "gone or dead"
                    } else if attacking {
                        "it attacked"
                    } else {
                        "6 s passed"
                    };
                    self.note(
                        "downed",
                        format!("Reflex: critical victim back up after {:.1} s ({reason})", now.duration_since(end).as_secs_f32()),
                    );
                }
                self.downed = None;
            } else {
                self.downed = Some((handle, started, ended, landed));
                out.downed = Some(crate::world::Downed {
                    handle,
                    since_throw: ended.map(|end| now.duration_since(end).as_secs_f32()),
                    landed,
                });
            }
        }
        self.was_throwing = throwing;
        self.was_dodging = p.dodging;
        self.was_guarding = p.guarding;
        scan.reflex = out;
    }

    /// 3.2: a No Quarter strike landed on `handle` (its HP drop is not a
    /// sign that the foe can be hit normally again).
    pub fn note_strike(&mut self, handle: eldenring::cs::FieldInsHandle, now: Instant) {
        self.strike = Some((handle, now));
    }

    /// Age of the latest occurrence and recent stacks of every reflex event.
    pub fn recent(&self, now: Instant) -> Vec<(CombatEvent, RecentEvent)> {
        self.history
            .iter()
            .filter_map(|(event, times)| {
                let last = times.last()?;
                let stacks = times
                    .iter()
                    .filter(|t| now.duration_since(**t) <= STACK_WINDOW)
                    .count()
                    .clamp(1, u16::MAX as usize) as u16;
                Some((
                    *event,
                    RecentEvent {
                        age_seconds: now.duration_since(*last).as_secs_f32(),
                        stacks,
                    },
                ))
            })
            .collect()
    }
}
