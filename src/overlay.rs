//! Ascension reward/collection UI and optional diagnostic panel.
use crate::session::RuntimeView;
use crate::{cards_ui::CardUi, runtime::SharedRuntime};
use hudhook::{
    ImguiRenderLoop,
    imgui::{self, Condition, StyleColor},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub struct AscensionOverlay {
    view: Arc<Mutex<RuntimeView>>,
    expanded: bool,
    cards: Option<CardUi>,
    shared: Option<Arc<SharedRuntime>>,
    modal: bool,
    /// 0.7: project root (card art) and the log.
    root: PathBuf,
    session: crate::session::Session,
    /// 0.8.1: the virtual cursor while an Ascension menu owns the mouse, and
    /// the buttons last handed to ImGui.
    cursor: Option<[f32; 2]>,
    buttons: u8,
}
impl AscensionOverlay {
    pub fn new(session: crate::session::Session, shared: Option<Arc<SharedRuntime>>, root: PathBuf) -> Self {
        Self {
            view: session.view.clone(),
            expanded: shared.is_none(),
            cards: shared.clone().map(CardUi::new),
            shared,
            modal: false,
            root,
            session,
            cursor: None,
            buttons: 0,
        }
    }

    /// 0.7: decode the card atlases and upload them as textures (once per
    /// renderer; the pixels are dropped afterwards).
    fn load_art(&mut self, render: &mut dyn hudhook::RenderContext) {
        // 1.0: always loaded; the Settings tab only hides or shows them.
        let Some(cards) = self.cards.as_mut() else {
            return;
        };

        let started = std::time::Instant::now();
        let pack = match crate::art::ArtPack::load(&self.root) {
            Ok(pack) => pack,
            Err(error) => {
                self.session.status(format!("Card art unavailable, cards are drawn without pictures: {error}"));
                return;
            }
        };
        let mut textures = Vec::with_capacity(pack.atlases.len());
        for atlas in &pack.atlases {
            match render.load_texture(&atlas.rgba, atlas.width, atlas.height) {
                Ok(id) => textures.push(id),
                Err(error) => {
                    self.session.status(format!(
                        "Card art texture upload failed ({}x{}): {error:?}; cards are drawn without pictures",
                        atlas.width, atlas.height
                    ));
                    return;
                }
            }
        }
        cards.set_art(crate::cards_ui::CardArt::new(&pack, textures));
        self.session.status(format!(
            "Card art: {} illustrations in {} textures ({:.0} MB) loaded in {} ms",
            pack.cards.len(),
            pack.atlases.len(),
            pack.bytes() as f64 / 1e6,
            started.elapsed().as_millis()
        ));
    }
}
impl ImguiRenderLoop for AscensionOverlay {
    fn initialize<'a>(
        &'a mut self,
        ctx: &mut imgui::Context,
        render: &'a mut dyn hudhook::RenderContext,
    ) {
        self.load_art(render);

        ctx.set_ini_filename(None);
        let font = |size: f32| imgui::FontSource::TtfData {
            data: include_bytes!("../assets/fonts/DejaVuSans.ttf"),
            size_pixels: size,
            config: Some(imgui::FontConfig {
                oversample_h: 2,
                oversample_v: 2,
                glyph_ranges: imgui::FontGlyphRanges::from_slice(&[0x20, 0x17f, 0x2000, 0x219f, 0]),
                ..Default::default()
            }),
        };
        ctx.fonts().add_font(&[font(18.0)]);
        let title = ctx.fonts().add_font(&[font(34.0)]);
        let heading = ctx.fonts().add_font(&[font(23.0)]);
        // 2.3: Noto Serif Display (SIL OFL, assets/fonts) for the combat labels
        // and banners, Latin-1 only.
        let display = |size: f32| imgui::FontSource::TtfData {
            data: include_bytes!("../assets/fonts/NotoSerifDisplay-Regular.ttf"),
            size_pixels: size,
            config: Some(imgui::FontConfig {
                oversample_h: 2,
                oversample_v: 2,
                glyph_ranges: imgui::FontGlyphRanges::from_slice(&[0x20, 0xff, 0]),
                ..Default::default()
            }),
        };
        let moment = ctx.fonts().add_font(&[display(44.0)]);
        let banner = ctx.fonts().add_font(&[display(60.0)]);
        if let Some(cards) = self.cards.as_mut() {
            cards.set_fonts(crate::cards_ui::Fonts {
                title: Some(title),
                heading: Some(heading),
                moment: Some(moment),
                banner: Some(banner),
            });
        }
        ctx.io_mut().config_flags.insert(
            imgui::ConfigFlags::NAV_ENABLE_KEYBOARD | imgui::ConfigFlags::NAV_ENABLE_GAMEPAD,
        );
        let style = ctx.style_mut();
        style.window_rounding = 7.0;
        style.window_padding = [20.0, 17.0];
        style.item_spacing = [10.0, 9.0];
        style.colors[StyleColor::WindowBg as usize] = [0.055, 0.057, 0.050, 0.94];
        style.colors[StyleColor::Text as usize] = [0.88, 0.85, 0.75, 1.0];
        style.colors[StyleColor::Border as usize] = [0.49, 0.39, 0.20, 0.8];
    }
    fn before_render<'a>(
        &'a mut self,
        ctx: &mut imgui::Context,
        _: &'a mut dyn hudhook::RenderContext,
    ) {
        feed_controller(ctx.io_mut());
        // 0.8.1: Elden Ring keeps the Windows cursor in the middle while you
        // play; in Ascension's own menus the mouse is taken from the game
        // (DirectInput) and drives this virtual cursor instead.
        let hooked = crate::mouse::HOOKED.load(std::sync::atomic::Ordering::Acquire);
        let capture = self.modal && hooked;
        crate::mouse::CAPTURE.store(capture, std::sync::atomic::Ordering::Relaxed);
        let io = ctx.io_mut();
        if capture {
            let size = io.display_size;
            let pos = self.cursor.get_or_insert([size[0] * 0.5, size[1] * 0.5]);
            let (dx, dy) = crate::mouse::take_delta();
            pos[0] = (pos[0] + dx as f32).clamp(0.0, (size[0] - 1.0).max(0.0));
            pos[1] = (pos[1] + dy as f32).clamp(0.0, (size[1] - 1.0).max(0.0));
            io.add_mouse_pos_event(*pos);
            let wheel = crate::mouse::take_wheel();
            if wheel != 0 {
                io.add_mouse_wheel_event([0.0, wheel as f32 / 120.0]);
            }
            let buttons = crate::mouse::buttons();
            for (bit, button) in [(0u8, imgui::MouseButton::Left), (1, imgui::MouseButton::Right), (2, imgui::MouseButton::Middle)] {
                let now = buttons & (1 << bit) != 0;
                if now != (self.buttons & (1 << bit) != 0) {
                    io.add_mouse_button_event(button, now);
                }
            }
            self.buttons = buttons;
            io.mouse_draw_cursor = true;
        } else {
            if self.cursor.take().is_some() {
                crate::mouse::reset();
                for button in [imgui::MouseButton::Left, imgui::MouseButton::Right, imgui::MouseButton::Middle] {
                    io.add_mouse_button_event(button, false);
                }
                self.buttons = 0;
            }
            io.mouse_draw_cursor = self.modal;
        }
    }
    fn render(&mut self, ui: &mut imgui::Ui) {
        self.modal = self.cards.as_mut().is_some_and(|cards| cards.render(ui));
        if let Some(shared) = &self.shared {
            shared
                .modal
                .store(self.modal, std::sync::atomic::Ordering::Release);
        }
        if ui.is_key_pressed_no_repeat(imgui::Key::F8) {
            self.expanded = !self.expanded;
        }
        if !self.expanded {
            return;
        }
        let Ok(view) = self.view.try_lock() else {
            return;
        };
        ui.window("ASCENSION | Runtime")
            .position([28.0, 28.0], Condition::FirstUseEver)
            .size([470.0, 0.0], Condition::Always)
            .title_bar(false)
            .resizable(false)
            .movable(false)
            .collapsible(false)
            .mouse_inputs(false)
            .build(|| {
                ui.text_colored(
                    [0.82, 0.66, 0.33, 1.0],
                    "E L D E N   R I N G :   A S C E N S I O N",
                );
                ui.text_disabled("F7 collection   /   F8 diagnostics");
                ui.separator();
                ui.text_wrapped(&view.phase);
                if let Some(error) = &view.error {
                    ui.text_colored([1.0, 0.40, 0.32, 1.0], error);
                }
                if let Some(snapshot) = &view.snapshot {
                    ui.text(format!(
                        "World {}   |   Offline {}   |   NG+ {}",
                        if snapshot.loaded { "loaded" } else { "waiting" },
                        snapshot.offline,
                        snapshot.ng_plus
                    ));
                    if let Some(identity) = &snapshot.identity {
                        ui.text(format!(
                            "{}   /   slot {}",
                            identity.name, identity.save_slot
                        ));
                        ui.text_disabled(if identity.verified {
                            "Persistent Ascension identity attached"
                        } else {
                            "Preparing persistent Ascension identity"
                        });
                    }
                    let r = &snapshot.resources;
                    ui.text(format!(
                        "HP {}/{}    FP {}/{}    STA {}/{}",
                        r.hp, r.max_hp, r.fp, r.max_fp, r.stamina, r.max_stamina
                    ));
                    let a = &snapshot.attributes;
                    ui.text(format!(
                        "VIG {}   MND {}   END {}   STR {}",
                        a.vigor, a.mind, a.endurance, a.strength
                    ));
                    ui.text(format!(
                        "DEX {}   INT {}   FAI {}   ARC {}",
                        a.dexterity, a.intelligence, a.faith, a.arcane
                    ));
                    ui.separator();
                    ui.text(format!(
                        "Boss flags: {} read / {} defeated",
                        snapshot.boss_defeated.len(),
                        snapshot.boss_defeated.values().filter(|&&v| v).count()
                    ));
                    ui.text(format!(
                        "Observed transitions: {}   Samples: {}",
                        view.flag_transitions, view.samples
                    ));
                    if !snapshot.can_observe {
                        ui.text_colored(
                            [0.9, 0.62, 0.25, 1.0],
                            "Flag snapshot not accepted as progression baseline",
                        );
                    }
                    if let Some(shared) = &self.shared {
                        if let Ok(progress) = shared.view.try_lock() {
                            ui.text_wrapped(&progress.status);
                            if let Some(error) = &progress.error {
                                ui.text_colored([1.0, 0.4, 0.3, 1.0], error);
                            }
                        }
                    }
                } else {
                    ui.text("Waiting for first main-thread snapshot...");
                }
            });
    }
    fn message_filter(&self, io: &imgui::Io) -> hudhook::MessageFilter {
        let _ = io;
        if self.modal {
            hudhook::MessageFilter::InputAll
        } else {
            hudhook::MessageFilter::empty()
        }
    }

}

fn feed_controller(io: &mut imgui::Io) {
    use windows::Win32::UI::Input::XboxController::*;
    let mut state = XINPUT_STATE::default();
    let connected = (0..4).any(|index| unsafe { XInputGetState(index, &mut state) } == 0);
    io.backend_flags
        .set(imgui::BackendFlags::HAS_GAMEPAD, connected);
    let buttons = if connected {
        state.Gamepad.wButtons.0
    } else {
        0
    };
    for (key, mask) in [
        (imgui::Key::GamepadFaceDown, XINPUT_GAMEPAD_A.0),
        (imgui::Key::GamepadFaceRight, XINPUT_GAMEPAD_B.0),
        (imgui::Key::GamepadFaceLeft, XINPUT_GAMEPAD_X.0),
        (imgui::Key::GamepadFaceUp, XINPUT_GAMEPAD_Y.0),
        (imgui::Key::GamepadDpadLeft, XINPUT_GAMEPAD_DPAD_LEFT.0),
        (imgui::Key::GamepadDpadRight, XINPUT_GAMEPAD_DPAD_RIGHT.0),
        (imgui::Key::GamepadDpadUp, XINPUT_GAMEPAD_DPAD_UP.0),
        (imgui::Key::GamepadDpadDown, XINPUT_GAMEPAD_DPAD_DOWN.0),
        (imgui::Key::GamepadBack, XINPUT_GAMEPAD_BACK.0),
    ] {
        io.add_key_event(key, buttons & mask != 0);
    }
    let x = if connected {
        state.Gamepad.sThumbLX as f32 / 32767.0
    } else {
        0.0
    };
    let y = if connected {
        state.Gamepad.sThumbLY as f32 / 32767.0
    } else {
        0.0
    };
    for (key, value) in [
        (imgui::Key::GamepadLStickLeft, -x),
        (imgui::Key::GamepadLStickRight, x),
        (imgui::Key::GamepadLStickUp, y),
        (imgui::Key::GamepadLStickDown, -y),
    ] {
        let value = ((value - 0.2) / 0.8).clamp(0.0, 1.0);
        io.add_key_analog_event(key, value > 0.0, value);
    }
}
