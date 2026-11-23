//! Real reward and collection UI. Selection submits epoch-scoped commands;
//! ownership shown here comes only from committed progression snapshots.
//! Visuals are drawn with ImGui draw lists (rarity glow, ornaments, category
//! glyphs, level-evolution pips); the selection state machine is unchanged.
//! 0.7: card illustrations (src/art.rs atlases), a face-down deal with card
//! flips and rarity bursts, sounds (src/sound.rs) and the Build tab.
use crate::{
    core::{BossTier, Card, Offer, Rarity},
    powers::{AttributeValues, Power},
    procs::{ProcContext, proc_damage},
    runtime::{HudView, ProgressView, SharedRuntime},
    sound::{self, Cue},
    synergies::Synergy,
};
use hudhook::imgui::{
    Condition, DrawListMut, FontId, Key, MouseButton, StyleColor, StyleVar, TextureId, Ui, WindowFlags,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

/// 0.7 deal: when each card flips (s after the offer appears) and for how long.
const FLIP_START: f32 = 0.35;
const FLIP_GAP: f32 = 0.24;
const FLIP_TIME: f32 = 0.5;

/// 0.7: card pictures uploaded to the overlay (one texture per atlas).
pub struct CardArt {
    textures: Vec<TextureId>,
    sizes: Vec<[u32; 2]>,
    tile: [u32; 2],
    columns: u32,
    per_atlas: u32,
    back: u32,
    cards: BTreeMap<String, u32>,
}
/// One picture: texture and the uv rectangle of its tile.
#[derive(Clone, Copy)]
pub struct Tile {
    texture: TextureId,
    uv0: [f32; 2],
    uv1: [f32; 2],
    /// Width / height of the tile in pixels.
    aspect: f32,
}
impl CardArt {
    pub fn new(pack: &crate::art::ArtPack, textures: Vec<TextureId>) -> Self {
        Self {
            textures,
            sizes: pack.sizes(),
            tile: pack.tile,
            columns: pack.columns,
            per_atlas: pack.per_atlas,
            back: pack.back,
            cards: pack.cards.clone(),
        }
    }
    fn tile(&self, index: u32) -> Option<Tile> {
        let (atlas, uv0, uv1) = crate::art::tile_uv(self.tile, self.columns, self.per_atlas, &self.sizes, index)?;
        Some(Tile {
            texture: *self.textures.get(atlas)?,
            uv0,
            uv1,
            aspect: self.tile[0] as f32 / self.tile[1].max(1) as f32,
        })
    }
    fn card(&self, id: &str) -> Option<Tile> {
        self.tile(*self.cards.get(id)?)
    }
    fn back(&self) -> Option<Tile> {
        self.tile(self.back)
    }
}
impl Tile {
    /// Crop to `aspect` (width / height), centred horizontally and at
    /// `focus` (0 top .. 1 bottom) vertically; the motifs sit a little high.
    fn crop(self, aspect: f32, focus: f32) -> Self {
        if !aspect.is_finite() || aspect <= 0.0 {
            return self;
        }
        let (du, dv) = (self.uv1[0] - self.uv0[0], self.uv1[1] - self.uv0[1]);
        if aspect > self.aspect {
            let keep = dv * self.aspect / aspect;
            let top = self.uv0[1] + (dv - keep) * focus.clamp(0.0, 1.0);
            Self { uv0: [self.uv0[0], top], uv1: [self.uv1[0], top + keep], ..self }
        } else {
            let keep = du * aspect / self.aspect;
            let left = self.uv0[0] + (du - keep) * 0.5;
            Self { uv0: [left, self.uv0[1]], uv1: [left + keep, self.uv1[1]], ..self }
        }
    }
}
// TextureIds are plain indices into the overlay's descriptor heap.
unsafe impl Send for CardArt {}
unsafe impl Sync for CardArt {}

/// Height of the picture band at the top of a reward card.
fn banner_height(size: [f32; 2]) -> f32 {
    (size[0] * 0.30).min(size[1] * 0.20).clamp(56.0, 128.0)
}

fn reveal_cue(card: &Card, power: Option<&Power>) -> Option<Cue> {
    // 3.4: the jackpot.
    if is_prismatic(power) {
        return Some(Cue::MaxWin);
    }
    if card.previous_level == 0 && power.is_some_and(|p| p.is_evolution()) {
        return Some(Cue::Evolution);
    }
    match card.display_rarity {
        Rarity::Common | Rarity::Uncommon => None,
        Rarity::Rare => Some(Cue::RevealRare),
        Rarity::Epic => Some(Cue::RevealEpic),
        Rarity::Legendary => Some(Cue::RevealLegendary),
        Rarity::Mythic => Some(Cue::RevealMythic),
    }
}

const GOLD: [f32; 4] = [0.83, 0.69, 0.39, 1.0];
const GOLD_DIM: [f32; 4] = [0.55, 0.45, 0.24, 1.0];
const INK: [f32; 4] = [0.045, 0.050, 0.043, 0.98];
const PAPER: [f32; 4] = [0.88, 0.86, 0.78, 1.0];
const MUTED: [f32; 4] = [0.58, 0.60, 0.53, 1.0];
const WARN: [f32; 4] = [0.91, 0.62, 0.30, 1.0];
/// 0.5: curse cards (drawback text, frame accent).
const CURSE: [f32; 4] = [0.86, 0.22, 0.30, 1.0];
/// 2.2: what new attribute points add (Level Up panel).
const GAIN: [f32; 4] = [0.50, 0.88, 0.45, 1.0];
/// 3.1: an attribute raised by a talisman, tear or Great Rune (the game's blue).
const BOOSTED: [f32; 4] = [0.48, 0.72, 1.0, 1.0];

/// Fonts loaded by the overlay (title 34 px, heading 24 px).
#[derive(Clone, Copy, Default)]
pub struct Fonts {
    pub title: Option<FontId>,
    pub heading: Option<FontId>,
    /// 2.3: serif display faces for combat labels (44 px) and banners (60 px).
    pub moment: Option<FontId>,
    pub banner: Option<FontId>,
}
// FontIds point into the ImGui atlas and are only used on the render thread.
unsafe impl Send for Fonts {}
unsafe impl Sync for Fonts {}

pub fn rarity_color(rarity: Rarity) -> [f32; 4] {
    match rarity {
        Rarity::Common => [0.66, 0.67, 0.64, 1.0],
        Rarity::Uncommon => [0.39, 0.72, 0.44, 1.0],
        Rarity::Rare => [0.37, 0.62, 0.95, 1.0],
        Rarity::Epic => [0.72, 0.47, 0.90, 1.0],
        Rarity::Legendary => [0.94, 0.75, 0.31, 1.0],
        Rarity::Mythic => [0.92, 0.31, 0.28, 1.0],
    }
}
/// 3.4: every colour, cycling (Prismatic cards).
fn rainbow(t: f32, phase: f32) -> [f32; 4] {
    let h = (t * 0.35 + phase).rem_euclid(1.0) * 6.0;
    let c = |k: f32| (((h + k) % 6.0 - 3.0).abs() - 1.0).clamp(0.0, 1.0) * 0.75 + 0.25;
    [c(0.0), c(4.0), c(2.0), 1.0]
}
fn is_prismatic(power: Option<&Power>) -> bool {
    power.is_some_and(|p| p.category == "prismatic")
}
/// A card's colour: its rarity's, or every colour for a Prismatic card.
fn card_color(power: Option<&Power>, rarity: Rarity, t: f32) -> [f32; 4] {
    if is_prismatic(power) { rainbow(t, 0.0) } else { rarity_color(rarity) }
}
fn card_rarity_name(power: Option<&Power>, rarity: Rarity) -> &'static str {
    if is_prismatic(power) { "PRISMATIC" } else { rarity_name(rarity) }
}
fn rarity_name(rarity: Rarity) -> &'static str {
    match rarity {
        Rarity::Common => "COMMON",
        Rarity::Uncommon => "UNCOMMON",
        Rarity::Rare => "RARE",
        Rarity::Epic => "EPIC",
        Rarity::Legendary => "LEGENDARY",
        Rarity::Mythic => "MYTHIC",
    }
}
fn tier_name(tier: BossTier) -> &'static str {
    match tier {
        BossTier::Minor => "MINOR BOSS",
        BossTier::Greater => "GREATER BOSS",
        BossTier::Major => "MAJOR BOSS",
        BossTier::Remembrance => "REMEMBRANCE",
    }
}
fn tint(color: [f32; 4], amount: f32) -> [f32; 4] {
    [
        INK[0] + (color[0] - INK[0]) * amount,
        INK[1] + (color[1] - INK[1]) * amount,
        INK[2] + (color[2] - INK[2]) * amount,
        1.0,
    ]
}
fn alpha(color: [f32; 4], a: f32) -> [f32; 4] {
    [color[0], color[1], color[2], color[3] * a.clamp(0.0, 1.0)]
}
fn roman(level: u8) -> &'static str {
    match level {
        0 => "-",
        1 => "I",
        2 => "II",
        3 => "III",
        4 => "IV",
        5 => "V",
        _ => "?",
    }
}
fn pressed(ui: &Ui, keys: &[Key]) -> bool {
    keys.iter().any(|key| ui.is_key_pressed_no_repeat(*key))
}
fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

enum Waiting {
    Choice {
        epoch: u64,
        offer: u64,
        power: String,
        target: u8,
    },
    Reroll {
        epoch: u64,
        offer: u64,
    },
    /// Banish, lock or skip: done when the offer changes.
    Action {
        epoch: u64,
        offer: u64,
        label: &'static str,
    },
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Cards,
    Paths,
    Codex,
    Combos,
    You,
    Guide,
    Settings,
}
pub struct CardUi {
    shared: Arc<SharedRuntime>,
    last_view: Option<ProgressView>,
    last_hud: HudView,
    binding: Option<(u64, u64)>,
    deferred: Option<(u64, u64)>,
    selected: Option<usize>,
    focus: usize,
    waiting: Option<Waiting>,
    collection: bool,
    error: Option<String>,
    last_modal: bool,
    release_guard: Option<Instant>,
    fonts: Fonts,
    /// (epoch, offer id, ui time when first drawn) for the reveal animation.
    revealed: Option<(u64, u64, f64)>,
    tab: Tab,
    /// Codex filter: 0 all, 1 owned, 2 not owned.
    codex_filter: u8,
    pub hud_visible: bool,
    /// 1.0: live [ui] settings (Settings tab), saved to ascension.toml.
    ui_settings: crate::config::UiSettings,
    settings_note: Option<(String, bool)>,
    /// Banish armed for (offer id, slot) until this time: a second press commits.
    banish_armed: Option<(u64, usize, Instant)>,
    /// Skip armed for this offer until this time: a second press commits.
    skip_armed: Option<(u64, Instant)>,
    /// Embers seen last frame and the latest gain, for the HUD toast.
    last_embers: Option<u32>,
    ember_toast: Option<(u32, Instant)>,
    /// 3.4: the MAX WIN of a Prismatic reveal (offer key, start).
    max_win: Option<((u64, u64), Instant)>,
    /// 3.4: when the title screen banner appeared.
    title_since: Option<Instant>,
    /// 0.7: card pictures (None until the overlay uploaded them).
    art: Option<CardArt>,
    /// 0.7: sounds already played for this reveal: (epoch, offer, bits:
    /// card i revealed = 1 << i, deal = 0x80).
    reveal_cues: Option<(u64, u64, u8)>,
}
impl CardUi {
    pub fn new(shared: Arc<SharedRuntime>) -> Self {
        let ui_settings = shared.ui.clone();
        Self {
            shared,
            last_view: None,
            last_hud: HudView::default(),
            binding: None,
            deferred: None,
            selected: None,
            focus: 0,
            waiting: None,
            collection: false,
            error: None,
            last_modal: false,
            release_guard: None,
            fonts: Fonts::default(),
            revealed: None,
            tab: Tab::Cards,
            codex_filter: 0,
            hud_visible: true,
            ui_settings,
            settings_note: None,
            banish_armed: None,
            skip_armed: None,
            last_embers: None,
            ember_toast: None,
            max_win: None,
            title_since: None,
            art: None,
            reveal_cues: None,
        }
    }
    pub fn set_fonts(&mut self, fonts: Fonts) {
        self.fonts = fonts;
    }
    pub fn set_art(&mut self, art: CardArt) {
        self.art = Some(art);
    }
    fn art_for(&self, id: &str) -> Option<Tile> {
        if !self.ui_settings.card_art {
            return None;
        }
        self.art.as_ref().and_then(|a| a.card(id))
    }
    /// Play `cue` once per reveal (bit in the reveal's mask).
    fn cue_once(&mut self, key: (u64, u64), bit: u8, cue: Option<Cue>) {
        let mask = match self.reveal_cues {
            Some((e, o, mask)) if (e, o) == key => mask,
            _ => 0,
        };
        if mask & bit != 0 {
            return;
        }
        self.reveal_cues = Some((key.0, key.1, mask | bit));
        if let Some(cue) = cue {
            sound::play(cue);
        }
    }


    /// Returns true only while a visible, safe modal owns input. The caller
    /// handles keyboard/mouse/controller suppression for that native frame.
    pub fn render(&mut self, ui: &mut Ui) -> bool {
        // 3.4: the title screen: the mod's name and version, rising embers.
        if self.shared.title_screen.load(Ordering::Acquire) {
            let since = *self.title_since.get_or_insert_with(Instant::now);
            draw_title(ui, self.fonts, since.elapsed().as_secs_f32());
        } else {
            self.title_since = None;
        }
        if let Ok(view) = self.shared.view.try_lock() {
            self.last_view = Some(view.clone());
        }
        if let Ok(hud) = self.shared.hud.try_lock() {
            self.last_hud = hud.clone();
        }
        let Some(view) = self.last_view.clone() else {
            return self.finish(ui, false, false);
        };
        let epoch = self.shared.epoch.load(Ordering::Acquire);
        if view.epoch != epoch || !view.ready {
            self.binding = None;
            self.selected = None;
            self.waiting = None;
            self.collection = false;
            return self.finish(ui, false, false);
        }
        self.acknowledge(&view);
        if let Some(previous) = self.last_embers {
            if view.embers > previous {
                self.ember_toast = Some((view.embers - previous, Instant::now()));
            }
        }
        self.last_embers = Some(view.embers);
        let safe = self.shared.safe.load(Ordering::Acquire);
        let toggle = pressed(ui, &[Key::F7, Key::GamepadBack]);
        if toggle && safe && self.waiting.is_none() {
            if self.collection {
                self.collection = false;
            } else if let Some(offer) = &view.offer {
                let key = (view.epoch, offer.id);
                if self.deferred == Some(key) {
                    self.deferred = None;
                } else {
                    self.collection = true;
                }
            } else {
                self.collection = true;
            }
        }
        if pressed(ui, &[Key::F6]) {
            self.hud_visible = !self.hud_visible;
        }
        // 2.1: what the attributes give, beside the game's Level Up page.
        if self.shared.level_up_open.load(Ordering::Acquire) && !self.collection {
            self.render_attribute_panel(ui);
        }
        if !safe {
            self.render_hud(ui, &view, false);
            return self.finish(ui, false, false);
        }
        let styles = [
            ui.push_style_color(StyleColor::WindowBg, [0.0, 0.0, 0.0, 0.0]),
            ui.push_style_color(StyleColor::ChildBg, [0.0, 0.0, 0.0, 0.0]),
            ui.push_style_color(StyleColor::Text, PAPER),
            ui.push_style_color(StyleColor::Border, [0.42, 0.37, 0.24, 0.85]),
            ui.push_style_color(StyleColor::Button, [0.16, 0.15, 0.11, 0.95]),
            ui.push_style_color(StyleColor::ButtonHovered, [0.29, 0.25, 0.15, 1.0]),
            ui.push_style_color(StyleColor::ButtonActive, [0.42, 0.34, 0.16, 1.0]),
            ui.push_style_color(StyleColor::Tab, [0.12, 0.11, 0.08, 1.0]),
            ui.push_style_color(StyleColor::TabHovered, [0.34, 0.28, 0.15, 1.0]),
            ui.push_style_color(StyleColor::TabActive, [0.42, 0.34, 0.16, 1.0]),
            ui.push_style_color(StyleColor::Separator, alpha(GOLD_DIM, 0.6)),
            ui.push_style_color(StyleColor::FrameBg, [0.10, 0.10, 0.08, 1.0]),
            ui.push_style_color(StyleColor::CheckMark, GOLD),
        ];
        let vars = [
            ui.push_style_var(StyleVar::WindowRounding(8.0)),
            ui.push_style_var(StyleVar::ChildRounding(6.0)),
            ui.push_style_var(StyleVar::FrameRounding(4.0)),
            ui.push_style_var(StyleVar::WindowPadding([20.0, 18.0])),
            ui.push_style_var(StyleVar::ItemSpacing([10.0, 8.0])),
            ui.push_style_var(StyleVar::WindowBorderSize(0.0)),
        ];
        let modal = if self.collection {
            self.render_collection(ui, &view)
        } else if let Some(offer) = &view.offer {
            if self.deferred == Some((view.epoch, offer.id)) {
                false
            } else {
                self.render_offer(ui, &view, offer)
            }
        } else {
            false
        };
        drop(vars);
        drop(styles);
        if !modal {
            self.render_hud(ui, &view, true);
        }
        self.finish(ui, modal, safe)
    }

    fn finish(&mut self, ui: &Ui, wants_modal: bool, safe: bool) -> bool {
        // 3.4: the MAX WIN plays over everything, wherever the frame ends.
        if let Some((_, at)) = self.max_win {
            let since = at.elapsed().as_secs_f32();
            // The key stays, so the same offer never plays it twice.
            if since < MAX_WIN_TIME {
                draw_max_win(ui, self.fonts, since);
            }
        }
        if !safe {
            self.last_modal = false;
            self.release_guard = None;
            return self.modal(false);
        }
        if wants_modal {
            self.last_modal = true;
            self.release_guard = None;
            return self.modal(true);
        }
        if self.last_modal {
            self.last_modal = false;
            self.release_guard = Some(Instant::now());
        }
        if let Some(since) = self.release_guard {
            let held = [
                Key::Enter,
                Key::KeypadEnter,
                Key::Escape,
                Key::GamepadFaceDown,
                Key::GamepadFaceRight,
                Key::GamepadBack,
                Key::GamepadDpadLeft,
                Key::GamepadDpadRight,
                Key::GamepadDpadUp,
                Key::GamepadDpadDown,
                Key::LeftArrow,
                Key::RightArrow,
                Key::F7,
            ]
            .iter()
            .any(|key| ui.is_key_down(*key))
                || MouseButton::VARIANTS
                    .iter()
                    .any(|button| ui.is_mouse_down(*button));
            if held && !(ui.io().app_focus_lost && since.elapsed() >= Duration::from_millis(500)) {
                return self.modal(true);
            }
            self.release_guard = None;
        }
        self.modal(false)
    }

    fn modal(&self, value: bool) -> bool {
        self.shared.modal.store(value, Ordering::Release);
        value
    }
    fn acknowledge(&mut self, view: &ProgressView) {
        let complete = match &self.waiting {
            Some(Waiting::Choice {
                epoch,
                offer,
                power,
                target,
            }) => {
                view.epoch != *epoch
                    || (view.levels.get(power).copied().unwrap_or(0) >= *target
                        && view.offer.as_ref().map(|o| o.id) != Some(*offer))
            }
            Some(Waiting::Reroll { epoch, offer }) | Some(Waiting::Action { epoch, offer, .. }) => {
                view.epoch != *epoch || view.offer.as_ref().map(|o| o.id) != Some(*offer)
            }
            None => false,
        };
        if complete {
            self.waiting = None;
            self.selected = None;
            self.binding = None;
            self.error = None;
        } else if self.waiting.is_some() && view.error.is_some() {
            self.waiting = None;
            self.error = view.error.clone();
        }
    }

    fn render_offer(&mut self, ui: &Ui, view: &ProgressView, offer: &Offer) -> bool {
        let n = offer.cards.len();
        if !(3..=4).contains(&n) {
            return false;
        }
        let key = (view.epoch, offer.id);
        if self.binding != Some(key) {
            self.binding = Some(key);
            self.selected = None;
            self.focus = 0;
            self.error = None;
        }
        let now = ui.time();
        if self.revealed.is_none_or(|(e, o, _)| (e, o) != key) {
            self.revealed = Some((key.0, key.1, now));
        }
        let mut shown_for = (now - self.revealed.map(|r| r.2).unwrap_or(now)) as f32;
        let busy = self.waiting.is_some();
        // 0.7: cards are dealt face down and flip one by one (not in the
        // compact layout). Any key or click during the deal finishes it, and
        // that input does nothing else.
        let probe = ui.io().display_size;
        let roomy = (probe[0] - 32.0).max(240.0).min(1400.0) >= 980.0 && (probe[1] - 36.0).max(280.0).min(900.0) >= 640.0;
        let animate = self.ui_settings.card_animations && roomy;
        // 3.4: without the card flips the MAX WIN plays as the offer opens.
        if !animate
            && self.max_win.is_none_or(|(k, _)| k != key)
            && offer.cards.iter().any(|c| is_prismatic(self.shared.library.get(&c.power_id)))
        {
            self.max_win = Some((key, Instant::now()));
            sound::play(Cue::MaxWin);
        }
        let reveal_end = FLIP_START + FLIP_GAP * n.saturating_sub(1) as f32 + FLIP_TIME;
        let mut input_eaten = false;
        if animate && shown_for < reveal_end {
            let skip = ui.is_mouse_clicked(MouseButton::Left)
                || pressed(
                    ui,
                    &[
                        Key::Enter,
                        Key::KeypadEnter,
                        Key::Space,
                        Key::GamepadFaceDown,
                        Key::Alpha1,
                        Key::Alpha2,
                        Key::Alpha3,
                        Key::Alpha4,
                        Key::LeftArrow,
                        Key::RightArrow,
                        Key::GamepadDpadLeft,
                        Key::GamepadDpadRight,
                    ],
                );
            if skip {
                if let Some(revealed) = self.revealed.as_mut() {
                    revealed.2 -= 30.0;
                }
                shown_for += 30.0;
                input_eaten = true;
            }
        }
        if animate {
            self.cue_once(key, 0x80, Some(Cue::Deal));
        } else {
            // One cue for the whole offer: the best card's reveal.
            let best = offer
                .cards
                .iter()
                .filter_map(|c| reveal_cue(c, self.shared.library.get(&c.power_id)))
                .max_by_key(|c| c.priority());
            self.cue_once(key, 0x80, best);
        }
        let supported: Vec<bool> = offer
            .cards
            .iter()
            .map(|card| {
                self.shared
                    .library
                    .get(&card.power_id)
                    .is_some_and(|p| p.required().is_subset(&view.capabilities))
            })
            .collect();
        let selected_at_frame_start = self.selected;
        let cancel = pressed(ui, &[Key::Escape, Key::GamepadFaceRight]);
        if cancel && !busy {
            self.deferred = Some(key);
            return false;
        }
        if !busy && !input_eaten {
            for (index, key) in [Key::Alpha1, Key::Alpha2, Key::Alpha3, Key::Alpha4].iter().enumerate().take(n) {
                if pressed(ui, &[*key]) {
                    self.select(index);
                }
            }
            if pressed(ui, &[Key::LeftArrow, Key::GamepadDpadLeft]) {
                self.select((self.focus.min(n - 1) + n - 1) % n);
            }
            if pressed(ui, &[Key::RightArrow, Key::GamepadDpadRight]) {
                self.select((self.focus.min(n - 1) + 1) % n);
            }
        }
        if self.focus >= n {
            self.focus = 0;
        }
        let confirm_key = !input_eaten && pressed(ui, &[Key::Enter, Key::KeypadEnter, Key::GamepadFaceDown]);
        let reroll_key = pressed(ui, &[Key::R, Key::GamepadFaceUp]);
        let banish_key = pressed(ui, &[Key::X, Key::GamepadFaceLeft]);
        let lock_key = pressed(ui, &[Key::L, Key::GamepadL1]);
        let skip_key = pressed(ui, &[Key::S, Key::GamepadR1]);
        let mut banish_click = false;
        let mut lock_click = false;
        let mut skip_click = false;
        if self
            .skip_armed
            .is_some_and(|(id, until)| id != offer.id || Instant::now() > until)
        {
            self.skip_armed = None;
        }
        let skip_enabled = self.last_hud.skip_enabled;
        // 0.6 Fate cards change the Lock cost and what a skip pays.
        let skip_runes = (f64::from(crate::core::skip_runes(self.last_hud.level.max(1), offer.victory.tier, self.last_hud.ng_plus))
            * view.skip_rate.max(1.0))
        .round() as u32;
        let skip_embers = view.skip_embers.max(crate::core::SKIP_EMBERS);
        let lock_cost = if view.ready { view.lock_cost } else { crate::core::LOCK_COST };
        let locked_here = view.locked.as_deref();
        let can_lock = self.selected.is_some_and(|i| {
            offer.cards.get(i).is_some_and(|c| Some(c.power_id.as_str()) != locked_here)
        }) && view.embers >= lock_cost;
        if self
            .banish_armed
            .is_some_and(|(id, slot, until)| id != offer.id || Some(slot) != self.selected || Instant::now() > until)
        {
            self.banish_armed = None;
        }
        let paid_reroll = offer.rerolls_remaining == 0;
        let can_reroll = offer.distinct_power_count() >= 3
            && (!paid_reroll || view.embers >= crate::core::EMBER_REROLL_COST);
        let can_banish = self.selected.is_some() && view.embers >= crate::core::BANISH_COST;
        let display = ui.io().display_size;
        let width = (display[0] - 32.0).max(240.0).min(1400.0);
        let height = (display[1] - 36.0).max(280.0).min(900.0);
        let compact = width < 980.0 || height < 640.0;
        backdrop(ui, display, offer_glow(offer));
        let mut confirm_click = false;
        let mut double_click = false;
        let mut reroll_click = false;
        let mut defer_click = false;
        let fonts = self.fonts;
        let hud = self.last_hud.clone();
        ui.window("Ascension reward###ascension_cards")
            .position(
                [(display[0] - width) * 0.5, (display[1] - height) * 0.5],
                Condition::Always,
            )
            .size([width, height], Condition::Always)
            .title_bar(false)
            .resizable(false)
            .movable(false)
            .collapsible(false)
            .flags(WindowFlags::NO_SCROLLBAR | WindowFlags::NO_SCROLL_WITH_MOUSE)
            .build(|| {
                // 3.0: an Ascended Boss pays rarer cards (and one more from two affixes).
                let ascended = offer.victory.ascended;
                let extra = if ascended > 0 {
                    format!(
                        "  ·  ASCENDED VICTORY: RARER CARDS{}",
                        if ascended >= 2 { ", ONE MORE CARD" } else { "" }
                    )
                } else if n == 4 {
                    "  ·  RANK REWARD: FOURTH CARD".to_owned()
                } else {
                    String::new()
                };
                header(ui, fonts, "A S C E N S I O N", &format!(
                    "{}  ·  CHOOSE ONE POWER{extra}",
                    tier_name(offer.victory.tier),
                ));
                let finite_pool = offer.distinct_power_count() < n;
                if finite_pool {
                    centered(ui, "Only the final upgrades remain. Repeated cards still grant exactly one level.", MUTED);
                }
                let footer = 190.0;
                let body_height = (ui.content_region_avail()[1] - footer).max(120.0);
                let available = ui.content_region_avail()[0];
                if compact {
                    let tab_width = ((available - 10.0 * (n as f32 - 1.0)) / n as f32).max(50.0);
                    for (index, card) in offer.cards.iter().enumerate() {
                        if index > 0 {
                            ui.same_line();
                        }
                        let name = self
                            .shared
                            .library
                            .get(&card.power_id)
                            .map(|p| p.name.as_str())
                            .unwrap_or("Unavailable power");
                        let label = format!(
                            "{} {}\n{} {}{}###card_tab_{index}",
                            index + 1,
                            name,
                            card_rarity_name(self.shared.library.get(&card.power_id), card.display_rarity),
                            roman(card.target_level),
                            if card.target_level == 5 { "  MAX" } else { "" }
                        );
                        let color = card_color(self.shared.library.get(&card.power_id), card.display_rarity, now as f32);
                        let selected = self.selected == Some(index);
                        let colors = [
                            ui.push_style_color(StyleColor::Border, color),
                            ui.push_style_color(StyleColor::Text, color),
                            ui.push_style_color(StyleColor::Button, tint(color, if selected { 0.25 } else { 0.10 })),
                            ui.push_style_color(StyleColor::ButtonHovered, tint(color, 0.30)),
                            ui.push_style_color(StyleColor::ButtonActive, tint(color, 0.40)),
                        ];
                        let border = ui.push_style_var(StyleVar::FrameBorderSize(if selected { 3.0 } else { 2.0 }));
                        let disabled = ui.begin_disabled(busy);
                        if ui.button_with_size(label, [tab_width, 54.0]) {
                            self.select(index);
                        }
                        drop(disabled);
                        drop(border);
                        drop(colors);
                    }
                    let card = &offer.cards[self.focus.min(n - 1)];
                    let pos = ui.cursor_screen_pos();
                    let size = [available, (body_height - 62.0).max(70.0)];
                    let focused_color = card_color(self.shared.library.get(&card.power_id), card.display_rarity, now as f32);
                    card_frame(&ui.get_window_draw_list(), pos, size, focused_color, true, false, now as f32, 1.0);
                    let art = self.art_for(&card.power_id);
                    let band = if let Some(tile) = art {
                        banner(&ui.get_window_draw_list(), pos, size, tile, size[0], focused_color, 1.0)
                    } else {
                        0.0
                    };
                    ui.child_window("focused_card")
                        .size(size)
                        .build(|| {
                            card_body(ui, fonts, card, self.shared.library.get(&card.power_id), view, &self.shared, &hud, 1.0, band);
                            if !supported[self.focus.min(n - 1)] {
                                ui.text_colored(WARN, "Temporarily unavailable. Your offer is saved.");
                            }
                        });
                } else {
                    let gap = if n == 4 { 16.0 } else { 22.0 };
                    let card_width = ((available - gap * (n as f32 - 1.0)) / n as f32).min(420.0);
                    let start_x = ui.cursor_screen_pos()[0] + (available - card_width * n as f32 - gap * (n as f32 - 1.0)) * 0.5;
                    let top = ui.cursor_screen_pos()[1];
                    for (index, card) in offer.cards.iter().enumerate() {
                        let appear = ease((shown_for - 0.12 * index as f32) / 0.35);
                        let selected = self.selected == Some(index);
                        let lift = if selected { 10.0 } else { 0.0 };
                        let pos = [
                            start_x + index as f32 * (card_width + gap),
                            top + (1.0 - appear) * 40.0 + 12.0 - lift,
                        ];
                        let size = [card_width, body_height - 24.0];
                        ui.set_cursor_screen_pos(pos);
                        let hovered = ui.is_mouse_hovering_rect(pos, [pos[0] + size[0], pos[1] + size[1]]);
                        let color = card_color(self.shared.library.get(&card.power_id), card.display_rarity, now as f32);
                        let art = self.art_for(&card.power_id);
                        // 0.7 flip: 0 face down .. 0.5 edge-on .. 1 face up.
                        let flip_at = FLIP_START + FLIP_GAP * index as f32;
                        let flip = if animate { ((shown_for - flip_at) / FLIP_TIME).clamp(0.0, 1.0) } else { 1.0 };
                        if animate && flip >= 0.5 {
                            let cue = reveal_cue(card, self.shared.library.get(&card.power_id));
                            self.cue_once(key, 1 << index, cue);
                            if is_prismatic(self.shared.library.get(&card.power_id))
                                && self.max_win.is_none_or(|(k, _)| k != key)
                            {
                                self.max_win = Some((key, Instant::now()));
                            }
                            if let Some(strength) = burst_strength(card, self.shared.library.get(&card.power_id)) {
                                reveal_burst(
                                    &ui.get_window_draw_list(),
                                    [pos[0] + size[0] * 0.5, pos[1] + size[1] * 0.42],
                                    size,
                                    color,
                                    shown_for - flip_at - FLIP_TIME * 0.5,
                                    strength,
                                );
                            }
                        }
                        if flip < 1.0 {
                            let squash = (std::f32::consts::PI * flip).cos().abs().max(0.03);
                            let w = size[0] * squash;
                            let p0 = [pos[0] + (size[0] - w) * 0.5, pos[1]];
                            let dl = ui.get_window_draw_list();
                            if flip < 0.5 {
                                let back = self.art.as_ref().filter(|_| self.ui_settings.card_art).and_then(|a| a.back());
                                card_back(&dl, p0, [w, size[1]], back, appear, now as f32);
                            } else {
                                card_frame(&dl, p0, [w, size[1]], color, false, false, now as f32, appear);
                                if let Some(tile) = art {
                                    banner(&dl, p0, [w, size[1]], tile, size[0], color, appear);
                                }
                            }
                            drop(dl);
                            ui.dummy(size);
                            continue;
                        }
                        let text_alpha = if animate { appear.min(ease((shown_for - flip_at - FLIP_TIME) / 0.25)) } else { appear };
                        card_frame(&ui.get_window_draw_list(), pos, size, color, selected, hovered, now as f32, appear);
                        let band = if let Some(tile) = art {
                            banner(&ui.get_window_draw_list(), pos, size, tile, size[0], color, appear)
                        } else {
                            0.0
                        };
                        if let Some(power) = self.shared.library.get(&card.power_id) {
                            let dl = ui.get_window_draw_list();
                            let max = [pos[0] + size[0], pos[1] + size[1]];
                            if power.curse.is_some() {
                                let pulse = 0.55 + 0.25 * (now as f32 * 2.0).sin();
                                dl.add_rect([pos[0] + 3.0, pos[1] + 3.0], [max[0] - 3.0, max[1] - 3.0], alpha(CURSE, pulse * appear))
                                    .thickness(2.0)
                                    .rounding(6.0)
                                    .build();
                            }
                            if power.is_evolution() && card.previous_level == 0 {
                                let pulse = 0.5 + 0.5 * (now as f32 * 3.0).sin().abs();
                                for i in 0..3 {
                                    let grow = 6.0 + i as f32 * 5.0;
                                    dl.add_rect([pos[0] - grow, pos[1] - grow], [max[0] + grow, max[1] + grow], alpha(GOLD, 0.18 * pulse * appear))
                                        .thickness(2.0)
                                        .rounding(10.0 + grow)
                                        .build();
                                }
                            }
                            if view.locked.as_deref() == Some(card.power_id.as_str()) {
                                padlock(&dl, [max[0] - 26.0, pos[1] + 26.0], 9.0, alpha(GOLD, appear));
                            }
                        }
                        let fade = ui.push_style_var(StyleVar::Alpha(text_alpha.max(0.05)));
                        ui.child_window(format!("reward_card_{index}"))
                            .size(size)
                            .flags(WindowFlags::NO_SCROLLBAR)
                            .build(|| {
                                card_body(ui, fonts, card, self.shared.library.get(&card.power_id), view, &self.shared, &hud, text_alpha, band);
                                if !supported[index] {
                                    ui.text_colored(WARN, "Temporarily unavailable. Your offer is saved.");
                                }
                                // Whole card is the select target; double click confirms.
                                let bottom = ui.window_size();
                                ui.set_cursor_pos([0.0, 0.0]);
                                let disabled = ui.begin_disabled(busy || !supported[index]);
                                if ui.invisible_button(format!("select_area_{index}"), bottom) {
                                    self.select(index);
                                }
                                if ui.is_item_hovered() && ui.is_mouse_double_clicked(MouseButton::Left) {
                                    self.select(index);
                                    double_click = true;
                                }
                                drop(disabled);
                                // Key hint at the bottom of the card.
                                let hint = if selected {
                                    format!("SELECTED  [{}]", index + 1)
                                } else {
                                    format!("[{}]  SELECT", index + 1)
                                };
                                let w = ui.calc_text_size(&hint)[0];
                                ui.get_window_draw_list().add_text(
                                    [pos[0] + (size[0] - w) * 0.5, pos[1] + size[1] - 30.0],
                                    alpha(if selected { GOLD } else { MUTED }, text_alpha),
                                    &hint,
                                );
                            });
                        drop(fade);
                    }
                    ui.set_cursor_screen_pos([ui.cursor_screen_pos()[0], top + body_height]);
                }
                ornament_line(ui, alpha(GOLD_DIM, 0.8));
                if busy {
                    centered(
                        ui,
                        match self.waiting {
                            Some(Waiting::Reroll { .. }) => "Drawing new cards...",
                            Some(Waiting::Action { label, .. }) => label,
                            _ => "Saving your card...",
                        },
                        GOLD,
                    );
                }
                // 2.0: one row of two-line buttons (action and key / cost).
                let skip_width = if skip_enabled { 200.0 } else { 0.0 };
                let widths = [230.0, 180.0, 160.0, 160.0, skip_width, 140.0];
                let spacing = 10.0;
                let shown = widths.iter().filter(|w| **w > 0.0).count() as f32;
                let total: f32 = widths.iter().sum::<f32>() + spacing * (shown - 1.0);
                let left = ui.cursor_pos()[0];
                ui.set_cursor_pos([left + ((ui.content_region_avail()[0] - total) * 0.5).max(0.0), ui.cursor_pos()[1]]);
                let tall = 48.0;
                let spaced = |ui: &Ui| ui.same_line_with_spacing(0.0, spacing);
                let gold = [
                    ui.push_style_color(StyleColor::Button, [0.36, 0.28, 0.12, 1.0]),
                    ui.push_style_color(StyleColor::ButtonHovered, [0.52, 0.40, 0.17, 1.0]),
                    ui.push_style_color(StyleColor::ButtonActive, [0.64, 0.50, 0.22, 1.0]),
                ];
                let disabled = ui.begin_disabled(busy || self.selected.is_none() || self.selected.is_some_and(|index| !supported[index]));
                confirm_click = ui.button_with_size("CHOOSE\n[Enter]", [widths[0], tall]);
                drop(disabled);
                drop(gold);
                spaced(ui);
                let disabled = ui.begin_disabled(busy || !can_reroll);
                reroll_click = ui.button_with_size(
                    if paid_reroll {
                        format!("REROLL  [R]\n{} Ember", crate::core::EMBER_REROLL_COST)
                    } else {
                        format!("REROLL  [R]\n{} free", offer.rerolls_remaining)
                    },
                    [widths[1], tall],
                );
                drop(disabled);
                spaced(ui);
                let disabled = ui.begin_disabled(busy || !can_lock);
                lock_click = ui.button_with_size(
                    if lock_cost == 0 { "LOCK  [L]\nfree".to_owned() } else { format!("LOCK  [L]\n{lock_cost} Embers") },
                    [widths[2], tall],
                );
                drop(disabled);
                spaced(ui);
                let armed = self.banish_armed.is_some();
                let red = armed.then(|| {
                    [
                        ui.push_style_color(StyleColor::Button, [0.45, 0.10, 0.08, 1.0]),
                        ui.push_style_color(StyleColor::ButtonHovered, [0.62, 0.14, 0.10, 1.0]),
                    ]
                });
                let disabled = ui.begin_disabled(busy || !can_banish);
                banish_click = ui.button_with_size(
                    if armed { "BANISH?\npress X again".to_owned() } else { format!("BANISH  [X]\n{} Embers", crate::core::BANISH_COST) },
                    [widths[3], tall],
                );
                drop(disabled);
                drop(red);
                if skip_enabled {
                    spaced(ui);
                    let armed = self.skip_armed.is_some();
                    let red = armed.then(|| {
                        [
                            ui.push_style_color(StyleColor::Button, [0.36, 0.22, 0.08, 1.0]),
                            ui.push_style_color(StyleColor::ButtonHovered, [0.50, 0.32, 0.10, 1.0]),
                        ]
                    });
                    let disabled = ui.begin_disabled(busy);
                    skip_click = ui.button_with_size(
                        if armed { "SKIP?\npress S again".to_owned() } else { format!("SKIP  [S]\n+{} runes", grouped(skip_runes)) },
                        [widths[4], tall],
                    );
                    drop(disabled);
                    drop(red);
                }
                spaced(ui);
                let disabled = ui.begin_disabled(busy);
                defer_click = ui.button_with_size("LATER\n[Esc]", [widths[5], tall]);
                drop(disabled);
                let mut note = format!("{} Embers", view.embers);
                if let Some(name) = locked_here.and_then(|id| self.shared.library.get(id)) {
                    note = format!("{note}   ·   Locked: {}", name.name);
                }
                if self.skip_armed.is_some() {
                    note = format!("Skip: no card, +{} runes and +{skip_embers} Ember", grouped(skip_runes));
                }
                if self.banish_armed.is_some() {
                    if let Some(name) = self
                        .selected
                        .and_then(|i| offer.cards.get(i))
                        .and_then(|c| self.shared.library.get(&c.power_id))
                    {
                        note = format!("Banish: {} will never show up again", name.name);
                    }
                }
                centered(ui, &note, alpha(GOLD, 0.85));
                if let Some(error) = self.error.as_ref().or(view.error.as_ref()) {
                    let color = ui.push_style_color(StyleColor::Text, [0.95, 0.40, 0.32, 1.0]);
                    ui.text_wrapped(error);
                    drop(color);
                }
            });
        if !busy {
            if defer_click {
                self.deferred = Some(key);
                return false;
            }
            if (banish_click || banish_key) && can_banish {
                let slot = self.selected.unwrap_or(self.focus);
                if self.banish_armed.is_some_and(|(id, s, _)| id == offer.id && s == slot) {
                    match self.shared.banish(view.epoch, offer.id, slot) {
                        Ok(()) => {
                            self.waiting = Some(Waiting::Action {
                                epoch: view.epoch,
                                offer: offer.id,
                                label: "Banishing the card...",
                            });
                            self.revealed = None;
                            self.error = None;
                        }
                        Err(error) => self.error = Some(error),
                    }
                    self.banish_armed = None;
                } else {
                    self.banish_armed = Some((offer.id, slot, Instant::now() + Duration::from_secs(5)));
                }
                return true;
            }
            if (lock_click || lock_key) && can_lock {
                let slot = self.selected.unwrap_or(self.focus).min(n - 1);
                match self.shared.lock(view.epoch, offer.id, slot) {
                    Ok(()) => {
                        self.waiting = Some(Waiting::Action {
                            epoch: view.epoch,
                            offer: offer.id,
                            label: "Locking the card...",
                        });
                        self.error = None;
                        self.selected = None;
                    }
                    Err(error) => self.error = Some(error),
                }
                return true;
            }
            if (skip_click || skip_key) && skip_enabled {
                if self.skip_armed.is_some_and(|(id, _)| id == offer.id) {
                    match self.shared.skip(view.epoch, offer.id) {
                        Ok(()) => {
                            self.waiting = Some(Waiting::Action {
                                epoch: view.epoch,
                                offer: offer.id,
                                label: "Skipping this reward...",
                            });
                            self.error = None;
                        }
                        Err(error) => self.error = Some(error),
                    }
                    self.skip_armed = None;
                } else {
                    self.skip_armed = Some((offer.id, Instant::now() + Duration::from_secs(5)));
                }
                return true;
            }
            if (reroll_click || reroll_key) && can_reroll {
                match self.shared.reroll(view.epoch, offer.id) {
                    Ok(()) => {
                        self.waiting = Some(Waiting::Reroll {
                            epoch: view.epoch,
                            offer: offer.id,
                        });
                        self.revealed = None;
                        self.error = None;
                    }
                    Err(error) => self.error = Some(error),
                }
            } else if confirm_click || confirm_key || double_click {
                // ImGui navigation can activate SELECT with the same A/Enter
                // press. Require selection to predate the confirmation frame.
                let may_commit = confirm_click
                    || double_click
                    || (selected_at_frame_start.is_some()
                        && selected_at_frame_start == self.selected);
                if let Some(index) = self.selected {
                    if !may_commit {
                        return true;
                    }
                    if !supported[index] {
                        self.error = Some(
                            "This card is temporarily unavailable. Your offer remains saved."
                                .into(),
                        );
                        return true;
                    }
                    let card = &offer.cards[index];
                    match self
                        .shared
                        .choose(view.epoch, offer.id, card.power_id.clone())
                    {
                        Ok(()) => {
                            let evolution = card.previous_level == 0
                                && self.shared.library.get(&card.power_id).is_some_and(|p| p.is_evolution());
                            sound::play(if evolution { Cue::Evolution } else { Cue::Choose });

                            self.waiting = Some(Waiting::Choice {
                                epoch: view.epoch,
                                offer: offer.id,
                                power: card.power_id.clone(),
                                target: card.target_level,
                            });
                            self.error = None;
                        }
                        Err(error) => self.error = Some(error),
                    }
                } else {
                    self.select(self.focus);
                }
            }
        }
        true
    }

    /// 0.7: a small card picture before a list entry (same line).
    fn thumbnail(&self, ui: &Ui, id: &str, strength: f32) {
        let Some(tile) = self.art_for(id) else {
            return;
        };
        let p = ui.cursor_screen_pos();
        let size = [44.0, 22.0];
        let t = tile.crop(size[0] / size[1], 0.42);
        ui.get_window_draw_list()
            .add_image_rounded(t.texture, p, [p[0] + size[0], p[1] + size[1]], 3.0)
            .uv_min(t.uv0)
            .uv_max(t.uv1)
            .col([strength, strength, strength, 1.0])
            .build();
        ui.dummy(size);
        ui.same_line();
    }

    fn select(&mut self, index: usize) {
        self.focus = index.min(3);
        self.selected = Some(self.focus);
        self.error = None;
    }

    fn render_collection(&mut self, ui: &Ui, view: &ProgressView) -> bool {
        if pressed(ui, &[Key::Escape, Key::GamepadFaceRight]) {
            self.collection = false;
            return false;
        }
        let display = ui.io().display_size;
        let width = (display[0] - 48.0).max(240.0).min(1280.0);
        let height = (display[1] - 48.0).max(260.0).min(940.0);
        backdrop(ui, display, GOLD);
        let mut close = false;
        let mut reward = false;
        let fonts = self.fonts;
        let hud = self.last_hud.clone();
        let window_pos = [(display[0] - width) * 0.5, (display[1] - height) * 0.5];
        panel_frame(&ui.get_background_draw_list(), window_pos, [width, height]);
        ui.window("Ascension collection###ascension_collection")
            .position(window_pos, Condition::Always)
            .size([width, height], Condition::Always)
            .title_bar(false)
            .movable(false)
            .resizable(false)
            .collapsible(false)
            .build(|| {
                let levels: u32 = crate::core::total_levels(&view.levels);
                header(
                    ui,
                    fonts,
                    "A S C E N S I O N",
                    &format!(
                        "{} cards  ·  {} card levels  ·  {}  ·  {} Embers",
                        view.levels.len(),
                        levels,
                        crate::core::rank_name(levels),
                        view.embers
                    ),
                );
                if let Some(_bar) = ui.tab_bar("collection_tabs") {
                    for (tab, label) in [
                        (Tab::Cards, "Cards"),
                        (Tab::Paths, "Paths"),
                        (Tab::Codex, "Codex"),
                        (Tab::Combos, "Combos"),
                        (Tab::You, "You"),
                        (Tab::Guide, "Guide"),
                        (Tab::Settings, "Settings"),
                    ] {
                        if let Some(_item) = ui.tab_item(label) {
                            self.tab = tab;
                        }
                    }
                }
                let body = (ui.content_region_avail()[1] - 50.0).max(80.0);
                ui.child_window("collection_body").size([0.0, body]).build(|| match self.tab {
                    Tab::Cards => self.cards_tab(ui, fonts, view, &hud),
                    Tab::Paths => self.paths_tab(ui, fonts, view),
                    Tab::Codex => self.codex_tab(ui, view),
                    Tab::Combos => self.combos_tab(ui, fonts, view, &hud),
                    Tab::You => self.you_tab(ui, fonts, view, &hud),
                    Tab::Guide => guide_tab(ui, fonts),
                    Tab::Settings => self.settings_tab(ui),
                });
                close = ui.button_with_size("Close  [F7]", [180.0, 34.0]);
                if view.offer.is_some() {
                    ui.same_line();
                    reward = ui.button_with_size("Back to reward", [180.0, 34.0]);
                }
            });
        if close {
            self.collection = false;
            return false;
        }
        if reward {
            self.collection = false;
            self.deferred = None;
        }
        true
    }

    /// 2.0: your cards, with the nine Paths on top.
    fn cards_tab(&self, ui: &Ui, fonts: Fonts, view: &ProgressView, hud: &HudView) {
        let library = &self.shared.library;
        path_strip(ui, &crate::paths::points(&view.levels, library));
        ui.dummy([0.0, 6.0]);
        if view.levels.is_empty() {
            ui.text_colored(MUTED, "No cards yet. Beat a boss to choose your first one.");
            return;
        }
        let mut owned: Vec<(&String, &u8)> = view.levels.iter().collect();
        owned.sort_by_key(|(id, level)| {
            let power = library.get(id);
            let rarity = power
                .map(|p| p.level_rarities[usize::from((**level).clamp(1, 5) - 1)])
                .unwrap_or(Rarity::Common);
            (std::cmp::Reverse(rarity), std::cmp::Reverse(**level), power.map(|p| p.name.clone()).unwrap_or_default())
        });
        let available = ui.content_region_avail()[0];
        let columns = ((available / 380.0).floor() as usize).clamp(1, 4);
        let tile_w = (available - 12.0 * (columns as f32 - 1.0)) / columns as f32;
        let tile_h = 158.0;
        for (i, (id, level)) in owned.iter().enumerate() {
            if i % columns != 0 {
                ui.same_line_with_spacing(0.0, 12.0);
            }
            let pos = ui.cursor_screen_pos();
            let Some(power) = library.get(id) else {
                ui.child_window(format!("tile_{id}")).size([tile_w, tile_h]).border(true).build(|| {
                    ui.text_colored(WARN, format!("Unknown card {}", roman(**level)));
                    ui.text_wrapped("Kept in your progress; this version does not know it.");
                });
                continue;
            };
            let level = (**level).clamp(1, 5);
            let rarity = power.level_rarities[usize::from(level - 1)];
            let tile_color = card_color(Some(power), rarity, ui.time() as f32);
            card_frame(&ui.get_window_draw_list(), pos, [tile_w, tile_h], tile_color, false, false, ui.time() as f32, 1.0);
            if let Some(tile) = self.art_for(id) {
                let dl = ui.get_window_draw_list();
                let p0 = [pos[0] + 4.0, pos[1] + 4.0];
                let p1 = [pos[0] + tile_w - 4.0, pos[1] + 58.0];
                let t = tile.crop((p1[0] - p0[0]) / (p1[1] - p0[1]), 0.42);
                dl.add_image_rounded(t.texture, p0, p1, 6.0)
                    .uv_min(t.uv0)
                    .uv_max(t.uv1)
                    .col([1.0, 1.0, 1.0, 0.9])
                    .round_bot_left(false)
                    .round_bot_right(false)
                    .build();
                dl.add_rect_filled_multicolor(p0, p1, alpha(INK, 0.94), alpha(INK, 0.15), alpha(INK, 0.35), alpha(INK, 0.97));
            }
            ui.child_window(format!("tile_{id}")).size([tile_w, tile_h]).flags(WindowFlags::NO_SCROLLBAR).build(|| {
                let dl = ui.get_window_draw_list();
                glyph(&dl, [pos[0] + 30.0, pos[1] + 32.0], 15.0, tile_color, &power.category);
                level_pips(&dl, [pos[0] + tile_w - 112.0, pos[1] + 20.0], power, level, level, ui.time() as f32);
                drop(dl);
                ui.set_cursor_pos([58.0, 12.0]);
                with_font(ui, fonts.heading, || ui.text_colored(tile_color, &power.name));
                ui.set_cursor_pos([58.0, ui.cursor_pos()[1] - 4.0]);
                match power.family {
                    Some(path) => ui.text_colored(path.color(), format!("{}  ·  {}", path.name().to_uppercase(), card_rarity_name(Some(power), rarity))),
                    None => ui.text_colored(MUTED, card_rarity_name(Some(power), rarity)),
                }
                ui.set_cursor_pos([14.0, 64.0]);
                let wrap = ui.push_text_wrap_pos_with_pos(tile_w - 14.0);
                ui.text(&power.levels[usize::from(level - 1)].description);
                if let Some(curse) = &power.curse {
                    ui.text_colored(CURSE, format!("Curse: {curse}"));
                }
                if let Some(estimate) = proc_estimate(power, level, &self.shared, hud) {
                    ui.text_colored(GOLD, estimate);
                }
                if !power.required().is_subset(&view.capabilities) {
                    ui.text_colored(WARN, "Inactive right now.");
                }
                drop(wrap);
            });
        }
    }

    /// 2.0: the nine Paths, their points and bonuses.
    fn paths_tab(&self, ui: &Ui, fonts: Fonts, view: &ProgressView) {
        use crate::paths::{PATHS, TIERS};
        let library = &self.shared.library;
        let points = crate::paths::points(&view.levels, library);
        ui.text_colored(MUTED, "Every card belongs to a Path. Each card level is 1 point. 5, 10 and 15 points unlock a bonus; 6 opens the Path's two Keystone cards.");
        ui.dummy([0.0, 4.0]);
        for path in PATHS {
            let have = points[path.index()];
            let color = path.color();
            with_font(ui, fonts.heading, || ui.text_colored(if have > 0 { color } else { alpha(color, 0.6) }, path.name()));
            ui.same_line();
            ui.text_colored(MUTED, format!("{}  ·  {have} points", path.theme()));
            let w = ui.content_region_avail()[0].min(560.0);
            let p = ui.cursor_screen_pos();
            {
                let dl = ui.get_window_draw_list();
                let y = p[1] + 3.0;
                dl.add_rect([p[0], y], [p[0] + w, y + 6.0], [0.16, 0.15, 0.12, 1.0]).filled(true).rounding(3.0).build();
                let share = (f32::from(have.min(TIERS[2])) / f32::from(TIERS[2])).clamp(0.0, 1.0);
                if share > 0.0 {
                    dl.add_rect([p[0], y], [p[0] + w * share, y + 6.0], color).filled(true).rounding(3.0).build();
                }
                for t in TIERS {
                    let x = p[0] + w * f32::from(t) / f32::from(TIERS[2]);
                    diamond(&dl, [x.min(p[0] + w - 5.0), y + 3.0], 5.0, if have >= t { GOLD } else { MUTED }, have >= t);
                }
            }
            ui.dummy([w, 14.0]);
            for bonus in path.bonuses() {
                let on = have >= bonus.points;
                let x0 = ui.cursor_pos()[0];
                ui.text_colored(if on { GOLD } else { alpha(MUTED, 0.8) }, format!("{:>2}   {}", bonus.points, bonus.name));
                ui.same_line_with_pos(x0 + 190.0);
                ui.text_colored(if on { PAPER } else { alpha(MUTED, 0.8) }, bonus.text);
            }
            // 3.0: the Path's Keystones and when they open.
            let keystones: Vec<&str> = library
                .powers
                .iter()
                .filter(|p| p.category == "keystone" && p.family == Some(path))
                .map(|p| p.name.as_str())
                .collect();
            if !keystones.is_empty() {
                let gate = crate::paths::KEYSTONE_GATE;
                let open = have >= gate;
                let x0 = ui.cursor_pos()[0];
                ui.text_colored(if open { GOLD } else { alpha(MUTED, 0.8) }, format!("{gate:>2}   Keystones"));
                ui.same_line_with_pos(x0 + 190.0);
                ui.text_colored(
                    if open { PAPER } else { alpha(MUTED, 0.8) },
                    format!("{} can show up in rewards", keystones.join(" and ")),
                );
            }
            let names: Vec<String> = view
                .levels
                .iter()
                .filter_map(|(id, level)| {
                    library
                        .get(id)
                        .filter(|p| !p.hidden && p.family == Some(path))
                        .map(|p| format!("{} {}", p.name, roman(*level)))
                })
                .collect();
            if !names.is_empty() {
                let wrap = ui.push_text_wrap_pos_with_pos(0.0);
                ui.text_colored(alpha(color, 0.8), format!("Your cards: {}", names.join(", ")));
                drop(wrap);
            }
            ui.dummy([0.0, 8.0]);
        }
    }

    /// Every card in the game, grouped by Path.
    fn codex_tab(&mut self, ui: &Ui, view: &ProgressView) {
        let library = &self.shared.library;
        for (i, label) in ["All", "Owned", "Not owned"].iter().enumerate() {
            if i > 0 {
                ui.same_line();
            }
            if ui.radio_button_bool(*label, self.codex_filter == i as u8) {
                self.codex_filter = i as u8;
            }
        }
        let total = library.powers.iter().filter(|p| !p.hidden).count();
        let found = library.powers.iter().filter(|p| !p.hidden && view.levels.contains_key(&p.id)).count();
        ui.same_line();
        ui.text_colored(MUTED, format!("     {found} of {total} cards owned"));
        ui.separator();
        let mut powers: Vec<&Power> = library.powers.iter().filter(|p| !p.hidden).collect();
        powers.sort_by_key(|p| (p.family.map(|f| f.index()).unwrap_or(99), p.base_rarity, p.name.clone()));
        let mut group: Option<Option<crate::paths::Path>> = None;
        for power in powers {
            let level = view.levels.get(&power.id).copied().unwrap_or(0);
            if (self.codex_filter == 1 && level == 0) || (self.codex_filter == 2 && level > 0) {
                continue;
            }
            if group != Some(power.family) {
                group = Some(power.family);
                ui.dummy([0.0, 6.0]);
                match power.family {
                    Some(path) => {
                        ui.text_colored(path.color(), path.name().to_uppercase());
                        ui.same_line();
                        ui.text_colored(MUTED, path.theme());
                    }
                    None => ui.text_colored(GOLD, "OTHER"),
                }
            }
            let color = card_color(Some(power), power.base_rarity, ui.time() as f32);
            self.thumbnail(ui, &power.id, if level > 0 { 1.0 } else { 0.5 });
            ui.text_colored(if level > 0 { color } else { alpha(color, 0.7) }, &power.name);
            ui.same_line();
            ui.text_colored(
                MUTED,
                if power.category == "prismatic" {
                    "PRISMATIC  ·  ultra rare".to_owned()
                } else {
                    format!("{} -> {}", rarity_name(power.base_rarity), rarity_name(power.level_rarities[4]))
                },
            );
            if level > 0 {
                ui.same_line();
                ui.text_colored(GOLD, format!("owned {}", roman(level)));
            }
            ui.indent_by(54.0);
            let wrap = ui.push_text_wrap_pos_with_pos(0.0);
            if !power.signature.is_empty() {
                ui.text_colored(GOLD, format!("Signature card of {}", self.boss_list(&power.signature)));
            }
            if power.is_evolution() {
                ui.text_colored(alpha(rarity_color(Rarity::Mythic), 0.9), format!("Evolves from {}", self.parents(power)));
            }
            // 3.0: Keystones open with their Path.
            if let (true, Some(path)) = (power.category == "keystone", power.family) {
                let have = crate::paths::points(&view.levels, &self.shared.library)[path.index()];
                let gate = crate::paths::KEYSTONE_GATE;
                if have >= gate {
                    ui.text_colored(path.color(), format!("Keystone of {}: can show up in rewards now", path.name()));
                } else {
                    ui.text_colored(MUTED, format!("Keystone of {}: shows up at {gate} {} points (you have {have})", path.name(), path.name()));
                }
            }
            ui.text_colored(PAPER, format!("I: {}", power.levels[0].description));
            ui.text_colored(MUTED, format!("V: {}", power.levels[4].description));
            if let Some(curse) = &power.curse {
                ui.text_colored(CURSE, format!("Curse: {curse}"));
            }
            drop(wrap);
            ui.unindent_by(54.0);
        }
    }

    /// Synergies (owned card sets) and evolutions (two cards at V).
    fn combos_tab(&self, ui: &Ui, fonts: Fonts, view: &ProgressView, hud: &HudView) {
        let library = &self.shared.library;
        let mut rows: Vec<(&Synergy, crate::synergies::Progress)> = self
            .shared
            .synergies
            .synergies
            .iter()
            .map(|s| (s, s.progress(&view.levels, &hud.attributes, library)))
            .collect();
        rows.sort_by_key(|(s, p)| (!p.complete, std::cmp::Reverse(p.met * 100 / p.needed.max(1)), s.name.clone()));
        let complete = rows.iter().filter(|(_, p)| p.complete).count();
        with_font(ui, fonts.heading, || ui.text_colored(GOLD, "Synergies"));
        ui.same_line();
        ui.text_colored(MUTED, format!("{complete} of {} active  ·  own the cards and the bonus turns on by itself", rows.len()));
        for (s, progress) in rows {
            let color = if progress.complete { rarity_color(s.rarity) } else { alpha(MUTED, 0.9) };
            ui.dummy([0.0, 2.0]);
            ui.text_colored(color, &s.name);
            ui.same_line();
            ui.text_colored(
                if progress.complete { GOLD } else { MUTED },
                if progress.complete { "ACTIVE".to_owned() } else { format!("{}/{}", progress.met, progress.needed) },
            );
            ui.indent_by(18.0);
            let wrap = ui.push_text_wrap_pos_with_pos(0.0);
            ui.text(&s.description);
            let mut needs: Vec<String> = s
                .all
                .iter()
                .map(|(id, level)| format!("{} {}", library.get(id).map(|p| p.name.as_str()).unwrap_or(id), roman(*level)))
                .collect();
            if let Some(any) = &s.any_of {
                let names: Vec<&str> = any.ids.iter().map(|id| library.get(id).map(|p| p.name.as_str()).unwrap_or(id)).collect();
                needs.push(format!("any {} of: {}", any.count, names.join(", ")));
            }
            if !needs.is_empty() {
                ui.text_colored(MUTED, format!("Needs: {}", needs.join(" + ")));
            }
            drop(wrap);
            ui.unindent_by(18.0);
        }
        ui.dummy([0.0, 10.0]);
        let evolutions: Vec<&Power> = library.powers.iter().filter(|p| p.is_evolution() && !p.hidden).collect();
        let owned = evolutions.iter().filter(|p| view.levels.contains_key(&p.id)).count();
        with_font(ui, fonts.heading, || ui.text_colored(rarity_color(Rarity::Mythic), "Evolutions"));
        ui.same_line();
        ui.text_colored(MUTED, format!("{owned} of {} awakened  ·  raise both parent cards to V", evolutions.len()));
        for power in evolutions {
            let level = view.levels.get(&power.id).copied().unwrap_or(0);
            let ready = power.requires.iter().all(|(id, need)| view.levels.get(id).copied().unwrap_or(0) >= *need);
            ui.dummy([0.0, 2.0]);
            self.thumbnail(ui, &power.id, if level > 0 || ready { 1.0 } else { 0.5 });
            let mythic = rarity_color(Rarity::Mythic);
            ui.text_colored(if level > 0 || ready { mythic } else { alpha(mythic, 0.6) }, &power.name);
            ui.same_line();
            if level > 0 {
                ui.text_colored(GOLD, format!("owned {}", roman(level)));
            } else if ready {
                ui.text_colored(GOLD, "in your next reward");
            }
            ui.indent_by(54.0);
            let parts: Vec<String> = power
                .requires
                .iter()
                .map(|(id, need)| {
                    let have = view.levels.get(id).copied().unwrap_or(0);
                    format!("{} {}/{}", library.get(id).map(|p| p.name.as_str()).unwrap_or(id), roman(have), roman(*need))
                })
                .collect();
            ui.text_colored(MUTED, parts.join("   "));
            let wrap = ui.push_text_wrap_pos_with_pos(0.0);
            ui.text(&power.levels[0].description);
            drop(wrap);
            ui.unindent_by(54.0);
        }
    }

    /// 2.0: what your cards add up to, what your attributes add, and the run.
    fn you_tab(&self, ui: &Ui, fonts: Fonts, view: &ProgressView, hud: &HudView) {
        use crate::powers::{Activation, EffectTrigger, Modifier, aggregate_modifiers};
        let library = &self.shared.library;
        let points = crate::paths::points(&view.levels, library);
        let mut steady: Vec<Modifier> = Vec::new();
        let mut situational = 0;
        let mut add = |entry: &crate::powers::PowerLevel, trigger: EffectTrigger| {
            if entry.modifiers.is_empty() {
                return;
            }
            if trigger == EffectTrigger::Passive && matches!(entry.activation, Activation::Always) {
                steady.extend_from_slice(&entry.modifiers);
            } else {
                situational += 1;
            }
        };
        for (id, level) in &view.levels {
            if let Some(power) = library.get(id) {
                add(&power.levels[usize::from((*level).clamp(1, 5) - 1)], power.trigger);
            }
        }
        for id in crate::paths::hidden_powers(&points) {
            if let Some(power) = library.get(id) {
                add(&power.levels[0], power.trigger);
            }
        }
        for synergy in &self.shared.synergies.synergies {
            if synergy.progress(&view.levels, &hud.attributes, library).complete {
                if synergy.activation.as_ref().is_none_or(|a| matches!(a, Activation::Always)) {
                    steady.extend_from_slice(&synergy.modifiers);
                } else {
                    situational += 1;
                }
            }
        }
        let totals = aggregate_modifiers(steady.iter());
        with_font(ui, fonts.heading, || ui.text_colored(GOLD, "Your attributes"));
        ui.same_line();
        ui.text_colored(MUTED, "level them at a Site of Grace; each one does more than in the base game");
        ui.dummy([0.0, 2.0]);
        attribute_list(ui, hud.attributes, None, self.shared.attribute_strength, true, hud.attribute_boost);
        if hud.attribute_boost.iter().any(|b| *b > 0) {
            ui.text_colored(alpha(BOOSTED, 0.9), "Blue: raised by a talisman, a physick tear, a Great Rune or a card. The bonuses above already count it.");
        }
        ui.dummy([0.0, 8.0]);
        with_font(ui, fonts.heading, || ui.text_colored(GOLD, "Always on from your cards"));
        let lines: Vec<String> = BONUS_ORDER
            .iter()
            .filter_map(|field| totals.iter().find(|m| m.field == *field).and_then(|m| bonus_line(m.field, m.value)))
            .collect();
        if lines.is_empty() {
            ui.text_colored(MUTED, "Nothing yet.");
        }
        let half = lines.len().div_ceil(2);
        let x0 = ui.cursor_pos()[0];
        let top = ui.cursor_pos()[1];
        let mut first_column_end = top;
        for (i, line) in lines.iter().enumerate() {
            if i == half {
                first_column_end = ui.cursor_pos()[1];
                ui.set_cursor_pos([x0 + 420.0, top]);
            } else if i > half {
                ui.set_cursor_pos([x0 + 420.0, ui.cursor_pos()[1]]);
            }
            ui.text(line);
        }
        if lines.len() > half {
            let y = ui.cursor_pos()[1].max(first_column_end);
            ui.set_cursor_pos([x0, y]);
        }
        let mut extra: Vec<String> = Vec::new();
        if hud.attack_speed > 0.001 {
            extra.push(format!("Attacks {:.0}% faster", hud.attack_speed * 100.0));
        }
        if hud.item_speed > 0.001 {
            extra.push(format!("Flasks and items {:.0}% faster", hud.item_speed * 100.0));
        }
        if hud.lifesteal > 0.0001 {
            extra.push(format!("Heal {:.1}% of the damage you deal", hud.lifesteal * 100.0));
        }
        if hud.ally_boost.iter().any(|v| *v > 0.001) {
            extra.push(format!(
                "Allies: +{:.0}% damage, +{:.0}% HP",
                hud.ally_boost[0] * 100.0,
                hud.ally_boost[1] * 100.0
            ));
        }
        if !extra.is_empty() {
            ui.text_colored(PAPER, format!("In total (cards and attributes): {}", extra.join("   ·   ")));
        }
        if situational > 0 {
            ui.text_colored(MUTED, format!("+ {situational} more that switch on in the right moment (low HP, after a dodge, in boss fights...)."));
        }
        ui.dummy([0.0, 8.0]);
        with_font(ui, fonts.heading, || ui.text_colored(GOLD, "Your run"));
        let line = |label: &str, value: String| {
            let x0 = ui.cursor_pos()[0];
            ui.text_colored(MUTED, label);
            ui.same_line_with_pos(x0 + 210.0);
            ui.text(value);
        };
        let total = crate::core::total_levels(&view.levels);
        line(
            "Rank",
            format!(
                "{}  ({total} card levels){}",
                crate::core::rank_name(total),
                crate::core::next_rank(total)
                    .map(|(name, at)| format!("  ·  {name} at {at}"))
                    .unwrap_or_default()
            ),
        );
        line(
            "Embers",
            format!(
                "{}  ·  reroll {}, lock {}, banish {}",
                view.embers,
                crate::core::EMBER_REROLL_COST,
                if view.ready { view.lock_cost } else { crate::core::LOCK_COST },
                crate::core::BANISH_COST
            ),
        );
        let w = &self.shared.world;
        let t = f32::from(hud.tide.min(10));
        line(
            "The Tide",
            format!(
                "{}  ·  enemies +{:.0}% HP, +{:.0}% damage  ·  runes x{:.2}  ·  rises every {} card levels",
                roman_tide(hud.tide),
                (w.enemy_hp * (1.0 + w.tide_hp * t) - 1.0) * 100.0,
                (w.enemy_damage * (1.0 + w.tide_damage * t) - 1.0) * 100.0,
                w.rune_multiplier * (1.0 + w.tide_runes * t),
                w.tide_step
            ),
        );
        line("Rewards", format!("{} chosen  ·  {} waiting  ·  {} skipped", view.total_choices, view.pending, view.skipped));
        if let Some(locked) = view.locked.as_ref() {
            line("Locked card", library.get(locked).map(|p| p.name.clone()).unwrap_or_else(|| locked.clone()));
        }
        if !view.banished.is_empty() {
            line("Banished", format!("{} cards", view.banished.len()));
        }
        if hud.intervention_armed || hud.intervention_cooldown > 0.0 {
            line(
                "Grace",
                if hud.intervention_armed {
                    "ready: survives one deadly blow".into()
                } else {
                    format!("back in {}", clock(hud.intervention_cooldown))
                },
            );
        }
        if !hud.cycle_affixes.is_empty() {
            let names: Vec<String> = hud.cycle_affixes.iter().map(|a| affix_label(*a, hud.adaptive_element, true)).collect();
            line(&format!("NG+{} bosses", hud.ng_plus), names.join("  ·  "));
        }
        // 3.1: every active effect's damage bonus (cards, buffs, tears,
        // talismans); card spells get it too.
        {
            const ELEMENTS: [&str; 5] = ["physical", "magic", "fire", "lightning", "holy"];
            let parts: Vec<String> = ELEMENTS
                .iter()
                .zip(hud.damage_rates)
                .filter(|(_, rate)| *rate > 0.0 && (rate - 1.0).abs() >= 0.005)
                .map(|(name, rate)| format!("{name} {:+.0}%", (rate - 1.0) * 100.0))
                .collect();
            if !parts.is_empty() {
                line("Damage now", format!("{}  ·  weapon and card spells", parts.join("  ·  ")));
            }
        }
        // 3.0 Ascended Bosses.
        if hud.tide > 0 {
            let chance = (20 + 6 * u32::from(hud.tide.saturating_sub(1))).min(80);
            line(
                "Ascended Bosses",
                format!("about {chance}% of bosses rise with affixes (Major and Remembrance: +10%)  ·  beat one for rarer cards and Embers"),
            );
        }
        let r = &hud.reflexes;
        line(
            "This session",
            format!(
                "{} perfect dodges  ·  {} parries  ·  {} criticals  ·  {} stance breaks  ·  {} elites slain",
                r.perfect_dodges, r.parries, r.criticals, r.stance_breaks, hud.elites_killed
            ),
        );
        if !hud.procs.is_empty() {
            ui.dummy([0.0, 8.0]);
            with_font(ui, fonts.heading, || ui.text_colored(GOLD, "Card spells"));
            for status in &hud.procs {
                let name = library.get(&status.power).map(|p| p.name.as_str()).unwrap_or(&status.power);
                line(name, format!("cast {} times  ·  last hit {}", status.fired, status.last_damage));
            }
        }
        if !hud.status_buildup.is_empty() {
            ui.dummy([0.0, 8.0]);
            with_font(ui, fonts.heading, || ui.text_colored(GOLD, "Status on your weapon"));
            let parts: Vec<String> = hud
                .status_buildup
                .iter()
                .map(|(status, value, _)| format!("{} +{value}", status.replace('_', " ")))
                .collect();
            ui.text(parts.join("   ·   "));
        }
        // 3.0 Chronicle: the bosses you beat and the card each one gave.
        if !view.chronicle.is_empty() {
            ui.dummy([0.0, 8.0]);
            with_font(ui, fonts.heading, || ui.text_colored(GOLD, "Chronicle"));
            ui.same_line();
            ui.text_colored(MUTED, format!("{} rewards taken, newest first", view.chronicle.len()));
            const SHOWN: usize = 40;
            for (n, record) in view.chronicle.iter().enumerate().rev().take(SHOWN) {
                let boss = self
                    .shared
                    .boss_names
                    .get(&record.victory.key.boss_id)
                    .map(String::as_str)
                    .unwrap_or(record.victory.key.boss_id.as_str());
                let (name, color) = library
                    .get(&record.power_id)
                    .map(|p| (p.name.as_str(), rarity_color(p.level_rarities[usize::from(record.level.clamp(1, 5) - 1)])))
                    .unwrap_or((record.power_id.as_str(), PAPER));
                let x0 = ui.cursor_pos()[0];
                ui.text_colored(MUTED, format!("{:>3}", n + 1));
                ui.same_line_with_pos(x0 + 40.0);
                let cycle = record.victory.key.cycle;
                ui.text(if cycle > 0 { format!("{boss}  (NG+{cycle})") } else { boss.to_owned() });
                ui.same_line_with_pos(x0 + 360.0);
                ui.text_colored(color, format!("{name} {}", roman(record.level)));
                if record.victory.ascended > 0 {
                    ui.same_line();
                    ui.text_colored([0.95, 0.42, 0.30, 1.0], format!("ascended ×{}", record.victory.ascended));
                }
            }
            if view.chronicle.len() > SHOWN {
                ui.text_colored(MUTED, format!("and {} earlier", view.chronicle.len() - SHOWN));
            }
        }
    }

    /// 1.0: Settings, applied at once and saved to config/ascension.toml.
    fn settings_tab(&mut self, ui: &Ui) {
        ui.text_colored(MUTED, "Changes apply at once and are saved.");
        ui.dummy([0.0, 4.0]);
        let mut changed = false;
        let mut sound_changed = false;
        let s = &mut self.ui_settings;
        changed |= ui.checkbox("Card pictures", &mut s.card_art);
        changed |= ui.checkbox("Card flip animation", &mut s.card_animations);
        if ui.checkbox("Menu sounds", &mut s.sounds) {
            changed = true;
            sound_changed = true;
        }
        if ui.checkbox("Combat sounds (perfect dodge, parry, critical)", &mut s.reflex_sounds) {
            changed = true;
            sound_changed = true;
        }
        let mut volume = s.sound_volume;
        ui.set_next_item_width(320.0);
        ui.slider("Volume", 0.0, 1.0, &mut volume);
        if (volume - s.sound_volume).abs() > f32::EPSILON {
            s.sound_volume = volume;
        }
        if ui.is_item_deactivated_after_edit() {
            changed = true;
            sound_changed = true;
        }
        ui.checkbox("Combat tracker on screen (F6)", &mut self.hud_visible);
        if sound_changed {
            crate::sound::configure(&self.ui_settings);
            crate::sound::play(Cue::RevealRare);
        }
        if changed {
            self.settings_note = Some(match crate::config::save_ui_settings(&self.shared.root, &self.ui_settings) {
                Ok(()) => ("Saved.".to_owned(), true),
                Err(error) => (error, false),
            });
        }
        if let Some((note, ok)) = &self.settings_note {
            ui.text_colored(if *ok { rarity_color(Rarity::Uncommon) } else { WARN }, note);
        }
        ui.dummy([0.0, 8.0]);
        ui.text_colored(MUTED, "Difficulty and runes: [world] and [balance] in config/ascension.toml (read when the game starts).");
    }

    /// 2.1: a read-only panel on the right of the game's Level Up page: what
    /// every attribute adds on top of the base game, with your numbers.
    fn render_attribute_panel(&self, ui: &Ui) {
        let hud = self.last_hud.clone();
        let now = hud.attributes;
        // 2.2: the page's points before you confirm them; without them, what
        // one more point in each attribute adds.
        let pending = self
            .shared
            .level_preview
            .lock()
            .ok()
            .and_then(|p| *p)
            .filter(|p| *p != now);
        let next = |v: u32| (v + 1).min(99);
        let after = pending.unwrap_or(AttributeValues {
            vigor: next(now.vigor),
            mind: next(now.mind),
            endurance: next(now.endurance),
            strength: next(now.strength),
            dexterity: next(now.dexterity),
            intelligence: next(now.intelligence),
            faith: next(now.faith),
            arcane: next(now.arcane),
        });
        let display = ui.io().display_size;
        let width = (display[0] * 0.30).clamp(400.0, 560.0);
        let styles = [
            ui.push_style_color(StyleColor::WindowBg, [0.04, 0.04, 0.035, 0.92]),
            ui.push_style_color(StyleColor::Border, alpha(GOLD_DIM, 0.8)),
            ui.push_style_color(StyleColor::Text, PAPER),
            ui.push_style_color(StyleColor::Separator, alpha(GOLD_DIM, 0.6)),
        ];
        let rounding = ui.push_style_var(StyleVar::WindowRounding(8.0));
        let padding = ui.push_style_var(StyleVar::WindowPadding([16.0, 12.0]));
        let spacing = ui.push_style_var(StyleVar::ItemSpacing([6.0, 2.0]));
        ui.window("Attribute bonuses###ascension_level_up")
            .position([display[0] - width - 28.0, (display[1] * 0.08).max(16.0)], Condition::Always)
            .size([width, 0.0], Condition::Always)
            .title_bar(false)
            .resizable(false)
            .movable(false)
            .collapsible(false)
            .focus_on_appearing(false)
            .flags(WindowFlags::NO_INPUTS | WindowFlags::NO_NAV | WindowFlags::NO_SCROLLBAR)
            .build(|| {
                with_font(ui, self.fonts.heading, || ui.text_colored(GOLD, "ATTRIBUTE BONUSES"));
                if pending.is_some() {
                    ui.text_colored(GAIN, "Before you confirm: what your new points change (green = gain).");
                } else {
                    ui.text_colored(GAIN, "Green: what one more point adds.");
                }
                ui.separator();
                attribute_list(ui, now, Some((after, pending.is_some())), self.shared.attribute_strength, false, hud.attribute_boost);
            });
        drop(spacing);
        drop(padding);
        drop(rounding);
        drop(styles);
    }

    /// "Godrick the Grafted" or "Rennala, Queen of the Full Moon and 1 more".
    fn boss_list(&self, ids: &[String]) -> String {
        let names: Vec<&str> = ids
            .iter()
            .map(|id| self.shared.boss_names.get(id).map(String::as_str).unwrap_or(id))
            .collect();
        match names.as_slice() {
            [] => String::new(),
            [one] => (*one).to_owned(),
            [first, rest @ ..] => format!("{first} and {} more", rest.len()),
        }
    }
    fn parents(&self, power: &Power) -> String {
        power
            .requires
            .iter()
            .map(|(id, level)| format!("{} {}", self.shared.library.get(id).map(|p| p.name.as_str()).unwrap_or(id), roman(*level)))
            .collect::<Vec<_>>()
            .join(" + ")
    }

    /// Compact combat tracker: spell powers with cooldown sweeps, divine
    /// intervention, hit streak, and a reminder for a postponed reward.
    fn render_hud(&self, ui: &Ui, view: &ProgressView, safe: bool) {
        if !self.hud_visible {
            return;
        }
        let hud = &self.last_hud;
        if !hud.hud_ok {
            return;
        }
        let reward_waiting = view.offer.is_some()
            && (self.deferred == view.offer.as_ref().map(|o| (view.epoch, o.id)) || !safe);
        let intervention_owned = hud.intervention_armed || hud.intervention_cooldown > 0.0;
        let toast = [hud.ember_gain, self.ember_toast]
            .into_iter()
            .flatten()
            .filter(|(_, at)| at.elapsed() < Duration::from_secs(4))
            .max_by_key(|(_, at)| *at);
        let rune_toast = hud.rune_gain.filter(|(_, at)| at.elapsed() < Duration::from_secs(5));
        let affix_line = hud.boss_fight && !hud.boss_affixes.is_empty();
        // 2.3: a combat label (centre) and a banner (top) for the moments
        // that matter, Elden Ring style (moments.rs, draw_flash).
        if let Some(banner) = hud.banner.as_ref().filter(|f| f.alive()) {
            draw_flash(ui, self.fonts, banner);
        }
        if let Some(flash) = hud.flash.as_ref().filter(|f| f.alive()) {
            draw_flash(ui, self.fonts, flash);
        }
        // 1.0: a one-time hint (top centre, 10 s).
        if let Some((text, at)) = hud.hint.as_ref().filter(|(_, at)| at.elapsed() < Duration::from_secs(10)) {
            let display = ui.io().display_size;
            let dl = ui.get_background_draw_list();
            let fade = 1.0 - (at.elapsed().as_secs_f32() / 10.0).clamp(0.0, 1.0).powi(3);
            let w = ui.calc_text_size(text)[0];
            let at = [display[0] * 0.5 - w * 0.5, display[1] * 0.12];
            dl.add_rect([at[0] - 14.0, at[1] - 8.0], [at[0] + w + 14.0, at[1] + 26.0], [0.02, 0.02, 0.02, 0.7 * fade])
                .filled(true)
                .rounding(6.0)
                .build();
            dl.add_text(at, alpha(GOLD, fade), text);
        }
        if hud.procs.is_empty()
            && !intervention_owned
            && !reward_waiting
            && hud.streak < 3
            && hud.elites_nearby.is_empty()
            && toast.is_none()
            && rune_toast.is_none()
            && !affix_line
            && !(hud.tide > 0 && hud.in_combat)
        {
            return;
        }
        let display = ui.io().display_size;
        let dl = ui.get_background_draw_list();
        let t = ui.time() as f32;
        let mut x = display[0] - 28.0;
        let y = 34.0;
        let r = 17.0;
        for status in hud.procs.iter().rev() {
            let Some(power) = self.shared.library.get(&status.power) else {
                continue;
            };
            let level = view.levels.get(&status.power).copied().unwrap_or(1).clamp(1, 5);
            let color = card_color(Some(power), power.level_rarities[usize::from(level - 1)], ui.time() as f32);
            let center = [x - r, y + r];
            let ready = status.ready_in <= 0.0 || status.period <= 0.0;
            dl.add_circle(center, r + 3.0, [0.0, 0.0, 0.0, 0.55]).filled(true).build();
            // 0.7: the card's picture in the round icon, the glyph without art.
            let art = self.art_for(&status.power);
            if let Some(tile) = art {
                let t = tile.crop(1.0, 0.42);
                let shade = if ready { 1.0 } else { 0.55 };
                dl.add_image_rounded(t.texture, [center[0] - r, center[1] - r], [center[0] + r, center[1] + r], r)
                    .uv_min(t.uv0)
                    .uv_max(t.uv1)
                    .col([shade, shade, shade, 1.0])
                    .build();
            } else {
                dl.add_circle(center, r, tint(color, 0.35)).filled(true).build();
            }
            if !ready {
                let share = (status.ready_in / status.period).clamp(0.0, 1.0);
                sweep(&dl, center, r, share, [0.0, 0.0, 0.0, 0.62]);
            }
            if art.is_none() {
                glyph(&dl, center, r * 0.62, if ready { color } else { alpha(color, 0.6) }, &power.category);
            }

            dl.add_circle(center, r, alpha(color, if ready { 0.9 + 0.1 * (t * 4.0).sin() } else { 0.55 }))
                .thickness(2.0)
                .num_segments(32)
                .build();
            if let Some((have, need)) = status.progress {
                let label = format!("{have}/{need}");
                let w = ui.calc_text_size(&label)[0];
                let at = [center[0] - w * 0.5, center[1] + r + 3.0];
                dl.add_text([at[0] + 1.0, at[1] + 1.0], [0.0, 0.0, 0.0, 0.85], &label);
                dl.add_text(at, if have >= need { GOLD } else { PAPER }, &label);
            }
            x -= 2.0 * r + 10.0;
        }
        let mut line_y = y + 2.0 * r + if hud.procs.iter().any(|p| p.progress.is_some()) { 34.0 } else { 14.0 };
        let right = display[0] - 28.0;
        let mut text_right = |text: &str, color: [f32; 4]| {
            let w = ui.calc_text_size(text)[0];
            dl.add_text([right - w + 1.0, line_y + 1.0], [0.0, 0.0, 0.0, 0.8], text);
            dl.add_text([right - w, line_y], color, text);
            line_y += 22.0;
        };
        if intervention_owned {
            if hud.intervention_armed {
                text_right("Grace: ready", alpha(GOLD, 0.85 + 0.15 * (t * 2.0).sin()));
            } else {
                text_right(&format!("Grace: {}", clock(hud.intervention_cooldown)), MUTED);
            }
        }
        if hud.streak >= 3 {
            text_right(&format!("Streak x{}", hud.streak), PAPER);
        }
        for (affixes, distance) in &hud.elites_nearby {
            text_right(
                &format!("Ascended Elite: {affixes}  {distance:.0} m"),
                [0.95, 0.42, 0.30, 0.85 + 0.15 * (t * 3.0).sin()],
            );
        }
        if let Some((amount, at)) = toast {
            let fade = 1.0 - (at.elapsed().as_secs_f32() / 4.0).clamp(0.0, 1.0);
            text_right(&format!("+{amount} Ascension Embers  ({})", view.embers), alpha(GOLD, fade));
        }
        if let Some((amount, at)) = rune_toast {
            let fade = 1.0 - (at.elapsed().as_secs_f32() / 5.0).clamp(0.0, 1.0);
            text_right(&format!("+{} runes (skipped reward)", grouped(amount)), alpha(GOLD, fade));
        }
        if affix_line {
            let names: Vec<String> = hud.boss_affixes.iter().map(|a| affix_label(*a, hud.adaptive_element, false)).collect();
            text_right(&format!("Boss affix: {}", names.join(" + ")), [0.95, 0.42, 0.30, 0.9]);
        }
        if reward_waiting {
            let pulse = 0.65 + 0.35 * (t * 3.0).sin().abs();
            text_right("Ascension reward waiting  [F7]", alpha(GOLD, pulse));
        }
        // 0.8: the Tide during fights.
        if hud.tide > 0 && hud.in_combat {
            text_right(&format!("Tide {}", roman_tide(hud.tide)), alpha([0.55, 0.75, 0.95, 1.0], 0.85));
        }
    }
}

// ------------------------------------------------------------------ drawing ---

/// 2.3: a combat label or a banner, Elden Ring style: a dark band that fades
/// at its ends, gold rules growing from the centre, the title tracking in
/// with a soft glow and shadow, an overline and a subtitle, and the screen
/// edges glowing once in its colour.
fn draw_flash(ui: &Ui, fonts: Fonts, flash: &crate::moments::Flash) {
    let display = ui.io().display_size;
    let age = flash.at.elapsed().as_secs_f32();
    let life = flash.duration();
    if age >= life || display[0] < 100.0 {
        return;
    }
    let tail = if flash.big { 0.9 } else { 0.45 };
    let a = (age / 0.1).min(1.0) * (1.0 - ((age - (life - tail)) / tail).clamp(0.0, 1.0));
    let ease = 1.0 - (1.0 - (age / if flash.big { 0.6 } else { 0.3 }).min(1.0)).powi(3);
    let color = flash.color;
    let font = if flash.big { fonts.banner } else { fonts.moment };
    let (tracking_end, tracking_extra) = if flash.big { (7.0, 22.0) } else { (3.0, 12.0) };
    let tracking = tracking_end + tracking_extra * (1.0 - ease);
    let chars: Vec<String> = flash.title.chars().map(String::from).collect();
    let mut widths = Vec::with_capacity(chars.len());
    let mut title_h = 0.0_f32;
    with_font(ui, font, || {
        for ch in &chars {
            let size = ui.calc_text_size(ch);
            widths.push(size[0]);
            title_h = title_h.max(size[1]);
        }
    });
    let title_w: f32 = widths.iter().sum::<f32>() + tracking * chars.len().saturating_sub(1) as f32;
    // Overline: small capitals with wide spacing.
    let over: Vec<String> = flash.overline.chars().map(String::from).collect();
    let over_widths: Vec<f32> = over.iter().map(|c| ui.calc_text_size(c)[0]).collect();
    let over_w: f32 = over_widths.iter().sum::<f32>() + 3.0 * over.len().saturating_sub(1) as f32;
    let line_h = ui.text_line_height();
    let sub_w = if flash.subtitle.is_empty() { 0.0 } else { ui.calc_text_size(&flash.subtitle)[0] };
    let over_h = if over.is_empty() { 0.0 } else { line_h + 6.0 };
    let sub_h = if flash.subtitle.is_empty() { 0.0 } else { line_h + 10.0 };
    let pad = if flash.big { 18.0 } else { 12.0 };
    let band_h = pad + over_h + title_h + sub_h + pad;
    let cx = display[0] * 0.5;
    let top = display[1] * if flash.big { 0.17 } else { 0.31 } - band_h * 0.5;
    let half = if flash.big {
        display[0] * 0.5
    } else {
        (title_w.max(sub_w) * 0.5 + 180.0).min(display[0] * 0.5)
    };
    let dl = ui.get_background_draw_list();
    // The band: dark in the middle, clear at both ends.
    let dark = [0.0, 0.0, 0.0, if flash.big { 0.62 } else { 0.5 } * a];
    let clear = [0.0, 0.0, 0.0, 0.0];
    dl.add_rect_filled_multicolor([cx - half, top], [cx, top + band_h], clear, dark, dark, clear);
    dl.add_rect_filled_multicolor([cx, top], [cx + half, top + band_h], dark, clear, clear, dark);
    // Rules above and below, growing from the centre, a diamond on the lower one.
    let rule = if flash.big { half * 0.62 } else { title_w * 0.5 + 70.0 } * ease;
    for y in [top, top + band_h] {
        dl.add_line([cx - rule, y], [cx + rule, y], alpha(color, 0.6 * a)).thickness(1.2).build();
        dl.add_line([cx - rule * 0.6, y], [cx + rule * 0.6, y], alpha(color, 0.35 * a)).thickness(2.4).build();
    }
    diamond(&dl, [cx, top + band_h], 4.5, alpha(color, 0.9 * a), true);
    let mut y = top + pad;
    if !over.is_empty() {
        let mut x = cx - over_w * 0.5;
        for (ch, w) in over.iter().zip(&over_widths) {
            dl.add_text([x, y], alpha(MUTED, 0.95 * a), ch);
            x += w + 3.0;
        }
        y += over_h;
    }
    with_font(ui, font, || {
        let mut x = cx - title_w * 0.5;
        let glow = if flash.big { 3.0 } else { 2.2 };
        for (ch, w) in chars.iter().zip(&widths) {
            for (dx, dy) in [(glow, 0.0), (-glow, 0.0), (0.0, glow), (0.0, -glow), (glow * 0.7, glow * 0.7), (-glow * 0.7, glow * 0.7), (glow * 0.7, -glow * 0.7), (-glow * 0.7, -glow * 0.7)] {
                dl.add_text([x + dx, y + dy], alpha(color, 0.09 * a), ch);
            }
            dl.add_text([x + 2.0, y + 2.5], [0.0, 0.0, 0.0, 0.75 * a], ch);
            dl.add_text([x, y], alpha(color, a), ch);
            x += w + tracking;
        }
    });
    y += title_h + 10.0;
    if !flash.subtitle.is_empty() {
        let at = [cx - sub_w * 0.5, y];
        dl.add_text([at[0] + 1.0, at[1] + 1.0], [0.0, 0.0, 0.0, 0.8 * a], &flash.subtitle);
        dl.add_text(at, alpha(PAPER, 0.92 * a), &flash.subtitle);
    }
    // The screen edges glow once in the moment's colour.
    if flash.glow > 0.0 && age < 0.6 {
        let g = flash.glow * (1.0 - age / 0.6).powi(2) * 0.45;
        let edge = display[0].min(display[1]) * 0.16;
        let (w, h) = (display[0], display[1]);
        let c = alpha(color, g);
        let z = alpha(color, 0.0);
        dl.add_rect_filled_multicolor([0.0, 0.0], [w, edge], c, c, z, z);
        dl.add_rect_filled_multicolor([0.0, h - edge], [w, h], z, z, c, c);
        dl.add_rect_filled_multicolor([0.0, 0.0], [edge, h], c, z, z, c);
        dl.add_rect_filled_multicolor([w - edge, 0.0], [w, h], z, c, c, z);
    }
}

fn with_font(ui: &Ui, font: Option<FontId>, f: impl FnOnce()) {
    match font {
        Some(font) => {
            let token = ui.push_font(font);
            f();
            drop(token);
        }
        None => f(),
    }
}

fn header(ui: &Ui, fonts: Fonts, title: &str, subtitle: &str) {
    with_font(ui, fonts.title, || centered(ui, title, GOLD));
    centered(ui, subtitle, MUTED);
    ornament_line(ui, alpha(GOLD, 0.75));
}

fn centered(ui: &Ui, text: &str, color: [f32; 4]) {
    let available = ui.content_region_avail()[0];
    let x = ui.cursor_pos()[0] + ((available - ui.calc_text_size(text)[0]) * 0.5).max(0.0);
    let y = ui.cursor_pos()[1];
    ui.set_cursor_pos([x, y]);
    ui.text_colored(color, text);
}

/// A gold rule with a diamond in the middle.
fn ornament_line(ui: &Ui, color: [f32; 4]) {
    let pos = ui.cursor_screen_pos();
    let w = ui.content_region_avail()[0];
    let y = pos[1] + 6.0;
    let mid = pos[0] + w * 0.5;
    let dl = ui.get_window_draw_list();
    dl.add_line([pos[0] + w * 0.12, y], [mid - 12.0, y], color).thickness(1.0).build();
    dl.add_line([mid + 12.0, y], [pos[0] + w * 0.88, y], color).thickness(1.0).build();
    diamond(&dl, [mid, y], 5.0, color, true);
    ui.dummy([w, 14.0]);
}

fn diamond(dl: &DrawListMut, c: [f32; 2], r: f32, color: [f32; 4], filled: bool) {
    let points = vec![[c[0], c[1] - r], [c[0] + r, c[1]], [c[0], c[1] + r], [c[0] - r, c[1]], [c[0], c[1] - r]];
    if filled {
        dl.add_triangle(points[0], points[1], points[2], color).filled(true).build();
        dl.add_triangle(points[0], points[2], points[3], color).filled(true).build();
    } else {
        dl.add_polyline(points, color).thickness(1.5).build();
    }
}

fn backdrop(ui: &Ui, display: [f32; 2], glow: [f32; 4]) {
    let dl = ui.get_background_draw_list();
    dl.add_rect([0.0, 0.0], display, [0.0, 0.0, 0.0, 0.70]).filled(true).build();
    // Soft rarity glow behind the cards, darker edges.
    let band = display[1] * 0.25;
    dl.add_rect_filled_multicolor(
        [0.0, display[1] * 0.5 - band],
        [display[0], display[1] * 0.5],
        [0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0],
        alpha(glow, 0.10),
        alpha(glow, 0.10),
    );
    dl.add_rect_filled_multicolor(
        [0.0, display[1] * 0.5],
        [display[0], display[1] * 0.5 + band],
        alpha(glow, 0.10),
        alpha(glow, 0.10),
        [0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0],
    );
}

fn offer_glow(offer: &Offer) -> [f32; 4] {
    offer
        .cards
        .iter()
        .map(|c| c.display_rarity)
        .max()
        .map(rarity_color)
        .unwrap_or(GOLD)
}

fn panel_frame(dl: &DrawListMut, pos: [f32; 2], size: [f32; 2]) {
    let max = [pos[0] + size[0], pos[1] + size[1]];
    dl.add_rect_filled_multicolor(pos, max, [0.07, 0.07, 0.06, 0.97], [0.07, 0.07, 0.06, 0.97], [0.035, 0.035, 0.03, 0.98], [0.035, 0.035, 0.03, 0.98]);
    dl.add_rect(pos, max, alpha(GOLD, 0.7)).thickness(1.5).rounding(8.0).build();
    dl.add_rect([pos[0] + 5.0, pos[1] + 5.0], [max[0] - 5.0, max[1] - 5.0], alpha(GOLD_DIM, 0.45)).thickness(1.0).rounding(6.0).build();
}

/// Card body with rarity glow, double border and corner diamonds.
#[allow(clippy::too_many_arguments)]
fn card_frame(dl: &DrawListMut, pos: [f32; 2], size: [f32; 2], color: [f32; 4], selected: bool, hovered: bool, t: f32, appear: f32) {
    let max = [pos[0] + size[0], pos[1] + size[1]];
    let pulse = if selected { 0.75 + 0.25 * (t * 3.0).sin() } else { 1.0 };
    let strength = if selected { 1.5 } else if hovered { 1.1 } else { 0.7 } * pulse * appear;
    for i in 0..5 {
        let grow = (i + 1) as f32 * 3.5;
        dl.add_rect([pos[0] - grow, pos[1] - grow], [max[0] + grow, max[1] + grow], alpha(color, 0.07 * strength * (1.0 - i as f32 / 5.0)))
            .rounding(10.0 + grow)
            .filled(true)
            .build();
    }
    let top = tint(color, 0.22);
    let bottom = [0.03, 0.032, 0.028, 0.98];
    dl.add_rect_filled_multicolor(pos, max, alpha(top, appear), alpha(top, appear), alpha(bottom, appear), alpha(bottom, appear));
    dl.add_rect(pos, max, alpha(color, appear)).thickness(if selected { 3.0 } else { 2.0 }).rounding(7.0).build();
    dl.add_rect([pos[0] + 6.0, pos[1] + 6.0], [max[0] - 6.0, max[1] - 6.0], alpha(GOLD_DIM, 0.55 * appear)).thickness(1.0).rounding(5.0).build();
    for corner in [[pos[0] + 6.0, pos[1] + 6.0], [max[0] - 6.0, pos[1] + 6.0], [pos[0] + 6.0, max[1] - 6.0], [max[0] - 6.0, max[1] - 6.0]] {
        diamond(dl, corner, 4.5, alpha(color, appear), true);
    }
}

/// 0.7: the picture band at the top of a card; returns its height. `full_w`
/// is the unsquashed card width, so a flipping card shows the same crop
/// squeezed (that is what makes it read as a turn).
fn banner(dl: &DrawListMut, pos: [f32; 2], size: [f32; 2], tile: Tile, full_w: f32, color: [f32; 4], appear: f32) -> f32 {
    let band = banner_height([full_w, size[1]]);
    let p0 = [pos[0] + 7.0, pos[1] + 7.0];
    let p1 = [pos[0] + size[0] - 7.0, pos[1] + 7.0 + band];
    if p1[0] - p0[0] < 2.0 {
        return band;
    }
    let t = tile.crop((full_w - 14.0).max(1.0) / band, 0.42);
    dl.add_image_rounded(t.texture, p0, p1, 5.0)
        .uv_min(t.uv0)
        .uv_max(t.uv1)
        .col([1.0, 1.0, 1.0, appear])
        .round_bot_left(false)
        .round_bot_right(false)
        .build();
    // Fade the lower part into the card's own colour.
    let base = tint(color, 0.18);
    let clear = [base[0], base[1], base[2], 0.0];
    let solid = [base[0], base[1], base[2], 0.92 * appear];
    dl.add_rect_filled_multicolor([p0[0], p1[1] - band * 0.45], p1, clear, clear, solid, solid);
    dl.add_line([p0[0], p1[1]], [p1[0], p1[1]], alpha(color, 0.7 * appear)).thickness(1.5).build();
    band
}

/// 0.7: a face-down card: the Ascension sigil in a gold frame.
fn card_back(dl: &DrawListMut, pos: [f32; 2], size: [f32; 2], art: Option<Tile>, appear: f32, t: f32) {
    let max = [pos[0] + size[0], pos[1] + size[1]];
    dl.add_rect_filled_multicolor(
        pos,
        max,
        alpha([0.09, 0.08, 0.06, 0.98], appear),
        alpha([0.09, 0.08, 0.06, 0.98], appear),
        alpha([0.03, 0.03, 0.025, 0.98], appear),
        alpha([0.03, 0.03, 0.025, 0.98], appear),
    );
    if size[0] > 24.0 {
        let w = size[0] - 16.0;
        let h = w * 0.5;
        let y = pos[1] + size[1] * 0.45 - h * 0.5;
        if let Some(tile) = art {
            dl.add_image(tile.texture, [pos[0] + 8.0, y], [max[0] - 8.0, y + h])
                .uv_min(tile.uv0)
                .uv_max(tile.uv1)
                .col([1.0, 1.0, 1.0, appear])
                .build();
        } else {
            let c = [pos[0] + size[0] * 0.5, y + h * 0.5];
            dl.add_circle(c, h * 0.38, alpha(GOLD, appear)).thickness(2.0).num_segments(48).build();
            diamond(dl, c, h * 0.16, alpha(GOLD, appear), true);
        }
        // A slow light band sweeping across the back.
        let k = (t * 0.45).fract();
        let x = pos[0] + size[0] * (k * 1.6 - 0.3);
        let band = size[0] * 0.18;
        let glow = alpha([1.0, 0.86, 0.55, 0.10], appear);
        let none = [1.0, 0.86, 0.55, 0.0];
        let (l, r) = ((x - band).max(pos[0]), (x + band).min(max[0]));
        if r > l {
            dl.add_rect_filled_multicolor([l, pos[1]], [x.clamp(l, r), max[1]], none, glow, glow, none);
            dl.add_rect_filled_multicolor([x.clamp(l, r), pos[1]], [r, max[1]], glow, none, none, glow);
        }
    }
    dl.add_rect(pos, max, alpha(GOLD, 0.9 * appear)).thickness(2.0).rounding(7.0).build();
    dl.add_rect([pos[0] + 6.0, pos[1] + 6.0], [max[0] - 6.0, max[1] - 6.0], alpha(GOLD_DIM, 0.6 * appear))
        .thickness(1.0)
        .rounding(5.0)
        .build();
    if size[0] > 16.0 {
        for corner in [[pos[0] + 6.0, pos[1] + 6.0], [max[0] - 6.0, pos[1] + 6.0], [pos[0] + 6.0, max[1] - 6.0], [max[0] - 6.0, max[1] - 6.0]] {
            diamond(dl, corner, 4.5, alpha(GOLD, appear), true);
        }
    }
}

/// How strongly a revealed card bursts (Rare and better; evolutions most).
fn burst_strength(card: &Card, power: Option<&Power>) -> Option<f32> {
    if is_prismatic(power) {
        return Some(2.0);
    }
    if card.previous_level == 0 && power.is_some_and(|p| p.is_evolution()) {
        return Some(1.4);
    }
    match card.display_rarity {
        Rarity::Common | Rarity::Uncommon => None,
        Rarity::Rare => Some(0.55),
        Rarity::Epic => Some(0.8),
        Rarity::Legendary => Some(1.1),
        Rarity::Mythic => Some(1.4),
    }
}

/// 3.4: the title screen. Golden embers rise from a warm glow along the
/// bottom edge, and the mod's name and version sit in the lower left, like
/// other big mods sign their title screens. Drawn behind every window, faint
/// enough to leave the game's own menu readable.
fn draw_title(ui: &Ui, fonts: Fonts, since: f32) {
    let display = ui.io().display_size;
    let (w, h) = (display[0], display[1]);
    if w < 200.0 {
        return;
    }
    let a = (since / 1.5).min(1.0);
    let t = ui.time() as f32;
    let dl = ui.get_background_draw_list();
    // Warm light from below.
    let glow = [0.85, 0.55, 0.18, 0.16 * a];
    let clear = [0.85, 0.55, 0.18, 0.0];
    dl.add_rect_filled_multicolor([0.0, h * 0.72], [w, h], clear, clear, glow, glow);
    // Embers.
    for i in 0..110u32 {
        let r = |k: u32| ((i.wrapping_mul(2_246_822_519).wrapping_add(k.wrapping_mul(3_266_489_917)) >> 9) % 10_000) as f32 / 10_000.0;
        let period = 9.0 + r(1) * 9.0;
        let phase = ((t + r(2) * period) % period) / period;
        let x = r(3) * w + (t * (0.4 + r(4)) + r(5) * 6.0).sin() * 22.0;
        let y = h + 10.0 - phase * h * (0.55 + r(6) * 0.4);
        let life = (phase * 4.0).min(1.0) * (1.0 - phase).powf(0.6);
        let size = 1.2 + r(7) * 2.4;
        let c = [1.0, 0.62 + r(8) * 0.3, 0.25, (0.25 + 0.45 * r(9)) * life * a];
        dl.add_circle([x, y], size * 2.6, alpha(c, 0.18)).filled(true).build();
        dl.add_circle([x, y], size, c).filled(true).build();
    }
    // The mod's name and version, lower left.
    let x = 46.0;
    let base = h - 48.0;
    let line_h = ui.text_line_height();
    let mut title_h = 0.0f32;
    let mut title_w = 0.0f32;
    with_font(ui, fonts.moment, || {
        let size = ui.calc_text_size("ASCENSION");
        title_h = size[1];
        title_w = size[0];
    });
    let title_y = base - line_h - 6.0 - title_h;
    let over_y = title_y - line_h - 2.0;
    let mut cx = x;
    for ch in "ELDEN RING".chars().map(String::from) {
        dl.add_text([cx, over_y], alpha(GOLD_DIM, a), &ch);
        cx += ui.calc_text_size(&ch)[0] + 4.0;
    }
    with_font(ui, fonts.moment, || {
        for (dx, dy) in [(2.5, 0.0), (-2.5, 0.0), (0.0, 2.5), (0.0, -2.5)] {
            dl.add_text([x + dx, title_y + dy], alpha(GOLD, 0.10 * a), "ASCENSION");
        }
        dl.add_text([x + 2.0, title_y + 2.5], [0.0, 0.0, 0.0, 0.8 * a], "ASCENSION");
        dl.add_text([x, title_y], alpha([0.95, 0.80, 0.48, 1.0], a), "ASCENSION");
    });
    let rule_y = title_y + title_h + 3.0;
    dl.add_line([x, rule_y], [x + title_w, rule_y], alpha(GOLD, 0.55 * a)).thickness(1.2).build();
    diamond(&dl, [x + title_w * 0.5, rule_y], 3.5, alpha(GOLD, 0.9 * a), true);
    let version = format!("version {}  ·  roguelite cards for the Lands Between", env!("CARGO_PKG_VERSION"));
    dl.add_text([x + 1.0, base + 1.0 - line_h * 0.2], [0.0, 0.0, 0.0, 0.8 * a], &version);
    dl.add_text([x, base - line_h * 0.2], alpha(MUTED, a), &version);
}

const MAX_WIN_TIME: f32 = 4.2;

/// 3.4: a Prismatic card turned over. Every colour at once: rainbow rays
/// turning from the centre, the screen edges pulsing, rainbow confetti
/// raining down, and MAX WIN in giant letters that bounce in, each letter
/// its own colour.
fn draw_max_win(ui: &Ui, fonts: Fonts, since: f32) {
    let display = ui.io().display_size;
    let (w, h) = (display[0], display[1]);
    let fade_in = (since / 0.15).min(1.0);
    let fade_out = 1.0 - ((since - (MAX_WIN_TIME - 0.8)) / 0.8).clamp(0.0, 1.0);
    let a = fade_in * fade_out;
    let dl = ui.get_foreground_draw_list();
    let center = [w * 0.5, h * 0.45];
    // A white flash at the start.
    let flash = (1.0 - since / 0.35).clamp(0.0, 1.0);
    if flash > 0.0 {
        dl.add_rect([0.0, 0.0], [w, h], [1.0, 1.0, 1.0, 0.45 * flash]).filled(true).build();
    }
    // Rainbow rays.
    let reach = w.max(h);
    for i in 0..32 {
        let angle = std::f32::consts::TAU * i as f32 / 32.0 + since * 0.6;
        let spread = 0.05;
        let c = rainbow(since, i as f32 / 32.0);
        dl.add_triangle(
            center,
            [center[0] + (angle - spread).cos() * reach, center[1] + (angle - spread).sin() * reach],
            [center[0] + (angle + spread).cos() * reach, center[1] + (angle + spread).sin() * reach],
            alpha(c, 0.10 * a),
        )
        .filled(true)
        .build();
    }
    // Screen edges pulsing in every colour.
    let pulse = 0.5 + 0.5 * (since * 9.0).sin();
    let edge = w.min(h) * 0.14;
    let c = alpha(rainbow(since * 2.0, 0.0), (0.25 + 0.25 * pulse) * a);
    let z = alpha(c, 0.0);
    dl.add_rect_filled_multicolor([0.0, 0.0], [w, edge], c, c, z, z);
    dl.add_rect_filled_multicolor([0.0, h - edge], [w, h], z, z, c, c);
    dl.add_rect_filled_multicolor([0.0, 0.0], [edge, h], c, z, z, c);
    dl.add_rect_filled_multicolor([w - edge, 0.0], [w, h], z, c, c, z);
    // Confetti.
    for i in 0..160u32 {
        let r = |k: u32| ((i.wrapping_mul(2_654_435_761).wrapping_add(k.wrapping_mul(40_503)) >> 8) % 10_000) as f32 / 10_000.0;
        let start = r(1) * 0.6;
        let t = since - start;
        if t < 0.0 {
            continue;
        }
        let x = r(2) * w + (t * (1.5 + r(3) * 2.0) + r(4) * 6.0).sin() * 30.0;
        let y = -20.0 + t * (h * (0.25 + r(5) * 0.35));
        if y > h + 20.0 {
            continue;
        }
        let size = 4.0 + r(6) * 6.0;
        let spin = t * (3.0 + r(7) * 5.0);
        let (cs, sn) = (spin.cos() * size, spin.sin() * size * 0.5);
        let c = alpha(rainbow(since, r(8)), 0.95 * a);
        dl.add_triangle([x - cs, y - sn], [x + cs, y + sn], [x + sn, y - cs], c).filled(true).build();
    }
    // MAX WIN, letter by letter.
    let text: Vec<String> = "MAX WIN".chars().map(String::from).collect();
    let font = fonts.banner;
    let mut widths = Vec::new();
    let mut height = 0.0f32;
    with_font(ui, font, || {
        for ch in &text {
            let size = ui.calc_text_size(ch);
            widths.push(size[0]);
            height = height.max(size[1]);
        }
    });
    let tracking = 14.0 + 40.0 * (1.0 - (since / 0.5).min(1.0)).powi(2);
    let total: f32 = widths.iter().sum::<f32>() + tracking * (text.len() - 1) as f32;
    let top = h * 0.16;
    // A dark band so the letters read over any picture.
    let band = [0.0, 0.0, 0.0, 0.55 * a];
    let clear = [0.0, 0.0, 0.0, 0.0];
    dl.add_rect_filled_multicolor([0.0, top - 18.0], [w * 0.5, top + height + 54.0], clear, band, band, clear);
    dl.add_rect_filled_multicolor([w * 0.5, top - 18.0], [w, top + height + 54.0], band, clear, clear, band);
    with_font(ui, font, || {
        let mut x = w * 0.5 - total * 0.5;
        for (i, (ch, cw)) in text.iter().zip(&widths).enumerate() {
            let bounce = ((since * 7.0 - i as f32 * 0.6).sin() * 10.0) * (1.0 - since / MAX_WIN_TIME).max(0.0);
            let y = top - bounce.abs();
            let c = rainbow(since * 1.5, i as f32 * 0.13);
            for (dx, dy) in [(4.0, 0.0), (-4.0, 0.0), (0.0, 4.0), (0.0, -4.0)] {
                dl.add_text([x + dx, y + dy], alpha(c, 0.25 * a), ch);
            }
            dl.add_text([x + 3.0, y + 4.0], [0.0, 0.0, 0.0, 0.85 * a], ch);
            dl.add_text([x, y], alpha(c, a), ch);
            x += cw + tracking;
        }
    });
    let subtitle = "PRISMATIC CARD";
    let sub: Vec<String> = subtitle.chars().map(String::from).collect();
    let sub_w: f32 = sub.iter().map(|c| ui.calc_text_size(c)[0] + 4.0).sum();
    let mut x = w * 0.5 - sub_w * 0.5;
    let y = top + height + 14.0;
    for (i, ch) in sub.iter().enumerate() {
        dl.add_text([x + 1.0, y + 1.0], [0.0, 0.0, 0.0, 0.8 * a], ch);
        dl.add_text([x, y], alpha(rainbow(since, i as f32 * 0.07), a), ch);
        x += ui.calc_text_size(ch)[0] + 4.0;
    }
}

/// 0.7: light rays and a ring expanding from a card that just turned over.
fn reveal_burst(dl: &DrawListMut, center: [f32; 2], size: [f32; 2], color: [f32; 4], since: f32, strength: f32) {
    const LENGTH: f32 = 1.1;
    if !(0.0..LENGTH).contains(&since) {
        return;
    }
    let k = since / LENGTH;
    let fade = (1.0 - k).powi(2) * strength.min(1.5);
    let reach = size[1] * (0.45 + 0.55 * ease(k));
    let rays = if strength >= 1.0 { 22 } else { 14 };
    for i in 0..rays {
        let a = std::f32::consts::TAU * i as f32 / rays as f32 + since * 0.5;
        let spread = 0.045 + 0.02 * ((i * 7) % 3) as f32;
        let long = reach * (0.75 + 0.25 * ((i * 5) % 4) as f32 / 3.0);
        dl.add_triangle(
            center,
            [center[0] + (a - spread).cos() * long, center[1] + (a - spread).sin() * long],
            [center[0] + (a + spread).cos() * long, center[1] + (a + spread).sin() * long],
            alpha(color, 0.11 * fade),
        )
        .filled(true)
        .build();
    }
    let ring = size[0] * 0.35 + size[0] * 0.55 * ease(k);
    dl.add_circle(center, ring, alpha(color, 0.55 * fade)).thickness(3.0).num_segments(64).build();
    dl.add_circle(center, ring * 0.82, alpha(GOLD, 0.30 * fade)).thickness(1.5).num_segments(64).build();
}

/// Five diamonds: each level's rarity colour; owned levels filled, the level

/// this card grants pulses.
fn level_pips(dl: &DrawListMut, origin: [f32; 2], power: &Power, previous: u8, target: u8, t: f32) {
    for k in 0..5u8 {
        let c = [origin[0] + k as f32 * 22.0 + 8.0, origin[1]];
        let color = rarity_color(power.level_rarities[usize::from(k)]);
        if k + 1 <= previous {
            diamond(dl, c, 7.0, color, true);
        } else if k + 1 == target {
            diamond(dl, c, 7.0 + 1.5 * (t * 4.0).sin().abs(), alpha(color, 0.9), true);
            diamond(dl, c, 10.0, alpha(color, 0.5), false);
        } else {
            diamond(dl, c, 7.0, alpha(color, 0.45), false);
        }
    }
}

/// Small vector icon per category.
fn glyph(dl: &DrawListMut, c: [f32; 2], s: f32, color: [f32; 4], category: &str) {
    let line = |a: [f32; 2], b: [f32; 2]| dl.add_line(a, b, color).thickness(2.0).build();
    match category {
        "offense" => {
            line([c[0] - s, c[1] + s], [c[0] + s, c[1] - s]);
            line([c[0] - s, c[1] - s], [c[0] + s, c[1] + s]);
            line([c[0] - s * 0.6, c[1] + s * 0.2], [c[0] - s * 0.2, c[1] + s * 0.6]);
            line([c[0] + s * 0.6, c[1] + s * 0.2], [c[0] + s * 0.2, c[1] + s * 0.6]);
        }
        "defense" => {
            let pts = vec![[c[0] - s, c[1] - s * 0.8], [c[0] + s, c[1] - s * 0.8], [c[0] + s * 0.8, c[1] + s * 0.2], [c[0], c[1] + s], [c[0] - s * 0.8, c[1] + s * 0.2], [c[0] - s, c[1] - s * 0.8]];
            dl.add_polyline(pts, color).thickness(2.0).build();
        }
        "recovery" => {
            dl.add_circle([c[0], c[1] + s * 0.25], s * 0.7, color).thickness(2.0).build();
            dl.add_triangle([c[0], c[1] - s], [c[0] - s * 0.62, c[1] - s * 0.05], [c[0] + s * 0.62, c[1] - s * 0.05], color).thickness(2.0).build();
        }
        "endurance" => {
            for k in 0..3 {
                let dy = (k as f32 - 1.0) * s * 0.6;
                line([c[0] - s * 0.8, c[1] + dy + s * 0.3], [c[0], c[1] + dy - s * 0.3]);
                line([c[0], c[1] + dy - s * 0.3], [c[0] + s * 0.8, c[1] + dy + s * 0.3]);
            }
        }
        "magic" | "arcane_arsenal" => {
            let mut pts = Vec::new();
            for k in 0..=10 {
                let a = std::f32::consts::PI * (k as f32 / 5.0) - std::f32::consts::FRAC_PI_2;
                let r = if k % 2 == 0 { s } else { s * 0.42 };
                pts.push([c[0] + a.cos() * r, c[1] + a.sin() * r]);
            }
            dl.add_polyline(pts, color).thickness(2.0).build();
            if category == "arcane_arsenal" {
                dl.add_circle(c, s * 1.25, alpha(color, 0.6)).thickness(1.0).build();
            }
        }
        "mobility" => {
            line([c[0] - s, c[1] + s * 0.6], [c[0] + s, c[1] - s * 0.6]);
            line([c[0] + s, c[1] - s * 0.6], [c[0] + s * 0.25, c[1] - s * 0.7]);
            line([c[0] + s, c[1] - s * 0.6], [c[0] + s * 0.7, c[1] + s * 0.05]);
            line([c[0] - s * 0.6, c[1] + s], [c[0] + s * 0.4, c[1] + s * 0.4]);
        }
        "exploration" => {
            dl.add_circle(c, s * 0.9, color).thickness(2.0).build();
            dl.add_circle(c, s * 0.35, color).filled(true).build();
        }
        "status" => {
            dl.add_circle([c[0], c[1] - s * 0.15], s * 0.75, color).thickness(2.0).build();
            dl.add_circle([c[0] - s * 0.3, c[1] - s * 0.2], s * 0.16, color).filled(true).build();
            dl.add_circle([c[0] + s * 0.3, c[1] - s * 0.2], s * 0.16, color).filled(true).build();
            line([c[0] - s * 0.3, c[1] + s * 0.9], [c[0] + s * 0.3, c[1] + s * 0.9]);
        }
        "elemental" => {
            dl.add_triangle([c[0], c[1] - s], [c[0] - s * 0.7, c[1] + s * 0.5], [c[0] + s * 0.7, c[1] + s * 0.5], color).thickness(2.0).build();
            dl.add_circle([c[0], c[1] + s * 0.4], s * 0.45, color).thickness(2.0).build();
        }
        "economy" => {
            dl.add_circle(c, s, color).thickness(2.0).build();
            diamond(dl, c, s * 0.5, color, false);
        }
        "roguelite" => {
            dl.add_rect([c[0] - s * 0.85, c[1] - s * 0.85], [c[0] + s * 0.85, c[1] + s * 0.85], color).thickness(2.0).rounding(3.0).build();
            for (dx, dy) in [(-0.4, -0.4), (0.4, 0.4), (0.0, 0.0), (0.4, -0.4), (-0.4, 0.4)] {
                dl.add_circle([c[0] + s * dx, c[1] + s * dy], s * 0.12, color).filled(true).build();
            }
        }
        "attribute_synergy" => {
            dl.add_circle([c[0] - s * 0.35, c[1]], s * 0.6, color).thickness(2.0).build();
            dl.add_circle([c[0] + s * 0.35, c[1]], s * 0.6, color).thickness(2.0).build();
        }
        // 0.5: a cracked ring for curses, a rising double chevron for evolutions.
        "cursed" => {
            dl.add_circle(c, s, color).thickness(2.0).num_segments(24).build();
            line([c[0] - s * 0.15, c[1] - s], [c[0] + s * 0.2, c[1] - s * 0.2]);
            line([c[0] + s * 0.2, c[1] - s * 0.2], [c[0] - s * 0.2, c[1] + s * 0.3]);
            line([c[0] - s * 0.2, c[1] + s * 0.3], [c[0] + s * 0.1, c[1] + s]);
        }
        // 0.6: crossed blades with a spark for combat reflexes.
        "reflex" => {
            line([c[0] - s, c[1] + s], [c[0] + s * 0.7, c[1] - s * 0.7]);
            line([c[0] + s, c[1] + s], [c[0] - s * 0.7, c[1] - s * 0.7]);
            line([c[0] - s * 0.55, c[1] + s * 0.25], [c[0] - s * 0.25, c[1] + s * 0.55]);
            line([c[0] + s * 0.55, c[1] + s * 0.25], [c[0] + s * 0.25, c[1] + s * 0.55]);
            diamond(dl, [c[0], c[1] - s * 0.95], s * 0.2, color, true);
        }
        // 0.9: a banner on a pole for Command (allies).
        "command" => {
            line([c[0] - s * 0.6, c[1] + s], [c[0] - s * 0.6, c[1] - s]);
            let pts = vec![[c[0] - s * 0.6, c[1] - s], [c[0] + s * 0.8, c[1] - s * 0.7], [c[0] + s * 0.4, c[1] - s * 0.2], [c[0] + s * 0.8, c[1] + s * 0.3], [c[0] - s * 0.6, c[1]]];
            dl.add_polyline(pts, color).thickness(2.0).build();
        }
        "evolution" => {
            for dy in [0.15, -0.45] {
                line([c[0] - s * 0.8, c[1] + s * (dy + 0.45)], [c[0], c[1] + s * (dy - 0.25)]);
                line([c[0], c[1] + s * (dy - 0.25)], [c[0] + s * 0.8, c[1] + s * (dy + 0.45)]);
            }
            diamond(dl, [c[0], c[1] - s * 0.95], s * 0.22, color, true);
        }
        _ => {
            let mut pts = Vec::new();
            for k in 0..=6 {
                let a = std::f32::consts::PI / 3.0 * k as f32;
                pts.push([c[0] + a.cos() * s, c[1] + a.sin() * s]);
            }
            dl.add_polyline(pts, color).thickness(2.0).build();
        }
    }
}

/// Darken the remaining share of a cooldown as a pie (clockwise from the top).
fn sweep(dl: &DrawListMut, c: [f32; 2], r: f32, share: f32, color: [f32; 4]) {
    let steps = (32.0 * share).ceil().max(1.0) as usize;
    let start = -std::f32::consts::FRAC_PI_2;
    let total = std::f32::consts::TAU * share;
    for k in 0..steps {
        let a0 = start + total * k as f32 / steps as f32;
        let a1 = start + total * (k + 1) as f32 / steps as f32;
        dl.add_triangle(c, [c[0] + a0.cos() * r, c[1] + a0.sin() * r], [c[0] + a1.cos() * r, c[1] + a1.sin() * r], color)
            .filled(true)
            .build();
    }
}

/// `band`: height of the card picture drawn above (0 = no picture, emblem only).
#[allow(clippy::too_many_arguments)]
fn card_body(
    ui: &Ui,
    fonts: Fonts,
    card: &Card,
    power: Option<&Power>,
    view: &ProgressView,
    shared: &SharedRuntime,
    hud: &HudView,
    appear: f32,
    band: f32,
) {
    let color = card_color(power, card.display_rarity, ui.time() as f32);
    let origin = ui.window_pos();
    let width = ui.window_size()[0];
    let Some(power) = power else {
        ui.text_wrapped("This card is unknown to this version. Your reward is kept.");
        return;
    };
    // Emblem, on the lower edge of the picture when there is one.
    // (Only one window draw list may be alive at a time.)
    let (emblem_y, radius, name_y) = if band > 0.0 { (band + 7.0, 24.0, band + 36.0) } else { (46.0, 30.0, 84.0) };
    {
        let dl = ui.get_window_draw_list();
        let center = [origin[0] + width * 0.5, origin[1] + emblem_y];
        dl.add_circle(center, radius, alpha([0.0, 0.0, 0.0, 0.75], appear)).filled(true).build();
        dl.add_circle(center, radius, alpha(color, appear)).thickness(2.0).num_segments(40).build();
        dl.add_circle(center, radius - 5.0, alpha(GOLD_DIM, 0.6 * appear)).thickness(1.0).num_segments(40).build();
        glyph(&dl, center, radius * 0.47, alpha(color, appear), &power.category);
    }
    ui.set_cursor_pos([0.0, name_y]);
    with_font(ui, fonts.heading, || centered(ui, &power.name, color));
    centered(
        ui,
        &format!(
            "{}  ·  {}{}",
            card_rarity_name(Some(power), card.display_rarity),
            roman(card.target_level),
            if card.target_level == 5 { "  ·  MAX" } else { "" }
        ),
        alpha(color, 0.9),
    );
    let pips_y = origin[1] + ui.cursor_pos()[1] + 10.0;
    level_pips(&ui.get_window_draw_list(), [origin[0] + width * 0.5 - 52.0, pips_y], power, card.previous_level, card.target_level, ui.time() as f32);
    ui.dummy([0.0, 24.0]);
    let fresh = card.previous_level == 0;
    let ribbon = if fresh && power.category == "prismatic" {
        "PRISMATIC CARD  ·  MAX WIN".to_owned()
    } else if fresh && !power.signature.is_empty() {
        "SIGNATURE CARD".to_owned()
    } else if fresh && power.is_evolution() {
        "EVOLUTION".to_owned()
    } else if fresh && power.category == "keystone" {
        "KEYSTONE".to_owned()
    } else if fresh {
        "NEW CARD".to_owned()
    } else {
        format!("UPGRADE  {} → {}", roman(card.previous_level), roman(card.target_level))
    };
    centered(ui, &ribbon, if power.category == "prismatic" { color } else if power.is_evolution() { rarity_color(Rarity::Mythic) } else { GOLD });
    // 2.0: the Path this card feeds, and a bonus it would unlock.
    let mut unlock = None;
    if let Some(path) = power.family {
        let before = crate::paths::points(&view.levels, &shared.library)[path.index()];
        let after = before.saturating_add(card.target_level.saturating_sub(card.previous_level));
        centered(ui, &format!("{} PATH   {before} → {after}", path.name().to_uppercase()), path.color());
        unlock = path.bonuses().into_iter().find(|b| before < b.points && after >= b.points);
    }
    if view.locked.as_deref() == Some(card.power_id.as_str()) {
        centered(ui, "LOCKED  ·  comes back next reward", alpha(GOLD, 0.9));
    }
    if power.curse.is_some() {
        centered(ui, "CURSED", CURSE);
    }
    ornament_line(ui, alpha(GOLD_DIM, 0.7));
    ui.indent_by(14.0);
    let wrap = ui.push_text_wrap_pos_with_pos(width - 18.0);
    if (1..=5).contains(&card.target_level) {
        ui.text_colored(PAPER, &power.levels[usize::from(card.target_level - 1)].description);
        if let Some(curse) = &power.curse {
            ui.text_colored(CURSE, format!("Curse: {curse}"));
        }
        if let Some(estimate) = proc_estimate(power, card.target_level, shared, hud) {
            ui.text_colored(GOLD, estimate);
        }
        if card.previous_level > 0 && card.previous_level <= 5 {
            ui.dummy([0.0, 2.0]);
            ui.text_colored(MUTED, format!("Now: {}", power.levels[usize::from(card.previous_level - 1)].description));
        }
    }
    if let Some(bonus) = unlock {
        ui.dummy([0.0, 2.0]);
        ui.text_colored(GOLD, format!("Path bonus {}: {}", bonus.name, bonus.text));
    }
    if fresh && !power.signature.is_empty() {
        let names: Vec<&str> = power
            .signature
            .iter()
            .filter_map(|id| shared.boss_names.get(id).map(String::as_str))
            .take(1)
            .collect();
        if let Some(boss) = names.first() {
            ui.text_colored(alpha(GOLD, 0.8), format!("The power of {boss}."));
        }
    }
    if power.is_evolution() && fresh {
        let parents: Vec<String> = power
            .requires
            .iter()
            .map(|(id, level)| format!("{} {}", shared.library.get(id).map(|p| p.name.as_str()).unwrap_or(id), roman(*level)))
            .collect();
        ui.text_colored(alpha(rarity_color(Rarity::Mythic), 0.9), format!("Evolved from {}", parents.join(" + ")));
    }
    // Synergy hint: completes a combo.
    let mut after = view.levels.clone();
    after.insert(card.power_id.clone(), card.target_level);
    for synergy in shared.synergies.synergies.iter().filter(|s| s.involves(&card.power_id)) {
        let before = synergy.progress(&view.levels, &hud.attributes, &shared.library);
        let next = synergy.progress(&after, &hud.attributes, &shared.library);
        if !before.complete && next.complete {
            ui.text_colored(rarity_color(synergy.rarity), format!("Completes synergy {}", synergy.name));
        }
    }
    drop(wrap);
    ui.unindent_by(14.0);
}

/// "About 240 magic damage" for spell cards (same curve as the engine).
fn proc_estimate(power: &Power, level: u8, shared: &SharedRuntime, hud: &HudView) -> Option<String> {
    let spec = power.levels.get(usize::from(level.clamp(1, 5) - 1))?.proc.as_ref()?;
    let def = shared.proc_defs.get(&spec.proc)?;
    let context = ProcContext {
        character_level: hud.level.max(1),
        attributes: hud.attributes,
        ng_plus: hud.ng_plus,
        ..Default::default()
    };
    // 3.1: your buffs (tears, talismans, incantations, cards) count too: the
    // spell is your attack and the game applies them to it.
    let boost: f32 = def
        .element
        .iter()
        .zip(hud.damage_rates)
        .map(|(share, rate)| share * if rate > 0.0 { rate } else { 1.0 })
        .sum::<f32>()
        .max(0.1);
    let damage = proc_damage(def, spec.power, &context) * boost;
    let buffed = if (boost - 1.0).abs() >= 0.01 {
        format!(" (buffs {:+.0}%)", (boost - 1.0) * 100.0)
    } else {
        String::new()
    };
    Some(format!("About {:.0} {} damage now{buffed}", damage, crate::procs::element_label(def)))
}

/// Order of the "Always on" list in the You tab.
const BONUS_ORDER: [crate::powers::EffectField; 40] = {
    use crate::powers::EffectField as F;
    [
        F::PhysicsAttackRate,
        F::SlashAttackRate,
        F::BlowAttackRate,
        F::ThrustAttackRate,
        F::MagicAttackRate,
        F::FireAttackRate,
        F::ThunderAttackRate,
        F::DarkAttackRate,
        F::SaAttackPowerRate,
        F::StaminaAttackRate,
        F::NeutralDamageCutRate,
        F::SlashDamageCutRate,
        F::BlowDamageCutRate,
        F::ThrustDamageCutRate,
        F::MagicDamageCutRate,
        F::FireDamageCutRate,
        F::ThunderDamageCutRate,
        F::DarkDamageCutRate,
        F::ToughnessDamageCutRate,
        F::MaxHpRate,
        F::MaxMpRate,
        F::MaxStaminaRate,
        F::StaminaRecoverChangeSpeed,
        F::ConsumeStaminaRate,
        F::GuardStaminaMult,
        F::ChangeHpRate,
        F::ChangeHpPoint,
        F::ChangeMpRate,
        F::ChangeMpPoint,
        F::MagicConsumptionRate,
        F::MiracleConsumptionRate,
        F::ArtsConsumptionRate,
        F::ChangeHpEstusFlaskCorrectRate,
        F::ChangeMpEstusFlaskCorrectRate,
        F::SoulRate,
        F::ItemDropRate,
        F::EquipWeightChangeRate,
        F::AddVigor,
        F::AddEndurance,
        F::AddArcane,
    ]
};

/// One plain line for a summed bonus: "+18% physical damage".
fn bonus_line(field: crate::powers::EffectField, value: f32) -> Option<String> {
    use crate::powers::EffectField as F;
    let percent = ((value - 1.0) * 100.0).round() as i32;
    let rate = |label: &str| (percent != 0).then(|| format!("{}{percent}% {label}", if percent > 0 { "+" } else { "" }));
    let flat = |label: &str| {
        let n = value.round() as i32;
        (n != 0).then(|| format!("{}{n} {label}", if n > 0 { "+" } else { "" }))
    };
    match field {
        F::PhysicsAttackRate => rate("physical damage"),
        F::SlashAttackRate => rate("slash damage"),
        F::BlowAttackRate => rate("strike damage"),
        F::ThrustAttackRate => rate("pierce damage"),
        F::MagicAttackRate => rate("magic damage"),
        F::FireAttackRate => rate("fire damage"),
        F::ThunderAttackRate => rate("lightning damage"),
        F::DarkAttackRate => rate("holy damage"),
        F::SaAttackPowerRate => rate("stance damage"),
        F::StaminaAttackRate => rate("damage to enemy stamina"),
        F::NeutralDamageCutRate => rate("physical damage taken"),
        F::SlashDamageCutRate => rate("slash damage taken"),
        F::BlowDamageCutRate => rate("strike damage taken"),
        F::ThrustDamageCutRate => rate("pierce damage taken"),
        F::MagicDamageCutRate => rate("magic damage taken"),
        F::FireDamageCutRate => rate("fire damage taken"),
        F::ThunderDamageCutRate => rate("lightning damage taken"),
        F::DarkDamageCutRate => rate("holy damage taken"),
        F::ToughnessDamageCutRate => rate("stagger taken"),
        F::MaxHpRate => rate("max HP"),
        F::MaxMpRate => rate("max FP"),
        F::MaxStaminaRate => rate("max stamina"),
        F::StaminaRecoverChangeSpeed => flat("stamina regen"),
        F::ConsumeStaminaRate => rate("stamina costs"),
        F::GuardStaminaMult => rate("stamina lost when blocking"),
        F::ChangeHpRate => (value.abs() > 0.001).then(|| format!("Heal {:.1}% HP per second", -value)),
        F::ChangeHpPoint => (value.abs() > 0.5).then(|| format!("Heal {:.0} HP per second", -value)),
        F::ChangeMpRate => (value.abs() > 0.001).then(|| format!("Regain {:.1}% FP per second", -value)),
        F::ChangeMpPoint => (value.abs() > 0.5).then(|| format!("Regain {:.0} FP per second", -value)),
        F::MagicConsumptionRate => rate("sorcery FP cost"),
        F::MiracleConsumptionRate => rate("incantation FP cost"),
        F::ArtsConsumptionRate => rate("skill FP cost"),
        F::ChangeHpEstusFlaskCorrectRate => rate("Crimson Flask healing"),
        F::ChangeMpEstusFlaskCorrectRate => rate("Cerulean Flask FP"),
        F::SoulRate => rate("runes"),
        F::ItemDropRate => flat("item discovery"),
        F::EquipWeightChangeRate => rate("equip load"),
        F::AddVigor => flat("Vigor"),
        F::AddEndurance => flat("Endurance"),
        F::AddArcane => flat("Arcane"),
        _ => None,
    }
}

/// 2.1: the attribute overhaul, one block per attribute (two columns when
/// there is room).
/// The attributes' lines. With `gain` = (after, pending), the numbers are
/// the `after` ones with what they add over `now` in green; `pending` means
/// those are the page's new points (headers show "15 -> 17").
fn attribute_list(
    ui: &Ui,
    now: AttributeValues,
    gain: Option<(AttributeValues, bool)>,
    strength: f32,
    columns: bool,
    boost: [u32; 8],
) {
    use crate::attributes::{Part, format_gain, rows, same_shape};
    let before = rows(now, strength);
    // Without new points on the page the lines keep today's numbers and only
    // the green gains come from one more point.
    let after = gain.map(|(a, _)| rows(a, strength));
    let pending = gain.is_some_and(|(_, p)| p);
    let width = ui.content_region_avail()[0];
    let two = columns && width >= 760.0;
    let column = if two { width * 0.5 } else { width };
    let x0 = ui.cursor_pos()[0];
    let top = ui.cursor_pos()[1];
    let mut ends = [top, top];
    for (i, row) in before.iter().enumerate() {
        let c = if two { i % 2 } else { 0 };
        let x = x0 + column * c as f32;
        ui.set_cursor_pos([x, ends[c]]);
        let next = after.as_ref().map(|a| &a[i]);
        let raised = pending && next.is_some_and(|n| n.points > row.points);
        // 3.3: like the game's own Level Up page, with new points on it only
        // the attributes you are raising are shown.
        if pending && !raised {
            continue;
        }
        // 3.1: a boosted attribute (talisman, tear, Great Rune) in the
        // game's blue, with the boost beside it.
        let boosted = boost.get(i).copied().unwrap_or(0);
        let number = if boosted > 0 { BOOSTED } else { GOLD };
        ui.text_colored(GOLD, row.name);
        ui.same_line_with_spacing(0.0, 6.0);
        if raised {
            let n = next.map(|n| n.points).unwrap_or(row.points);
            ui.text_colored(number, format!("{} \u{2192} {n}", row.points));
        } else {
            ui.text_colored(number, row.points.to_string());
        }
        if boosted > 0 {
            ui.same_line_with_spacing(0.0, 4.0);
            ui.text_colored(alpha(BOOSTED, 0.8), format!("(+{boosted})"));
        }
        ui.same_line();
        ui.text_colored(MUTED, row.job);
        for (l, line) in row.lines.iter().enumerate() {
            let other = next.and_then(|n| n.lines.get(l));
            // 3.3: of a raised attribute, only the lines that change.
            if raised {
                let changes = other.is_some_and(|o| {
                    !same_shape(line, o)
                        || line.iter().zip(o.iter()).any(|(b, a)| match (b, a) {
                            (Part::Num { value: v, unit, sign }, Part::Num { value: w, .. }) => {
                                format_gain(*v, *w, *unit, *sign).is_some()
                            }
                            _ => false,
                        })
                });
                if !changes {
                    continue;
                }
            }
            ui.set_cursor_pos([x + 14.0, ui.cursor_pos()[1]]);
            // Raised on the page: show the new numbers. Otherwise today's.
            let shown = if raised { other.unwrap_or(line) } else { line };
            let comparable = other.is_some_and(|o| same_shape(line, o));
            let mut first = true;
            let mut put = |text: &str, color: Option<[f32; 4]>| {
                if text.is_empty() {
                    return;
                }
                if !first {
                    ui.same_line_with_spacing(0.0, 0.0);
                }
                first = false;
                match color {
                    Some(color) => ui.text_colored(color, text),
                    None => ui.text(text),
                }
            };
            for (k, part) in shown.iter().enumerate() {
                put(&part.show(), None);
                if let (true, Part::Num { unit, sign, .. }) = (comparable && (raised || !pending), part) {
                    let (Some(Part::Num { value: b, .. }), Some(Part::Num { value: a, .. })) =
                        (line.get(k), other.and_then(|o| o.get(k)))
                    else {
                        continue;
                    };
                    if let Some(text) = format_gain(*b, *a, *unit, *sign) {
                        put(&format!(" ({text})"), Some(GAIN));
                    }
                }
            }
            // A line that changes its words (Faith reaching 30).
            if (raised || !pending) && other.is_some_and(|o| !same_shape(line, o)) {
                if raised {
                    put("  (new)", Some(GAIN));
                } else {
                    put("  (the next point unlocks it)", Some(GAIN));
                }
            }
        }
        ends[c] = ui.cursor_pos()[1] + 5.0;
    }
    ui.set_cursor_pos([x0, ends[0].max(ends[1])]);
}

/// 2.0: the nine Paths as small chips (name, points, three tier marks).
fn path_strip(ui: &Ui, points: &crate::paths::PathPoints) {
    use crate::paths::PATHS;
    let available = ui.content_region_avail()[0];
    let per_row = if available >= 1060.0 { 9 } else { 5 };
    let gap = 8.0;
    let w = ((available - gap * (per_row as f32 - 1.0)) / per_row as f32).max(80.0);
    let h = 44.0;
    for (i, path) in PATHS.iter().enumerate() {
        if i % per_row != 0 {
            ui.same_line_with_spacing(0.0, gap);
        }
        let p = ui.cursor_screen_pos();
        let have = points[path.index()];
        let tier = crate::paths::tier(have);
        let color = path.color();
        {
            let dl = ui.get_window_draw_list();
            dl.add_rect(p, [p[0] + w, p[1] + h], tint(color, if have > 0 { 0.20 } else { 0.08 })).filled(true).rounding(5.0).build();
            dl.add_rect(p, [p[0] + w, p[1] + h], alpha(color, if have > 0 { 0.85 } else { 0.30 })).thickness(1.0).rounding(5.0).build();
            dl.add_text([p[0] + 8.0, p[1] + 4.0], if have > 0 { color } else { alpha(color, 0.5) }, path.name().to_uppercase());
            let number = have.to_string();
            let nw = ui.calc_text_size(&number)[0];
            dl.add_text([p[0] + w - 8.0 - nw, p[1] + 4.0], if have > 0 { PAPER } else { MUTED }, &number);
            for t in 0..3 {
                let c = [p[0] + 13.0 + t as f32 * 15.0, p[1] + h - 11.0];
                diamond(&dl, c, 4.5, if t < tier { color } else { alpha(MUTED, 0.55) }, t < tier);
            }
        }
        ui.dummy([w, h]);
        if ui.is_item_hovered() {
            ui.tooltip(|| {
                ui.text_colored(color, format!("{}  ·  {}", path.name(), path.theme()));
                for bonus in path.bonuses() {
                    let on = have >= bonus.points;
                    ui.text_colored(if on { GOLD } else { MUTED }, format!("{:>2}  {}: {}", bonus.points, bonus.name, bonus.text));
                }
            });
        }
    }
}

/// 2.0: the whole mod in a few plain lines.
fn guide_tab(ui: &Ui, fonts: Fonts) {
    let section = |title: &str, lines: &[&str]| {
        ui.dummy([0.0, 6.0]);
        with_font(ui, fonts.heading, || ui.text_colored(GOLD, title));
        for line in lines {
            ui.text_wrapped(line);
        }
    };
    ui.text_colored(MUTED, format!("Elden Ring: Ascension {}", env!("CARGO_PKG_VERSION")));
    section("Bosses give cards", &[
        "Beat a boss for the first time and pick 1 of 3 cards. A card is a permanent power for this character. It stays after death and in NG+.",
        "Bigger bosses give rarer cards. Great bosses always offer their own Signature card.",
        "Ultra rare: Prismatic cards, every colour at once and a MAX WIN when they turn over. Ridiculous on their own; bigger bosses and luck make them a little more likely.",
        "Bonuses stack with soft caps: damage counts fully up to +60% and half beyond, damage taken down to -35% and half beyond. Strong builds feel strong, never silly.",
    ]);
    section("Levels", &[
        "Picking a card you own raises its level: I to V. Every level is stronger. Two cards at V can awaken an Evolution.",
    ]);
    section("Paths", &[
        "Every card belongs to one of nine Paths. Each card level is 1 point in its Path. At 5, 10 and 15 points the Path gives a bonus. See the Paths tab.",
        "At 6 points a Path opens its two Keystones: legendary cards that bend a build around one idea, some with a price.",
    ]);
    section("Embers", &[
        "Bosses and Ascended Elites (glowing enemies) give Embers. Spend them on the reward screen: Reroll, Lock a card for next time, or Banish a card forever. You can also Skip a reward for runes.",
    ]);
    section("Your attributes", &[
        "Level up at a Site of Grace as always. On top of the base game every attribute has its own job:",
        "Vigor survival  ·  Mind FP and spirit summons  ·  Endurance stamina and poise  ·  Strength heavy and charged blows  ·  Dexterity attack speed and ripostes  ·  Intelligence dodge timing and card spell recharge  ·  Faith protection and flasks  ·  Arcane runes, luck and statuses.",
        "The exact numbers are beside the Level Up page and in the You tab. On the Level Up page, green shows what your new points add before you confirm (or what one more point adds).",
        "Talismans, physick tears, Great Runes and cards that raise an attribute raise its bonuses too (shown in blue). Damage buffs (tears, talismans, incantations) also boost your card spells.",
    ]);
    section("Big moments", &[
        "Stance breaks, parries, ripostes, backstabs, executions, rampages, perfect dodges and status effects get their own label and sound; the label also names your cards that react.",
        "An Ascended Elite felled, a new Path bonus, death defied, the Tide rising and an Ascended Boss get a banner at the top. Sounds: Settings tab.",
        "The You tab keeps your Chronicle: every boss you beat and the card it gave.",
        "Spells from criticals, parries and stance breaks wait until the enemy can be hit again (not while it is thrown or getting up), so they never fly through it.",
    ]);
    section("The world", &[
        "Enemies are tougher and give more runes. The Tide rises as your cards grow: enemies get a bit stronger, runes a bit richer. In NG+ bosses gain affixes.",
        "From Tide I a boss may rise Ascended with affixes of its own (a banner warns you). Beat it for rarer cards, +1 Ember per affix, and one more card with two or more.",
    ]);
    section("Keys", &[
        "F7 collection  ·  F6 combat tracker  ·  Reward: 1-4 or click, Enter to choose, R reroll, L lock, X banish, S skip, Esc later.",
    ]);
}

fn roman_tide(tide: u8) -> &'static str {
    ["0", "I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X"][usize::from(tide.min(10))]
}

/// 12 345 678 style.

fn grouped(value: u32) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// Small padlock for the locked card.
fn padlock(dl: &DrawListMut, c: [f32; 2], s: f32, color: [f32; 4]) {
    dl.add_rect([c[0] - s, c[1] - s * 0.2], [c[0] + s, c[1] + s * 1.1], color)
        .filled(true)
        .rounding(2.0)
        .build();
    dl.add_circle([c[0], c[1] - s * 0.2], s * 0.65, color)
        .thickness(2.5)
        .num_segments(16)
        .build();
    dl.add_circle([c[0], c[1] + s * 0.4], s * 0.22, [0.05, 0.05, 0.04, 1.0])
        .filled(true)
        .build();
}

fn clock(seconds: f32) -> String {
    let s = seconds.max(0.0).round() as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

/// "Adaptive (fire)" with the damage kind it resists now; with the summary
/// for the statistics tab.
fn affix_label(affix: crate::fate::BossAffix, element: usize, summary: bool) -> String {
    let name = match affix {
        crate::fate::BossAffix::Adaptive => format!(
            "Adaptive: {}",
            crate::fate::ELEMENTS[element.min(crate::fate::ELEMENTS.len() - 1)]
        ),
        other => other.name().to_owned(),
    };
    if summary {
        format!("{name} ({})", affix.summary())
    } else {
        name
    }
}
