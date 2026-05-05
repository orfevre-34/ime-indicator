# Installer

Windows 用インストーラ（exe 形式）を [Inno Setup 6](https://jrsoftware.org/isdl.php) で生成する。

## 必要なもの

- Inno Setup 6 (`ISCC.exe`)。`winget install JRSoftware.InnoSetup` または公式サイトからインストール。
- リリースビルド済みの `target\release\ime-indicator.exe`（`cargo build --release` で作成）。

## ビルド手順

PowerShell から一発で:

```powershell
pwsh .\installer\build.ps1
```

これが `cargo build --release` → アイコン生成チェック → ISCC.exe 自動検出 →
インストーラコンパイル までやってくれる。

手動でやるなら:

```sh
cargo build --release
python tools/gen_icon.py            # 初回のみ
"%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe" installer\setup.iss
```

`winget install JRSoftware.InnoSetup` で入れた場合、ISCC.exe は通常
`%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe` にある。
システム全体にインストールされている場合は
`C:\Program Files (x86)\Inno Setup 6\ISCC.exe`。

成果物: `installer\out\IMEIndicator-Setup.exe`（lzma2 圧縮で約 2 MB）

## インストーラの挙動

- インストール先: 既定で `C:\Program Files\IMEIndicator\`（管理者権限）または
  ユーザー指定でユーザーフォルダにも置ける（`PrivilegesRequiredOverridesAllowed=dialog`）。
- ショートカット: スタートメニューに「IME Indicator」を追加。
- **自動起動**: インストーラのオプションでチェックすると `HKCU\…\Run` に登録。
  あとでトレイメニューからも切り替え可能。
- アンインストール時はプロセスを kill してからファイル削除 + Run キーから削除。

## アンインストール

通常のアプリと同様に「アプリと機能」から削除できる。
