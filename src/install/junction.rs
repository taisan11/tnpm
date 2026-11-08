//! Privilege-free Windows directory junctions (the equivalent of mklink /J).
//! Windows symlink_dir requires Developer Mode or an elevated token; junctions
//! use the mount-point reparse tag and work for ordinary users on NTFS.
use std::{
    fs, io,
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
    ptr,
};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn DeviceIoControl(
        file: *mut std::ffi::c_void,
        code: u32,
        input: *mut std::ffi::c_void,
        input_length: u32,
        output: *mut std::ffi::c_void,
        output_length: u32,
        returned: *mut u32,
        overlapped: *mut std::ffi::c_void,
    ) -> i32;
}

pub(super) fn create(target: &Path, destination: &Path) -> io::Result<()> {
    // Junctions only support absolute local drive paths.
    let target = fs::canonicalize(target)?;
    let path = target.to_string_lossy();
    let local = path.strip_prefix(r"\\?\").unwrap_or(&path);
    if local.starts_with(r"UNC\") || !local.as_bytes().get(1).is_some_and(|c| *c == b':') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Windows directory junctions require a local drive target",
        ));
    }
    let substitute: Vec<u16> = format!(r"\??\{local}").encode_utf16().collect();
    let printable: Vec<u16> = local.encode_utf16().collect();
    let substitute_len = substitute.len() * 2;
    let print_len = printable.len() * 2;
    let data_len = 8 + substitute_len + 2 + print_len + 2;
    if data_len > 0x3ff0 || data_len > u16::MAX as usize {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "junction target is too long"));
    }
    let mut data = Vec::with_capacity(data_len + 8);
    data.extend_from_slice(&0xA000_0003u32.to_le_bytes()); // IO_REPARSE_TAG_MOUNT_POINT
    data.extend_from_slice(&(data_len as u16).to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes()); // reserved
    data.extend_from_slice(&0u16.to_le_bytes()); // SubstituteNameOffset
    data.extend_from_slice(&(substitute_len as u16).to_le_bytes());
    data.extend_from_slice(&((substitute_len + 2) as u16).to_le_bytes()); // PrintNameOffset
    data.extend_from_slice(&(print_len as u16).to_le_bytes());
    for ch in substitute {
        data.extend_from_slice(&ch.to_le_bytes());
    }
    data.extend_from_slice(&0u16.to_le_bytes());
    for ch in printable {
        data.extend_from_slice(&ch.to_le_bytes());
    }
    data.extend_from_slice(&0u16.to_le_bytes());

    fs::create_dir(destination)?;
    let result = (|| -> io::Result<()> {
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00A4;
        let handle = fs::OpenOptions::new()
            .write(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(destination)?;
        let mut returned = 0u32;
        let success = unsafe {
            DeviceIoControl(
                handle.as_raw_handle(),
                FSCTL_SET_REPARSE_POINT,
                data.as_mut_ptr().cast(),
                data.len() as u32,
                ptr::null_mut(),
                0,
                &mut returned,
                ptr::null_mut(),
            )
        };
        if success == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir(destination);
    }
    result
}
