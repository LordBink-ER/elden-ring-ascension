//! 0.7 "Wygląd": interface and reflex sounds (tools/build_sounds.py), played
//! with winmm PlaySound, asynchronously, from memory. PlaySound has a single
//! channel, so a cue never interrupts a more important one that is still
//! playing (a rare reveal is not cut by a card deal). The game's own volume
//! settings do not apply; [ui] sound_volume scales the samples once at start.
use std::{
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};
use windows::{
    Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT},
    core::PCWSTR,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    Deal,
    RevealRare,
    RevealEpic,
    RevealLegendary,
    RevealMythic,
    Choose,
    Evolution,
    Dodge,
    Parry,
    Critical,
    // 2.3 "Impact": combat moments.
    StanceBreak,
    Backstab,
    Execution,
    Rampage,
    Block,
    GuardCounter,
    Blood,
    Frost,
    Poison,
    Sleep,
    Madness,
    Blight,
    // 2.3: banners.
    EliteFelled,
    PathBonus,
    DeathDefied,
    TideRises,
    /// 3.4: a Prismatic card turned over (the loudest cue).
    MaxWin,
}
impl Cue {
    /// A cue never cuts a higher one that still holds the single channel.
    pub fn priority(self) -> u64 {
        match self {
            Self::Deal => 1,
            Self::Block | Self::Blood | Self::Frost | Self::Poison | Self::Sleep | Self::Madness | Self::Blight => 2,
            Self::Dodge | Self::Parry | Self::Critical | Self::GuardCounter => 3,
            Self::StanceBreak | Self::Backstab | Self::Execution | Self::Rampage => 4,
            Self::Choose => 5,
            Self::RevealRare => 6,
            Self::RevealEpic | Self::PathBonus | Self::TideRises | Self::EliteFelled => 7,
            Self::RevealLegendary | Self::DeathDefied => 8,
            Self::Evolution => 9,
            Self::RevealMythic => 10,
            Self::MaxWin => 11,
        }
    }
    /// Combat sounds ([ui] reflex_sounds); the rest are interface sounds.
    fn reflex(self) -> bool {
        matches!(
            self,
            Self::Dodge
                | Self::Parry
                | Self::Critical
                | Self::StanceBreak
                | Self::Backstab
                | Self::Execution
                | Self::Rampage
                | Self::Block
                | Self::GuardCounter
                | Self::Blood
                | Self::Frost
                | Self::Poison
                | Self::Sleep
                | Self::Madness
                | Self::Blight
        )
    }
    /// Only the first part of a cue matters for interruption (tails may be cut).
    fn hold_ms(self) -> u64 {
        match self {
            Self::Deal => 250,
            Self::Block | Self::Blood | Self::Frost | Self::Poison | Self::Sleep | Self::Madness | Self::Blight => 300,
            Self::Dodge | Self::Parry | Self::Critical | Self::GuardCounter => 400,
            Self::StanceBreak | Self::Backstab | Self::Execution | Self::Rampage => 600,
            Self::Choose => 500,
            Self::RevealRare => 700,
            Self::RevealEpic => 900,
            Self::PathBonus | Self::TideRises | Self::EliteFelled => 1400,
            Self::RevealLegendary | Self::Evolution | Self::DeathDefied => 1300,
            Self::RevealMythic => 1800,
            Self::MaxWin => 3500,
        }
    }
}

const FILES: [(Cue, &[u8]); 27] = [
    (Cue::Deal, include_bytes!("../assets/sounds/deal.wav")),
    (Cue::RevealRare, include_bytes!("../assets/sounds/reveal_rare.wav")),
    (Cue::RevealEpic, include_bytes!("../assets/sounds/reveal_epic.wav")),
    (Cue::RevealLegendary, include_bytes!("../assets/sounds/reveal_legendary.wav")),
    (Cue::RevealMythic, include_bytes!("../assets/sounds/reveal_mythic.wav")),
    (Cue::Choose, include_bytes!("../assets/sounds/choose.wav")),
    (Cue::Evolution, include_bytes!("../assets/sounds/evolution.wav")),
    (Cue::Dodge, include_bytes!("../assets/sounds/dodge.wav")),
    (Cue::Parry, include_bytes!("../assets/sounds/parry.wav")),
    (Cue::Critical, include_bytes!("../assets/sounds/critical.wav")),
    (Cue::StanceBreak, include_bytes!("../assets/sounds/stance_break.wav")),
    (Cue::Backstab, include_bytes!("../assets/sounds/backstab.wav")),
    (Cue::Execution, include_bytes!("../assets/sounds/execution.wav")),
    (Cue::Rampage, include_bytes!("../assets/sounds/rampage.wav")),
    (Cue::Block, include_bytes!("../assets/sounds/block.wav")),
    (Cue::GuardCounter, include_bytes!("../assets/sounds/guard_counter.wav")),
    (Cue::Blood, include_bytes!("../assets/sounds/status_blood.wav")),
    (Cue::Frost, include_bytes!("../assets/sounds/status_frost.wav")),
    (Cue::Poison, include_bytes!("../assets/sounds/status_poison.wav")),
    (Cue::Sleep, include_bytes!("../assets/sounds/status_sleep.wav")),
    (Cue::Madness, include_bytes!("../assets/sounds/status_madness.wav")),
    (Cue::Blight, include_bytes!("../assets/sounds/status_blight.wav")),
    (Cue::EliteFelled, include_bytes!("../assets/sounds/elite_felled.wav")),
    (Cue::PathBonus, include_bytes!("../assets/sounds/path_bonus.wav")),
    (Cue::DeathDefied, include_bytes!("../assets/sounds/death_defied.wav")),
    (Cue::TideRises, include_bytes!("../assets/sounds/tide_rises.wav")),
    (Cue::MaxWin, include_bytes!("../assets/sounds/max_win.wav")),
];

struct Bank {
    sounds: Vec<(Cue, &'static [u8])>,
    interface: bool,
    reflexes: bool,
    start: Instant,
    /// (priority << 32) | (ms since start until which it holds the channel)
    busy: AtomicU64,
}
static BANK: Mutex<Option<Bank>> = Mutex::new(None);
/// Scaled copies per volume step (0.05), so changing the volume in Settings
/// never leaks more than one copy per step.
static SCALED: Mutex<Vec<(u32, Vec<(Cue, &'static [u8])>)>> = Mutex::new(Vec::new());

/// Scale 16-bit PCM in a canonical 44-byte-header WAV (ours) by `volume`.
fn scaled(wav: &[u8], volume: f32) -> Option<&'static [u8]> {
    if wav.len() < 44 || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" || &wav[36..40] != b"data" {
        return None;
    }
    let mut copy = wav.to_vec();
    for sample in copy[44..].chunks_exact_mut(2) {
        let value = f32::from(i16::from_le_bytes([sample[0], sample[1]])) * volume;
        let bytes = (value.round().clamp(-32768.0, 32767.0) as i16).to_le_bytes();
        sample.copy_from_slice(&bytes);
    }
    Some(Box::leak(copy.into_boxed_slice()))
}

/// Prepare the bank. Returns how many sounds are ready.
pub fn init(settings: &crate::config::UiSettings) -> usize {
    configure(settings)
}

/// 1.0: (re)configure from the settings, also live from the Settings tab.
pub fn configure(settings: &crate::config::UiSettings) -> usize {
    let volume = if settings.sound_volume.is_finite() { settings.sound_volume.clamp(0.0, 1.0) } else { 0.0 };
    let enabled = settings.sounds && volume > 0.0;
    let step = (volume * 20.0).round() as u32;
    let sounds: Vec<(Cue, &'static [u8])> = if enabled {
        let Ok(mut cache) = SCALED.lock() else {
            return 0;
        };
        match cache.iter().find(|(s, _)| *s == step) {
            Some((_, set)) => set.clone(),
            None => {
                let set: Vec<(Cue, &'static [u8])> = FILES
                    .iter()
                    .filter_map(|(cue, wav)| scaled(wav, step as f32 / 20.0).map(|data| (*cue, data)))
                    .collect();
                cache.push((step, set.clone()));
                set
            }
        }
    } else {
        Vec::new()
    };
    let count = sounds.len();
    if let Ok(mut bank) = BANK.lock() {
        *bank = Some(Bank {
            sounds,
            interface: enabled,
            reflexes: enabled && settings.reflex_sounds,
            start: Instant::now(),
            busy: AtomicU64::new(0),
        });
    }
    count
}

pub fn play(cue: Cue) {
    let Ok(guard) = BANK.try_lock() else {
        return;
    };
    let Some(bank) = guard.as_ref() else {
        return;
    };
    if if cue.reflex() { !bank.reflexes } else { !bank.interface } {
        return;
    }
    let Some((_, data)) = bank.sounds.iter().find(|(c, _)| *c == cue) else {
        return;
    };
    let now = bank.start.elapsed().as_millis() as u64;
    let busy = bank.busy.load(Ordering::Acquire);
    let (holder, until) = (busy >> 32, busy & 0xffff_ffff);
    if now < until && holder > cue.priority() {
        return;
    }
    let next = (cue.priority() << 32) | ((now + cue.hold_ms()) & 0xffff_ffff);
    if bank
        .busy
        .compare_exchange(busy, next, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    // SND_MEMORY: the "name" is the in-memory WAV image, kept alive forever.
    unsafe {
        let _ = PlaySoundW(PCWSTR(data.as_ptr().cast()), None, SND_ASYNC | SND_MEMORY | SND_NODEFAULT);
    }
}
