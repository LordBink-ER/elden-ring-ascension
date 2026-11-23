//! DLL entrypoint + unsupported host rejection, without starting Elden Ring.
#[cfg(windows)]
fn main() {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW},
        core::{PCSTR, PCWSTR},
    };
    let path = std::env::args_os()
        .nth(1)
        .expect("usage: load_probe.exe path/to/ascension.dll");
    let wide: Vec<u16> = path.encode_wide().chain(Some(0)).collect();
    let module = unsafe { LoadLibraryW(PCWSTR(wide.as_ptr())) }.expect("DLL must load");
    let pointer = unsafe { GetProcAddress(module, PCSTR(c"ascension_init".as_ptr().cast())) }
        .expect("initializer must be exported");
    let initialize: extern "C" fn() -> bool = unsafe { std::mem::transmute(pointer) };
    assert!(initialize());
    std::thread::sleep(std::time::Duration::from_secs(2));
    println!("Initializer invoked. Check isolated logs for unsupported executable rejection.");
    // Do not FreeLibrary while a native bootstrap may still exist.
}
#[cfg(not(windows))]
fn main() {
    println!("This DLL load probe is a Windows executable for an isolated Wine prefix.");
}
