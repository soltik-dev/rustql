; Windows installer for RustQL (Inno Setup 6). The release workflow builds it
; from the repository root after `cargo build --release`:
;
;   iscc /DAppVersion=0.9.0 packaging\windows\rustql.iss
;
; It installs for the current user by default, with no administrator rights;
; the first page offers installing for everyone instead.

#ifndef AppVersion
  #error Pass the version: iscc /DAppVersion=X.Y.Z
#endif
#define Root SourcePath + "..\.."

[Setup]
; Windows identifies the installed application by this id: never change it
AppId={{B37F01D6-1CCD-4D2E-A405-D657B03A8AE6}
AppName=RustQL
AppVersion={#AppVersion}
AppVerName=RustQL {#AppVersion}
AppPublisher=Alberto López
AppPublisherURL=https://rustql.com
AppSupportURL=https://github.com/soltik-dev/rustql/issues
AppUpdatesURL=https://rustql.com
AppCopyright=Copyright © 2026 Alberto López
VersionInfoVersion={#AppVersion}
DefaultDirName={autopf}\RustQL
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
SetupIconFile={#Root}\assets\icon\icon.ico
UninstallDisplayIcon={app}\rustql.exe
UninstallDisplayName=RustQL
OutputDir={#Root}\dist
OutputBaseFilename=rustql-{#AppVersion}-windows-x86_64-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ShowLanguageDialog=auto

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "fr"; MessagesFile: "compiler:Languages\French.isl"
Name: "it"; MessagesFile: "compiler:Languages\Italian.isl"
Name: "pt"; MessagesFile: "compiler:Languages\Portuguese.isl"
Name: "ptbr"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Root}\target\release\rustql.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "{#Root}\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\RustQL"; Filename: "{app}\rustql.exe"
Name: "{autodesktop}\RustQL"; Filename: "{app}\rustql.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\rustql.exe"; Description: "{cm:LaunchProgram,RustQL}"; Flags: nowait postinstall skipifsilent
