// Mac ライクな「a / あ」インジケータ。フォアグラウンドの IME 状態変化を検出し、
// キャレット付近に小さなオーバーレイをフェード表示する。
//
// 検出経路は 2 系統:
//
//  1. 低レベルキーボードフック (WH_KEYBOARD_LL) で IME 切替系のキーを直接捕まえる。
//     Chrome/Edge/VS Code/Windows Terminal などモダンな TSF 系アプリでは ImmGetContext
//     で状態を読めないので、こちらが主検出経路。
//  2. 100ms 周期のポーリング (ImmGetContext + AttachThreadInput)。古典的な IMM
//     系アプリ向けの保険。マウスで IME バーを操作した場合などキー以外の経路でも拾える。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod caret;
mod ime;
mod overlay;

use std::cell::RefCell;
use std::sync::atomic::{AtomicI32, AtomicIsize, Ordering};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CallNextHookEx, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW,
    HHOOK, KBDLLHOOKSTRUCT, KillTimer, LoadCursorW, MSG, PostMessageW, PostQuitMessage,
    RegisterClassExW, SW_SHOWNOACTIVATE, SetTimer, SetWindowsHookExW, ShowWindow, TranslateMessage,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_APP, WM_DESTROY, WM_KEYUP, WM_SYSKEYUP, WM_TIMER,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::w;

use crate::app::App;
use crate::ime::ImeMode;
use crate::overlay::Overlay;

const TIMER_POLL: usize = 1;
const TIMER_FADE: usize = 2;
const POLL_INTERVAL_MS: u32 = 100;
const FADE_INTERVAL_MS: u32 = 16;

/// キーボードフックから WndProc にモード変化を通知するためのアプリ独自メッセージ。
const WM_APP_IME_CHANGED: u32 = WM_APP + 1;

const MODE_ALPHA: i32 = 0;
const MODE_HIRAGANA: i32 = 1;
const MODE_OTHER: i32 = 2;

/// キーボードフックは extern "system" の関数。State (RefCell<…>) を直接触れないので、
/// 「現在モード」「オーバーレイの HWND」を atomic で共有する。
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

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: Default::default(),
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: Default::default(),
            hCursor: LoadCursorW(None, windows::Win32::UI::WindowsAndMessaging::IDC_ARROW)?,
            hbrBackground: Default::default(),
            lpszMenuName: windows::core::PCWSTR::null(),
            lpszClassName: class_name,
            hIconSm: Default::default(),
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
        #[cfg(debug_assertions)]
        eprintln!("ime-indicator: window shown");

        // フックから PostMessage で通知できるよう、HWND を atomic に共有。
        OVERLAY_HWND_ATOM.store(hwnd.0 as isize, Ordering::Relaxed);

        // 起動時の現在モード推定（ImmGetContext が効くアプリだけ取れる）。
        let initial_mode = ime::read_current_mode().unwrap_or(ImeMode::Alpha);
        MODE_ATOM.store(mode_to_int(initial_mode), Ordering::Relaxed);
        #[cfg(debug_assertions)]
        eprintln!("ime-indicator: initial mode = {initial_mode:?}");

        let mut app_state = App::new();
        app_state.current_mode = initial_mode;
        app_state.last_mode = Some(initial_mode);

        // 起動の見える化として 1 度フェード表示。
        let anchor = caret::indicator_anchor();
        let (w_px, _) = overlay.size_px();
        app_state.on_mode_changed(initial_mode, (anchor.0 - w_px / 2, anchor.1));

        STATE.with(|s| {
            *s.borrow_mut() = Some(State {
                overlay,
                app: app_state,
            });
        });

        // 低レベルキーボードフック登録。これが効かないとモダン TSF アプリで何も拾えない。
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

        SetTimer(Some(hwnd), TIMER_POLL, POLL_INTERVAL_MS, None);
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
                let id = wparam.0;
                if id == TIMER_POLL {
                    on_poll_tick(hwnd);
                } else if id == TIMER_FADE {
                    on_fade_tick(hwnd);
                }
                LRESULT(0)
            }
            m if m == WM_APP_IME_CHANGED => {
                let new_mode = int_to_mode(wparam.0 as i32);
                apply_mode_change(hwnd, new_mode);
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = KillTimer(Some(hwnd), TIMER_POLL);
                let _ = KillTimer(Some(hwnd), TIMER_FADE);
                let raw = HOOK_HANDLE.swap(0, Ordering::Relaxed);
                if raw != 0 {
                    let _ = UnhookWindowsHookEx(HHOOK(raw as *mut _));
                }
                STATE.with(|s| s.borrow_mut().take()); // overlay を drop
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// IME モード変化が来たときに呼ぶ。常駐表示なのでフェードはやり直さず、
/// 表示中の文字と座標だけ即座に差し替えて 1 度描画する。
fn apply_mode_change(_hwnd: HWND, mode: ImeMode) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let Some(state) = s.as_mut() else { return };
        if mode == state.app.current_mode {
            return;
        }
        let anchor = caret::indicator_anchor();
        let (w_px, _) = state.overlay.size_px();
        let pos = (anchor.0 - w_px / 2, anchor.1);
        #[cfg(debug_assertions)]
        eprintln!(
            "ime-indicator: {:?} -> {:?} @ {:?}",
            state.app.current_mode, mode, pos
        );
        state.app.on_mode_changed(mode, pos);
        let opacity = state.app.current_opacity();
        let _ = state
            .overlay
            .render(pos.0, pos.1, state.app.current_mode, opacity);
    });
}

fn on_poll_tick(_hwnd: HWND) {
    let mode_opt = ime::read_current_mode();

    #[cfg(debug_assertions)]
    {
        use std::cell::Cell;
        thread_local! {
            static POLL_COUNT: Cell<u32> = const { Cell::new(0) };
        }
        let n = POLL_COUNT.with(|c| {
            let v = c.get() + 1;
            c.set(v);
            v
        });
        if n.is_multiple_of(10) {
            let fg_hwnd = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
            let cur = MODE_ATOM.load(Ordering::Relaxed);
            eprintln!(
                "ime-indicator: poll#{n} fg=0x{:x} imm={:?} state={:?}",
                fg_hwnd.0 as usize,
                mode_opt,
                int_to_mode(cur)
            );
        }
    }

    if let Some(mode) = mode_opt {
        let new_int = mode_to_int(mode);
        let prev_int = MODE_ATOM.swap(new_int, Ordering::Relaxed);
        if prev_int != new_int {
            // フックと同じ経路（apply_mode_change）に流す。
            let raw = OVERLAY_HWND_ATOM.load(Ordering::Relaxed);
            if raw != 0 {
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(raw as *mut _)),
                        WM_APP_IME_CHANGED,
                        WPARAM(new_int as usize),
                        LPARAM(0),
                    );
                }
            }
        }
    }

    // 常駐表示なので、ポーリングのたびにキャレット位置を再取得して再描画する。
    // 16ms で回すと UIA のコストがバカにならないので、100ms 毎の再描画に留める。
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let Some(state) = s.as_mut() else { return };
        let anchor = caret::indicator_anchor();
        let (w_px, _) = state.overlay.size_px();
        let pos = (anchor.0 - w_px / 2, anchor.1);
        state.app.anchor = pos;
        let opacity = state.app.current_opacity();
        let _ = state
            .overlay
            .render(pos.0, pos.1, state.app.current_mode, opacity);
    });
}

fn on_fade_tick(hwnd: HWND) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let Some(state) = s.as_mut() else { return };

        let opacity = state.app.current_opacity();
        let (x, y) = state.app.anchor;
        if let Err(e) = state.overlay.render(x, y, state.app.current_mode, opacity) {
            #[cfg(debug_assertions)]
            eprintln!("ime-indicator: render error: {e}");
            let _ = e;
        }

        // 起動時のフェードインが終わったら 16ms タイマーを止める。以降の再描画は
        // 100ms ポーリングの方で十分（常駐表示で動かない）。
        if !state.app.is_animating() {
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_FADE);
            }
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

/// 押されたキーから次のモードを決める。`Some(toggle)` のときは現在状態を反転する。
enum KeyAction {
    SetAlpha,
    SetHiragana,
    SetKatakana,
    Toggle,
}

fn vk_to_action(vk: u32) -> Option<KeyAction> {
    match vk {
        // 半角/全角キー (VK_KANJI / VK_HANJA = 0x19): IME on/off トグル。
        0x19 => Some(KeyAction::Toggle),
        // VK_KANA / VK_HANGUL = 0x15: トグル扱い。
        0x15 => Some(KeyAction::Toggle),
        // VK_IME_ON = 0x16 / VK_IME_OFF = 0x1A.
        0x16 => Some(KeyAction::SetHiragana),
        0x1A => Some(KeyAction::SetAlpha),
        // VK_DBE_ALPHANUMERIC = 0xF0 (英数 / lang0)
        0xF0 => Some(KeyAction::SetAlpha),
        // VK_DBE_KATAKANA = 0xF1
        0xF1 => Some(KeyAction::SetKatakana),
        // VK_DBE_HIRAGANA = 0xF2 (かな / lang1)
        0xF2 => Some(KeyAction::SetHiragana),
        // VK_DBE_SBCSCHAR (0xF3) / VK_DBE_DBCSCHAR (0xF4): 半角/全角トグル系。
        0xF3 | 0xF4 => Some(KeyAction::Toggle),
        _ => None,
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // HC_ACTION = 0。それ以外は触らずに次のフックへ。
    if code == 0 {
        let msg_kind = wparam.0 as u32;
        // KEYUP のときだけ反応（押しっぱなしの自動リピートで暴れないように）。
        if msg_kind == WM_KEYUP || msg_kind == WM_SYSKEYUP {
            let kbd = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            let vk = kbd.vkCode;
            #[cfg(debug_assertions)]
            eprintln!("ime-indicator: keyup vk=0x{:x} sc=0x{:x}", vk, kbd.scanCode);

            if let Some(action) = vk_to_action(vk) {
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
                let prev = MODE_ATOM.swap(new, Ordering::Relaxed);
                if prev != new {
                    let raw = OVERLAY_HWND_ATOM.load(Ordering::Relaxed);
                    if raw != 0 {
                        unsafe {
                            let _ = PostMessageW(
                                Some(HWND(raw as *mut _)),
                                WM_APP_IME_CHANGED,
                                WPARAM(new as usize),
                                LPARAM(0),
                            );
                        }
                    }
                }
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}
