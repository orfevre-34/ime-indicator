use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::Ime::{
    IME_CMODE_NATIVE, IME_CONVERSION_MODE, IME_SENTENCE_MODE, ImmGetContext,
    ImmGetConversionStatus, ImmGetOpenStatus, ImmReleaseContext,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeMode {
    /// IME OFF: 直接入力（Mac の "A" 相当）
    Alpha,
    /// IME ON かつ かな変換モード（Mac の "あ" 相当）
    Hiragana,
    /// IME ON かつ かな以外の変換モード（カタカナ・全角英数など）
    Other,
}

/// フォアグラウンドアプリの IME 状態を読む。
///
/// モダン Windows のセキュリティ制限により、別プロセスの HWND に対して
/// `ImmGetContext` を素直に呼んでも `is_invalid()` で失敗する。これを回避するため、
/// `AttachThreadInput` で相手スレッドに一時的に入力キューをアタッチしてから読む。
///
/// `AttachThreadInput` には副作用があるが、フォーカス/キャプチャ/アクティブウィンドウは
/// 共有しても、こちらは `WS_EX_NOACTIVATE` のオーバーレイなのでフォアグラウンドを
/// 奪うことは無い。100ms ポーリングの瞬間だけアタッチ → デタッチするので影響は最小。
pub fn read_current_mode() -> Option<ImeMode> {
    unsafe {
        let fg: HWND = GetForegroundWindow();
        if fg.0.is_null() {
            return None;
        }

        let fg_tid = GetWindowThreadProcessId(fg, None);
        if fg_tid == 0 {
            return None;
        }
        let my_tid = GetCurrentThreadId();

        // 別スレッドのときだけアタッチ。同スレッド (=自プロセスがフォアグラウンド) の
        // ときは普通に取れる。
        let attached = if fg_tid != my_tid {
            AttachThreadInput(my_tid, fg_tid, true).as_bool()
        } else {
            false
        };

        let result = read_via_himc(fg);

        if attached {
            let _ = AttachThreadInput(my_tid, fg_tid, false);
        }

        result
    }
}

unsafe fn read_via_himc(hwnd: HWND) -> Option<ImeMode> {
    unsafe {
        let himc = ImmGetContext(hwnd);
        if himc.is_invalid() {
            return None;
        }

        let open = ImmGetOpenStatus(himc).as_bool();
        let mut conv = IME_CONVERSION_MODE::default();
        let mut sent = IME_SENTENCE_MODE::default();
        let _ = ImmGetConversionStatus(himc, Some(&mut conv as *mut _), Some(&mut sent as *mut _));
        let _ = ImmReleaseContext(hwnd, himc);

        Some(if !open {
            ImeMode::Alpha
        } else if (conv.0 & IME_CMODE_NATIVE.0) != 0 {
            ImeMode::Hiragana
        } else {
            ImeMode::Other
        })
    }
}
