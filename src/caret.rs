use std::cell::RefCell;
use std::ffi::c_void;
use std::mem::ManuallyDrop;

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Ole::{
    SafeArrayAccessData, SafeArrayGetLBound, SafeArrayGetUBound, SafeArrayUnaccessData,
};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::System::Variant::{VARENUM, VARIANT, VARIANT_0, VARIANT_0_0, VT_I4};
use windows::Win32::UI::Accessibility::{
    AccessibleObjectFromWindow, CUIAutomation, IAccessible, IUIAutomation,
    IUIAutomationTextPattern, UIA_TextPatternId,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    GUITHREADINFO, GetCursorPos, GetForegroundWindow, GetGUIThreadInfo, GetWindowRect,
    GetWindowThreadProcessId, OBJID_CARET,
};
use windows::core::Interface;

thread_local! {
    /// IUIAutomation はシングルトン的に使い回す（CoCreateInstance のコストを毎回払わない）。
    static UIA: RefCell<Option<IUIAutomation>> = const { RefCell::new(None) };
}

/// インジケータを表示すべきスクリーン座標（左上）を返す。
///
/// 優先順位:
/// 1. UI Automation で取得したテキストキャレット位置（Chrome/Edge/VS Code 等の TSF アプリ向け）
/// 2. `GUITHREADINFO.rcCaret`（古典的な IMM アプリ向け）
/// 3. フォアグラウンドウィンドウの中央付近（マウス位置にはフォールバックしない）
/// 4. それも取れない時のみ画面の合理的な位置にフォールバック
pub fn indicator_anchor() -> (i32, i32) {
    if let Some(p) = caret_via_uia() {
        #[cfg(debug_assertions)]
        eprintln!("ime-indicator: anchor=UIA ({}, {})", p.x, p.y);
        return (p.x + 4, p.y + 4);
    }
    if let Some(p) = caret_via_guithreadinfo() {
        #[cfg(debug_assertions)]
        eprintln!("ime-indicator: anchor=GUITHREADINFO ({}, {})", p.x, p.y);
        return (p.x + 4, p.y + 4);
    }
    if let Some(p) = caret_via_msaa() {
        #[cfg(debug_assertions)]
        eprintln!("ime-indicator: anchor=MSAA ({}, {})", p.x, p.y);
        return (p.x + 4, p.y + 4);
    }
    if let Some(p) = focused_window_anchor() {
        #[cfg(debug_assertions)]
        eprintln!("ime-indicator: anchor=focused-window ({}, {})", p.0, p.1);
        return p;
    }
    let p = cursor_pos_with_offset();
    #[cfg(debug_assertions)]
    eprintln!("ime-indicator: anchor=cursor ({}, {})", p.0, p.1);
    p
}

/// MSAA (Microsoft Active Accessibility) の `OBJID_CARET` でフォーカス子ウィンドウの
/// システムキャレット矩形を取得する。`CreateCaret` を使う標準コントロール（多くの
/// ネイティブ Win32 / 一部の Electron も含む）で動作する。
fn caret_via_msaa() -> Option<POINT> {
    unsafe {
        let focus = focused_subwindow()?;
        let mut iacc: Option<IAccessible> = None;
        AccessibleObjectFromWindow(
            focus,
            OBJID_CARET.0 as u32,
            &IAccessible::IID,
            &mut iacc as *mut _ as *mut *mut c_void,
        )
        .ok()?;
        let iacc = iacc?;

        let mut x = 0i32;
        let mut y = 0i32;
        let mut w = 0i32;
        let mut h = 0i32;
        // CHILDID_SELF = 0 を VT_I4 で。windows-rs の VARIANT は union なので
        // 手書きで組み立てる。
        let varchild = childid_self_variant();
        iacc.accLocation(&mut x, &mut y, &mut w, &mut h, &varchild)
            .ok()?;
        if w == 0 && h == 0 {
            // キャレットが登録されていない（多くの TSF アプリ）。
            return None;
        }
        Some(POINT { x, y: y + h })
    }
}

fn childid_self_variant() -> VARIANT {
    let inner = ManuallyDrop::new(VARIANT_0_0 {
        vt: VARENUM(VT_I4.0),
        wReserved1: 0,
        wReserved2: 0,
        wReserved3: 0,
        Anonymous: unsafe { std::mem::zeroed() },
    });
    // Anonymous は union; lVal 相当を 0 に。zeroed で OK。
    VARIANT {
        Anonymous: VARIANT_0 { Anonymous: inner },
    }
}

/// フォアグラウンドのスレッドにアタッチして `GetFocus` で「実際にキー入力を受ける
/// 子ウィンドウ」を取り、見つからなければトップレベルウィンドウを返す。
fn focused_subwindow() -> Option<HWND> {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return None;
        }
        let fg_tid = GetWindowThreadProcessId(fg, None);
        if fg_tid == 0 {
            return Some(fg);
        }
        let my_tid = GetCurrentThreadId();
        let attached = if fg_tid != my_tid {
            AttachThreadInput(my_tid, fg_tid, true).as_bool()
        } else {
            false
        };
        let focus = GetFocus();
        if attached {
            let _ = AttachThreadInput(my_tid, fg_tid, false);
        }
        if !focus.0.is_null() {
            Some(focus)
        } else {
            Some(fg)
        }
    }
}

/// UI Automation でフォーカス中の要素のキャレット位置を取得する。TSF 系アプリでも有効。
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

            // フォーカス要素が TextPattern を持たない（純粋なボタン等）なら諦める。
            let pattern_unknown = focused.GetCurrentPattern(UIA_TextPatternId).ok()?;
            let text_pattern: IUIAutomationTextPattern = pattern_unknown.cast().ok()?;

            // 現在の選択範囲（キャレットだけのときは幅 0 の range が 1 つ）。
            let selection = text_pattern.GetSelection().ok()?;
            if selection.Length().ok()? == 0 {
                return None;
            }
            let range = selection.GetElement(0).ok()?;

            // GetBoundingRectangles は SAFEARRAY<f64>: [x0,y0,w0,h0, x1,y1,w1,h1, ...]。
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
        GetGUIThreadInfo(tid, &mut info).ok()?;

        let r = info.rcCaret;
        if r.left == 0 && r.right == 0 && r.top == 0 && r.bottom == 0 {
            return None;
        }

        let mut owner = info.hwndCaret;
        if owner.0.is_null() {
            owner = GetFocus();
            if owner.0.is_null() {
                return None;
            }
        }

        let mut pt = POINT {
            x: r.left,
            y: r.bottom,
        };
        let _ = ClientToScreen(owner, &mut pt);
        Some(pt)
    }
}

/// フォーカス子ウィンドウ（無ければフォアグラウンドのトップレベル）の左下寄りに
/// フォールバック。最大化された VS Code などでも編集領域に近い位置に出る。
/// マウスカーソルの位置よりは「ユーザーの注視点」に近い。
fn focused_window_anchor() -> Option<(i32, i32)> {
    unsafe {
        let target = focused_subwindow()?;
        let mut rect = RECT::default();
        GetWindowRect(target, &mut rect).ok()?;
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
