use std::{
    ffi::CStr,
    fmt, io, iter,
    os::windows::{ffi::OsStrExt, io::RawHandle},
    path::Path,
    ptr,
    sync::Arc,
};
use talpid_types::ErrorExt;
use widestring::U16CStr;
use winapi::{
    shared::{
        guiddef::GUID,
        minwindef::{BOOL, FARPROC, HINSTANCE, HMODULE},
    },
    um::libloaderapi::{
        FreeLibrary, GetProcAddress, LoadLibraryExW, LOAD_WITH_ALTERED_SEARCH_PATH,
    },
};


type WintunOpenAdapterFn =
    unsafe extern "stdcall" fn(pool: *const u16, name: *const u16) -> RawHandle;

type WintunCreateAdapterFn = unsafe extern "stdcall" fn(
    pool: *const u16,
    name: *const u16,
    requested_guid: *const GUID,
    reboot_required: *mut BOOL,
) -> RawHandle;

type WintunFreeAdapterFn = unsafe extern "stdcall" fn(adapter: RawHandle);

type WintunDeleteAdapterFn = unsafe extern "stdcall" fn(
    adapter: RawHandle,
    force_close_sessions: BOOL,
    reboot_required: *mut BOOL,
) -> BOOL;


pub struct WintunDll {
    handle: HINSTANCE,
    func_open: WintunOpenAdapterFn,
    func_create: WintunCreateAdapterFn,
    func_free: WintunFreeAdapterFn,
    func_delete: WintunDeleteAdapterFn,
}

unsafe impl Sync for WintunDll {}

type RebootRequired = bool;

/// A new Wintun adapter that is destroyed when dropped.
#[derive(Debug)]
pub struct TemporaryWintunAdapter {
    pub adapter: WintunAdapter,
}

impl TemporaryWintunAdapter {
    pub fn create(
        dll_handle: Arc<WintunDll>,
        pool: &U16CStr,
        name: &U16CStr,
        requested_guid: Option<GUID>,
    ) -> io::Result<(Self, RebootRequired)> {
        let (adapter, reboot_required) =
            WintunAdapter::create(dll_handle, pool, name, requested_guid)?;
        Ok((TemporaryWintunAdapter { adapter }, reboot_required))
    }
}

impl Drop for TemporaryWintunAdapter {
    fn drop(&mut self) {
        if let Err(error) = unsafe {
            self.adapter
                .dll_handle
                .delete_adapter(self.adapter.handle, true)
        } {
            log::error!(
                "{}",
                error.display_chain_with_msg("Failed to delete Wintun adapter")
            );
        }
    }
}

/// Represents a Wintun adapter.
pub struct WintunAdapter {
    dll_handle: Arc<WintunDll>,
    handle: RawHandle,
}

impl fmt::Debug for WintunAdapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WintunAdapter")
            .field("handle", &self.handle)
            .finish()
    }
}

unsafe impl Send for WintunAdapter {}

impl WintunAdapter {
    pub fn open(dll_handle: Arc<WintunDll>, pool: &U16CStr, name: &U16CStr) -> io::Result<Self> {
        Ok(Self {
            handle: dll_handle.open_adapter(pool, name)?,
            dll_handle,
        })
    }

    pub fn create(
        dll_handle: Arc<WintunDll>,
        pool: &U16CStr,
        name: &U16CStr,
        requested_guid: Option<GUID>,
    ) -> io::Result<(Self, RebootRequired)> {
        let (handle, restart_required) = dll_handle.create_adapter(pool, name, requested_guid)?;
        Ok((Self { dll_handle, handle }, restart_required))
    }

    pub fn delete(self, force_close_sessions: bool) -> io::Result<RebootRequired> {
        unsafe {
            self.dll_handle
                .delete_adapter(self.handle, force_close_sessions)
        }
    }
}

impl Drop for WintunAdapter {
    fn drop(&mut self) {
        unsafe { self.dll_handle.free_adapter(self.handle) };
    }
}

impl WintunDll {
    pub fn new(resource_dir: &Path) -> io::Result<Self> {
        let wintun_dll: Vec<u16> = resource_dir
            .join("wintun.dll")
            .as_os_str()
            .encode_wide()
            .chain(iter::once(0u16))
            .collect();

        let handle = unsafe {
            LoadLibraryExW(
                wintun_dll.as_ptr(),
                ptr::null_mut(),
                LOAD_WITH_ALTERED_SEARCH_PATH,
            )
        };
        if handle == ptr::null_mut() {
            return Err(io::Error::last_os_error());
        }

        Ok(WintunDll {
            handle,
            func_open: unsafe {
                std::mem::transmute(Self::get_proc_address(
                    handle,
                    CStr::from_bytes_with_nul(b"WintunOpenAdapter\0").unwrap(),
                )?)
            },
            func_create: unsafe {
                std::mem::transmute(Self::get_proc_address(
                    handle,
                    CStr::from_bytes_with_nul(b"WintunCreateAdapter\0").unwrap(),
                )?)
            },
            func_delete: unsafe {
                std::mem::transmute(Self::get_proc_address(
                    handle,
                    CStr::from_bytes_with_nul(b"WintunDeleteAdapter\0").unwrap(),
                )?)
            },
            func_free: unsafe {
                std::mem::transmute(Self::get_proc_address(
                    handle,
                    CStr::from_bytes_with_nul(b"WintunFreeAdapter\0").unwrap(),
                )?)
            },
        })
    }

    unsafe fn get_proc_address(handle: HMODULE, name: &CStr) -> io::Result<FARPROC> {
        let handle = GetProcAddress(handle, name.as_ptr());
        if handle == ptr::null_mut() {
            return Err(io::Error::last_os_error());
        }
        Ok(handle)
    }

    pub fn open_adapter(&self, pool: &U16CStr, name: &U16CStr) -> io::Result<RawHandle> {
        let handle = unsafe { (self.func_open)(pool.as_ptr(), name.as_ptr()) };
        if handle == ptr::null_mut() {
            return Err(io::Error::last_os_error());
        }
        Ok(handle)
    }

    pub fn create_adapter(
        &self,
        pool: &U16CStr,
        name: &U16CStr,
        requested_guid: Option<GUID>,
    ) -> io::Result<(RawHandle, RebootRequired)> {
        let guid_ptr = match requested_guid.as_ref() {
            Some(guid) => guid as *const _,
            None => ptr::null_mut(),
        };
        let mut reboot_required = 0;
        let handle = unsafe {
            (self.func_create)(pool.as_ptr(), name.as_ptr(), guid_ptr, &mut reboot_required)
        };
        if handle == ptr::null_mut() {
            return Err(io::Error::last_os_error());
        }
        Ok((handle, reboot_required != 0))
    }

    pub unsafe fn delete_adapter(
        &self,
        adapter: RawHandle,
        force_close_sessions: bool,
    ) -> io::Result<RebootRequired> {
        let mut reboot_required = 0;
        let force_close_sessions = if force_close_sessions { 1 } else { 0 };
        let result = (self.func_delete)(adapter, force_close_sessions, &mut reboot_required);
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(reboot_required != 0)
    }

    pub unsafe fn free_adapter(&self, adapter: RawHandle) {
        (self.func_free)(adapter);
    }
}

impl Drop for WintunDll {
    fn drop(&mut self) {
        unsafe { FreeLibrary(self.handle) };
    }
}
