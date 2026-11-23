//! 2.3 "Impact": what the combat labels say, their colours and sounds, and
//! the big banners for the moments that matter (an Ascended Elite felled, a
//! Path bonus, death defied, the Tide rising). The overlay draws them
//! (cards_ui.rs `draw_flash`); the runtime picks them each tick.
use crate::{
    powers::{CombatEvent, PowerLibrary, StatusKind},
    sound::Cue,
    world::{CriticalKind, ReflexEvents},
};
use std::{collections::BTreeMap, time::Instant};

/// One label or banner on screen.
#[derive(Clone, Debug)]
pub struct Flash {
    /// Small spaced capitals above a banner's title ("RESONANCE PATH · 5").
    pub overline: String,
    pub title: String,
    pub subtitle: String,
    pub color: [f32; 4],
    /// A banner (top of the screen, long) or a combat label (short).
    pub big: bool,
    /// How strongly the screen edges glow in its colour (0..1).
    pub glow: f32,
    pub at: Instant,
}
impl Flash {
    /// Seconds on screen.
    pub fn duration(&self) -> f32 {
        if self.big { 4.6 } else { 1.7 }
    }
    pub fn alive(&self) -> bool {
        self.at.elapsed().as_secs_f32() < self.duration()
    }
}

const DODGE: [f32; 4] = [0.62, 0.86, 1.0, 1.0];
const PARRY: [f32; 4] = [1.0, 0.92, 0.66, 1.0];
const STANCE: [f32; 4] = [1.0, 0.66, 0.26, 1.0];
const CRITICAL: [f32; 4] = [1.0, 0.80, 0.40, 1.0];
const BACKSTAB: [f32; 4] = [0.78, 0.60, 1.0, 1.0];
const BLOOD_RED: [f32; 4] = [0.95, 0.26, 0.24, 1.0];
const GUARD: [f32; 4] = [0.86, 0.86, 0.80, 1.0];
pub const GOLD: [f32; 4] = [0.93, 0.78, 0.45, 1.0];

fn status_look(kind: StatusKind) -> (&'static str, [f32; 4], Cue) {
    match kind {
        StatusKind::Poison => ("POISONED", [0.56, 0.86, 0.36, 1.0], Cue::Poison),
        StatusKind::ScarletRot => ("SCARLET ROT", [0.92, 0.46, 0.30, 1.0], Cue::Poison),
        StatusKind::BloodLoss => ("BLOOD LOSS", [0.92, 0.18, 0.22, 1.0], Cue::Blood),
        StatusKind::DeathBlight => ("DEATH BLIGHT", [0.70, 0.62, 0.82, 1.0], Cue::Blight),
        StatusKind::Frostbite => ("FROSTBITE", [0.62, 0.88, 1.0, 1.0], Cue::Frost),
        StatusKind::Sleep => ("SLEEP", [0.72, 0.68, 0.98, 1.0], Cue::Sleep),
        StatusKind::Madness => ("MADNESS", [1.0, 0.74, 0.22, 1.0], Cue::Madness),
    }
}

/// The strongest moment of this tick's reflexes as a label, with its sound.
/// Its subtitle names your cards that react to it.
pub fn combat(
    r: &ReflexEvents,
    library: &PowerLibrary,
    levels: &BTreeMap<String, u8>,
    now: Instant,
) -> Option<(Flash, Cue)> {
    use CombatEvent as E;
    let mut status = None;
    let mut note = String::new();
    let (title, color, glow, cue, events): (&str, [f32; 4], f32, Cue, Vec<E>) = if r.execution.is_some() {
        ("EXECUTED", BLOOD_RED, 0.55, Cue::Execution, vec![E::Execution])
    } else if r.rampage.is_some() {
        note = format!("{} kills in 6 s", r.rampage_kills.max(3));
        ("RAMPAGE", BLOOD_RED, 0.55, Cue::Rampage, vec![E::Rampage, E::Kill])
    } else if let Some(kind) = r.critical_kind {
        match kind {
            CriticalKind::Backstab => ("BACKSTAB", BACKSTAB, 0.4, Cue::Backstab, vec![E::Backstab, E::CriticalHit]),
            // A break the gauges missed is first named here.
            CriticalKind::StanceCritical if !r.stance_breaks.is_empty() => {
                note = "Critical blow".into();
                ("STANCE BROKEN", STANCE, 0.5, Cue::StanceBreak, vec![E::PoiseBreak, E::CriticalHit])
            }
            CriticalKind::StanceCritical => ("CRITICAL BLOW", CRITICAL, 0.45, Cue::Critical, vec![E::CriticalHit]),
            CriticalKind::Riposte => ("RIPOSTE", PARRY, 0.45, Cue::Critical, vec![E::Parry, E::CriticalHit]),
            CriticalKind::Critical => ("CRITICAL", CRITICAL, 0.35, Cue::Critical, vec![E::CriticalHit]),
        }
    } else if !r.stance_breaks.is_empty() {
        ("STANCE BROKEN", STANCE, 0.5, Cue::StanceBreak, vec![E::PoiseBreak])
    } else if r.parry.is_some() {
        ("PARRIED", PARRY, 0.4, Cue::Parry, vec![E::Parry])
    } else if r.perfect_dodge.is_some() {
        ("PERFECT DODGE", DODGE, 0.3, Cue::Dodge, vec![E::PerfectDodge])
    } else if r.guard_counter.is_some() {
        ("GUARD COUNTER", PARRY, 0.3, Cue::GuardCounter, vec![E::GuardCounter])
    } else if let Some((kind, _)) = r.status_procs.first() {
        let (name, color, cue) = status_look(*kind);
        status = Some(*kind);
        let mut events = vec![E::StatusProc];
        events.extend(kind.event());
        (name, color, 0.25, cue, events)
    } else if r.block.is_some() {
        ("BLOCKED", GUARD, 0.0, Cue::Block, vec![E::Block])
    } else {
        return None;
    };
    let names = library.reacting(levels, &events, status);
    let cards = match names.len() {
        0 => String::new(),
        1..=3 => names.join("  ·  "),
        n => format!("{}  ·  +{} more", names[..2].join("  ·  "), n - 2),
    };
    let subtitle = match (note.is_empty(), cards.is_empty()) {
        (true, _) => cards,
        (false, true) => note,
        (false, false) => format!("{note}  ·  {cards}"),
    };
    Some((
        Flash {
            overline: String::new(),
            title: title.to_owned(),
            subtitle,
            color,
            big: false,
            glow,
            at: now,
        },
        cue,
    ))
}

/// A banner for a moment that matters.
pub fn banner(overline: impl Into<String>, title: impl Into<String>, subtitle: impl Into<String>, color: [f32; 4], now: Instant) -> Flash {
    Flash {
        overline: overline.into(),
        title: title.into(),
        subtitle: subtitle.into(),
        color,
        big: true,
        glow: 0.35,
        at: now,
    }
}

/// Roman numerals for the Tide (1..=10).
pub fn roman(n: u8) -> &'static str {
    ["0", "I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X"][usize::from(n.min(10))]
}
