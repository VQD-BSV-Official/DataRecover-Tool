//! Ký tự ổ (C:, E:, F:, H:...) -> đĩa vật lý + offset + độ dài.
//! Không dùng crate ngoài: gọi thẳng kernel32. Tra thông tin không cần quyền admin (access = 0).

pub struct Extent {
    pub disk: u32,
    pub start: u64,
    #[allow(dead_code)]
    pub len: u64,
}

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;
    pub const FILE_SHARE_READ: u32 = 1;
    pub const FILE_SHARE_WRITE: u32 = 2;
    pub const OPEN_EXISTING: u32 = 3;
    pub const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: u32 = 0x0056_0000;

    #[link(name = "kernel32")]
    extern "system" {
        pub fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *mut c_void,
            disposition: u32,
            flags: u32,
            template: *mut c_void,
        ) -> *mut c_void;
        pub fn DeviceIoControl(
            h: *mut c_void,
            code: u32,
            inbuf: *const c_void,
            insize: u32,
            outbuf: *mut c_void,
            outsize: u32,
            returned: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        pub fn CloseHandle(h: *mut c_void) -> i32;
        pub fn GetLogicalDrives() -> u32;
    }
}

#[cfg(windows)]
pub fn resolve(letter: char) -> Option<Extent> {
    use std::{ffi::c_void, ptr::{null, null_mut}};

    let path: Vec<u16> = format!(r"\\.\{}:", letter.to_ascii_uppercase())
        .encode_utf16()
        .chain(Some(0))
        .collect();

    unsafe {
        let h = sys::CreateFileW(
            path.as_ptr(),
            0,
            sys::FILE_SHARE_READ | sys::FILE_SHARE_WRITE,
            null_mut(),
            sys::OPEN_EXISTING,
            0,
            null_mut(),
        );
        if h as isize == -1 {
            return None;
        }
        // Bố cục VOLUME_DISK_EXTENTS (x64, little-endian), mỗi ô u64 = 8 byte:
        //   [0] = NumberOfDiskExtents (u32, đệm 4 byte)
        //   [1] = DiskNumber          (u32, đệm 4 byte)
        //   [2] = StartingOffset      (i64)
        //   [3] = ExtentLength        (i64)
        let mut buf = [0u64; 128];
        let mut ret = 0u32;
        let ok = sys::DeviceIoControl(
            h,
            sys::IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
            null(),
            0,
            buf.as_mut_ptr() as *mut c_void,
            1024,
            &mut ret,
            null_mut(),
        );
        sys::CloseHandle(h);
        if ok == 0 {
            return None;
        }
        if (buf[0] as u32) != 1 {
            return None; // volume span nhiều đĩa: chưa hỗ trợ
        }
        Some(Extent { disk: buf[1] as u32, start: buf[2], len: buf[3] })
    }
}

#[cfg(windows)]
pub fn all() -> Vec<(char, Extent)> {
    let mask = unsafe { sys::GetLogicalDrives() };
    (0..26u8)
        .filter(|i| (mask >> i) & 1 == 1)
        .filter_map(|i| {
            let c = (b'A' + i) as char;
            resolve(c).map(|e| (c, e))
        })
        .collect()
}

#[cfg(not(windows))]
pub fn resolve(_: char) -> Option<Extent> {
    None
}

#[cfg(not(windows))]
pub fn all() -> Vec<(char, Extent)> {
    Vec::new()
}

/// Đường dẫn đĩa vật lý theo số đĩa.
pub fn physical(disk: u32) -> String {
    format!(r"\\.\PhysicalDrive{disk}")
}

/// Lấy số đĩa từ đường dẫn dạng \\.\PhysicalDriveN.
pub fn disk_no(path: &str) -> Option<u32> {
    path.trim_start_matches(|c: char| !c.is_ascii_digit()).parse().ok()
}
