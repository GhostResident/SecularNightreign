use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    io,
    mem::size_of,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
    ptr::{self, null, null_mut},
    sync::LazyLock,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use regex::Regex;
use windows::{
    Win32::{
        Foundation::HWND,
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
        },
        UI::Shell::{
            FOLDERID_Downloads, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog,
            IFileOpenDialog, KNOWN_FOLDER_FLAG, SHGetKnownFolderPath, SIGDN_FILESYSPATH,
        },
    },
    core::{HRESULT, w},
};
use windows_sys::Win32::{
    Foundation::{HWND as RawWindow, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW,
    },
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        Threading::{GetExitCodeProcess, GetProcessId, WaitForSingleObject},
    },
    UI::{
        Shell::{
            SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
            ShellExecuteExW,
        },
        WindowsAndMessaging::{
            EnumChildWindows, EnumWindows, GetWindowTextW, GetWindowThreadProcessId,
            IsWindowVisible, PostMessageW, SMTO_ABORTIFHUNG, SW_SHOWNORMAL, SendMessageTimeoutW,
            WM_CLOSE, WM_GETTEXT,
        },
    },
};

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain([0]).collect()
}

struct Apartment;

impl Apartment {
    fn enter() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).ok()?;
        }
        Ok(Self)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

pub fn browse_folder(owner: usize) -> Result<Option<String>> {
    let _apartment = Apartment::enter()?;
    // The dialog and its COM interfaces live on this STA thread.
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        dialog.SetOptions(dialog.GetOptions()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)?;
        dialog.SetTitle(w!("Выберите папку"))?;
        match dialog.Show(Some(HWND(owner as _))) {
            Err(error) if error.code() == HRESULT(0x800704c7_u32 as i32) => return Ok(None),
            other => other?,
        }
        let item = dialog.GetResult()?;
        let pointer = item.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = pointer.to_string();
        CoTaskMemFree(Some(pointer.0.cast()));
        Ok(Some(path?))
    }
}

pub fn downloads() -> Result<PathBuf> {
    unsafe {
        let raw = SHGetKnownFolderPath(&FOLDERID_Downloads, KNOWN_FOLDER_FLAG(0), None)?;
        let path = OsString::from_wide(raw.as_wide());
        CoTaskMemFree(Some(raw.0.cast()));
        Ok(path.into())
    }
}

pub fn open_folder(path: &Path) -> Result<()> {
    let _apartment = Apartment::enter()?;
    shell_execute(path, None, None, false).map(|_| ())
}

pub fn file_version(path: &Path) -> Option<String> {
    let path = wide(path);
    let mut reserved = 0;
    let size = unsafe { GetFileVersionInfoSizeW(path.as_ptr(), &mut reserved) };
    if size == 0 || size > 1024 * 1024 {
        return None;
    }
    let mut data = vec![0_u8; size as usize];
    unsafe {
        if GetFileVersionInfoW(path.as_ptr(), 0, size, data.as_mut_ptr().cast()) == 0 {
            return None;
        }
    }
    let query = |key: &str| {
        let key = wide(key);
        let mut value = null_mut();
        let mut len = 0;
        let ok =
            unsafe { VerQueryValueW(data.as_ptr().cast(), key.as_ptr(), &mut value, &mut len) };
        (ok != 0 && !value.is_null()).then_some((value, len as usize))
    };
    let checked_bytes = |pointer: *mut std::ffi::c_void, bytes: usize| {
        let start = pointer.addr().checked_sub(data.as_ptr().addr())?;
        data.get(start..start.checked_add(bytes)?)
    };
    if let Some((value, len)) = query(r"\VarFileInfo\Translation")
        && let Some(languages) = checked_bytes(value, len)
    {
        for language in languages.chunks_exact(4) {
            let id = u16::from_le_bytes([language[0], language[1]]);
            let page = u16::from_le_bytes([language[2], language[3]]);
            let key = format!(r"\StringFileInfo\{id:04x}{page:04x}\FileVersion");
            if let Some((text, len)) = query(&key)
                && let Some(bytes) = len.checked_mul(2).and_then(|len| checked_bytes(text, len))
            {
                let text: Vec<_> = bytes
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                let version = String::from_utf16_lossy(&text)
                    .trim_matches('\0')
                    .trim()
                    .to_owned();
                if !version.is_empty() {
                    return Some(version);
                }
            }
        }
    }
    let (value, len) = query("\\")?;
    if len < size_of::<VS_FIXEDFILEINFO>() {
        return None;
    }
    let info = checked_bytes(value, size_of::<VS_FIXEDFILEINFO>())?;
    let fixed = unsafe { ptr::read_unaligned(info.as_ptr().cast::<VS_FIXEDFILEINFO>()) };
    (fixed.dwSignature == 0xfeef04bd).then(|| {
        format!(
            "{}.{}.{}.{}",
            fixed.dwFileVersionMS >> 16,
            fixed.dwFileVersionMS & 0xffff,
            fixed.dwFileVersionLS >> 16,
            fixed.dwFileVersionLS & 0xffff
        )
    })
}

fn shell_execute(
    path: &Path,
    arguments: Option<&OsStr>,
    cwd: Option<&Path>,
    process: bool,
) -> Result<Option<OwnedHandle>> {
    let path = wide(path);
    let arguments = arguments.map(wide);
    let cwd = cwd.map(wide);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC
            | SEE_MASK_FLAG_NO_UI
            | if process { SEE_MASK_NOCLOSEPROCESS } else { 0 },
        lpFile: path.as_ptr(),
        lpParameters: arguments.as_ref().map_or(null(), |arg| arg.as_ptr()),
        lpDirectory: cwd.as_ref().map_or(null(), |dir| dir.as_ptr()),
        nShow: SW_SHOWNORMAL,
        ..Default::default()
    };
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        return Err(io::Error::last_os_error()).context("Windows не смог открыть файл");
    }
    Ok((!info.hProcess.is_null()).then(|| unsafe { OwnedHandle::from_raw_handle(info.hProcess) }))
}

pub fn install(exe: &Path, root: &Path, tick: impl Fn(u64, Option<&str>)) -> Result<()> {
    let _apartment = Apartment::enter()?;
    let name = exe
        .file_name()
        .context("Нет имени установщика")?
        .to_string_lossy()
        .to_lowercase();
    let args = if name.contains("setup") && !name.contains("nightreign") {
        // NSIS reads /D= literally from the end of the Windows command line.
        let mut args = OsString::from("/S /D=");
        args.push(root);
        args
    } else {
        let mut args = OsString::from("/VERYSILENT /SUPPRESSMSGBOXES /NOCANCEL /NORESTART /DIR=\"");
        args.push(root);
        args.push("\"");
        args
    };
    let process = shell_execute(exe, Some(&args), exe.parent(), true)?
        .context("Windows не вернул процесс установщика")?;
    let handle = process.as_raw_handle();
    let pid = unsafe { GetProcessId(handle) };
    let start = Instant::now();
    let mut known_children = HashSet::from([pid]);
    let mut elapsed_notice = false;
    loop {
        match unsafe { WaitForSingleObject(handle, 3000) } {
            WAIT_OBJECT_0 => break,
            WAIT_TIMEOUT => {}
            _ => {
                return Err(io::Error::last_os_error()).context("Не удалось дождаться установщика");
            }
        }
        let elapsed = start.elapsed();
        let closed_crc = close_finished_crc(pid, &mut known_children);
        let mut message = closed_crc.then_some("RapidCRC закончил проверку и закрыт");
        if !elapsed_notice && elapsed >= Duration::from_secs(1200) {
            elapsed_notice = true;
            message = Some("Установщик работает дольше 20 минут. Ожидаем его завершения.");
        }
        tick(elapsed.as_secs(), message);
    }
    let mut code = 0;
    if unsafe { GetExitCodeProcess(handle, &mut code) } == 0 {
        return Err(io::Error::last_os_error()).context("Не удалось узнать код установщика");
    }
    if code != 0 {
        bail!("Установщик завершился с кодом {code}");
    }
    Ok(())
}

fn visit_processes(mut visit: impl FnMut(&PROCESSENTRY32W) -> bool) -> io::Result<()> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
    let raw = snapshot.as_raw_handle();
    let mut process = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    if unsafe { Process32FirstW(raw, &mut process) } == 0 {
        return Err(io::Error::last_os_error());
    }
    while visit(&process) && unsafe { Process32NextW(raw, &mut process) } != 0 {}
    Ok(())
}

pub fn nightreign_running() -> Result<bool> {
    let mut running = false;
    visit_processes(|process| {
        let name = process
            .szExeFile
            .split(|&c| c == 0)
            .next()
            .unwrap_or_default();
        running = OsString::from_wide(name).eq_ignore_ascii_case("nightreign.exe");
        !running
    })
    .context("Не удалось проверить, закрыта ли игра")?;
    Ok(running)
}

fn close_finished_crc(pid: u32, known: &mut HashSet<u32>) -> bool {
    if pid == 0 {
        return false;
    }
    let mut processes = Vec::new();
    if visit_processes(|entry| {
        processes.push((entry.th32ProcessID, entry.th32ParentProcessID));
        true
    })
    .is_err()
    {
        return false;
    }
    loop {
        let before = known.len();
        for &(child, parent) in &processes {
            if known.contains(&parent) {
                known.insert(child);
            }
        }
        if known.len() == before {
            break;
        }
    }

    struct Scan<'a> {
        children: &'a HashSet<u32>,
        closed: bool,
    }
    unsafe extern "system" fn scan_window(hwnd: RawWindow, data: isize) -> i32 {
        let scan = unsafe { &mut *(data as *mut Scan<'_>) };
        let mut pid = 0;
        let mut title = [0_u16; 256];
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if !scan.children.contains(&pid) || unsafe { IsWindowVisible(hwnd) } == 0 {
            return 1;
        }
        let count = unsafe { GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32) };
        if count > 0
            && String::from_utf16_lossy(&title[..count as usize])
                .to_lowercase()
                .contains("rapidcrc")
        {
            let mut done = false;
            unsafe { EnumChildWindows(hwnd, Some(scan_control), &mut done as *mut bool as isize) };
            if done && unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) } != 0 {
                scan.closed = true;
            }
        }
        1
    }
    unsafe extern "system" fn scan_control(hwnd: RawWindow, data: isize) -> i32 {
        static DONE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"(?i)\b([1-9]\d*)\s*/\s*(\d+)\s*done\b").unwrap());
        let mut text = [0_u16; 512];
        let mut count = 0;
        let ok = unsafe {
            SendMessageTimeoutW(
                hwnd,
                WM_GETTEXT,
                text.len(),
                text.as_mut_ptr() as isize,
                SMTO_ABORTIFHUNG,
                100,
                &mut count,
            )
        };
        if ok != 0 && count > 0 && count < text.len() {
            let text = String::from_utf16_lossy(&text[..count]);
            if DONE.captures_iter(&text).any(|hit| hit[1] == hit[2]) {
                unsafe { *(data as *mut bool) = true };
                return 0;
            }
        }
        1
    }
    let mut scan = Scan {
        children: known,
        closed: false,
    };
    unsafe { EnumWindows(Some(scan_window), &mut scan as *mut Scan<'_> as isize) };
    scan.closed
}
