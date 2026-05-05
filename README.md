# IME Indicator

Mac の「a / あ」風オーバーレイを Windows でも表示する常駐ツール。
IME の状態が切り替わったとき、キャレット付近に半透明のインジケータをフェード表示します。

- **トリガーキー連動表示**: 半角/全角・lang0/lang1・カナ/英数・VK_KANJI 等の IME 切替系キーを押すたびに表示が出て、押されない期間が 1.5 秒経つとフェードアウト。モードが実際に変わらないキー押下（例: 既に Hiragana で再度 lang1）でも必ず表示する。普通の文字キーや矢印キーでは何も起きない
- **クリックスルー**: ウィンドウ操作には一切干渉しない
- **キャレット追従**: `GUITHREADINFO.rcCaret` 取得、取れない場合はマウスカーソルへフォールバック
- **per-pixel alpha**: Direct2D 描画 + WIC bitmap + `UpdateLayeredWindow(ULW_ALPHA)` でアンチエイリアス含めて綺麗に合成
- **軽量**: リリースバイナリ 約 130 KB（目標 < 1 MB）

## 動作要件

- Windows 10 22H2 / Windows 11
- Rust 1.85+ (edition 2024 対応)

## 開発

```sh
cargo run                  # 開発実行（コンソール出力あり）
cargo build --release      # リリースビルド（windows サブシステム）
cargo clippy --all-targets
cargo fmt
```

リリースバイナリのサイズ確認は `/release-check` カスタムコマンドを使用。

## アイコン表示の意味

| 表示 | 意味 |
|---|---|
| `A` | IME OFF（直接入力） |
| `あ` | IME ON / ひらがな（NATIVE モード） |
| `カ` | IME ON / かな以外（カタカナ・全角英数など） |

## 設計

詳細は [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)、AI 開発エージェント向けの方針は [CLAUDE.md](CLAUDE.md) を参照。

モジュール構成:

- [src/ime.rs](src/ime.rs): `ImmGetOpenStatus` / `ImmGetConversionStatus` の薄いラッパー
- [src/caret.rs](src/caret.rs): キャレット位置取得 + マウスカーソルへのフォールバック
- [src/overlay.rs](src/overlay.rs): レイヤードウィンドウ + 32bpp DIB + WIC + Direct2D 描画
- [src/app.rs](src/app.rs): フェードの状態機械（Hidden / FadeIn / Visible / FadeOut）
- [src/main.rs](src/main.rs): DPI 認識・COM 初期化・ウィンドウクラス登録・`WM_TIMER` ベースのループ

## 既知の制約

- Chrome / Electron / 一部 UWP では `rcCaret` が空。マウスカーソル位置にフォールバック
- UAC 昇格ウィンドウは IME 状態を読めない（同等以上の権限が必要）
- マルチモニタで DPI が違う場合、起動時のシステム DPI で固定（インジケータが移動した瞬間のリビルドは未対応）

## ライセンス

MIT
