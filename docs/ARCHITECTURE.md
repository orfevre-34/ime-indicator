# Architecture

## モジュール構成（計画）

```
src/
├── main.rs       # エントリポイント、メッセージループ
├── app.rs        # 状態機械（IME 状態変化 → 描画指示）
├── ime.rs        # IME 状態取得（ポーリング）
├── caret.rs      # キャレット位置取得 + フォールバック
├── overlay.rs    # レイヤードウィンドウ + Direct2D 描画
└── config.rs     # TOML 設定の読み込み
```

`config.rs` は MVP では不要。最初は定数で運用してから外出しする。

## イベント駆動

メインスレッドはウィンドウメッセージループ。別スレッドが 100ms 毎に IME 状態をポーリングし、変化があったら `PostMessage` でメインスレッドに通知。

```
Polling thread ──[mode change]──> WM_APP_IME_CHANGED ──> App state machine ──> Overlay redraw
```

メッセージ名（暫定）:
- `WM_APP_IME_CHANGED` (WM_APP + 0x01)
- `WM_APP_FADE_TICK`   (WM_APP + 0x02)

## 重要な Win32 API

### IME 状態取得

```rust
use windows::Win32::UI::Input::Ime::{
    ImmGetContext, ImmGetOpenStatus, ImmGetConversionStatus, ImmReleaseContext,
};

unsafe fn read_ime_state(hwnd: HWND) -> Option<ImeMode> {
    let himc = ImmGetContext(hwnd);
    if himc.is_invalid() { return None; }

    let open = ImmGetOpenStatus(himc).as_bool();
    let mut conv = 0u32;
    let mut sent = 0u32;
    ImmGetConversionStatus(himc, Some(&mut conv), Some(&mut sent));
    ImmReleaseContext(hwnd, himc);

    Some(if open { ImeMode::Hiragana } else { ImeMode::Alpha })
}
```

### キャレット位置取得

```rust
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GUITHREADINFO,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetWindowThreadProcessId;

unsafe fn caret_pos() -> Option<(i32, i32)> {
    let hwnd = GetForegroundWindow();
    let mut pid = 0u32;
    let tid = GetWindowThreadProcessId(hwnd, Some(&mut pid));

    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    GetGUIThreadInfo(tid, &mut info).ok()?;

    // ウィンドウ座標 → スクリーン座標変換が必要
    let mut pt = POINT { x: info.rcCaret.left, y: info.rcCaret.bottom };
    ClientToScreen(info.hwndCaret, &mut pt);
    Some((pt.x, pt.y))
}
```

取れなかったら `GetCursorPos` を使う。

### オーバーレイウィンドウ

```rust
let style_ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE;
```

- `WS_EX_LAYERED`: アルファ合成
- `WS_EX_TRANSPARENT`: クリックスルー
- `WS_EX_TOOLWINDOW`: タスクバーに出さない
- `WS_EX_TOPMOST`: 最前面
- `WS_EX_NOACTIVATE`: フォーカスを奪わない

描画は `UpdateLayeredWindow` 経由。Direct2D で `ID2D1HwndRenderTarget` を作るパターンと、メモリ DC に Direct2D で描画して `UpdateLayeredWindow` する 2 通りがあるが、後者の方が WS_EX_LAYERED + per-pixel alpha と相性が良い。

## 描画イメージ（おしゃれ要素）

- 横 56px × 縦 36px 程度の角丸（半径 10px）
- 背景: rgba(28, 28, 30, 0.85)（ダークグレー半透明）
- 文字: SF Pro 風 → Segoe UI Variable Display Semibold + Noto Sans JP Bold
- ドロップシャドウ: blur 12px、offset (0, 4)、rgba(0,0,0,0.25)
- フェード: 120ms ease-out（in） / 200ms ease-in（out）
- 表示時間: 1.5s（フェード除く）

## ハマりどころ・対策メモ

| 問題 | 対策 |
|---|---|
| Chrome/Electron で `rcCaret` 空 | `GetCursorPos` フォールバック |
| UAC 昇格プロセスの IME 読めない | ドキュメント明記、UI上の警告は出さない |
| 高 DPI 環境で座標ズレ | `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)` |
| ポーリング 100ms の CPU 負荷 | プロセスは Idle 優先度で OK。実測 < 0.1% |
| 変換モード切替（カタカナ等）の検知漏れ | キーフックではなくポーリングで `fdwConversion` も比較 |

## 将来拡張（参考）

- TOML 設定（色・サイズ・表示時間・常時表示モード）
- スタートアップ登録（タスクスケジューラ経由）
- インジケータの位置オフセット調整
