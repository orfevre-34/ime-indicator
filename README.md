# IME Indicator

Mac の「a / あ」風オーバーレイを Windows でも表示する常駐ツール。
IME の状態が切り替わったとき、キャレット付近に半透明のインジケータをフェード表示します。

> 軽量（バイナリ < 1MB / メモリ < 15MB 目標）かつおしゃれな見た目を目指しています。
> 開発状況: スケルトン段階。実装はこれから。

## 開発

```sh
cargo run                  # 開発実行
cargo build --release      # リリースビルド
cargo clippy --all-targets
cargo fmt
```

設計の詳細は [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)、AI 開発エージェント向けの方針は [CLAUDE.md](CLAUDE.md) を参照。

## 動作要件

- Windows 10 22H2 / Windows 11
- Rust 1.85+ (edition 2024 対応)

## ライセンス

MIT
