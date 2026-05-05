use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::Ime::{
    IME_CMODE_NATIVE, IME_CONVERSION_MODE, IME_SENTENCE_MODE, ImmGetContext,
    ImmGetConversionStatus, ImmGetOpenStatus, ImmReleaseContext,
};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeMode {
    /// IME OFF: 直接入力（Mac の "A" 相当）
    Alpha,
    /// IME ON かつ かな変換モード（Mac の "あ" 相当）
    Hiragana,
    /// IME ON かつ かな以外の変換モード（カタカナ・全角英数など）
    Other,
}

pub fn read_current_mode() -> Option<ImeMode> {
    // GetForegroundWindow / ImmGetContext は Win32 API 呼び出しで unsafe。
    // 失敗時は None を返す。安全な薄いラッパーに包んでおく。
    unsafe {
        let hwnd: HWND = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }

        let himc = ImmGetContext(hwnd);
        if himc.is_invalid() {
            return None;
        }

        let open = ImmGetOpenStatus(himc).as_bool();
        let mut conv = IME_CONVERSION_MODE::default();
        let mut sent = IME_SENTENCE_MODE::default();
        let _ = ImmGetConversionStatus(
            himc,
            Some(&mut conv as *mut _),
            Some(&mut sent as *mut _),
        );
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
