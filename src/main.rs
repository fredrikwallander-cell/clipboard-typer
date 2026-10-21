#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("Clipboard Typer only supports Windows.");

use std::{
    env, fs,
    io::Write,
    mem::size_of,
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use windows::{
    Win32::{
        Foundation::{
            COLORREF, CloseHandle, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
        },
        Graphics::Gdi::{CreateSolidBrush, HBRUSH, PtInRect},
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::{
            DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard},
            LibraryLoader::GetModuleHandleW,
            Memory::{GlobalLock, GlobalUnlock},
            Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW},
            Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION},
        },
        UI::{
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, GetKeyboardLayout, HKL, INPUT, INPUT_0, INPUT_KEYBOARD,
                KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOD_ALT,
                MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey, SendInput, UnregisterHotKey,
                VIRTUAL_KEY, VK_CONTROL, VK_ESCAPE, VK_MENU, VK_RETURN, VK_SHIFT, VK_TAB,
                VkKeyScanExW,
            },
            Shell::{
                NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
                Shell_NotifyIconW, ShellExecuteW,
            },
            WindowsAndMessaging::{
                AppendMenuW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CallNextHookEx,
                CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
                DispatchMessageW, GW_HWNDNEXT, GetClassNameW, GetCursorPos, GetForegroundWindow,
                GetMessageW, GetTopWindow, GetWindow, GetWindowRect, GetWindowTextLengthW,
                GetWindowTextW, GetWindowThreadProcessId, HHOOK, HTTRANSPARENT, HWND_TOPMOST,
                IMAGE_ICON, IsWindowVisible, KillTimer, LR_DEFAULTSIZE, LR_SHARED, LWA_ALPHA,
                LoadImageW, MB_ICONERROR, MB_OK, MF_CHECKED, MF_POPUP, MF_SEPARATOR, MF_STRING,
                MF_UNCHECKED, MSG, MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassW,
                SW_HIDE, SW_SHOWNORMAL, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetForegroundWindow,
                SetLayeredWindowAttributes, SetTimer, SetWindowPos, SetWindowsHookExW, ShowWindow,
                TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage,
                UnhookWindowsHookEx, WH_MOUSE_LL, WINDOW_EX_STYLE, WM_APP, WM_COMMAND, WM_DESTROY,
                WM_HOTKEY, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_NCHITTEST, WM_NULL, WM_RBUTTONUP,
                WM_SETTINGCHANGE, WM_THEMECHANGED, WM_TIMER, WNDCLASSW, WS_EX_LAYERED,
                WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_OVERLAPPED, WS_POPUP,
            },
        },
    },
    core::{PCWSTR, w},
};

const HOTKEY_ID: i32 = 1;
const TRAY_ID: u32 = 1;
const WM_TRAY: u32 = WM_APP + 1;
const WM_TYPE_CLIPBOARD: u32 = WM_APP + 2;
const WM_TARGET_CLICKED: u32 = WM_APP + 3;
const TRACK_TARGET_TIMER_ID: usize = 2;
const MENU_TYPE: usize = 1001;
const MENU_EXIT: usize = 1002;
const MENU_DELAY_0: usize = 1100;
const MENU_DELAY_10: usize = 1110;
const MENU_DELAY_20: usize = 1120;
const MENU_DELAY_50: usize = 1150;
const MENU_DELAY_100: usize = 1200;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const ELEVATED_TYPE_MODE: &str = "--elevated-type";
const ELEVATED_HYPERV_MODE: &str = "--elevated-hyperv";

const HYPERV_TYPE_TEXT_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$text = [Console]::In.ReadToEnd()
$vmName = $env:CLIPBOARD_TYPER_VM_NAME
$hostName = $env:CLIPBOARD_TYPER_VM_HOST
$delay = [int]$env:CLIPBOARD_TYPER_DELAY_MS
$session = $null
$logDirectory = Join-Path $env:LOCALAPPDATA 'Clipboard Typer'
$logPath = Join-Path $logDirectory 'hyperv-helper.log'
try {
    $query = @{
        Namespace = 'root\virtualization\v2'
        ClassName = 'Msvm_ComputerSystem'
    }
    if ($hostName -and $hostName -ne [Environment]::MachineName) {
        $session = New-CimSession -ComputerName $hostName
        $query['CimSession'] = $session
    }
    $systems = @(Get-CimInstance @query)
    $vm = $systems | Where-Object { $_.ElementName -eq $vmName } | Select-Object -First 1
    if ($null -eq $vm) {
        New-Item -ItemType Directory -Path $logDirectory -Force | Out-Null
        $availableNames = ($systems | ForEach-Object ElementName) -join '; '
        "VM lookup failed. Requested='$vmName'; Host='$hostName'; Available='$availableNames'" |
            Set-Content -LiteralPath $logPath -Encoding UTF8
        exit 2
    }
    $keyboard = Get-CimAssociatedInstance -InputObject $vm -ResultClassName Msvm_Keyboard |
        Select-Object -First 1
    if ($null -eq $keyboard) { exit 3 }
    $characters = [Globalization.StringInfo]::GetTextElementEnumerator($text)
    while ($characters.MoveNext()) {
        $result = Invoke-CimMethod -InputObject $keyboard -MethodName TypeText -Arguments @{
            asciiText = [string]$characters.Current
        }
        if ([uint32]$result.ReturnValue -ne 0) { exit 4 }
        if ($delay -gt 0) { Start-Sleep -Milliseconds $delay }
    }
    exit 0
} catch {
    New-Item -ItemType Directory -Path $logDirectory -Force | Out-Null
    $_ | Out-String | Set-Content -LiteralPath $logPath -Encoding UTF8
    exit 5
} finally {
    if ($null -ne $session) { Remove-CimSession $session -ErrorAction SilentlyContinue }
}
"#;

static MOUSE_HOOK: Mutex<isize> = Mutex::new(0);
static APP_WINDOW: AtomicIsize = AtomicIsize::new(0);
static OVERLAY_WINDOW: AtomicIsize = AtomicIsize::new(0);
static TARGETING_NEXT_CLICK: AtomicBool = AtomicBool::new(false);
static CHARACTER_DELAY_MS: AtomicU64 = AtomicU64::new(20);

fn main() -> windows::core::Result<()> {
    if let Some(result) = run_elevated_helper_from_arguments() {
        return result;
    }

    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class_name = w!("ClipboardTyperMessageWindow");
        let window_class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: HINSTANCE(instance.0),
            lpszClassName: class_name,
            hbrBackground: HBRUSH::default(),
            ..Default::default()
        };

        if RegisterClassW(&window_class) == 0 {
            return Err(windows::core::Error::from_thread());
        }

        let overlay_class_name = w!("ClipboardTyperTargetOverlay");
        let overlay_class = WNDCLASSW {
            lpfnWndProc: Some(overlay_window_proc),
            hInstance: HINSTANCE(instance.0),
            lpszClassName: overlay_class_name,
            hbrBackground: CreateSolidBrush(COLORREF(0x0071CC2E)),
            ..Default::default()
        };
        if RegisterClassW(&overlay_class) == 0 {
            return Err(windows::core::Error::from_thread());
        }

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            w!("Clipboard Typer"),
            WS_OVERLAPPED,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            None,
            None,
            Some(instance.into()),
            None,
        )?;

        let overlay = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            overlay_class_name,
            w!("Clipboard Typer target"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        SetLayeredWindowAttributes(overlay, COLORREF(0), 72, LWA_ALPHA)?;
        OVERLAY_WINDOW.store(overlay.0 as isize, Ordering::Release);

        if RegisterHotKey(
            Some(hwnd),
            HOTKEY_ID,
            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
            u32::from(b'V'),
        )
        .is_err()
        {
            return Err(windows::core::Error::from_thread());
        }

        add_tray_icon(hwnd)?;
        let _ = SetTimer(Some(hwnd), TRACK_TARGET_TIMER_ID, 100, None);

        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        Ok(())
    }
}

unsafe extern "system" fn overlay_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCHITTEST {
        LRESULT(HTTRANSPARENT as isize)
    } else {
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_HOTKEY if wparam.0 as i32 == HOTKEY_ID => {
            queue_typing(hwnd, true);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TRACK_TARGET_TIMER_ID => {
            if TARGETING_NEXT_CLICK.load(Ordering::Acquire) && escape_requested() {
                disarm_next_click();
            }
            if TARGETING_NEXT_CLICK.load(Ordering::Acquire) {
                update_target_overlay(hwnd);
            }
            LRESULT(0)
        }
        WM_SETTINGCHANGE | WM_THEMECHANGED => {
            let _ = unsafe { update_tray_icon(hwnd) };
            LRESULT(0)
        }
        WM_TRAY => {
            match lparam.0 as u32 {
                WM_LBUTTONUP => {
                    let _ = arm_next_click(hwnd);
                }
                WM_RBUTTONUP => unsafe { show_tray_menu(hwnd) },
                _ => {}
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            match wparam.0 & 0xffff {
                MENU_TYPE => {
                    let _ = arm_next_click(hwnd);
                }
                MENU_DELAY_0 => CHARACTER_DELAY_MS.store(0, Ordering::Release),
                MENU_DELAY_10 => CHARACTER_DELAY_MS.store(10, Ordering::Release),
                MENU_DELAY_20 => CHARACTER_DELAY_MS.store(20, Ordering::Release),
                MENU_DELAY_50 => CHARACTER_DELAY_MS.store(50, Ordering::Release),
                MENU_DELAY_100 => CHARACTER_DELAY_MS.store(100, Ordering::Release),
                MENU_EXIT => unsafe {
                    let _ = DestroyWindow(hwnd);
                },
                _ => {}
            }
            LRESULT(0)
        }
        WM_TARGET_CLICKED => {
            disarm_next_click();
            // Allow the natural click to focus the chosen control and place its caret first.
            queue_typing(hwnd, false);
            LRESULT(0)
        }
        WM_TYPE_CLIPBOARD => {
            if let Some(text) = read_clipboard_text() {
                if let Some(target) = foreground_hyperv_target() {
                    if let Err(error) = launch_elevated_hyperv_helper(&text, &target) {
                        show_error(hwnd, &error);
                    }
                } else {
                    let target = unsafe { GetForegroundWindow() };
                    if window_process_is_elevated(target) {
                        if let Err(error) = launch_elevated_type_helper(&text, target) {
                            show_error(hwnd, &error);
                        }
                    } else {
                        type_text(&text);
                    }
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe {
                disarm_next_click();
                let _ = KillTimer(Some(hwnd), TRACK_TARGET_TIMER_ID);
                let _ = UnregisterHotKey(Some(hwnd), HOTKEY_ID);
                remove_tray_icon(hwnd);
                let overlay = OVERLAY_WINDOW.swap(0, Ordering::AcqRel);
                if overlay != 0 {
                    let _ = DestroyWindow(HWND(overlay as *mut _));
                }
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn queue_typing(hwnd: HWND, wait_for_modifiers: bool) {
    let raw_hwnd = hwnd.0 as usize;
    thread::spawn(move || {
        if wait_for_modifiers {
            if !wait_until_hotkey_released() {
                return;
            }
            thread::sleep(Duration::from_millis(30));
        } else {
            // Let the tray menu close or the target click finish before injecting input.
            thread::sleep(Duration::from_millis(150));
        }
        unsafe {
            let hwnd = HWND(raw_hwnd as *mut _);
            let _ = PostMessageW(Some(hwnd), WM_TYPE_CLIPBOARD, WPARAM(0), LPARAM(0));
        }
    });
}

fn wait_until_hotkey_released() -> bool {
    for _ in 0..500 {
        let control_down = unsafe { GetAsyncKeyState(VK_CONTROL.0 as i32) } < 0;
        let alt_down = unsafe { GetAsyncKeyState(VK_MENU.0 as i32) } < 0;
        if !control_down && !alt_down {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
}

fn arm_next_click(hwnd: HWND) -> windows::core::Result<()> {
    disarm_next_click();
    // Clear an older Escape transition before arming the next-click mode.
    unsafe {
        let _ = GetAsyncKeyState(VK_ESCAPE.0 as i32);
    }
    APP_WINDOW.store(hwnd.0 as isize, Ordering::Release);
    let hook = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), None, 0)? };
    *MOUSE_HOOK.lock().expect("mouse hook lock poisoned") = hook.0 as isize;
    TARGETING_NEXT_CLICK.store(true, Ordering::Release);
    Ok(())
}

fn disarm_next_click() {
    TARGETING_NEXT_CLICK.store(false, Ordering::Release);
    let overlay = OVERLAY_WINDOW.load(Ordering::Acquire);
    if overlay != 0 {
        unsafe {
            let _ = ShowWindow(HWND(overlay as *mut _), SW_HIDE);
        }
    }
    let raw_hook = std::mem::take(&mut *MOUSE_HOOK.lock().expect("mouse hook lock poisoned"));
    if raw_hook != 0 {
        unsafe {
            let _ = UnhookWindowsHookEx(HHOOK(raw_hook as *mut _));
        }
    }
}

fn update_target_overlay(own_window: HWND) {
    let raw_overlay = OVERLAY_WINDOW.load(Ordering::Acquire);
    if raw_overlay == 0 {
        return;
    }
    let overlay = HWND(raw_overlay as *mut _);
    let mut point = POINT::default();
    if unsafe { GetCursorPos(&mut point) }.is_err() {
        return;
    }

    let mut candidate = unsafe { GetTopWindow(None) }.ok();
    while let Some(window) = candidate {
        if window != overlay
            && window != own_window
            && unsafe { IsWindowVisible(window) }.as_bool()
            && is_typing_target(window, own_window)
        {
            let mut rectangle = RECT::default();
            if unsafe { GetWindowRect(window, &mut rectangle) }.is_ok()
                && unsafe { PtInRect(&rectangle, point) }.as_bool()
            {
                let width = rectangle.right - rectangle.left;
                let height = rectangle.bottom - rectangle.top;
                if width > 0 && height > 0 {
                    unsafe {
                        let _ = SetWindowPos(
                            overlay,
                            Some(HWND_TOPMOST),
                            rectangle.left,
                            rectangle.top,
                            width,
                            height,
                            SWP_NOACTIVATE | SWP_SHOWWINDOW,
                        );
                    }
                    return;
                }
            }
        }
        candidate = unsafe { GetWindow(window, GW_HWNDNEXT) }.ok();
    }

    unsafe {
        let _ = ShowWindow(overlay, SW_HIDE);
    }
}

unsafe extern "system" fn mouse_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0
        && wparam.0 as u32 == WM_LBUTTONDOWN
        && TARGETING_NEXT_CLICK.swap(false, Ordering::AcqRel)
    {
        let raw_hwnd = APP_WINDOW.load(Ordering::Acquire);
        if raw_hwnd != 0 {
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(raw_hwnd as *mut _)),
                    WM_TARGET_CLICKED,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn read_clipboard_text() -> Option<String> {
    for _ in 0..8 {
        if unsafe { OpenClipboard(None) }.is_ok() {
            let result = unsafe {
                match GetClipboardData(13) {
                    Ok(handle) => {
                        let global = windows::Win32::Foundation::HGLOBAL(handle.0);
                        let pointer = GlobalLock(global) as *const u16;
                        if pointer.is_null() {
                            None
                        } else {
                            let mut length = 0;
                            while *pointer.add(length) != 0 {
                                length += 1;
                            }
                            let text = String::from_utf16_lossy(std::slice::from_raw_parts(
                                pointer, length,
                            ));
                            let _ = GlobalUnlock(global);
                            Some(text)
                        }
                    }
                    Err(_) => None,
                }
            };
            unsafe { CloseClipboard().ok() };
            return result;
        }
        thread::sleep(Duration::from_millis(15));
    }
    None
}

fn type_text(text: &str) {
    let input_groups = input_groups_for_text(text);
    // Clear an older Escape transition while still honoring a currently held key.
    let escape_was_down = unsafe { GetAsyncKeyState(VK_ESCAPE.0 as i32) } < 0;
    if escape_was_down {
        return;
    }

    for (index, inputs) in input_groups.iter().enumerate() {
        if escape_requested() {
            break;
        }
        unsafe {
            let _ = SendInput(inputs, size_of::<INPUT>() as i32);
        }
        if index + 1 < input_groups.len() {
            let delay_ms = CHARACTER_DELAY_MS.load(Ordering::Acquire);
            if delay_ms > 0 {
                thread::sleep(Duration::from_millis(delay_ms));
            }
        }
    }
}

fn escape_requested() -> bool {
    let state = unsafe { GetAsyncKeyState(VK_ESCAPE.0 as i32) } as u16;
    state & 0x8001 != 0
}

struct HyperVTarget {
    vm_name: String,
    host_name: String,
}

fn run_elevated_helper_from_arguments() -> Option<windows::core::Result<()>> {
    let arguments: Vec<String> = env::args().collect();
    let mode = arguments.get(1).map(String::as_str);
    if mode != Some(ELEVATED_TYPE_MODE) && mode != Some(ELEVATED_HYPERV_MODE) {
        return None;
    }

    let result = (|| -> Result<(), String> {
        let expected_arguments = if mode == Some(ELEVATED_TYPE_MODE) {
            5
        } else {
            6
        };
        if arguments.len() != expected_arguments {
            return Err("The elevated typing helper received invalid arguments.".to_owned());
        }
        let input_path = PathBuf::from(&arguments[2]);
        let text = fs::read_to_string(&input_path)
            .map_err(|error| format!("Could not read elevated clipboard data: {error}"));
        let _ = fs::remove_file(&input_path);
        let text = text?;
        let delay_index = if mode == Some(ELEVATED_TYPE_MODE) {
            4
        } else {
            5
        };
        let delay = arguments[delay_index]
            .parse::<u64>()
            .map_err(|_| "The elevated typing helper received an invalid delay.".to_owned())?;
        CHARACTER_DELAY_MS.store(delay, Ordering::Release);

        if mode == Some(ELEVATED_TYPE_MODE) {
            let raw_window = arguments[3].parse::<usize>().map_err(|_| {
                "The elevated typing helper received an invalid target window.".to_owned()
            })?;
            let target = HWND(raw_window as *mut _);
            unsafe {
                let _ = SetForegroundWindow(target);
            }
            thread::sleep(Duration::from_millis(120));
            type_text(&text);
            return Ok(());
        }

        let target = HyperVTarget {
            vm_name: arguments[3].clone(),
            host_name: arguments[4].clone(),
        };
        type_text_into_hyperv(&text, &target)
    })();

    if let Err(error) = result {
        show_error(HWND::default(), &error);
    }
    Some(Ok(()))
}

fn window_process_is_elevated(window: HWND) -> bool {
    if window.0.is_null() {
        return false;
    }
    let mut process_id = 0;
    unsafe {
        GetWindowThreadProcessId(window, Some(&mut process_id));
    }
    if process_id == 0 {
        return false;
    }

    let process = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }
    {
        Ok(process) => process,
        Err(_) => return false,
    };
    let mut token = HANDLE::default();
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.is_err() {
        unsafe {
            let _ = CloseHandle(process);
        }
        return false;
    }

    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0;
    let elevated = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some((&raw mut elevation).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
        .is_ok()
            && elevation.TokenIsElevated != 0
    };
    unsafe {
        let _ = CloseHandle(token);
        let _ = CloseHandle(process);
    }
    elevated
}

fn foreground_hyperv_target() -> Option<HyperVTarget> {
    let foreground = unsafe { GetForegroundWindow() };
    let title = window_title(foreground)?;
    parse_hyperv_window_title(&title)
}

fn window_title(window: HWND) -> Option<String> {
    let length = unsafe { GetWindowTextLengthW(window) };
    if length <= 0 {
        return None;
    }
    let mut buffer = vec![0u16; length as usize + 1];
    let copied = unsafe { GetWindowTextW(window, &mut buffer) };
    if copied <= 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buffer[..copied as usize]))
}

fn parse_hyperv_window_title(title: &str) -> Option<HyperVTarget> {
    let title = title
        .strip_suffix(" - Anslutning till virtuell dator")
        .or_else(|| title.strip_suffix(" - Virtual Machine Connection"))?;
    let (vm_name, host_name) = title
        .rsplit_once(" på ")
        .or_else(|| title.rsplit_once(" on "))?;
    if vm_name.is_empty() || host_name.is_empty() {
        return None;
    }
    Some(HyperVTarget {
        vm_name: vm_name.to_owned(),
        host_name: host_name.to_owned(),
    })
}

fn launch_elevated_hyperv_helper(text: &str, target: &HyperVTarget) -> Result<(), String> {
    launch_elevated_helper(
        text,
        &[
            ELEVATED_HYPERV_MODE.to_owned(),
            target.vm_name.clone(),
            target.host_name.clone(),
            CHARACTER_DELAY_MS.load(Ordering::Acquire).to_string(),
        ],
    )
}

fn launch_elevated_type_helper(text: &str, target: HWND) -> Result<(), String> {
    launch_elevated_helper(
        text,
        &[
            ELEVATED_TYPE_MODE.to_owned(),
            (target.0 as usize).to_string(),
            CHARACTER_DELAY_MS.load(Ordering::Acquire).to_string(),
        ],
    )
}

fn launch_elevated_helper(text: &str, arguments: &[String]) -> Result<(), String> {
    let executable =
        env::current_exe().map_err(|error| format!("Could not locate Clipboard Typer: {error}"))?;
    let input_path = elevated_input_path();
    fs::write(&input_path, text.as_bytes()).map_err(|error| {
        format!("Could not prepare clipboard text for elevated typing: {error}")
    })?;

    let mut helper_arguments = Vec::with_capacity(arguments.len() + 1);
    helper_arguments.push(arguments[0].clone());
    helper_arguments.push(input_path.to_string_lossy().into_owned());
    helper_arguments.extend_from_slice(&arguments[1..]);
    let parameters = helper_arguments
        .iter()
        .map(|argument| quote_windows_argument(argument))
        .collect::<Vec<_>>()
        .join(" ");

    let executable_wide = null_terminated_wide(executable.as_os_str().to_string_lossy().as_ref());
    let parameters_wide = null_terminated_wide(&parameters);
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("runas"),
            PCWSTR(executable_wide.as_ptr()),
            PCWSTR(parameters_wide.as_ptr()),
            None,
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        let _ = fs::remove_file(&input_path);
        return Err("The selected window requires administrator approval. The elevation request was cancelled or could not be started.".to_owned());
    }
    Ok(())
}

fn elevated_input_path() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    env::temp_dir().join(format!(
        "clipboard-typer-{}-{unique}.txt",
        std::process::id()
    ))
}

fn quote_windows_argument(argument: &str) -> String {
    if !argument.is_empty()
        && !argument
            .chars()
            .any(|character| character.is_whitespace() || character == '"')
    {
        return argument.to_owned();
    }

    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for character in argument.chars() {
        if character == '\\' {
            backslashes += 1;
        } else if character == '"' {
            quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
            quoted.push('"');
            backslashes = 0;
        } else {
            quoted.push_str(&"\\".repeat(backslashes));
            backslashes = 0;
            quoted.push(character);
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

fn null_terminated_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn type_text_into_hyperv(text: &str, target: &HyperVTarget) -> Result<(), String> {
    let delay = CHARACTER_DELAY_MS.load(Ordering::Acquire).to_string();
    let mut child = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            HYPERV_TYPE_TEXT_SCRIPT,
        ])
        .env("CLIPBOARD_TYPER_VM_NAME", &target.vm_name)
        .env("CLIPBOARD_TYPER_VM_HOST", &target.host_name)
        .env("CLIPBOARD_TYPER_DELAY_MS", delay)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| format!("Could not start the Hyper-V typing helper: {error}"))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Could not open the Hyper-V typing helper input.".to_owned())?;
    if let Err(error) = stdin.write_all(text.as_bytes()) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("Could not send clipboard text to Hyper-V: {error}"));
    }
    drop(stdin);

    loop {
        if escape_requested() {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(());
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                let code = status.code().unwrap_or(-1);
                let diagnostic = if matches!(code, 2 | 5) {
                    " See %LOCALAPPDATA%\\Clipboard Typer\\hyperv-helper.log."
                } else {
                    ""
                };
                return Err(format!(
                    "Hyper-V rejected direct text input (helper exit code {code}).{diagnostic}"
                ));
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("Could not monitor Hyper-V text input: {error}"));
            }
        }
    }
}

fn show_error(owner: HWND, message: &str) {
    let message: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = MessageBoxW(
            Some(owner),
            PCWSTR(message.as_ptr()),
            w!("Clipboard Typer"),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn input_groups_for_text(text: &str) -> Vec<Vec<INPUT>> {
    let mut groups = Vec::with_capacity(text.chars().count());
    let mut characters = text.chars().peekable();
    let keyboard_layout = active_keyboard_layout();

    while let Some(character) = characters.next() {
        let mut inputs = Vec::with_capacity(8);
        match character {
            '\r' => {
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
                push_virtual_key(&mut inputs, VK_RETURN);
            }
            '\n' => push_virtual_key(&mut inputs, VK_RETURN),
            '\t' => push_virtual_key(&mut inputs, VK_TAB),
            _ => {
                if !push_layout_character(&mut inputs, character, keyboard_layout) {
                    let mut utf16 = [0u16; 2];
                    for unit in character.encode_utf16(&mut utf16) {
                        push_unicode(&mut inputs, *unit);
                    }
                }
            }
        }
        groups.push(inputs);
    }
    groups
}

fn active_keyboard_layout() -> HKL {
    let foreground = unsafe { GetForegroundWindow() };
    let thread_id = if foreground.0.is_null() {
        0
    } else {
        unsafe { GetWindowThreadProcessId(foreground, None) }
    };
    unsafe { GetKeyboardLayout(thread_id) }
}

fn push_layout_character(inputs: &mut Vec<INPUT>, character: char, layout: HKL) -> bool {
    let mut utf16 = [0u16; 2];
    let units = character.encode_utf16(&mut utf16);
    if units.len() != 1 {
        return false;
    }

    let mapping = unsafe { VkKeyScanExW(units[0], layout) };
    if mapping == -1 {
        return false;
    }
    let mapping = mapping as u16;
    push_mapped_key(inputs, VIRTUAL_KEY(mapping & 0x00ff), (mapping >> 8) as u8)
}

fn push_mapped_key(inputs: &mut Vec<INPUT>, key: VIRTUAL_KEY, modifiers: u8) -> bool {
    let shift = modifiers & 1 != 0;
    let control = modifiers & 2 != 0;
    let alt = modifiers & 4 != 0;

    // Remote/recovery consoles commonly discard synthetic AltGr modifiers. Let the caller
    // send those characters through KEYEVENTF_UNICODE, while retaining physical keys for
    // ordinary and Shift characters such as parentheses.
    if control && alt {
        return false;
    }

    if control {
        push_key_down(inputs, VK_CONTROL);
    }
    if alt {
        push_key_down(inputs, VK_MENU);
    }
    if shift {
        push_key_down(inputs, VK_SHIFT);
    }
    push_virtual_key(inputs, key);
    if shift {
        push_key_up(inputs, VK_SHIFT);
    }
    if alt {
        push_key_up(inputs, VK_MENU);
    }
    if control {
        push_key_up(inputs, VK_CONTROL);
    }
    true
}

fn push_virtual_key(inputs: &mut Vec<INPUT>, key: VIRTUAL_KEY) {
    push_key_down(inputs, key);
    push_key_up(inputs, key);
}

fn push_key_down(inputs: &mut Vec<INPUT>, key: VIRTUAL_KEY) {
    inputs.push(key_input(key.0, 0, KEYBD_EVENT_FLAGS(0)));
}

fn push_key_up(inputs: &mut Vec<INPUT>, key: VIRTUAL_KEY) {
    inputs.push(key_input(key.0, 0, KEYEVENTF_KEYUP));
}

fn push_unicode(inputs: &mut Vec<INPUT>, character: u16) {
    inputs.push(key_input(0, character, KEYEVENTF_UNICODE));
    inputs.push(key_input(0, character, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
}

fn key_input(virtual_key: u16, scan_code: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(virtual_key),
                wScan: scan_code,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

unsafe fn add_tray_icon(hwnd: HWND) -> windows::core::Result<()> {
    let icon = unsafe { load_theme_icon()? };
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: icon,
        ..Default::default()
    };
    write_wide_buffer(&mut data.szTip, "Clipboard Typer — Ctrl+Alt+V");
    if unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool() {
        Ok(())
    } else {
        Err(windows::core::Error::from_thread())
    }
}

unsafe fn load_theme_icon() -> windows::core::Result<windows::Win32::UI::WindowsAndMessaging::HICON>
{
    let instance = unsafe { GetModuleHandleW(None)? };
    let resource_id = if system_uses_light_theme() { 1 } else { 2 };
    let icon = unsafe {
        LoadImageW(
            Some(HINSTANCE(instance.0)),
            PCWSTR(std::ptr::with_exposed_provenance::<u16>(resource_id)),
            IMAGE_ICON,
            0,
            0,
            LR_DEFAULTSIZE | LR_SHARED,
        )?
    };
    Ok(windows::Win32::UI::WindowsAndMessaging::HICON(icon.0))
}

fn system_uses_light_theme() -> bool {
    let mut value = 1u32;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    status.is_ok() && value != 0
}

unsafe fn update_tray_icon(hwnd: HWND) -> windows::core::Result<()> {
    let data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        uFlags: NIF_ICON,
        hIcon: unsafe { load_theme_icon()? },
        ..Default::default()
    };
    if unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool() {
        Ok(())
    } else {
        Err(windows::core::Error::from_thread())
    }
}

unsafe fn remove_tray_icon(hwnd: HWND) {
    let data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        ..Default::default()
    };
    let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
}

unsafe fn show_tray_menu(hwnd: HWND) {
    let Ok(menu) = (unsafe { CreatePopupMenu() }) else {
        return;
    };
    let Ok(delay_menu) = (unsafe { CreatePopupMenu() }) else {
        unsafe {
            let _ = DestroyMenu(menu);
        }
        return;
    };
    unsafe {
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_TYPE,
            w!("Type clipboard on next click"),
        );
        let current_delay = CHARACTER_DELAY_MS.load(Ordering::Acquire);
        let checked = |delay| {
            if current_delay == delay {
                MF_CHECKED
            } else {
                MF_UNCHECKED
            }
        };
        let _ = AppendMenuW(delay_menu, MF_STRING | checked(0), MENU_DELAY_0, w!("0 ms"));
        let _ = AppendMenuW(
            delay_menu,
            MF_STRING | checked(10),
            MENU_DELAY_10,
            w!("10 ms"),
        );
        let _ = AppendMenuW(
            delay_menu,
            MF_STRING | checked(20),
            MENU_DELAY_20,
            w!("20 ms"),
        );
        let _ = AppendMenuW(
            delay_menu,
            MF_STRING | checked(50),
            MENU_DELAY_50,
            w!("50 ms"),
        );
        let _ = AppendMenuW(
            delay_menu,
            MF_STRING | checked(100),
            MENU_DELAY_100,
            w!("100 ms"),
        );
        let _ = AppendMenuW(menu, MF_POPUP, delay_menu.0 as usize, w!("Typing delay"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, MENU_EXIT, w!("Exit"));

        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(
            menu,
            TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            None,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
    }
}

fn is_typing_target(candidate: HWND, own_window: HWND) -> bool {
    if candidate.0.is_null() || candidate == own_window {
        return false;
    }
    let mut class_name = [0u16; 64];
    let length = unsafe { GetClassNameW(candidate, &mut class_name) } as usize;
    let class_name = String::from_utf16_lossy(&class_name[..length]);
    !matches!(
        class_name.as_str(),
        "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "Progman" | "WorkerW"
    )
}

fn write_wide_buffer<const N: usize>(buffer: &mut [u16; N], text: &str) {
    for (destination, source) in buffer
        .iter_mut()
        .take(N.saturating_sub(1))
        .zip(text.encode_utf16())
    {
        *destination = source;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_buffer_is_null_terminated_and_truncated_safely() {
        let mut buffer = [0u16; 5];
        write_wide_buffer(&mut buffer, "abcdef");
        assert_eq!(
            &buffer,
            &[b'a' as u16, b'b' as u16, b'c' as u16, b'd' as u16, 0]
        );
    }

    #[test]
    fn crlf_becomes_one_enter_keystroke() {
        // Three logical keys: A, Enter, B. CRLF must produce only one Enter group.
        let groups = input_groups_for_text("A\r\nB");
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[1].len(), 2);
    }

    #[test]
    fn supplementary_unicode_uses_both_utf16_units() {
        // 😀 is one logical character represented by two UTF-16 units.
        let groups = input_groups_for_text("😀");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 4);
    }

    #[test]
    fn alt_gr_mapping_falls_back_to_unicode() {
        let mut inputs = Vec::new();
        assert!(!push_mapped_key(
            &mut inputs,
            VIRTUAL_KEY(b'4' as u16),
            2 | 4
        ));
        assert!(inputs.is_empty());
    }

    #[test]
    fn shift_mapping_uses_physical_keystrokes() {
        let mut inputs = Vec::new();
        assert!(push_mapped_key(&mut inputs, VIRTUAL_KEY(b'8' as u16), 1));
        assert_eq!(inputs.len(), 4);
        assert!(
            inputs
                .iter()
                .all(|input| unsafe { !input.Anonymous.ki.dwFlags.contains(KEYEVENTF_UNICODE) })
        );
    }

    #[test]
    fn parses_swedish_hyperv_window_title() {
        let target = parse_hyperv_window_title(
            "Ny virtuell dator på NCSUPG-PC201 - Anslutning till virtuell dator",
        )
        .expect("Swedish VMConnect title should parse");
        assert_eq!(target.vm_name, "Ny virtuell dator");
        assert_eq!(target.host_name, "NCSUPG-PC201");
    }

    #[test]
    fn parses_english_hyperv_window_title() {
        let target = parse_hyperv_window_title("Test VM on HOST01 - Virtual Machine Connection")
            .expect("English VMConnect title should parse");
        assert_eq!(target.vm_name, "Test VM");
        assert_eq!(target.host_name, "HOST01");
    }
}
