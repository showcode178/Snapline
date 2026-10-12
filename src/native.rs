//! Windows integration. All HWND and OLE calls run on Slint's UI thread.

use std::{
    ffi::OsStr, marker::PhantomData, os::windows::ffi::OsStrExt, path::Path, rc::Rc, sync::OnceLock,
};
use windows::{
    Win32::{
        Foundation::{
            CloseHandle, E_FAIL, E_INVALIDARG, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND,
            ERROR_SUCCESS, GetLastError, HANDLE, HGLOBAL, HWND, POINT, RECT, SetLastError,
            WAIT_OBJECT_0,
        },
        Graphics::Gdi::{
            GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
            MonitorFromWindow,
        },
        Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT, SND_SYSTEM},
        System::{
            Com::IDataObject,
            DataExchange::{
                CloseClipboard, GetClipboardData, GetClipboardSequenceNumber,
                IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
            },
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
            Ole::{
                CF_BITMAP, CF_DIB, CF_DIBV5, CF_HDROP, CF_OEMTEXT, CF_TEXT, CF_UNICODETEXT,
                DROPEFFECT_COPY, DROPEFFECT_MOVE, OleFlushClipboard, OleInitialize,
                OleSetClipboard, OleUninitialize,
            },
            Registry::{
                HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
                RRF_RT_REG_DWORD, RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegGetValueW,
                RegSetValueExW,
            },
            Threading::{
                CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, OpenEventW, SetEvent,
                WaitForSingleObject,
            },
        },
        UI::{
            HiDpi::{GetDpiForMonitor, GetDpiForSystem, MDT_EFFECTIVE_DPI},
            Shell::{
                Common::ITEMIDLIST, ILClone, ILFindLastID, ILFree, ILRemoveLastID,
                SHCreateDataObject, SHDoDragDrop, SHParseDisplayName, ShellExecuteW,
            },
            WindowsAndMessaging::{
                GWL_EXSTYLE, GetClassNameW, GetForegroundWindow, GetPhysicalCursorPos,
                GetWindowLongPtrW, GetWindowRect, HWND_TOPMOST, IsIconic,
                SPI_GETCLIENTAREAANIMATION, SW_RESTORE, SW_SHOWNORMAL,
                SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetForegroundWindow, SetWindowLongPtrW,
                SetWindowPos, ShowWindow, SystemParametersInfoW, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
                WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
            },
        },
    },
    core::{Error, PCWSTR, Result, w},
};

/// Respect Windows' accessibility setting, falling back to reduced motion if unreadable.
pub fn animations_enabled() -> bool {
    let mut enabled = windows::core::BOOL(0);
    unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&mut enabled as *mut windows::core::BOOL).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS::default(),
        )
        .is_ok()
            && enabled.as_bool()
    }
}

// Static bytes must outlive asynchronous playback. This original PCM sound is
// a short pair of mechanical clicks, with no external files or audio process.
const SHUTTER_WAV: &[u8] = include_bytes!("../assets/shutter.wav");

/// Play through the Windows system-sounds volume without blocking or taking focus.
pub fn play_camera_shutter() {
    unsafe {
        let _ = PlaySoundW(
            PCWSTR(SHUTTER_WAV.as_ptr().cast()),
            None,
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT | SND_SYSTEM,
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonitorInfo {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

/// A session-local mutex prevents concurrent writers of the screenshot index.
pub struct SingleInstance {
    mutex: HANDLE,
    settings_event: HANDLE,
}

impl SingleInstance {
    pub fn acquire() -> Result<Option<Self>> {
        Self::acquire_named(
            w!("Local\\Snapline.Instance"),
            w!("Local\\Snapline.Settings"),
        )
    }

    fn acquire_named(mutex_name: PCWSTR, event_name: PCWSTR) -> Result<Option<Self>> {
        unsafe {
            // Publish the event first so a second process can notify immediately.
            let settings_event = CreateEventW(None, false, false, event_name)?;
            SetLastError(ERROR_SUCCESS);
            let mutex = match CreateMutexW(None, false, mutex_name) {
                Ok(handle) => handle,
                Err(error) => {
                    let _ = CloseHandle(settings_event);
                    return Err(error);
                }
            };
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(mutex);
                let _ = CloseHandle(settings_event);
                return Ok(None);
            }
            Ok(Some(Self {
                mutex,
                settings_event,
            }))
        }
    }

    /// A zero-time wait consumes one auto-reset request without blocking the UI.
    pub fn take_settings_request(&self) -> bool {
        unsafe { WaitForSingleObject(self.settings_event, 0) == WAIT_OBJECT_0 }
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.mutex);
            let _ = CloseHandle(self.settings_event);
        }
    }
}

pub fn notify_existing() -> Result<()> {
    unsafe {
        let event = OpenEventW(EVENT_MODIFY_STATE, false, w!("Local\\Snapline.Settings"))?;
        let result = SetEvent(event);
        let _ = CloseHandle(event);
        result
    }
}

/// Keep alive on the UI thread for the event loop; OLE apartments are thread-bound.
pub struct OleGuard(PhantomData<Rc<()>>);

impl OleGuard {
    pub fn initialize() -> Result<Self> {
        unsafe { OleInitialize(None)? };
        Ok(Self(PhantomData))
    }
}

impl Drop for OleGuard {
    fn drop(&mut self) {
        unsafe { OleUninitialize() };
    }
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

fn change_extended_style(hwnd: HWND, style: isize) -> Result<()> {
    unsafe {
        SetLastError(ERROR_SUCCESS);
        let previous = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style);
        if previous == 0 && GetLastError() != ERROR_SUCCESS {
            return Err(Error::from_win32());
        }
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )
    }
}

pub fn configure_overlay(hwnd: HWND) -> Result<()> {
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    let style = (style | WS_EX_TOOLWINDOW.0 as isize | WS_EX_NOACTIVATE.0 as isize)
        & !(WS_EX_APPWINDOW.0 as isize);
    change_extended_style(hwnd, style)
}

/// Winit's focus_window injects Alt, which can trigger the foreground app's shortcuts.
pub fn activate_window(hwnd: HWND) {
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Use alongside the backend's set_cursor_hittest, which supplies layered-window
/// support when needed. The UI only enables hit testing over visible controls.
pub fn set_click_through(hwnd: HWND, enabled: bool) -> Result<()> {
    let current = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    let style = if enabled {
        current | WS_EX_TRANSPARENT.0 as isize | WS_EX_NOACTIVATE.0 as isize
    } else {
        current & !(WS_EX_TRANSPARENT.0 as isize)
    };
    if current != style {
        change_extended_style(hwnd, style)?;
    }
    Ok(())
}

pub fn cursor_position() -> Option<(i32, i32)> {
    let mut point = POINT::default();
    unsafe { GetPhysicalCursorPos(&mut point).ok()? };
    Some((point.x, point.y))
}

fn monitor_info(handle: HMONITOR) -> Option<MonitorInfo> {
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetMonitorInfoW(handle, &mut info).ok().ok()? };
    let (mut dpi_x, mut dpi_y) = (96, 96);
    unsafe {
        if GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).is_err() {
            dpi_x = GetDpiForSystem().max(96);
        }
    }
    Some(MonitorInfo {
        x: info.rcMonitor.left,
        y: info.rcMonitor.top,
        width: (info.rcMonitor.right - info.rcMonitor.left).max(0) as u32,
        height: (info.rcMonitor.bottom - info.rcMonitor.top).max(0) as u32,
        scale: f64::from(dpi_x.max(96)) / 96.0,
    })
}

pub fn monitor_at_cursor() -> Option<MonitorInfo> {
    let (x, y) = cursor_position()?;
    let handle = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    monitor_info(handle)
}

pub fn monitor_for_window(hwnd: HWND) -> Option<MonitorInfo> {
    let handle = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    monitor_info(handle)
}

pub fn place_overlay(hwnd: HWND, monitor: MonitorInfo, height: f64) -> Result<()> {
    if !height.is_finite() || height <= 0.0 || !monitor.scale.is_finite() {
        return Err(Error::new(E_INVALIDARG, "Invalid overlay dimensions"));
    }
    unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            monitor.x,
            monitor.y,
            monitor.width.min(i32::MAX as u32) as i32,
            (height * monitor.scale)
                .round()
                .clamp(1.0, monitor.height as f64) as i32,
            SWP_NOACTIVATE,
        )
    }
}

fn covers_monitor(rect: RECT, monitor: MonitorInfo) -> bool {
    rect.left <= monitor.x + 2
        && rect.top <= monitor.y + 2
        && i64::from(rect.right) >= i64::from(monitor.x) + i64::from(monitor.width) - 2
        && i64::from(rect.bottom) >= i64::from(monitor.y) + i64::from(monitor.height) - 2
}

pub fn fullscreen_on_monitor(monitor: &MonitorInfo) -> bool {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() || IsIconic(hwnd).as_bool() {
            return false;
        }
        let mut class_name = [0u16; 128];
        let length = GetClassNameW(hwnd, &mut class_name);
        let class_name = String::from_utf16_lossy(&class_name[..length.max(0) as usize]);
        if matches!(
            class_name.as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        ) {
            return false;
        }
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return false;
        }
        covers_monitor(rect, *monitor)
    }
}

pub fn clipboard_sequence() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

fn clipboard_format_present(format: u32) -> bool {
    format != 0 && unsafe { IsClipboardFormatAvailable(format).is_ok() }
}

fn clipboard_history_allowed(format: u32) -> bool {
    if !clipboard_format_present(format) {
        return true;
    }
    unsafe {
        if OpenClipboard(None).is_err() {
            return false;
        }
        let allowed = (|| {
            let handle = HGLOBAL(GetClipboardData(format).ok()?.0);
            if GlobalSize(handle) < std::mem::size_of::<u32>() {
                return None;
            }
            let data = GlobalLock(handle);
            if data.is_null() {
                return None;
            }
            let value = data.cast::<u32>().read_unaligned();
            let _ = GlobalUnlock(handle);
            Some(value != 0)
        })()
        .unwrap_or(false);
        let _ = CloseClipboard();
        allowed
    }
}

/// Capture screenshots, rather than text/file copies or explicitly private data.
pub fn clipboard_has_image_only() -> bool {
    static FORMATS: OnceLock<[u32; 5]> = OnceLock::new();
    let formats = FORMATS.get_or_init(|| unsafe {
        [
            RegisterClipboardFormatW(w!("PNG")),
            RegisterClipboardFormatW(w!("HTML Format")),
            RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing")),
            RegisterClipboardFormatW(w!("Clipboard Viewer Ignore")),
            RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory")),
        ]
    });
    let has_image = [
        CF_BITMAP.0 as u32,
        CF_DIB.0 as u32,
        CF_DIBV5.0 as u32,
        formats[0],
    ]
    .into_iter()
    .any(clipboard_format_present);
    let excluded = [
        CF_TEXT.0 as u32,
        CF_UNICODETEXT.0 as u32,
        CF_OEMTEXT.0 as u32,
        CF_HDROP.0 as u32,
        formats[1],
        formats[2],
        formats[3],
    ]
    .into_iter()
    .any(clipboard_format_present);
    has_image && !excluded && clipboard_history_allowed(formats[4])
}

fn shell_open(target: &OsStr) -> Result<()> {
    let target = wide(target);
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(target.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        Err(Error::new(
            E_FAIL,
            "Windows could not open the requested item",
        ))
    } else {
        Ok(())
    }
}

pub fn open_capture() -> Result<()> {
    shell_open(OsStr::new("ms-screenclip:"))
}

pub fn open_file(path: &Path) -> Result<()> {
    shell_open(path.as_os_str())
}

pub fn set_autostart(enabled: bool) -> Result<()> {
    unsafe {
        let mut key = HKEY::default();
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
        .ok()?;
        let result = if enabled {
            match std::env::current_exe() {
                Ok(executable) => {
                    let mut command = vec![b'"' as u16];
                    command.extend(executable.as_os_str().encode_wide());
                    command.extend([b'"' as u16, 0]);
                    let bytes: Vec<u8> =
                        command.iter().flat_map(|unit| unit.to_le_bytes()).collect();
                    RegSetValueExW(key, w!("Snapline"), None, REG_SZ, Some(&bytes)).ok()
                }
                Err(error) => Err(Error::new(E_FAIL, error.to_string())),
            }
        } else {
            let result = RegDeleteValueW(key, w!("Snapline"));
            if result == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                result.ok()
            }
        };
        let _ = RegCloseKey(key);
        result
    }
}

pub fn system_dark() -> bool {
    let mut light_theme = 1u32;
    let mut size = std::mem::size_of_val(&light_theme) as u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut light_theme as *mut u32).cast()),
            Some(&mut size),
        ) == ERROR_SUCCESS
            && light_theme == 0
    }
}

struct Pidl(*mut ITEMIDLIST);

impl Drop for Pidl {
    fn drop(&mut self) {
        unsafe { ILFree(Some(self.0)) };
    }
}

fn shell_path(path: &Path) -> Result<Vec<u16>> {
    let absolute =
        std::fs::canonicalize(path).map_err(|error| Error::new(E_FAIL, error.to_string()))?;
    let encoded: Vec<u16> = absolute.as_os_str().encode_wide().collect();
    // Shell PIDLs use DOS/UNC paths; canonicalize returns extended-length paths.
    let local_prefix: Vec<u16> = "\\\\?\\".encode_utf16().collect();
    let unc_prefix: Vec<u16> = "\\\\?\\UNC\\".encode_utf16().collect();
    let mut encoded = if encoded.starts_with(&unc_prefix) {
        "\\\\"
            .encode_utf16()
            .chain(encoded[unc_prefix.len()..].iter().copied())
            .collect()
    } else if encoded.starts_with(&local_prefix) {
        encoded[local_prefix.len()..].to_vec()
    } else {
        encoded
    };
    encoded.push(0);
    Ok(encoded)
}

fn shell_data_object(path: &Path) -> Result<IDataObject> {
    let path = shell_path(path)?;
    unsafe {
        let mut absolute = std::ptr::null_mut();
        SHParseDisplayName(PCWSTR(path.as_ptr()), None, &mut absolute, 0, None)?;
        let absolute = Pidl(absolute);
        let parent = Pidl(ILClone(absolute.0));
        if parent.0.is_null() || !ILRemoveLastID(Some(parent.0)).as_bool() {
            return Err(Error::new(
                E_FAIL,
                "Cannot locate the screenshot's parent folder",
            ));
        }
        let child = ILFindLastID(absolute.0);
        SHCreateDataObject(Some(parent.0), Some(&[child as *const ITEMIDLIST]), None)
    }
}

/// Copies an actual file object, including CF_HDROP, rather than path text.
pub fn copy_file(_hwnd: HWND, path: &Path) -> Result<()> {
    let _ole = OleGuard::initialize()?;
    let data = shell_data_object(path)?;
    unsafe {
        OleSetClipboard(&data)?;
        // Materialize formats so the clipboard survives exiting this application.
        OleFlushClipboard()
    }
}

/// Call while the primary button is held, on the UI thread. True means moved;
/// copies and cancellation are false. The caller removes missing moved cache items.
pub fn drag_file(hwnd: HWND, path: &Path) -> Result<bool> {
    let _ole = OleGuard::initialize()?;
    let data = shell_data_object(path)?;
    let effect =
        unsafe { SHDoDragDrop(Some(hwnd), &data, None, DROPEFFECT_COPY | DROPEFFECT_MOVE)? };
    Ok(effect.0 & DROPEFFECT_MOVE.0 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::{
        System::{
            Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL},
            Ole::ReleaseStgMedium,
        },
        UI::Shell::{DragQueryFileW, HDROP},
    };

    #[test]
    fn shutter_is_short_mono_pcm_wav() {
        assert_eq!(&SHUTTER_WAV[..4], b"RIFF");
        assert_eq!(&SHUTTER_WAV[8..16], b"WAVEfmt ");
        assert_eq!(
            u16::from_le_bytes(SHUTTER_WAV[20..22].try_into().unwrap()),
            1
        );
        assert_eq!(
            u16::from_le_bytes(SHUTTER_WAV[22..24].try_into().unwrap()),
            1
        );
        assert_eq!(
            u16::from_le_bytes(SHUTTER_WAV[34..36].try_into().unwrap()),
            16
        );
        assert_eq!(&SHUTTER_WAV[36..40], b"data");
        let bytes = u32::from_le_bytes(SHUTTER_WAV[40..44].try_into().unwrap()) as usize;
        let rate = u32::from_le_bytes(SHUTTER_WAV[24..28].try_into().unwrap()) as usize;
        assert_eq!(bytes + 44, SHUTTER_WAV.len());
        assert!((100..=150).contains(&(bytes * 1000 / (rate * 2))));
        assert!(SHUTTER_WAV[44..].iter().any(|&byte| byte != 0));
    }

    #[test]
    fn named_instance_excludes_second_writer_and_signals_once() {
        let name = format!("Local\\Snapline.Test.{}", std::process::id());
        let mutex_name = wide(OsStr::new(&format!("{name}.Mutex")));
        let event_name = wide(OsStr::new(&format!("{name}.Event")));
        let acquire = || {
            SingleInstance::acquire_named(PCWSTR(mutex_name.as_ptr()), PCWSTR(event_name.as_ptr()))
                .unwrap()
        };
        let first = acquire().unwrap();
        assert!(acquire().is_none());
        assert!(!first.take_settings_request());
        unsafe {
            SetEvent(first.settings_event).unwrap();
        }
        assert!(first.take_settings_request());
        assert!(!first.take_settings_request());
        drop(first);
        assert!(acquire().is_some());
    }

    #[test]
    fn fullscreen_detection_respects_negative_monitor_coordinates() {
        let monitor = MonitorInfo {
            x: -2560,
            y: -360,
            width: 2560,
            height: 1440,
            scale: 1.5,
        };
        assert!(covers_monitor(
            RECT {
                left: -2560,
                top: -360,
                right: 0,
                bottom: 1080
            },
            monitor
        ));
        assert!(!covers_monitor(
            RECT {
                left: -2550,
                top: -350,
                right: -10,
                bottom: 1070
            },
            monitor
        ));
    }

    #[test]
    fn shell_file_object_exposes_unicode_hdrop_path() {
        let _ole = OleGuard::initialize().expect("Initialize OLE STA");
        let folder =
            std::env::temp_dir().join(format!("snapline-shell-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("截图 样本.png");
        std::fs::write(&path, b"shell-format-test").unwrap();
        let data = shell_data_object(&path).expect("Create a real Shell file object");
        let format = FORMATETC {
            cfFormat: CF_HDROP.0,
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
            ..Default::default()
        };
        let mut medium =
            unsafe { data.GetData(&format) }.expect("Shell file object must contain CF_HDROP");
        let actual = unsafe {
            let drop_handle = HDROP(medium.u.hGlobal.0);
            assert_eq!(DragQueryFileW(drop_handle, u32::MAX, None), 1);
            let length = DragQueryFileW(drop_handle, 0, None);
            let mut encoded = vec![0u16; length as usize + 1];
            assert_eq!(DragQueryFileW(drop_handle, 0, Some(&mut encoded)), length);
            ReleaseStgMedium(&mut medium);
            String::from_utf16(&encoded[..length as usize]).unwrap()
        };
        let expected = shell_path(&path).unwrap();
        assert_eq!(
            actual,
            String::from_utf16(&expected[..expected.len() - 1]).unwrap()
        );
        drop(data);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(folder).unwrap();
    }
}
