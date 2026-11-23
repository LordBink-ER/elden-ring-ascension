//! 0.8.1: a free mouse cursor in Ascension's own menus.
//!
//! Elden Ring reads the mouse through DirectInput 8 and keeps the Windows
//! cursor in the middle of the window while you play (camera control), so an
//! overlay menu saw the cursor locked in the centre (user report, 0.8.0).
//! While an Ascension menu is open, the mouse device's reads are hooked:
//! movement, wheel and buttons are taken from the game (the camera stays still
//! and the character does not attack) and the movement drives a virtual
//! cursor that the overlay hands to ImGui. Clicks reach ImGui through window
//! messages as before. Same technique as the CS2 conversion's buy menu
//! (er-cs2mod cs2er/src/dinput.rs, used in game there); keyboard untouched.
use hudhook::mh::{MH_ApplyQueued, MH_Initialize, MhHook};
use std::{
    ffi::c_void,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering},
    },
};
use windows::{
    Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress, LoadLibraryA},
    core::{GUID, s},
};

const IID_IDIRECTINPUT8W: GUID =
    GUID::from_values(0xbf798031, 0x483a, 0x4da2, [0xaa, 0x99, 0x5d, 0x64, 0xed, 0x36, 0x97, 0x00]);
const GUID_SYS_MOUSE: GUID =
    GUID::from_values(0x6F1D2B60, 0xD5A0, 0x11CF, [0xBF, 0xC7, 0x44, 0x45, 0x53, 0x54, 0x00, 0x00]);
const DI8DEVTYPE_MOUSE: u8 = 0x12;

/// While set, the game gets no mouse movement, wheel or buttons.
pub static CAPTURE: AtomicBool = AtomicBool::new(false);
/// The hooks are in place (install succeeded).
pub static HOOKED: AtomicBool = AtomicBool::new(false);
/// Buttons held while capturing (bit 0 left, 1 right, 2 middle), for ImGui:
/// window messages may not arrive while the game owns the mouse.
static BUTTONS: AtomicU8 = AtomicU8::new(0);
static DX_STATE: AtomicI32 = AtomicI32::new(0);
static DY_STATE: AtomicI32 = AtomicI32::new(0);
static DX_DATA: AtomicI32 = AtomicI32::new(0);
static DY_DATA: AtomicI32 = AtomicI32::new(0);
static WHEEL: AtomicI32 = AtomicI32::new(0);

type GetDeviceStateFn = unsafe extern "system" fn(*mut c_void, u32, *mut u8) -> i32;
type GetDeviceDataFn = unsafe extern "system" fn(*mut c_void, u32, *mut u8, *mut u32, u32) -> i32;
type GetCapabilitiesFn = unsafe extern "system" fn(*mut c_void, *mut [u32; 11]) -> i32;
type DInput8CreateFn = unsafe extern "system" fn(usize, u32, *const GUID, *mut *mut c_void, usize) -> i32;
type CreateDeviceFn = unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void, usize) -> i32;

static TRAMP_STATE: OnceLock<GetDeviceStateFn> = OnceLock::new();
static TRAMP_DATA: OnceLock<GetDeviceDataFn> = OnceLock::new();
static GET_CAPS: OnceLock<GetCapabilitiesFn> = OnceLock::new();
static MOUSE_DEVICES: OnceLock<std::sync::Mutex<Vec<(usize, bool)>>> = OnceLock::new();

/// Mouse movement taken from the game since the last call (pixels-ish counts).
/// Immediate and buffered reads both see the same movement, so the larger is used.
pub fn take_delta() -> (i32, i32) {
    let s = (DX_STATE.swap(0, Ordering::Relaxed), DY_STATE.swap(0, Ordering::Relaxed));
    let d = (DX_DATA.swap(0, Ordering::Relaxed), DY_DATA.swap(0, Ordering::Relaxed));
    if s.0.abs() + s.1.abs() >= d.0.abs() + d.1.abs() { s } else { d }
}
/// Wheel notches (120 per notch) taken while capturing.
pub fn take_wheel() -> i32 {
    WHEEL.swap(0, Ordering::Relaxed)
}
/// Buttons held now (bit 0 left, 1 right, 2 middle), as read while capturing.
pub fn buttons() -> u8 {
    BUTTONS.load(Ordering::Relaxed)
}
pub fn reset() {
    let _ = take_delta();
    let _ = take_wheel();
    BUTTONS.store(0, Ordering::Relaxed);
}

unsafe fn is_mouse(dev: *mut c_void) -> bool {
    let map = MOUSE_DEVICES.get_or_init(Default::default);
    if let Ok(list) = map.lock() {
        if let Some((_, mouse)) = list.iter().find(|(d, _)| *d == dev as usize) {
            return *mouse;
        }
    }
    let mut caps = [0u32; 11];
    caps[0] = 44;
    let kind = match GET_CAPS.get() {
        Some(f) if unsafe { f(dev, &mut caps) } >= 0 => (caps[2] & 0xFF) as u8,
        _ => 0,
    };
    let mouse = kind == DI8DEVTYPE_MOUSE;
    if let Ok(mut list) = map.lock() {
        if list.len() < 64 {
            list.push((dev as usize, mouse));
        }
    }
    mouse
}

unsafe extern "system" fn get_device_state_hook(dev: *mut c_void, cb: u32, data: *mut u8) -> i32 {
    let hr = unsafe { (TRAMP_STATE.get().unwrap())(dev, cb, data) };
    // DIMOUSESTATE = 16 bytes, DIMOUSESTATE2 = 20 bytes.
    if hr >= 0 && !data.is_null() && (cb == 16 || cb == 20) && CAPTURE.load(Ordering::Relaxed) {
        unsafe {
            let p = data as *mut i32;
            DX_STATE.fetch_add(*p, Ordering::Relaxed);
            DY_STATE.fetch_add(*p.add(1), Ordering::Relaxed);
            WHEEL.fetch_add(*p.add(2), Ordering::Relaxed);
            let mut held = 0u8;
            for i in 0..3 {
                if *data.add(12 + i) & 0x80 != 0 {
                    held |= 1 << i;
                }
            }
            BUTTONS.store(held, Ordering::Relaxed);
            std::ptr::write_bytes(data, 0, cb as usize);
        }
    }
    hr
}

unsafe extern "system" fn get_device_data_hook(
    dev: *mut c_void,
    cb_obj: u32,
    rgdod: *mut u8,
    inout: *mut u32,
    flags: u32,
) -> i32 {
    let hr = unsafe { (TRAMP_DATA.get().unwrap())(dev, cb_obj, rgdod, inout, flags) };
    if hr < 0 || rgdod.is_null() || inout.is_null() || cb_obj < 8 || !CAPTURE.load(Ordering::Relaxed) {
        return hr;
    }
    if !unsafe { is_mouse(dev) } {
        return hr;
    }
    let n = unsafe { *inout } as usize;
    for i in 0..n {
        let rec = unsafe { rgdod.add(i * cb_obj as usize) };
        let ofs = unsafe { *(rec as *const u32) };
        let val = unsafe { &mut *(rec.add(4) as *mut i32) };
        match ofs {
            0 => _ = DX_DATA.fetch_add(*val, Ordering::Relaxed),
            4 => _ = DY_DATA.fetch_add(*val, Ordering::Relaxed),
            8 => _ = WHEEL.fetch_add(*val, Ordering::Relaxed),
            // DIMOFS_BUTTON0..2
            12..=14 => {
                let bit = 1u8 << (ofs - 12);
                if *val & 0x80 != 0 {
                    BUTTONS.fetch_or(bit, Ordering::Relaxed);
                } else {
                    BUTTONS.fetch_and(!bit, Ordering::Relaxed);
                }
            }
            _ => {}
        }
        // Movement becomes 0, presses become releases.
        *val = 0;
    }
    hr
}

/// Resolve the shared mouse device vtable through a throwaway device and hook it.
pub fn install() -> Result<(), String> {
    unsafe {
        let _ = MH_Initialize();
        let lib = LoadLibraryA(s!("dinput8.dll")).map_err(|e| format!("LoadLibrary dinput8: {e}"))?;
        let create = GetProcAddress(lib, s!("DirectInput8Create")).ok_or("no DirectInput8Create")?;
        let create: DInput8CreateFn = std::mem::transmute(create);
        let hinst = GetModuleHandleA(None).map_err(|e| e.to_string())?;
        let mut di: *mut c_void = std::ptr::null_mut();
        let hr = create(hinst.0 as usize, 0x0800, &IID_IDIRECTINPUT8W, &mut di, 0);
        if hr < 0 || di.is_null() {
            return Err(format!("DirectInput8Create hr={hr:#x}"));
        }
        let di_vt = *(di as *const *const usize);
        let create_device: CreateDeviceFn = std::mem::transmute(*di_vt.add(3));
        let mut dev: *mut c_void = std::ptr::null_mut();
        let hr = create_device(di, &GUID_SYS_MOUSE, &mut dev, 0);
        if hr < 0 || dev.is_null() {
            return Err(format!("CreateDevice(mouse) hr={hr:#x}"));
        }
        let vt = *(dev as *const *const usize);
        let _ = GET_CAPS.set(std::mem::transmute(*vt.add(3)));
        let state_addr = *vt.add(9) as *mut c_void;
        let data_addr = *vt.add(10) as *mut c_void;
        let h1 = MhHook::new(state_addr, get_device_state_hook as *mut c_void)
            .map_err(|e| format!("hook GetDeviceState: {e:?}"))?;
        let h2 = MhHook::new(data_addr, get_device_data_hook as *mut c_void)
            .map_err(|e| format!("hook GetDeviceData: {e:?}"))?;
        let _ = TRAMP_STATE.set(std::mem::transmute(h1.trampoline()));
        let _ = TRAMP_DATA.set(std::mem::transmute(h2.trampoline()));
        h1.queue_enable().map_err(|e| format!("{e:?}"))?;
        h2.queue_enable().map_err(|e| format!("{e:?}"))?;
        MH_ApplyQueued().ok().map_err(|e| format!("MH_ApplyQueued {e:?}"))?;
        // The throwaway device and hooks stay alive for the whole process.
        std::mem::forget(h1);
        std::mem::forget(h2);
        HOOKED.store(true, Ordering::Release);
        Ok(())

    }
}
