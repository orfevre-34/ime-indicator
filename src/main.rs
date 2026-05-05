// Mac ライクな「a / あ」インジケータ。IME 切替系のキー（lang0/lang1/半角全角
// 等）が押されたときだけ表示し、しばらく経つとフェードアウトする。モードが
// 実際に変わらなくても、トリガーキーの押下があれば必ず表示する。普通の
// 文字キーや矢印キー等は表示には影響しない。
//
// 検出は WH_KEYBOARD_LL の単一経路。以前は IMM ポーリングを保険として併用
// していたが、相手アプリの IME 状態を読むには AttachThreadInput が必要で、
// 入力キューを共有するこの API は変換中の IME を勝手に確定／取消させて
// ユーザーの入力を壊すことが分かったため廃止した。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod caret;
mod ime;
mod overlay;
mod startup;

use std::cell::RefCell;
use std::sync::atomic::{AtomicI32, AtomicIsize, Ordering};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CW_USEDEFAULT, CallNextHookEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, HHOOK, HICON,
    IDC_ARROW, IMAGE_ICON, KBDLLHOOKSTRUCT, KillTimer, LR_DEFAULTCOLOR, LR_SHARED, LoadCursorW,
    LoadImageW, MF_CHECKED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, PostMessageW,
    PostQuitMessage, RegisterClassExW, SW_SHOWNOACTIVATE, SetForegroundWindow, SetTimer,
    SetWindowsHookExW, ShowWindow, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_APP, WM_COMMAND, WM_DESTROY, WM_KEYUP, WM_LBUTTONUP,
    WM_RBUTTONUP, WM_SYSKEYUP, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::app::App;
use crate::ime::ImeMode;
use crate::overlay::Overlay;

const TIMER_FADE: usize = 2;
const FADE_INTERVAL_MS: u32 = 16;

/// IME トリガーキー押下 or ポーリングでモード変化を検出した通知。
/// wparam にモード(int)を載せる。モードが変わっていなくても表示寿命を延ばす目的で
/// この経路に投げてよい。
const WM_APP_IME_CHANGED: u32 = WM_APP + 1;
/// 通知領域 (タスクトレイ) アイコンからのコールバックメッセージ。
const WM_APP_TRAY: u32 = WM_APP + 3;

/// 通知領域に登録するアイコン ID。同一プロセス内で 1 個だけ使う。
const TRAY_ICON_ID: u32 = 1;
/// 埋め込みリソース内のアプリアイコン ID（app.rc の IDI_APP_ICON と一致させる）。
const IDI_APP_ICON: u16 = 1;

/// メニュー項目 ID。
const IDM_TOGGLE_AUTOSTART: u32 = 1001;
const IDM_QUIT: u32 = 1002;

const MODE_ALPHA: i32 = 0;
const MODE_HIRAGANA: i32 = 1;
const MODE_OTHER: i32 = 2;

static MODE_ATOM: AtomicI32 = AtomicI32::new(MODE_ALPHA);
static OVERLAY_HWND_ATOM: AtomicIsize = AtomicIsize::new(0);
static HOOK_HANDLE: AtomicIsize = AtomicIsize::new(0);

struct State {
    overlay: Overlay,
    app: App,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn main() -> windows::core::Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;

        let hinstance: HINSTANCE = GetModuleHandleW(None)?.into();
        let class_name = w!("ImeIndicatorOverlay");

        let app_icon = load_app_icon(hinstance);

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: Default::default(),
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: app_icon,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: Default::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: class_name,
            hIconSm: app_icon,
        };
        if RegisterClassExW(&wc) == 0 {
            return Err(windows::core::Error::from_thread());
        }

        let hwnd: HWND = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            class_name,
            w!("IME Indicator"),
            WS_POPUP,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            0,
            0,
            None,
            None,
            Some(hinstance),
            None,
        )?;

        let dpi = GetDpiForSystem();
        let dpi_scale = dpi as f32 / 96.0;
        #[cfg(debug_assertions)]
        eprintln!("ime-indicator: dpi={dpi} scale={dpi_scale}");

        let overlay = Overlay::new(hwnd, dpi_scale)?;
        #[cfg(debug_assertions)]
        eprintln!("ime-indicator: overlay created");

        // UpdateLayeredWindow を最初に通してから ShowWindow しないと初回表示が出ないことがある。
        overlay.render(-10_000, -10_000, ImeMode::Alpha, 0.0)?;
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        OVERLAY_HWND_ATOM.store(hwnd.0 as isize, Ordering::Relaxed);

        // 初期モードは推測しない（フォアグラウンドの IME 状態を読む手段は
        // クロスプロセスでは AttachThreadInput が必須で、それが入力を壊すため廃止）。
        // 起動直後は Alpha と仮定。ユーザーが最初にトリガーキーを押した瞬間に
        // 正しい状態に同期される。
        let initial_mode = ImeMode::Alpha;
        MODE_ATOM.store(mode_to_int(initial_mode), Ordering::Relaxed);

        let mut app_state = App::new();
        app_state.current_mode = initial_mode;
        app_state.last_mode = Some(initial_mode);

        // 起動時にも 1 度フェード表示（プロセスが動いている見える化を兼ねる）。
        let anchor = caret::indicator_anchor();
        let (w_px, _) = overlay.size_px();
        app_state.on_mode_changed(initial_mode, (anchor.0 - w_px / 2, anchor.1));

        STATE.with(|s| {
            *s.borrow_mut() = Some(State {
                overlay,
                app: app_state,
            });
        });

        match SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), None, 0) {
            Ok(hook) => {
                HOOK_HANDLE.store(hook.0 as isize, Ordering::Relaxed);
                #[cfg(debug_assertions)]
                eprintln!("ime-indicator: keyboard hook installed");
            }
            Err(e) => {
                #[cfg(debug_assertions)]
                eprintln!("ime-indicator: failed to install keyboard hook: {e}");
                let _ = e;
            }
        }

        // 通知領域アイコン登録。失敗しても致命ではない（インジケータ表示は継続）。
        if let Err(e) = add_tray_icon(hwnd, app_icon) {
            #[cfg(debug_assertions)]
            eprintln!("ime-indicator: failed to register tray icon: {e}");
            let _ = e;
        }

        // フェードイン用のタイマーだけ立てる。終わったら on_fade_tick が自分で止める。
        SetTimer(Some(hwnd), TIMER_FADE, FADE_INTERVAL_MS, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_TIMER => {
                if wparam.0 == TIMER_FADE {
                    on_fade_tick(hwnd);
                }
                LRESULT(0)
            }
            m if m == WM_APP_IME_CHANGED => {
                let new_mode = int_to_mode(wparam.0 as i32);
                apply_mode_change(hwnd, new_mode);
                LRESULT(0)
            }
            m if m == WM_APP_TRAY => {
                let evt = lparam.0 as u32 & 0xFFFF;
                if evt == WM_LBUTTONUP || evt == WM_RBUTTONUP {
                    show_tray_menu(hwnd);
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as u32;
                match id {
                    IDM_TOGGLE_AUTOSTART => {
                        let target = !startup::is_enabled();
                        if let Err(e) = startup::set_enabled(target) {
                            #[cfg(debug_assertions)]
                            eprintln!("ime-indicator: autostart toggle failed: {e}");
                            let _ = e;
                        }
                    }
                    IDM_QUIT => {
                        let _ = DestroyWindow(hwnd);
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = KillTimer(Some(hwnd), TIMER_FADE);
                let _ = remove_tray_icon(hwnd);
                let raw = HOOK_HANDLE.swap(0, Ordering::Relaxed);
                if raw != 0 {
                    let _ = UnhookWindowsHookEx(HHOOK(raw as *mut _));
                }
                STATE.with(|s| s.borrow_mut().take());
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn ensure_fade_timer(hwnd: HWND) {
    unsafe {
        SetTimer(Some(hwnd), TIMER_FADE, FADE_INTERVAL_MS, None);
    }
}

/// IME モード変化を反映。表示中の文字と座標を更新し、見せ始める / 見せ続ける。
fn apply_mode_change(hwnd: HWND, mode: ImeMode) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let Some(state) = s.as_mut() else { return };
        let anchor = caret::indicator_anchor();
        let (w_px, _) = state.overlay.size_px();
        let pos = (anchor.0 - w_px / 2, anchor.1);
        #[cfg(debug_assertions)]
        eprintln!(
            "ime-indicator: {:?} -> {:?} @ {:?}",
            state.app.current_mode, mode, pos
        );
        state.app.on_mode_changed(mode, pos);
    });
    ensure_fade_timer(hwnd);
}

fn on_fade_tick(hwnd: HWND) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let Some(state) = s.as_mut() else { return };

        let opacity = state.app.current_opacity();
        if state.app.is_hidden() {
            // 非表示。タイマー停止 + オフスクリーンに完全透明で 1 度描いて消す。
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_FADE);
            }
            let _ = state
                .overlay
                .render(-10_000, -10_000, state.app.current_mode, 0.0);
            return;
        }
        let (x, y) = state.app.anchor;
        if let Err(e) = state.overlay.render(x, y, state.app.current_mode, opacity) {
            #[cfg(debug_assertions)]
            eprintln!("ime-indicator: render error: {e}");
            let _ = e;
        }
    });
}

fn mode_to_int(m: ImeMode) -> i32 {
    match m {
        ImeMode::Alpha => MODE_ALPHA,
        ImeMode::Hiragana => MODE_HIRAGANA,
        ImeMode::Other => MODE_OTHER,
    }
}

fn int_to_mode(i: i32) -> ImeMode {
    match i {
        MODE_HIRAGANA => ImeMode::Hiragana,
        MODE_OTHER => ImeMode::Other,
        _ => ImeMode::Alpha,
    }
}

enum KeyAction {
    SetAlpha,
    SetHiragana,
    SetKatakana,
    Toggle,
}

fn vk_to_action(vk: u32) -> Option<KeyAction> {
    match vk {
        0x19 => Some(KeyAction::Toggle), // VK_KANJI / VK_HANJA (半角/全角)
        0x15 => Some(KeyAction::Toggle), // VK_KANA / VK_HANGUL
        0x16 => Some(KeyAction::SetHiragana), // VK_IME_ON
        0x1A => Some(KeyAction::SetAlpha), // VK_IME_OFF
        0xF0 => Some(KeyAction::SetAlpha), // VK_DBE_ALPHANUMERIC (英数 / lang0)
        0xF1 => Some(KeyAction::SetKatakana), // VK_DBE_KATAKANA
        0xF2 => Some(KeyAction::SetHiragana), // VK_DBE_HIRAGANA (かな / lang1)
        0xF3 | 0xF4 => Some(KeyAction::Toggle), // VK_DBE_SBCSCHAR / DBCSCHAR
        _ => None,
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == 0 {
        let msg_kind = wparam.0 as u32;
        if msg_kind == WM_KEYUP || msg_kind == WM_SYSKEYUP {
            let kbd = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            let vk = kbd.vkCode;
            #[cfg(debug_assertions)]
            eprintln!("ime-indicator: keyup vk=0x{:x} sc=0x{:x}", vk, kbd.scanCode);

            // 反応するのは IME トリガーキーだけ。それ以外は無視。
            let Some(action) = vk_to_action(vk) else {
                return unsafe { CallNextHookEx(None, code, wparam, lparam) };
            };
            let raw = OVERLAY_HWND_ATOM.load(Ordering::Relaxed);
            if raw != 0 {
                let target = HWND(raw as *mut _);
                let cur = MODE_ATOM.load(Ordering::Relaxed);
                let new = match action {
                    KeyAction::SetAlpha => MODE_ALPHA,
                    KeyAction::SetHiragana => MODE_HIRAGANA,
                    KeyAction::SetKatakana => MODE_OTHER,
                    KeyAction::Toggle => {
                        if cur == MODE_ALPHA {
                            MODE_HIRAGANA
                        } else {
                            MODE_ALPHA
                        }
                    }
                };
                MODE_ATOM.store(new, Ordering::Relaxed);
                // モードが実際に変わらなくてもトリガーキー押下なら必ず表示を起こす。
                unsafe {
                    let _ = PostMessageW(
                        Some(target),
                        WM_APP_IME_CHANGED,
                        WPARAM(new as usize),
                        LPARAM(0),
                    );
                }
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// 埋め込みリソースから 32px のアプリアイコンを読み込む。
fn load_app_icon(hinstance: HINSTANCE) -> HICON {
    unsafe {
        match LoadImageW(
            Some(hinstance),
            PCWSTR(IDI_APP_ICON as usize as *const u16),
            IMAGE_ICON,
            32,
            32,
            LR_DEFAULTCOLOR | LR_SHARED,
        ) {
            Ok(h) => HICON(h.0),
            Err(_) => HICON::default(),
        }
    }
}

fn add_tray_icon(hwnd: HWND, hicon: HICON) -> windows::core::Result<()> {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ICON_ID,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_APP_TRAY,
        hIcon: hicon,
        ..Default::default()
    };
    // Tooltip 文字列（最大 128 wchar）。
    let tip: &[u16] = &"IME Indicator\0".encode_utf16().collect::<Vec<u16>>();
    let n = tip.len().min(nid.szTip.len());
    nid.szTip[..n].copy_from_slice(&tip[..n]);

    unsafe { Shell_NotifyIconW(NIM_ADD, &nid).ok() }
}

fn remove_tray_icon(hwnd: HWND) -> windows::core::Result<()> {
    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ICON_ID,
        ..Default::default()
    };
    unsafe { Shell_NotifyIconW(NIM_DELETE, &nid).ok() }
}

fn show_tray_menu(hwnd: HWND) {
    unsafe {
        let menu = match CreatePopupMenu() {
            Ok(m) => m,
            Err(_) => return,
        };
        let autostart_flag = if startup::is_enabled() {
            MF_CHECKED
        } else {
            MF_UNCHECKED
        };
        let _ = AppendMenuW(
            menu,
            MF_STRING | autostart_flag,
            IDM_TOGGLE_AUTOSTART as usize,
            w!("Windows ログオン時に自動起動"),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, IDM_QUIT as usize, w!("終了"));

        let mut pt = windows::Win32::Foundation::POINT::default();
        let _ = GetCursorPos(&mut pt);

        // メニュー外クリックで閉じるためにフォアグラウンドにする必要がある。
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, pt.x, pt.y, Some(0), hwnd, None);
        let _ = DestroyMenu(menu);
    }
}
