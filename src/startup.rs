// Windows ログオン時自動起動の登録 / 解除。
//
// HKCU\Software\Microsoft\Windows\CurrentVersion\Run\IMEIndicator に
// 現在の exe へのフルパスを REG_SZ で書く / 消すだけのシンプルな実装。
// ユーザー権限のキーなので UAC 昇格不要。

use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::{PCWSTR, w};

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const VALUE_NAME: PCWSTR = w!("IMEIndicator");

/// 自動起動が登録されているか。
pub fn is_enabled() -> bool {
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, KEY_READ, &mut hkey)
            .ok()
            .is_err()
        {
            return false;
        }
        let mut len: u32 = 0;
        let r = RegQueryValueExW(hkey, VALUE_NAME, None, None, None, Some(&mut len)).ok();
        let _ = RegCloseKey(hkey);
        r.is_ok() && len > 0
    }
}

/// 自動起動の登録 (true) または解除 (false)。
pub fn set_enabled(enabled: bool) -> windows::core::Result<()> {
    unsafe {
        let mut hkey = HKEY::default();
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WRITE,
            None,
            &mut hkey,
            None,
        )
        .ok()?;

        let result = if enabled {
            match std::env::current_exe() {
                Ok(exe) => {
                    // ダブルクォートで囲んでおくとパスにスペースが入っていても安全。
                    let quoted = format!("\"{}\"", exe.display());
                    let utf16: Vec<u16> = quoted.encode_utf16().chain(std::iter::once(0)).collect();
                    let bytes = std::slice::from_raw_parts(
                        utf16.as_ptr() as *const u8,
                        std::mem::size_of_val(&*utf16),
                    );
                    RegSetValueExW(hkey, VALUE_NAME, None, REG_SZ, Some(bytes)).ok()
                }
                Err(_) => Err(windows::core::Error::from_thread()),
            }
        } else {
            let r = RegDeleteValueW(hkey, VALUE_NAME);
            if r == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                r.ok()
            }
        };

        let _ = RegCloseKey(hkey);
        result
    }
}
