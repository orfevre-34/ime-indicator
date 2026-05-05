use std::cell::RefCell;
use std::ffi::c_void;

use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Ole::{
    SafeArrayAccessData, SafeArrayGetLBound, SafeArrayGetUBound, SafeArrayUnaccessData,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationTextPattern, UIA_TextPatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GUITHREADINFO, GetCursorPos, GetForegroundWindow, GetGUIThreadInfo, GetWindowRect,
    GetWindowThreadProcessId,
};
use windows::core::Interface;

thread_local! {
    /// IUIAutomation はシングルトン的に使い回す（CoCreateInstance のコストを毎回払わない）。
    static UIA: RefCell<Option<IUIAutomation>> = const { RefCell::new(None) };
}

/// インジケータを表示すべきスクリーン座標（左上）を返す。
///
/// `AttachThreadInput` は副作用が大きく相手アプリの IME 変換を破壊することがあるため、
/// すべての経路で `AttachThreadInput` を使わない方式のみで構成する:
///
/// 1. UI Automation: フォーカス中の TextPattern の bounding rect を取得（読み取り API のみ）
/// 2. `GUITHREADINFO.rcCaret`: 古典的な IMM アプリで取れる
/// 3. フォアグラウンドウィンドウの左下寄り: 上記が取れない時のフォールバック
/// 4. マウスカーソル付近: 最終フォールバック（普通到達しない）
pub fn indicator_anchor() -> (i32, i32) {
    let (kind, pos) = resolve_anchor();
    log_anchor_if_changed(kind, pos);
    pos
}

fn resolve_anchor() -> (&'static str, (i32, i32)) {
    if let Some(p) = caret_via_uia() {
        return ("UIA", (p.x + 4, p.y + 4));
    }
    if let Some(p) = caret_via_guithreadinfo() {
        return ("GUITHREADINFO", (p.x + 4, p.y + 4));
    }
    if let Some(p) = foreground_window_anchor() {
        return ("foreground-window", p);
    }
    ("cursor", cursor_pos_with_offset())
}

fn log_anchor_if_changed(_kind: &'static str, _pos: (i32, i32)) {
    #[cfg(debug_assertions)]
    {
        thread_local! {
            static LAST: RefCell<Option<(&'static str, (i32, i32))>> = const {
                RefCell::new(None)
            };
        }
        LAST.with(|cell| {
            let mut last = cell.borrow_mut();
            if last.as_ref() != Some(&(_kind, _pos)) {
                eprintln!("ime-indicator: anchor={_kind} ({}, {})", _pos.0, _pos.1);
                *last = Some((_kind, _pos));
            }
        });
    }
}

/// UI Automation でフォーカス中の要素のキャレット位置を取得する。
/// 読み取り専用 API なので相手アプリの入力状態には影響しない。
fn caret_via_uia() -> Option<POINT> {
    UIA.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = unsafe {
                CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                    .ok()
            };
        }
        let auto = slot.as_ref()?;

        unsafe {
            let focused = auto.GetFocusedElement().ok()?;
            let pattern_unknown = focused.GetCurrentPattern(UIA_TextPatternId).ok()?;
            let text_pattern: IUIAutomationTextPattern = pattern_unknown.cast().ok()?;

            let selection = text_pattern.GetSelection().ok()?;
            if selection.Length().ok()? == 0 {
                return None;
            }
            let range = selection.GetElement(0).ok()?;

            let safearray_ptr = range.GetBoundingRectangles().ok()?;
            if safearray_ptr.is_null() {
                return None;
            }

            let lbound = SafeArrayGetLBound(safearray_ptr, 1).ok()?;
            let ubound = SafeArrayGetUBound(safearray_ptr, 1).ok()?;
            let count = (ubound - lbound + 1) as usize;
            if count < 4 {
                return None;
            }

            let mut data: *mut c_void = std::ptr::null_mut();
            SafeArrayAccessData(safearray_ptr, &mut data).ok()?;
            let arr = std::slice::from_raw_parts(data as *const f64, count);
            let x = arr[0];
            let y = arr[1];
            let h = arr[3];
            let _ = SafeArrayUnaccessData(safearray_ptr);

            // キャレット直下に出すため、矩形の左下を返す。
            Some(POINT {
                x: x as i32,
                y: (y + h) as i32,
            })
        }
    })
}

fn caret_via_guithreadinfo() -> Option<POINT> {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return None;
        }
        let tid = GetWindowThreadProcessId(fg, None);
        if tid == 0 {
            return None;
        }

        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        // GetGUIThreadInfo はクロススレッドでも安全に読める（読み取り API）。
        GetGUIThreadInfo(tid, &mut info).ok()?;

        let r = info.rcCaret;
        if r.left == 0 && r.right == 0 && r.top == 0 && r.bottom == 0 {
            return None;
        }

        // hwndCaret が無いと ClientToScreen で変換できない。AttachThreadInput を
        // 使えば GetFocus で代替できるが、ここは諦めて諦観する（副作用回避を優先）。
        let owner = info.hwndCaret;
        if owner.0.is_null() {
            return None;
        }

        let mut pt = POINT {
            x: r.left,
            y: r.bottom,
        };
        let _ = ClientToScreen(owner, &mut pt);
        Some(pt)
    }
}

/// フォアグラウンドのトップレベルウィンドウの左下寄りに出す簡易フォールバック。
/// `AttachThreadInput + GetFocus` で焦点子ウィンドウを取れば精度が上がるが、
/// 入力を壊す副作用を避けるためトップレベルだけで妥協する。
fn foreground_window_anchor() -> Option<(i32, i32)> {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return None;
        }
        let mut rect = RECT::default();
        GetWindowRect(fg, &mut rect).ok()?;
        // ウィンドウの左下寄り（編集領域の最終行近くを想定）。
        let x = rect.left + 32;
        let y = rect.bottom - 64;
        Some((x, y))
    }
}

fn cursor_pos_with_offset() -> (i32, i32) {
    unsafe {
        let mut pt = POINT::default();
        if GetCursorPos(&mut pt).is_ok() {
            (pt.x + 16, pt.y + 16)
        } else {
            (100, 100)
        }
    }
}
