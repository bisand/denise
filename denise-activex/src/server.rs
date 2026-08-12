//! The four exports that make a DLL a COM server, and self-registration.
//!
//! `regsvr32 denise_activex.dll` calls `DllRegisterServer`; a container asking
//! for the class id calls `DllGetClassObject`; a host tidying up calls
//! `DllCanUnloadNow`. Between them they are the entire contract a DLL has with
//! COM, and none of them is optional.

use std::sync::atomic::{AtomicUsize, Ordering};

use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, E_POINTER, HMODULE, MAX_PATH, S_FALSE, S_OK,
};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    GetModuleFileNameW, GetModuleHandleExW,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CLASSES_ROOT, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteKeyW, RegSetValueExW,
};
use windows_core::{GUID, HRESULT, Interface, PCWSTR};

use crate::control::DenisePanel;
use crate::factory::PanelFactory;
use crate::registry;

/// The control's class id, as a GUID.
///
/// The same value as [`registry::CLSID_TEXT`], which a test checks — the registry
/// holds the text form and COM compares the binary one, and two spellings of one
/// identity is precisely the sort of thing that silently disagrees.
pub const CLSID_DENISE_PANEL: GUID = GUID::from_u128(0x7F1B_483A_5853_4348_9081_D5BD_502B_51E8);

/// Live objects and outstanding server locks. `DllCanUnloadNow` is the only
/// reader, and a host that unloads a DLL still holding a live control crashes in
/// somebody else's stack frame.
static OUTSTANDING: AtomicUsize = AtomicUsize::new(0);

/// Records one more reason the DLL must stay loaded.
pub(crate) fn lock_server() {
    OUTSTANDING.fetch_add(1, Ordering::Relaxed);
}

/// Releases one.
pub(crate) fn unlock_server() {
    OUTSTANDING.fetch_sub(1, Ordering::Relaxed);
}

/// Hands a class object to COM.
///
/// # Safety
///
/// Called by COM with a valid class id, interface id and out-pointer.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut core::ffi::c_void,
) -> HRESULT {
    if ppv.is_null() || rclsid.is_null() || riid.is_null() {
        return E_POINTER;
    }
    // SAFETY: COM promises readable class and interface ids, and a writable
    // out-pointer.
    unsafe {
        ppv.write(core::ptr::null_mut());
        if *rclsid != CLSID_DENISE_PANEL {
            // The one honest answer for a class this server does not implement.
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: windows_core::IUnknown = PanelFactory.into();
        factory.query(riid, ppv)
    }
}

/// Whether COM may unload this DLL.
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if OUTSTANDING.load(Ordering::Relaxed) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}

/// Writes every registry value the control needs to be findable.
#[unsafe(no_mangle)]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    match register() {
        Ok(()) => S_OK,
        Err(e) => e.into(),
    }
}

/// Removes them again.
#[unsafe(no_mangle)]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    match unregister() {
        Ok(()) => S_OK,
        Err(e) => e.into(),
    }
}

fn register() -> windows_core::Result<()> {
    let path = server_path()?;
    for entry in registry::entries(&path) {
        write_value(&entry.key, &entry.name, &entry.value)?;
    }
    Ok(())
}

fn unregister() -> windows_core::Result<()> {
    for key in registry::keys_to_remove() {
        let wide = wide(&key);
        // SAFETY: `wide` is NUL-terminated and live for the call. A key that is
        // already gone is not a failure — unregistering twice, or after a partial
        // registration, must still end with the class absent.
        unsafe {
            let _ = RegDeleteKeyW(HKEY_CLASSES_ROOT, PCWSTR(wide.as_ptr()));
        }
    }
    Ok(())
}

/// Creates `key` under `HKEY_CLASSES_ROOT` and sets one string value in it.
///
/// An empty `name` means the key's default value, which is most of them.
fn write_value(key: &str, name: &str, value: &str) -> windows_core::Result<()> {
    let key_wide = wide(key);
    let name_wide = wide(name);
    let value_wide = wide(value);
    let mut handle = HKEY::default();

    // SAFETY: every pointer is to a NUL-terminated buffer live for the call, and
    // `handle` receives the opened key.
    unsafe {
        RegCreateKeyExW(
            HKEY_CLASSES_ROOT,
            PCWSTR(key_wide.as_ptr()),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut handle,
            None,
        )
        .ok()?;

        // The length is in *bytes* and includes the terminator, which is what
        // makes a value readable as a string rather than as a truncated one.
        let bytes = core::slice::from_raw_parts(
            value_wide.as_ptr().cast::<u8>(),
            value_wide.len() * core::mem::size_of::<u16>(),
        );
        let result = RegSetValueExW(
            handle,
            if name.is_empty() {
                PCWSTR::null()
            } else {
                PCWSTR(name_wide.as_ptr())
            },
            None,
            REG_SZ,
            Some(bytes),
        );
        let _ = RegCloseKey(handle);
        result.ok()?;
    }
    Ok(())
}

/// The full path of this DLL, which is what `InprocServer32` has to contain.
///
/// Taken from the loader rather than assumed: a server registered with the wrong
/// path is a class a host can see and cannot load.
fn server_path() -> windows_core::Result<String> {
    let mut module = HMODULE::default();
    // SAFETY: the address is of a function in this DLL, which is exactly what
    // `FROM_ADDRESS` wants. `UNCHANGED_REFCOUNT` means this does not pin the
    // module, which matters because nothing here unpins it.
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(server_path as *const () as *const u16),
            &mut module,
        )?;
    }

    let mut buffer = [0u16; MAX_PATH as usize];
    // SAFETY: `buffer` is live and its length is passed correctly.
    let written = unsafe { GetModuleFileNameW(Some(module), &mut buffer) };
    if written == 0 {
        return Err(windows_core::Error::from_thread());
    }
    Ok(String::from_utf16_lossy(&buffer[..written as usize]))
}

/// A NUL-terminated UTF-16 buffer, which is what every `W` entry point wants.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(core::iter::once(0)).collect()
}

/// Keeps the control type reachable from here, so `DllGetClassObject` and the
/// object it hands out cannot drift apart.
const _: fn() -> DenisePanel = DenisePanel::new;

#[cfg(test)]
mod tests {
    use super::*;

    /// The registry holds the class id as text and COM compares it as bytes. Two
    /// spellings of one identity is exactly what silently disagrees, and the
    /// symptom is a control that registers and will not instantiate.
    #[test]
    fn the_binary_and_text_class_ids_are_the_same_identity() {
        let text = format!("{{{:?}}}", CLSID_DENISE_PANEL).to_uppercase();
        assert_eq!(text, registry::CLSID_TEXT.to_uppercase());
    }

    #[test]
    fn a_wide_string_is_terminated() {
        let w = wide("AB");
        assert_eq!(w, vec![0x41, 0x42, 0x00]);
        assert_eq!(*wide("").last().expect("terminator"), 0);
    }

    /// A host that unloads a DLL still holding a live control crashes in somebody
    /// else's stack frame, so this has to start at "safe" and stop being safe the
    /// moment anything is outstanding.
    #[test]
    fn the_dll_only_unloads_with_nothing_outstanding() {
        assert_eq!(DllCanUnloadNow(), S_OK);
        lock_server();
        assert_eq!(DllCanUnloadNow(), S_FALSE);
        unlock_server();
        assert_eq!(DllCanUnloadNow(), S_OK);
    }
}
