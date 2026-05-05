# IME Indicator

Macライクな「a/あ」オーバーレイをWindowsで再現する常駐ツール。IMEの状態（ON/OFF・変換モード）が切り替わったとき、キャレット付近に半透明のインジケータをフェード表示する。

## 設計目標

- **軽量**: リリースバイナリ < 1MB、常駐メモリ < 15MB
- **おしゃれ**: 角丸 + ドロップシャドウ + フェードイン/アウト
- **邪魔しない**: クリックスルー、モード切替時のみ表示して 1.5s で消える

## 技術スタック

- Rust（edition 2024）+ `windows` crate（Win32 API バインディング）
- 描画は **Direct2D + DirectWrite**（GDI ではアンチエイリアス品質が出ない）
- 設定は TOML（GUIは作らない）

## ビルド・実行

```sh
cargo run                  # 開発実行
cargo build --release      # リリース
cargo clippy --all-targets # 静的解析
cargo fmt                  # 整形
```

リリースバイナリのサイズ確認は `/release-check` カスタムコマンドを使う。

## アーキテクチャ要約

詳細は [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) 参照。

- `ime`: `ImmGetOpenStatus` / `ImmGetConversionStatus` を 100ms 間隔でポーリング
- `caret`: `GUITHREADINFO.rcCaret` を取得 → 取れない場合は `GetCursorPos` にフォールバック
- `overlay`: `WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_TOPMOST` のレイヤードウィンドウに Direct2D 描画
- `app`: 状態機械。モード変化検出 → フェードイン → 1.5s 後フェードアウト

## 既知の制約（実装前から覚えておく）

- **キャレットが取れないアプリ**: Chrome / Electron / 一部 UWP では `rcCaret` が空。マウスカーソルにフォールバック
- **UAC 昇格ウィンドウ**: 同等以上の権限がないと IME 状態を読めない（割り切る）
- **マルチモニタ高 DPI**: モニタ毎の DPI を考慮した座標変換が必要

## スコープ外（やらない）

- 設定変更 GUI（TOML を直接編集してもらう）
- 自動アップデート、テレメトリ、クラッシュレポーター
- IME 切替操作そのもの（このツールは「監視と表示」のみ）
- 多言語サポート（日本語 IME 前提）

## コードスタイル

- `unsafe` ブロックは最小化し、安全な薄いラッパーに包む
- 抽象化は 3 回以上の重複が出てから。仮説的な将来要件で一般化しない
- 依存追加は慎重に。バイナリサイズ目標 (< 1MB) を毎回意識する
- コメントは「なぜ」のみ書く。「なに」は識別子で表現する
