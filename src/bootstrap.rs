use crate::{config::Config, game, overlay::AscensionOverlay, session::Session};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use windows::Win32::{
    Foundation::{HINSTANCE, HMODULE},
    System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleA},
};

static MODULE: AtomicUsize = AtomicUsize::new(0);
static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// ME3 initializes us outside loader lock. DllMain only records the module handle.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllMain(
    module: HINSTANCE,
    reason: u32,
    _: *mut std::ffi::c_void,
) -> i32 {
    if reason == 1 {
        MODULE.store(module.0 as usize, Ordering::Release);
    }
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn ascension_init() -> bool {
    if INITIALIZED.swap(true, Ordering::AcqRel) {
        return true;
    }
    let started = std::thread::Builder::new()
        .name("ascension-bootstrap".into())
        .spawn(|| {
            let _ = std::panic::catch_unwind(|| run());
        })
        .is_ok();
    if !started {
        INITIALIZED.store(false, Ordering::Release);
    }
    started
}

fn module_path(module: Option<HMODULE>) -> Result<PathBuf, String> {
    let mut buffer = vec![0u16; 32768];
    let len = unsafe { GetModuleFileNameW(module, &mut buffer) } as usize;
    if len == 0 || len >= buffer.len() {
        return Err("module path unavailable".into());
    }
    Ok(PathBuf::from(String::from_utf16_lossy(&buffer[..len])))
}

fn wine_builtin(module: HMODULE) -> bool {
    // A successfully loaded HMODULE has a readable mapped PE header. Wine's
    // loader identifies builtin modules by this same DOS-header signature.
    // https://github.com/wine-mirror/wine/blob/master/dlls/ntdll/loader.c
    let header = unsafe { std::slice::from_raw_parts(module.0.cast::<u8>(), 81) };
    let nt_offset = u32::from_le_bytes(header[60..64].try_into().unwrap());
    header.starts_with(b"MZ") && nt_offset >= 81 && &header[64..81] == b"Wine builtin DLL\0"
}

fn run() {
    let module = MODULE.load(Ordering::Acquire);
    let Ok(path) = module_path(Some(HMODULE(module as *mut _))) else {
        return;
    };
    let Some(root) = path.parent().and_then(|p| p.parent()).map(PathBuf::from) else {
        return;
    };
    let Ok(session) = Session::start(root.clone()) else {
        return;
    };
    let panic_session = session.clone();
    std::panic::set_hook(Box::new(move |info| {
        panic_session.error(format!("Rust panic; runtime stopped: {info}"))
    }));
    session.status(format!(
        "Ascension {} initializer entered",
        env!("CARGO_PKG_VERSION")
    ));
    let (config, bosses, compatibility) = match Config::load(&root) {
        Ok(config) => config,
        Err(error) => {
            session.error(format!("configuration rejected: {error}"));
            return;
        }
    };
    let exe_path = match module_path(None) {
        Ok(path) => path,
        Err(error) => {
            session.error(error);
            return;
        }
    };
    let bytes = match std::fs::read(&exe_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            session.error(format!("cannot verify executable: {error}"));
            return;
        }
    };
    let hash: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    drop(bytes);
    if !compatibility
        .sha256
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(&hash))
    {
        session.error(format!(
            "unsupported executable SHA256 {hash}; no game adapter or hooks installed"
        ));
        return;
    }
    session.status(format!(
        "Executable verified: {} / {hash}; waiting for game window",
        compatibility.product_version
    ));
    for name in [c"dinput8.dll", c"winhttp.dll"] {
        if let Ok(handle) = unsafe { GetModuleHandleA(windows::core::PCSTR(name.as_ptr().cast())) }
        {
            match module_path(Some(handle)) {
                Ok(origin) => {
                    let builtin = wine_builtin(handle);
                    session.status(format!(
                        "Module {:?}: {} (Wine builtin={builtin})",
                        name,
                        origin.display()
                    ));
                    // GetModuleFileName may report the app-directory search
                    // path even when Wine actually mapped a builtin module.
                    let same_folder =
                        origin
                            .parent()
                            .zip(exe_path.parent())
                            .is_some_and(|(a, b)| {
                                a.to_string_lossy()
                                    .eq_ignore_ascii_case(&b.to_string_lossy())
                            });
                    if same_folder && !builtin {
                        session.error(
                            "Existing game-folder proxy loaded; adapter and overlay disabled",
                        );
                        return;
                    }
                }
                Err(error) => {
                    session.error(error);
                    return;
                }
            }
        }
    }
    let flags = bosses.iter().map(|boss| boss.flag).collect();
    let mut controller = match crate::runtime::Runtime::new(
        root.clone(),
        config.clone(),
        bosses.clone(),
        session.clone(),
    ) {
        Ok(controller) => Some(controller),
        Err(error) => {
            session.error(format!("Ascension initialization stopped: {error}"));
            None
        }
    };
    let shared = controller
        .as_ref()
        .map(|controller| controller.shared.clone());
    if let Some(shared) = shared.clone() {
        let panic_session = session.clone();
        std::panic::set_hook(Box::new(move |info| {
            shared.safe.store(false, Ordering::Release);
            shared.modal.store(false, Ordering::Release);
            shared.roll_ms.store(0, Ordering::Release);
            shared.item_speed.store(0, Ordering::Release);
            shared.attack_speed.store(0, Ordering::Release);
            crate::mouse::CAPTURE.store(false, Ordering::Release);

            panic_session.error(format!("Ascension runtime stopped after panic: {info}"));
        }));
    }
    let callback_session = session.clone();
    let diagnostic_interval =
        std::time::Duration::from_millis(config.diagnostics.snapshot_interval_ms);
    let mut last_fallback_snapshot = std::time::Instant::now() - diagnostic_interval;
    let task = match game::register(flags, move |snapshot| {
        if let Some(controller) = controller.as_mut() {
            unsafe { controller.tick(snapshot) }
        } else if last_fallback_snapshot.elapsed() >= diagnostic_interval {
            callback_session.snapshot(snapshot);
            last_fallback_snapshot = std::time::Instant::now();
        }
    }) {
        Ok(task) => task,
        Err(error) => {
            session.error(error);
            return;
        }
    };
    session.status(format!(
        "Native Ascension task registered; {} encounter flags",
        bosses.len()
    ));
    if let Some(shared) = &shared {
        match crate::roll::install(shared.roll_ms.clone()) {
            Ok(tasks) => std::mem::forget(tasks),
            Err(error) => session.error(format!("Roll extension unavailable: {error}")),
        }
        match crate::haste::install(shared.item_speed.clone(), shared.attack_speed.clone(), shared.haste_log.clone()) {

            Ok(task) => {
                std::mem::forget(task);
                shared.item_speed_ready.store(true, Ordering::Release);
            }
            Err(error) => session.error(format!("Item use speed unavailable: {error}")),
        }
        // 3.3 No Quarter: a critical's victim can be hit while it gets up.
        match crate::pursuit::install_unguard() {
            Ok(tasks) => std::mem::forget(tasks),
            Err(error) => session.error(format!("No Quarter unavailable: {error}")),
        }
        match crate::native_input::install(shared.modal.clone(), shared.safe.clone()) {
            Ok(tasks) => std::mem::forget(tasks),
            Err(error) => session.error(format!("Card input capture unavailable: {error}")),
        }
    }
    if shared.is_some() {
        let ready = crate::sound::init(&config.ui);
        session.status(format!(
            "Interface sounds: {ready} ready (volume {:.2}, reflex sounds {})",
            config.ui.sound_volume,
            if config.ui.reflex_sounds { "on" } else { "off" }
        ));
    }
    if config.diagnostics.overlay || shared.is_some() {
        let result = hudhook::Hudhook::builder()
            .with::<hudhook::hooks::dx12::ImguiDx12Hooks>(AscensionOverlay::new(
                session.clone(),
                shared,
                root.clone(),
            ))

            .build()
            .apply();
        match result {
            Ok(_) => session.status("Ascension interface installed; F7 collection, F8 diagnostics"),
            Err(error) => session.error(format!("D3D12 overlay installation failed: {error:?}")),
        }
        // 0.8.1: a free cursor in Ascension's menus (the game keeps the
        // Windows cursor centred while you play).
        match crate::mouse::install() {
            Ok(()) => session.status("Mouse capture for Ascension menus installed"),
            Err(error) => session.status(format!(
                "Mouse capture unavailable, Ascension menus use keyboard and controller: {error}"
            )),
        }

    }
    // Pinned upstream cannot unregister native tasks. Keep callback and DLL alive.
    std::mem::forget(task);
    loop {
        std::thread::park();
    }
}
