//! Реалізація [`super`] на Windows.
#![allow(unsafe_code)]

use super::Folder;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use windows::core::GUID;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::SystemInformation::{GetSystemDirectoryW, GetSystemWindowsDirectoryW};
use windows::Win32::UI::Shell::{
    FOLDERID_CommonStartup, FOLDERID_LocalAppData, FOLDERID_Profile, FOLDERID_ProgramData,
    FOLDERID_ProgramFiles, FOLDERID_ProgramFilesX64, FOLDERID_ProgramFilesX86, FOLDERID_Public,
    FOLDERID_RoamingAppData, FOLDERID_SkyDrive, FOLDERID_Startup, SHGetKnownFolderPath,
    KNOWN_FOLDER_FLAG,
};

/// Викликає API виду «записати шлях у буфер, повернути довжину (або потрібний розмір)».
fn from_buffer(call: impl Fn(&mut [u16]) -> u32) -> Option<PathBuf> {
    let mut buf = vec![0u16; 260];
    loop {
        let n = call(&mut buf) as usize;
        if n == 0 {
            return None;
        }
        if n < buf.len() {
            return Some(PathBuf::from(OsString::from_wide(&buf[..n])));
        }
        buf.resize(n + 1, 0);
    }
}

pub(super) fn windows_dir() -> Option<PathBuf> {
    // SAFETY: буфер — локальний зріз відомої довжини.
    from_buffer(|b| unsafe { GetSystemWindowsDirectoryW(Some(b)) })
}

pub(super) fn system32() -> Option<PathBuf> {
    // SAFETY: буфер — локальний зріз відомої довжини.
    from_buffer(|b| unsafe { GetSystemDirectoryW(Some(b)) })
}

fn guid(folder: Folder) -> GUID {
    match folder {
        Folder::ProgramData => FOLDERID_ProgramData,
        Folder::ProgramFiles => FOLDERID_ProgramFiles,
        Folder::ProgramFilesX64 => FOLDERID_ProgramFilesX64,
        Folder::ProgramFilesX86 => FOLDERID_ProgramFilesX86,
        Folder::Profile => FOLDERID_Profile,
        Folder::Public => FOLDERID_Public,
        Folder::LocalAppData => FOLDERID_LocalAppData,
        Folder::RoamingAppData => FOLDERID_RoamingAppData,
        Folder::OneDrive => FOLDERID_SkyDrive,
        Folder::Startup => FOLDERID_Startup,
        Folder::CommonStartup => FOLDERID_CommonStartup,
    }
}

pub(super) fn known(folder: Folder) -> Option<PathBuf> {
    let id = guid(folder);
    // SAFETY: `id` — валідний GUID; рядок звільняється CoTaskMemFree нижче.
    let p = unsafe { SHGetKnownFolderPath(&id, KNOWN_FOLDER_FLAG(0), None) }.ok()?;
    // SAFETY: система повернула нуль-термінований рядок.
    let s = unsafe { p.to_string() }.ok();
    // SAFETY: рядок виділено SHGetKnownFolderPath через CoTaskMemAlloc.
    unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
    s.filter(|s| !s.is_empty()).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn system_folders_resolve() {
        let w = windows_dir().unwrap();
        assert!(w.is_absolute());
        assert!(system32().unwrap().starts_with(&w));
        assert_eq!(system_drive().unwrap().len(), 2);
        for f in [
            Folder::ProgramData,
            Folder::ProgramFiles,
            Folder::ProgramFilesX86,
            Folder::Profile,
            Folder::LocalAppData,
            Folder::CommonStartup,
        ] {
            assert!(known(f).unwrap().is_absolute(), "{f:?}");
        }
        assert!(explorer().unwrap().is_file());
    }
}
