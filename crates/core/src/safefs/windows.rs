//! Реалізація [`super`] на Windows: відкриття з `FILE_FLAG_OPEN_REPARSE_POINT`, перевірка
//! `GetFinalPathNameByHandleW`, видалення через `SetFileInformationByHandle`, перейменування —
//! через `NtSetInformationFile` з ім'ям відносно дескриптора теки призначення (Win32-обгортка
//! перетворює ім'я на повний шлях від поточної теки й відкидає `RootDirectory`).
#![allow(unsafe_code)]

use super::{same_path, SafeFsError};
use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use windows::Wdk::Storage::FileSystem::{
    FileRenameInformation, NtSetInformationFile, RtlNtStatusToDosErrorNoTeb,
};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    FileDispositionInfo, GetFinalPathNameByHandleW, SetFileInformationByHandle,
    FILE_DISPOSITION_INFO, FILE_NAME_NORMALIZED, FILE_RENAME_INFO, GETFINALPATHNAMEBYHANDLE_FLAGS,
    VOLUME_NAME_DOS,
};
use windows::Win32::System::IO::IO_STATUS_BLOCK;

const DELETE: u32 = 0x0001_0000;
const SYNCHRONIZE: u32 = 0x0010_0000;
const READ_CONTROL: u32 = 0x0002_0000;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_READ_ATTRIBUTES: u32 = 0x0080;
const FILE_TRAVERSE: u32 = 0x0020;
const FILE_ADD_FILE: u32 = 0x0002;
const FILE_SHARE_READ: u32 = 0x1;
const FILE_SHARE_WRITE: u32 = 0x2;
const FILE_SHARE_DELETE: u32 = 0x4;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

fn handle(f: &File) -> HANDLE {
    HANDLE(f.as_raw_handle())
}

fn os_err(e: &windows::core::Error) -> SafeFsError {
    // HRESULT_FROM_WIN32: код Win32 — у молодших 16 бітах.
    SafeFsError::Io(io::Error::from_raw_os_error(e.code().0 & 0xFFFF))
}

/// Справжній шлях відкритого об'єкта (`\\?\C:\…`).
fn final_path(f: &File) -> Result<String, SafeFsError> {
    let mut buf = vec![0u16; 512];
    loop {
        // SAFETY: дескриптор живий (позичено `f`), буфер — локальний зріз відомої довжини.
        let n = unsafe {
            GetFinalPathNameByHandleW(
                handle(f),
                &mut buf,
                GETFINALPATHNAMEBYHANDLE_FLAGS(FILE_NAME_NORMALIZED.0 | VOLUME_NAME_DOS.0),
            )
        } as usize;
        if n == 0 {
            return Err(io::Error::last_os_error().into());
        }
        if n < buf.len() {
            return Ok(String::from_utf16_lossy(&buf[..n]));
        }
        buf.resize(n + 1, 0);
    }
}

/// Відкриває об'єкт, не розкриваючи посилання в останньому компоненті, і перевіряє, що це
/// не посилання, тип збігається, а справжній шлях — саме `path`.
fn open_checked(path: &Path, dir: bool, access: u32, share: u32) -> Result<File, SafeFsError> {
    let flags = FILE_FLAG_OPEN_REPARSE_POINT | if dir { FILE_FLAG_BACKUP_SEMANTICS } else { 0 };
    let f = OpenOptions::new()
        .access_mode(access)
        .share_mode(share)
        .custom_flags(flags)
        .open(path)?;
    let m = f.metadata()?;
    let shown = path.display().to_string();
    if m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(SafeFsError::Reparse(shown));
    }
    if m.is_dir() != dir {
        return Err(SafeFsError::WrongKind(shown));
    }
    let actual = final_path(&f)?;
    if !same_path(&actual, &shown) {
        return Err(SafeFsError::Redirected {
            expected: shown,
            actual,
        });
    }
    Ok(f)
}

/// Позначає відкритий об'єкт на видалення (видаляється при закритті дескриптора).
fn mark_deleted(f: &File) -> Result<(), SafeFsError> {
    let info = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: дескриптор відкрито з DELETE; структура локальна, розмір збігається з типом.
    unsafe {
        SetFileInformationByHandle(
            handle(f),
            FileDispositionInfo,
            std::ptr::from_ref(&info).cast::<c_void>(),
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    }
    .map_err(|e| os_err(&e))
}

pub(super) fn delete(path: &Path, dir: bool) -> Result<(), SafeFsError> {
    let f = open_checked(
        path,
        dir,
        DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        // Як у DeleteFileW: чужі дескриптори без FILE_SHARE_DELETE дадуть «зайнято».
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    )?;
    mark_deleted(&f)
}

/// Відкриває теку призначення (її не можна перейменувати, доки дескриптор відкритий).
fn open_target_dir(to: &Path) -> Result<(File, &std::ffi::OsStr), SafeFsError> {
    let shown = to.display().to_string();
    let (Some(parent), Some(name)) = (to.parent(), to.file_name()) else {
        return Err(SafeFsError::WrongKind(shown));
    };
    let dir = open_checked(
        parent,
        true,
        FILE_TRAVERSE | FILE_ADD_FILE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
    )?;
    Ok((dir, name))
}

pub(super) fn move_file(from: &Path, to: &Path) -> Result<(), SafeFsError> {
    let src = open_checked(
        from,
        false,
        DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    )?;
    let (dir, name) = open_target_dir(to)?;
    let name: Vec<u16> = name.encode_wide().collect();

    // FILE_RENAME_INFO зі змінною довжиною імені: ім'я відносно RootDirectory (перевіреної теки),
    // тож посилання в шляху призначення не можуть увести перейменування в інше місце.
    let offset = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
    let size = offset + (name.len() + 1) * std::mem::size_of::<u16>();
    let mut buf = vec![0u64; size.div_ceil(8)];
    let info = buf.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    // SAFETY: буфер вирівняний на 8 і має місце для заголовка, імені й нуль-термінатора;
    // поля пишуться через сирі вказівники в межах цього буфера.
    unsafe {
        (*info).Anonymous.ReplaceIfExists = false;
        (*info).RootDirectory = handle(&dir);
        (*info).FileNameLength = (name.len() * std::mem::size_of::<u16>()) as u32;
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            std::ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
            name.len(),
        );
    }
    let mut iosb = IO_STATUS_BLOCK::default();
    // SAFETY: джерело відкрито з DELETE, тека — з FILE_ADD_FILE; `buf` і `iosb` живі до кінця
    // виклику. Розмітка FILE_RENAME_INFO збігається з FILE_RENAME_INFORMATION ядра.
    let status = unsafe {
        NtSetInformationFile(
            handle(&src),
            &mut iosb,
            info.cast::<c_void>(),
            size as u32,
            FileRenameInformation,
        )
    };
    if status.is_ok() {
        Ok(())
    } else {
        // SAFETY: чиста функція перетворення коду.
        let code = unsafe { RtlNtStatusToDosErrorNoTeb(status) };
        Err(SafeFsError::Io(io::Error::from_raw_os_error(code as i32)))
    }
}

pub(super) fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, SafeFsError> {
    use std::io::Read;
    let f = open_checked(
        path,
        false,
        GENERIC_READ | FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    )?;
    let mut out = Vec::new();
    f.take(limit + 1).read_to_end(&mut out)?;
    if out.len() as u64 > limit {
        return Err(SafeFsError::WrongKind(format!(
            "{}: більший за {limit} байтів",
            path.display()
        )));
    }
    Ok(out)
}

pub(super) fn open_dir_for_security(path: &Path) -> Result<File, SafeFsError> {
    open_checked(
        path,
        true,
        READ_CONTROL | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    )
}

pub(super) fn copy_file(from: &Path, to: &Path) -> Result<u64, SafeFsError> {
    let mut src = open_checked(
        from,
        false,
        GENERIC_READ | FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    )?;
    let modified = src.metadata()?.modified().ok();
    let mut dst = OpenOptions::new()
        .write(true)
        .create_new(true)
        .access_mode(GENERIC_WRITE | DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(to)?;
    let result = (|| {
        let actual = final_path(&dst)?;
        let shown = to.display().to_string();
        if !same_path(&actual, &shown) {
            return Err(SafeFsError::Redirected {
                expected: shown,
                actual,
            });
        }
        let n = io::copy(&mut src, &mut dst)?;
        if let Some(t) = modified {
            dst.set_modified(t)?;
        }
        dst.sync_all()?;
        Ok(n)
    })();
    if result.is_err() {
        // Новий файл (неповний або не там) прибираємо через той самий дескриптор.
        let _ = mark_deleted(&dst);
    }
    result
}

#[cfg(test)]
mod tests {
    //! Справжня файлова система в тимчасовій теці. Junction створюється `mklink /J`
    //! (прав адміністратора не потребує) і веде лише в іншу тимчасову теку.
    use super::super::{copy_file, delete_empty_dir, delete_file, move_file, SafeFsError};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let base = std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .trim_start_matches(r"\\?\")
                .to_string();
            let p =
                PathBuf::from(base).join(format!("optimax-safefs-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn junction(link: &Path, target: &Path) -> bool {
        Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .is_ok_and(|o| o.status.success())
    }

    #[test]
    fn delete_and_move_plain_files() {
        let t = TempDir::new("plain");
        let (a, b, d) = (t.0.join("a.tmp"), t.0.join("b.tmp"), t.0.join("sub"));
        fs::write(&a, b"hello").unwrap();
        fs::create_dir(&d).unwrap();
        move_file(&a, &d.join("a.tmp")).unwrap();
        assert!(!a.exists());
        assert_eq!(fs::read(d.join("a.tmp")).unwrap(), b"hello");
        assert_eq!(copy_file(&d.join("a.tmp"), &b).unwrap(), 5);
        assert_eq!(fs::read(&b).unwrap(), b"hello");
        // Наявний файл не перезаписується.
        assert!(copy_file(&b, &d.join("a.tmp")).is_err());
        assert!(move_file(&b, &d.join("a.tmp")).is_err());
        delete_file(&d.join("a.tmp")).unwrap();
        delete_empty_dir(&d).unwrap();
        delete_file(&b).unwrap();
        assert_eq!(fs::read_dir(&t.0).unwrap().count(), 0);
    }

    #[test]
    fn junction_in_the_middle_is_refused() {
        let t = TempDir::new("junction");
        let (victim, cache) = (t.0.join("victim"), t.0.join("cache"));
        fs::create_dir_all(&victim).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::write(victim.join("important.dll"), b"x").unwrap();
        let evil = cache.join("evil");
        if !junction(&evil, &victim) {
            eprintln!("mklink /J недоступний — тест пропущено");
            return;
        }
        let through = evil.join("important.dll");
        assert!(matches!(
            delete_file(&through),
            Err(SafeFsError::Redirected { .. })
        ));
        assert!(matches!(
            move_file(&through, &cache.join("moved.dll")),
            Err(SafeFsError::Redirected { .. })
        ));
        assert!(matches!(
            copy_file(&through, &cache.join("copied.dll")),
            Err(SafeFsError::Redirected { .. })
        ));
        // Переміщення В теку через junction теж не відбувається, і файл лишається на місці.
        // Тека призначення сама є junction — відмова на відкритті теки.
        fs::write(cache.join("planted.dll"), b"y").unwrap();
        assert!(matches!(
            move_file(&cache.join("planted.dll"), &evil.join("planted.dll")),
            Err(SafeFsError::Reparse(_))
        ));
        // Junction глибше в шляху призначення — відмова за справжнім шляхом теки.
        fs::create_dir(victim.join("sub")).unwrap();
        assert!(matches!(
            move_file(
                &cache.join("planted.dll"),
                &evil.join("sub").join("planted.dll")
            ),
            Err(SafeFsError::Redirected { .. })
        ));
        assert!(!victim.join("sub").join("planted.dll").exists());
        assert!(matches!(
            copy_file(&cache.join("planted.dll"), &evil.join("copied.dll")),
            Err(SafeFsError::Redirected { .. })
        ));
        assert!(victim.join("important.dll").exists());
        assert!(!victim.join("planted.dll").exists());
        assert!(!victim.join("copied.dll").exists());
        // Саме посилання — reparse point: не видаляється як файл і як каталог.
        assert!(delete_empty_dir(&evil).is_err());
        assert!(victim.exists());
        fs::remove_dir(&evil).unwrap();
    }

    #[test]
    fn busy_file_is_not_deleted() {
        use std::os::windows::fs::OpenOptionsExt;
        let t = TempDir::new("busy");
        let p = t.0.join("locked.tmp");
        fs::write(&p, b"x").unwrap();
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0x1)
            .open(&p)
            .unwrap();
        let err = delete_file(&p).unwrap_err();
        assert!(
            matches!(&err, SafeFsError::Io(e) if e.raw_os_error() == Some(32)),
            "{err}"
        );
        drop(lock);
        delete_file(&p).unwrap();
    }
}
