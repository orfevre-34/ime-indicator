---
description: リリースビルドを作成し、バイナリサイズを目標 (< 1MB) と比較して報告する
---

リリースビルドを作成して、バイナリサイズの計測結果を報告してください。

手順:

1. `cargo build --release` を実行（失敗したらエラーを要約して停止）
2. `target/release/ime-indicator.exe` のサイズを取得（PowerShell 経由でも `ls -l` 経由でも可）
3. 以下のフォーマットで結果を出力:
   - `<size>` KB / 目標 1024 KB（達成 / 超過 X KB）
   - 直前の計測値があれば差分
4. 超過していたら、`Cargo.toml` の `[profile.release]` 設定 (opt-level/lto/codegen-units/strip/panic) と依存 crate の `features` 過剰指定がないかを点検する

依存追加によるサイズ膨張を早期に発見するための日常コマンドとして使う想定。
