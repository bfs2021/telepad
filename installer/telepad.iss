; Telepad installer (Inno Setup 6). Built by scripts/package-windows.ps1 -Installer:
;   iscc /DAppVersion=0.5.0 /DSourceDir=<staging folder> /DOutputDir=dist installer\telepad.iss
; Per-user install (no admin rights): %LOCALAPPDATA%\Programs\Telepad, Start-menu
; shortcut, uninstaller. Data lives in %APPDATA%\Telepad (telepad.db, generated
; ssh config) and is kept on uninstall. The in-app updater replaces the two exes
; in the install folder, which the user can write to.

#ifndef AppVersion
  #error AppVersion is required (/DAppVersion=x.y.z)
#endif
#ifndef SourceDir
  #error SourceDir is required (/DSourceDir=...)
#endif
#ifndef OutputDir
  #define OutputDir "..\dist"
#endif

[Setup]
AppId={{6F4E2B7A-3C1D-4E8F-9A2B-5D7C1E0F3A64}
AppName=Telepad
AppVersion={#AppVersion}
AppVerName=Telepad {#AppVersion}
AppPublisher=yohannaftali
AppPublisherURL=https://github.com/yohannaftali/telepad
AppSupportURL=https://github.com/yohannaftali/telepad/issues
AppUpdatesURL=https://github.com/yohannaftali/telepad/releases
DefaultDirName={autopf}\Telepad
DefaultGroupName=Telepad
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=telepad-{#AppVersion}-windows-x86_64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\telepad.exe
SetupIconFile=..\crates\telepad-gui\assets\icon\telepad.ico
UninstallDisplayName=Telepad
LicenseFile={#SourceDir}\licenses\LICENSE-MIT
; The running app can be updated in place; ask to close it when (un)installing.
CloseApplications=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\telepad.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\telepad-check.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\README.txt"; DestDir: "{app}"; Flags: ignoreversion isreadme
Source: "{#SourceDir}\config.example"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion

[Icons]
Name: "{group}\Telepad"; Filename: "{app}\telepad.exe"
Name: "{group}\Uninstall Telepad"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Telepad"; Filename: "{app}\telepad.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\telepad.exe"; Description: "{cm:LaunchProgram,Telepad}"; Flags: nowait postinstall skipifsilent
