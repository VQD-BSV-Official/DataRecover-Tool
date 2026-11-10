pub mod info;
pub mod reader;
pub mod letters;

pub use reader::Device;

/// DeviceIoControl tối giản (std đã link kernel32).
#[cfg(windows)]
pub(crate) fn ioctl(f: &std::fs::File, code: u32, input: &[u8], out: &mut [u8]) -> std::io::Result<usize> {
    use std::{ffi::c_void, os::windows::io::AsRawHandle};
    #[link(name = "kernel32")]
    extern "system" {
        fn DeviceIoControl(h: *mut c_void, code: u32, i: *const c_void, ni: u32, o: *mut c_void, no: u32, ret: *mut u32, ov: *mut c_void) -> i32;
    }
    let mut n = 0u32;
    let ok = unsafe {
        DeviceIoControl(f.as_raw_handle() as _, code, input.as_ptr() as _, input.len() as u32, out.as_mut_ptr() as _, out.len() as u32, &mut n, std::ptr::null_mut())
    };
    if ok == 0 { Err(std::io::Error::last_os_error()) } else { Ok(n as usize) }
}
