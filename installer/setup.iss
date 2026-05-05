; IME Indicator - Inno Setup script
;
; ビルド前に `cargo build --release` を済ませて
; target\release\ime-indicator.exe が存在する状態でコンパイルする。
;
; コンパイル:
;   "C:\Program Files (x86)\Inno Setup 6\ISCC.exe" installer\setup.iss
;
; 出力:
;   installer\out\IMEIndicator-Setup.exe

#define AppName       "IME Indicator"
#define AppVersion    "0.1.0"
#define AppPublisher  "tikeg"
#define AppURL        ""
#define ExeName       "ime-indicator.exe"
#define BuildDir      "..\target\release"

[Setup]
AppId={{B6C2F4E8-1F9D-4F27-9B33-3E5C7D2A0E11}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppURL}
WizardStyle=modern
DefaultDirName={autopf}\IMEIndicator
DisableProgramGroupPage=yes
DefaultGroupName={#AppName}
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog commandline
OutputDir=out
OutputBaseFilename=IMEIndicator-Setup
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\{#ExeName}
Compression=lzma2
SolidCompression=yes
ArchitecturesInstallIn64BitMode=x64compatible
ArchitecturesAllowed=x64compatible
MinVersion=10.0.19041
LanguageDetectionMethod=uilanguage
WizardImageStretch=no
DisableWelcomePage=no
DisableDirPage=no
DisableReadyPage=no
DisableFinishedPage=no

[Languages]
Name: "ja"; MessagesFile: "compiler:Languages\Japanese.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "startup"; Description: "{cm:AutoStartTask}"; GroupDescription: "{cm:OptionsGroup}"; Flags: checkedonce

[Files]
Source: "{#BuildDir}\{#ExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion isreadme
Source: "..\docs\ARCHITECTURE.md"; DestDir: "{app}\docs"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#ExeName}"
Name: "{group}\{cm:UninstallProgram,{#AppName}}"; Filename: "{uninstallexe}"

[Registry]
; Windows ログオン時自動起動（HKCU は管理者権限不要）。
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; \
    ValueType: string; ValueName: "IMEIndicator"; \
    ValueData: """{app}\{#ExeName}"""; \
    Flags: uninsdeletevalue; \
    Tasks: startup

[Run]
Filename: "{app}\{#ExeName}"; Description: "{cm:LaunchProgram,{#AppName}}"; \
    Flags: nowait postinstall skipifsilent

[UninstallRun]
; アンインストール時、起動中のプロセスを終了させる。失敗は無視。
Filename: "{cmd}"; Parameters: "/C taskkill /IM {#ExeName} /F"; \
    Flags: runhidden; RunOnceId: "KillIME"

[CustomMessages]
ja.AutoStartTask=Windows ログオン時に自動起動する
ja.OptionsGroup=オプション:
en.AutoStartTask=Start automatically at Windows logon
en.OptionsGroup=Options:
