//! Small Win32 helpers: confirmation dialogs, single instance, UI language, process checks,
//! Authenticode verification, the Downloads folder, and autostart.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND};
use windows::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};
use windows::Win32::Security::WinTrust::{
    WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
    WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
    WinVerifyTrust,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Shell::{FOLDERID_Downloads, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
use windows::Win32::UI::WindowsAndMessaging::{
    IDYES, MB_ICONWARNING, MB_SETFOREGROUND, MB_TASKMODAL, MB_YESNO, MessageBoxW,
};
use windows::core::{PCWSTR, w};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn wide_path(p: &Path) -> Vec<u16> {
    p.as_os_str().encode_wide().chain(Some(0)).collect()
}

pub fn confirm(title: &str, text: &str) -> bool {
    let (title, text) = (wide(title), wide(text));
    let answer = unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_YESNO | MB_ICONWARNING | MB_TASKMODAL | MB_SETFOREGROUND,
        )
    };
    answer == IDYES
}

/// Holds a named mutex for the life of the process; `None` if another instance runs.
pub struct SingleInstance(HANDLE);

impl SingleInstance {
    pub fn acquire() -> Option<Self> {
        let handle =
            unsafe { CreateMutexW(None, true, w!("Local\\bt-w5-control-single-instance")) }.ok()?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                let _ = CloseHandle(handle);
            }
            return None;
        }
        Some(Self(handle))
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Two-letter code of the user's Windows display language, e.g. `nl` or `en`.
pub fn ui_language() -> String {
    let mut count = 0u32;
    let mut len = 0u32;
    unsafe {
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, None, &mut len).is_err() {
            return "en".into();
        }
        let mut buf = vec![0u16; len as usize];
        if GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            Some(windows::core::PWSTR(buf.as_mut_ptr())),
            &mut len,
        )
        .is_err()
        {
            return "en".into();
        }
        let first: String = String::from_utf16_lossy(&buf)
            .split('\0')
            .next()
            .unwrap_or("en")
            .to_string();
        first.split('-').next().unwrap_or("en").to_ascii_lowercase()
    }
}

pub fn process_running(exe_name: &str) -> bool {
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return false;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut found = false;
        let mut ok = Process32FirstW(snapshot, &mut entry).is_ok();
        while ok {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(0);
            if String::from_utf16_lossy(&entry.szExeFile[..len]).eq_ignore_ascii_case(exe_name) {
                found = true;
                break;
            }
            ok = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
        found
    }
}

/// Checks the file's Authenticode signature with the system trust policy.
pub fn verify_signature(path: &Path) -> Result<(), String> {
    let file = wide_path(path);
    let mut file_info = WINTRUST_FILE_INFO {
        cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: PCWSTR(file.as_ptr()),
        ..Default::default()
    };
    let mut data = WINTRUST_DATA {
        cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 {
            pFile: &mut file_info,
        },
        dwStateAction: WTD_STATEACTION_VERIFY,
        ..Default::default()
    };
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    let status = unsafe {
        WinVerifyTrust(
            HWND(std::ptr::null_mut()),
            &mut action,
            (&mut data as *mut WINTRUST_DATA).cast::<c_void>(),
        )
    };
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    unsafe {
        WinVerifyTrust(
            HWND(std::ptr::null_mut()),
            &mut action,
            (&mut data as *mut WINTRUST_DATA).cast::<c_void>(),
        );
    }
    if status == 0 {
        Ok(())
    } else {
        Err(format!("signature check failed (0x{:08X})", status as u32))
    }
}

/// The user's Downloads folder (follows a relocated folder).
pub fn downloads_dir() -> Result<PathBuf, String> {
    unsafe {
        let raw = SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT, None)
            .map_err(|e| e.to_string())?;
        let path = raw.to_string().map_err(|e| e.to_string());
        CoTaskMemFree(Some(raw.0 as *const c_void));
        path.map(PathBuf::from)
    }
}

/// Opens Explorer with `path` selected. Does not run the file.
pub fn reveal_in_explorer(path: &Path) {
    let mut arg = std::ffi::OsString::from("/select,");
    arg.push(path.as_os_str());
    let _ = std::process::Command::new("explorer.exe").arg(arg).spawn();
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "BT-W5 Control";
/// Name used by versions before the rename; removed so the app never starts twice at logon.
const LEGACY_RUN_VALUE: &str = "W5 Control";

pub fn remove_autostart() -> Result<(), String> {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey_with_flags(RUN_KEY, winreg::enums::KEY_SET_VALUE)
        .map_err(|e| e.to_string())?;
    delete_run_value(&key, LEGACY_RUN_VALUE)?;
    delete_run_value(&key, RUN_VALUE)
}

fn delete_run_value(key: &winreg::RegKey, name: &str) -> Result<(), String> {
    match key.delete_value(name) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

/// Registers `<exe> --startup` to run at logon (always on; not a user option).
pub fn ensure_autostart() -> Result<(), String> {
    let exe = stable_exe_path(std::env::current_exe().map_err(|e| e.to_string())?);
    let command = format!("\"{}\" --startup", exe.display());
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(RUN_KEY).map_err(|e| e.to_string())?;
    delete_run_value(&key, LEGACY_RUN_VALUE)?;
    let current: Option<String> = key.get_value(RUN_VALUE).ok();
    if current.as_deref() != Some(command.as_str()) {
        key.set_value(RUN_VALUE, &command)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Under Scoop the exe runs from `apps\<app>\<version>\`, which disappears after an update and
/// `scoop cleanup`. Scoop's `apps\<app>\current\` junction always points at the installed
/// version, so the logon entry uses that path when it exists.
fn stable_exe_path(exe: PathBuf) -> PathBuf {
    let (Some(name), Some(version_dir)) = (exe.file_name(), exe.parent()) else {
        return exe;
    };
    let Some(app_dir) = version_dir.parent() else {
        return exe;
    };
    if version_dir.file_name().is_some_and(|v| v == "current") {
        return exe;
    }
    let current = app_dir.join("current").join(name);
    let is_scoop = app_dir
        .parent()
        .and_then(|apps| apps.file_name())
        .is_some_and(|a| a.eq_ignore_ascii_case("apps"));
    if is_scoop && current.exists() {
        current
    } else {
        exe
    }
}
