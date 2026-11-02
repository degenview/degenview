; Inno Setup script for the per-user Windows installer. Built by tools/windows-installer.sh, which
; passes the defines below. docs/agent-guides/releases-updates.md ("Windows updates") says why it is
; per-user and what the installer leaves alone.

#ifndef AppVersion
  #error Pass /DAppVersion=<Cargo version>, for example 0.6.0-rc.12
#endif
#ifndef Tag
  #error Pass /DTag=<release tag>, for example v0.6.0-rc.12
#endif
#ifndef ExePath
  #error Pass /DExePath=<path to degenview.exe>
#endif
; The CPU the exe in ExePath is built for. One script makes both installers; they share the AppId below,
; so installing the arm64 one over an x64 install is an in-place move to the native build.
#ifndef Arch
  #define Arch "x64"
#endif
#if Arch == "arm64"
  #define ArchAllowed "arm64"
#elif Arch == "x64"
  ; x64os, not x64compatible: that would also admit Windows on ARM, which would then install the
  ; emulated build. Refusing steers those users to the native installer.
  #define ArchAllowed "x64os"
#else
  #error Arch must be x64 or arm64
#endif

[Setup]
; The installer's identity, and the key under Settings > Installed apps. NEVER change it: a new id
; makes the next installer install beside the old one instead of over it. The app writes the same
; id (`INSTALLER_APP_ID` in src/service/self_update/installed_version.rs, kept in step by a test) to keep
; the displayed version current after a self-update.
AppId={{79717C79-2D1E-4206-B25A-F3E626438640}
AppName=DegenView
AppVersion={#AppVersion}
AppPublisher=Nico Oelgart
AppPublisherURL=https://github.com/degenview/degenview
AppSupportURL=https://github.com/degenview/degenview/issues
; Per-user only, on purpose. With no admin rights {autopf} is %LOCALAPPDATA%\Programs, a folder the
; app can write to, which is what the self-updater needs; an all-users install under Program Files
; could not update itself. So there is no PrivilegesRequiredOverridesAllowed.
PrivilegesRequired=lowest
DefaultDirName={autopf}\DegenView
DisableProgramGroupPage=yes
ArchitecturesAllowed={#ArchAllowed}
ArchitecturesInstallIn64BitMode={#ArchAllowed}
LicenseFile=..\..\LICENSE.md
SetupIconFile=..\..\assets\brand\DegenView.ico
UninstallDisplayIcon={app}\degenview.exe
WizardStyle=modern
Compression=lzma2
SolidCompression=yes
; Ask a running DegenView to close before its exe is replaced; nothing needs reopening afterwards.
CloseApplications=yes
RestartApplications=no
OutputDir=..\..\dist
OutputBaseFilename=DegenView-{#Tag}-windows-{#Arch}-setup

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
; The exe carries no version resource, so Inno cannot compare versions: install what this setup holds.
Source: "{#ExePath}"; DestDir: "{app}"; DestName: "degenview.exe"; Flags: ignoreversion
; The fonts built into the app are under the SIL OFL, which wants its text to come with them.
Source: "..\..\assets\fonts\OFL-JetBrainsMono.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "..\..\assets\fonts\OFL-NotoSansSymbols2.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "..\..\assets\fonts\README.md"; DestDir: "{app}\licenses"; DestName: "FONTS.md"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\DegenView"; Filename: "{app}\degenview.exe"
Name: "{autodesktop}\DegenView"; Filename: "{app}\degenview.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\degenview.exe"; Description: "{cm:LaunchProgram,DegenView}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; What the self-updater leaves next to the exe. The user's data (%APPDATA%\es.nico.degenview) and
; cache are not the installer's and are kept.
Type: filesandordirs; Name: "{app}\.DegenView-update"
