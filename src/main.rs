// Mac ライクな「a / あ」インジケータ。任意のキー入力中はキャレット付近に小さな
// オーバーレイを出し続け、入力が止まって 1.5 秒経つとフェードアウトする。
//
// 検出経路:
//
//  1. 低レベルキーボードフック (WH_KEYBOARD_LL) で全キーストロークを取得し、
//     IME 切替系のキーはモード変化の通知に、それ以外のキーは「打鍵中」シグナル
//     として表示寿命を延ばす経路に流す。Chrome/Edge/VS Code/Windows Terminal
//     などモダン TSF アプリでも反応する。
//  2. 100ms 周期のポーリング (ImmGetContext + AttachThreadInput)。古典的な IMM
//     系アプリ向けに、キー以外（マウスで IME バー操作）でも変化を拾えるようにする。

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

/// IME のモードが変わった通知。wparam にモード(int)を載せる。
const WM_APP_IME_CHANGED: u32 = WM_APP + 1;
/// IME とは無関係なキー入力があった通知。表示寿命の延長に使う。
const WM_APP_KEY_ACTIVITY: u32 = WM_APP + 2;

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

        OVERLAY_HWND_ATOM.store(hwnd.0 as isize, Ordering::Relaxed);

        let initial_mode = ime::read_current_mode().unwrap_or(ImeMode::Alpha);
        MODE_ATOM.store(mode_to_int(initial_mode), Ordering::Relaxed);
        #[cfg(debug_assertions)]
        eprintln!("ime-indicator: initial mode = {initial_mode:?}");

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
            m if m == WM_APP_KEY_ACTIVITY => {
                apply_key_activity(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = KillTimer(Some(hwnd), TIMER_POLL);
                let _ = KillTimer(Some(hwnd), TIMER_FADE);
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

/// IME とは無関係なキー入力。モードは変えず、表示寿命を延ばすだけ。
/// Hidden / FadeOut からの復帰時はキャレット位置を取り直す。
fn apply_key_activity(hwnd: HWND) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let Some(state) = s.as_mut() else { return };
        if state.app.is_hidden()
            || matches!(state.app.phase, crate::app::Phase::FadeOut { .. })
        {
            let anchor = caret::indicator_anchor();
            let (w_px, _) = state.overlay.size_px();
            state.app.set_anchor((anchor.0 - w_px / 2, anchor.1));
        }
        state.app.on_key_activity();
    });
    ensure_fade_timer(hwnd);
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

    // 表示中はキャレット位置の追従だけ更新する（再描画は TIMER_FADE 任せ）。
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let Some(state) = s.as_mut() else { return };
        if state.app.is_visible() {
            let anchor = caret::indicator_anchor();
            let (w_px, _) = state.overlay.size_px();
            state.app.anchor = (anchor.0 - w_px / 2, anchor.1);
        }
    });
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
        0x19 => Some(KeyAction::Toggle),       // VK_KANJI / VK_HANJA (半角/全角)
        0x15 => Some(KeyAction::Toggle),       // VK_KANA / VK_HANGUL
        0x16 => Some(KeyAction::SetHiragana),  // VK_IME_ON
        0x1A => Some(KeyAction::SetAlpha),     // VK_IME_OFF
        0xF0 => Some(KeyAction::SetAlpha),     // VK_DBE_ALPHANUMERIC (英数 / lang0)
        0xF1 => Some(KeyAction::SetKatakana),  // VK_DBE_KATAKANA
        0xF2 => Some(KeyAction::SetHiragana),  // VK_DBE_HIRAGANA (かな / lang1)
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

            let raw = OVERLAY_HWND_ATOM.load(Ordering::Relaxed);
            if raw != 0 {
                let target = HWND(raw as *mut _);

                // IME 切替系のキーは「モード変化」を試みる。変わらなかったら通常の打鍵扱い。
                let mut handled_as_mode_change = false;
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
                        unsafe {
                            let _ = PostMessageW(
                                Some(target),
                                WM_APP_IME_CHANGED,
                                WPARAM(new as usize),
                                LPARAM(0),
                            );
                        }
                        handled_as_mode_change = true;
                    }
                }

                // モード変化でなかった場合は通常の打鍵として活動シグナルを送る。
                if !handled_as_mode_change {
                    unsafe {
                        let _ = PostMessageW(
                            Some(target),
                            WM_APP_KEY_ACTIVITY,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}
