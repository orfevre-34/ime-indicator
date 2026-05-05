use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    GUITHREADINFO, GetCursorPos, GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId,
};

/// インジケータを表示すべきスクリーン座標（左上）を返す。
///
/// キャレットが取れたらキャレット直下に置く。取れない場合（Chrome / Electron 等）は
/// マウスカーソル位置にオフセットして置く。
pub fn indicator_anchor() -> (i32, i32) {
    if let Some(p) = caret_screen_pos() {
        // キャレットの少し下にずらして、文字に重ならないようにする。
        return (p.x + 4, p.y + 4);
    }
    cursor_pos_with_offset()
}

fn caret_screen_pos() -> Option<POINT> {
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

        // rcCaret が空 (left==right==top==bottom==0) のときは取れていない扱い。
        let r = info.rcCaret;
        if r.left == 0 && r.right == 0 && r.top == 0 && r.bottom == 0 {
            return None;
        }

        // 取れているケースでも hwndCaret が無い場合は GetFocus を試す。
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

fn cursor_pos_with_offset() -> (i32, i32) {
    unsafe {
        let mut pt = POINT::default();
        if GetCursorPos(&mut pt).is_ok() {
            // カーソルにアイコンが被らないよう、右下にずらす。
            (pt.x + 16, pt.y + 16)
        } else {
            (100, 100)
        }
    }
}
