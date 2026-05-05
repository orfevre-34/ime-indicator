// Mac ライクな「a / あ」インジケータ。フォアグラウンドの IME 状態を
// ポーリングし、変化があったらキャレット付近に小さなオーバーレイをフェード表示する。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod caret;
mod ime;
mod overlay;

use std::cell::RefCell;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, SetProcessDpiAwarenessContext};
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer,
    LoadCursorW, MSG, PostQuitMessage, RegisterClassExW, SW_SHOWNOACTIVATE, SetTimer, ShowWindow,
    TranslateMessage, WM_DESTROY, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::w;

use crate::ime::ImeMode;

use crate::app::App;
use crate::overlay::Overlay;

// PER_MONITOR_AWARE_V2 = -4 を HANDLE 化したのが定数。windows crate では
// `windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2` を使う。
use windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2;

const TIMER_POLL: usize = 1;
const TIMER_FADE: usize = 2;
const POLL_INTERVAL_MS: u32 = 100;
const FADE_INTERVAL_MS: u32 = 16;

/// thread_local にアプリ状態とオーバーレイを置くことで、グローバル mut を避けつつ
/// WndProc から触れるようにする。Win32 メッセージループは単一スレッドなので OK。
struct State {
    overlay: Overlay,
    app: App,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn main() -> windows::core::Result<()> {
    unsafe {
        // 高 DPI 対応。失敗しても致命ではない（古い OS 等）。
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        // WIC は COM。STA でよい（UI スレッド）。
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;

        let hinstance: HINSTANCE = GetModuleHandleW(None)?.into();
        let class_name = w!("ImeIndicatorOverlay");

        // ウィンドウクラス登録。
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
        let atom = RegisterClassExW(&wc);
        if atom == 0 {
            return Err(windows::core::Error::from_thread());
        }

        // レイヤード + 透過 + ツールウィンドウ + 最前面 + 非アクティブ。
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

        // DPI スケール。SetProcessDpiAwarenessContext 後は GetDpiForSystem が
        // 実 DPI を返す。マルチモニタで厳密にやるなら GetDpiForWindow を使うが、
        // インジケータが移動した瞬間にリビルドする実装が必要になるため MVP では割愛。
        let dpi = GetDpiForSystem();
        let dpi_scale = dpi as f32 / 96.0;

        #[cfg(debug_assertions)]
        eprintln!("ime-indicator: dpi={dpi} scale={dpi_scale}");

        let overlay = Overlay::new(hwnd, dpi_scale)?;
        #[cfg(debug_assertions)]
        eprintln!("ime-indicator: overlay created");

        // レイヤードウィンドウは UpdateLayeredWindow を最初に必ず呼んでから ShowWindow
        // しないと「描画なしの空ウィンドウ」が一瞬だけ見えてしまう / ShowWindow 自体が
        // 効かないことがある。完全透明 (opacity=0) でオフスクリーンにプライミングしておく。
        overlay.render(-10_000, -10_000, ImeMode::Alpha, 0.0)?;
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        #[cfg(debug_assertions)]
        eprintln!("ime-indicator: window shown (priming render done)");

        // 起動直後の見える化: 現在の IME モードを一瞬出して、
        // 「ちゃんとプロセスが動いて描画もできている」ことをユーザに示す。
        let initial_mode = ime::read_current_mode().unwrap_or(ImeMode::Alpha);
        #[cfg(debug_assertions)]
        eprintln!("ime-indicator: detected initial mode = {initial_mode:?}");

        let mut app_state = App::new();
        app_state.current_mode = initial_mode;
        app_state.last_mode = Some(initial_mode);

        let anchor = caret::indicator_anchor();
        let (w, _) = overlay.size_px();
        app_state.on_mode_changed(initial_mode, (anchor.0 - w / 2, anchor.1));

        STATE.with(|s| {
            *s.borrow_mut() = Some(State {
                overlay,
                app: app_state,
            });
        });

        // ポーリング + 初期フェードの両タイマーを開始。
        SetTimer(Some(hwnd), TIMER_POLL, POLL_INTERVAL_MS, None);
        SetTimer(Some(hwnd), TIMER_FADE, FADE_INTERVAL_MS, None);

        // メッセージループ。
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
            WM_DESTROY => {
                let _ = KillTimer(Some(hwnd), TIMER_POLL);
                let _ = KillTimer(Some(hwnd), TIMER_FADE);
                STATE.with(|s| s.borrow_mut().take()); // overlay を drop
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn on_poll_tick(hwnd: HWND) {
    let Some(mode) = ime::read_current_mode() else {
        // フォアグラウンドが IME コンテキストを持たないアプリ（一部の Win32 / コマンドプロンプト等）。
        // 何もせず次のポーリングを待つ。
        return;
    };
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let Some(state) = s.as_mut() else { return };

        // 起動直後の空状態を埋める（最初の 1 回はトリガしない。今のモードを基準にする）。
        if state.app.last_mode.is_none() && !state.app.is_visible() {
            state.app.current_mode = mode;
            state.app.last_mode = Some(mode);
            #[cfg(debug_assertions)]
            eprintln!("ime-indicator: initial mode = {:?}", mode);
            return;
        }

        if mode != state.app.current_mode {
            let anchor = caret::indicator_anchor();
            let (w, _) = state.overlay.size_px();
            // アンカーから少し左にずらしてキャレット中央寄りに。
            let x = anchor.0 - w / 2;
            let y = anchor.1;
            #[cfg(debug_assertions)]
            eprintln!(
                "ime-indicator: {:?} -> {:?} @ ({}, {})",
                state.app.current_mode, mode, x, y
            );
            state.app.on_mode_changed(mode, (x, y));
            // フェード用の高頻度タイマー開始。ウィンドウは常に visible なので Show は不要。
            unsafe {
                SetTimer(Some(hwnd), TIMER_FADE, FADE_INTERVAL_MS, None);
            }
        }
    });
}

fn on_fade_tick(hwnd: HWND) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let Some(state) = s.as_mut() else { return };

        match state.app.tick() {
            Some(opacity) => {
                let (x, y) = state.app.anchor;
                if let Err(e) = state.overlay.render(x, y, state.app.current_mode, opacity) {
                    #[cfg(debug_assertions)]
                    eprintln!("ime-indicator: render error: {e}");
                    let _ = e;
                }
            }
            None => {
                // 非表示遷移。タイマー停止 + 完全透明な状態を一度描いて画面から消す。
                unsafe {
                    let _ = KillTimer(Some(hwnd), TIMER_FADE);
                }
                let _ = state
                    .overlay
                    .render(-10_000, -10_000, state.app.current_mode, 0.0);
            }
        }
    });
}
