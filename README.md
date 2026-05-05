# IME Indicator

Mac の「a / あ」風オーバーレイを Windows でも表示する常駐ツール。
IME 切替系のキーを押すと、現在の入力モードをキャレット付近に半透明のインジケータでフェード表示します。

<p align="center">
  <img src="assets/icon_preview.png" width="128" alt="アプリアイコン">
</p>

- **トリガーキー連動表示**: 半角/全角・lang0/lang1・カナ/英数・`VK_KANJI` 等の IME 切替系キーを押すたびに表示。モードが実際に変わらない押下（既に Hiragana で再度 lang1 等）でも必ず表示する。普通の文字キーや矢印キーでは何も起きない
- **キャレット直下に表示**: UI Automation → `GUITHREADINFO.rcCaret` → MSAA `OBJID_CARET` → フォーカス子ウィンドウ位置 の優先順位で実際のテキストキャレット位置を取得
- **per-pixel alpha**: Direct2D + `UpdateLayeredWindow(ULW_ALPHA)` でアンチエイリアス込みの綺麗な角丸表示
- **クリックスルー**: ウィンドウ操作には一切干渉しない
- **トレイアイコンから操作**: タスクトレイから「Windows ログオン時に自動起動」のトグルと「終了」が呼べる
- **軽量**: リリースバイナリ約 175 KB（目標 < 1 MB）

## 動作要件

- Windows 10 22H2 / Windows 11
- Rust 1.85+ (edition 2024 対応) — 開発時のみ

## インストール（エンドユーザー向け）

[インストーラのビルド手順](installer/README.md) で生成した `IMEIndicator-Setup.exe` をダブルクリック。
インストール時に「Windows ログオン時に自動起動する」のチェックを付けると登録される（あとでトレイメニューからも切替可）。

## トレイメニュー

タスクバーの通知領域にある「あ」アイコンをクリックすると下記メニューが出る:

- **Windows ログオン時に自動起動** — トグル。`HKCU\Software\Microsoft\Windows\CurrentVersion\Run\IMEIndicator` を書き換え（管理者権限不要）
- **終了**

## 開発

```sh
cargo run                  # 開発実行（コンソールに診断ログ出力）
cargo build --release      # リリースビルド（windows サブシステム）
cargo clippy --all-targets
cargo fmt

python tools/gen_icon.py   # アイコン .ico を再生成
```

リリースバイナリのサイズ確認は `/release-check` カスタムコマンド。
インストーラ生成は [installer/README.md](installer/README.md) 参照。

## アイコン表示の意味

| 表示 | 意味 |
|---|---|
| `A` | IME OFF（直接入力） |
| `あ` | IME ON / ひらがな（NATIVE モード） |
| `カ` | IME ON / かな以外（カタカナ・全角英数など） |

## 設計

詳細は [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)、AI 開発エージェント向けの方針は [CLAUDE.md](CLAUDE.md) を参照。

モジュール構成:

- [src/ime.rs](src/ime.rs): `ImmGetOpenStatus` / `ImmGetConversionStatus` ラッパー（IMM アプリ向け保険）
- [src/caret.rs](src/caret.rs): UI Automation / GUITHREADINFO / MSAA / フォーカス子ウィンドウの優先順位フォールバック
- [src/overlay.rs](src/overlay.rs): レイヤードウィンドウ + 32bpp DIB + ID2D1DCRenderTarget
- [src/app.rs](src/app.rs): 表示寿命の状態機械（Hidden / FadeIn / Visible / FadeOut）
- [src/startup.rs](src/startup.rs): `HKCU\…\Run` の自動起動レジストリ操作
- [src/main.rs](src/main.rs): DPI 認識・COM 初期化・ウィンドウ + トレイアイコン + メニュー・`WH_KEYBOARD_LL` フック・`WM_TIMER` ループ

## 既知の制約

- IMM API でフォアグラウンドの IME 状態を直接読めるのは古典的な IMM アプリだけ。モダン TSF アプリ (Chrome / Edge / VS Code 等) では低レベルキーボードフックでトリガーキー押下を直接検出する
- Chrome / Electron / 一部 UWP は UI Automation でキャレット矩形を出さないことがあり、その場合は MSAA → フォーカス子ウィンドウ位置にフォールバック
- UAC 昇格ウィンドウは別権限プロセスからのフックが効かないので、その上で IME を切り替えても拾えない
- マルチモニタで DPI が違う場合、起動時のシステム DPI で固定（移動時の再構築は未対応）

## ライセンス

MIT
