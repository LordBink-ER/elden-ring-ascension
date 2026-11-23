//! Game-independent Ascension progression. No in-game addresses or fictitious effects.
mod catalog;
mod draw;
mod state;
mod store;

pub use catalog::{BossTier, Catalog, DrawConfig, MetaEffect, PowerDef, Rarity};
pub use state::{
    BANISH_COST, Card, ChoiceRecord, EMBER_REROLL_COST, LOCK_COST, LockedCard, Offer,
    Progression, RANKS, RewardKey, SKIP_EMBERS, Victory, level_up_cost, next_rank, offer_size,
    lock_cost, pity_factor, rank_name, rank_rerolls, skip_bonus, skip_runes, tier_embers,
    total_levels,
};
pub use store::Store;

#[derive(Debug)]
pub enum CoreError {
    Invalid(String),
    NotBaselined,
    CycleMismatch { stored: u32, observed: u32 },
    NoOffer,
    StaleOffer,
    NoRerolls,
    NotEnoughEmbers { have: u32, need: u32 },
    PoolExhausted { available: usize },
    AlreadyExists,
    NotFound,
    Locked,
    Corrupt(String),
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(s) => write!(f, "invalid Ascension data: {s}"),
            Self::NotBaselined => write!(
                f,
                "boss snapshot must be baselined before observing victories"
            ),
            Self::CycleMismatch { stored, observed } => write!(
                f,
                "save cycle mismatch: stored {stored}, observed {observed}"
            ),
            Self::NoOffer => write!(f, "there is no prepared reward"),
            Self::StaleOffer => write!(f, "reward changed; refresh the selection"),
            Self::NoRerolls => write!(f, "this reward has no rerolls remaining"),
            Self::NotEnoughEmbers { have, need } => {
                write!(f, "not enough Ascension Embers ({have} of {need})")
            }
            Self::PoolExhausted { available } => write!(
                f,
                "no eligible upgrade is available (remaining: {available}); reward remains queued"
            ),
            Self::AlreadyExists => {
                write!(f, "progression already exists; refusing to overwrite it")
            }
            Self::NotFound => write!(
                f,
                "progression does not exist; create it explicitly after identifying the character"
            ),
            Self::Locked => write!(f, "another process holds this character's progression lock"),
            Self::Corrupt(s) => write!(
                f,
                "progression integrity failure: {s}; automatic reset is forbidden"
            ),
            Self::Io(e) => e.fmt(f),
            Self::Json(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for CoreError {}
impl From<std::io::Error> for CoreError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<serde_json::Error> for CoreError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}
pub type Result<T> = std::result::Result<T, CoreError>;

pub(crate) fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
}

/// Proc catalog ids: lowercase ASCII words joined by underscores.
pub fn valid_proc_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}
