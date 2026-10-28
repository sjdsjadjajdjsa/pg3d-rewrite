; Windows installer (Inno Setup 6). Installs for the current user only, so it needs no
; admin rights. Built by .github/workflows/release.yml:
;
;   iscc /DVersion=0.1.0 /DSource=C:\full\path\to\dist\pg3d-rewrite packaging\windows\pg3d-rewrite.iss
;
; The import writes the game's files into {app}\exporter\private (and its Python packages
; into {app}\exporter\tools); uninstalling removes them too.

#ifndef Version
  #define Version "0.0.0"
#endif
#ifndef Source
  #define Source "..\..\dist\pg3d-rewrite"
#endif

[Setup]
AppId={{6F2C9A41-8E3B-4D57-B1A0-9C4E7D2F5B18}
AppName=pg3d-rewrite
AppVersion={#Version}
AppVerName=pg3d-rewrite {#Version}
AppPublisher=cj
AppCopyright=cj, MIT
VersionInfoDescription=pg3d-rewrite installer
AppSupportURL=https://github.com/sjdsjadjajdjsa/pg3d-rewrite
AppUpdatesURL=https://github.com/sjdsjadjajdjsa/pg3d-rewrite/releases
DefaultDirName={userpf}\pg3d-rewrite
DisableProgramGroupPage=yes
DisableDirPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
LicenseFile={#Source}\LICENSE
UninstallDisplayIcon={app}\pg3d-rewrite.exe
UninstallDisplayName=pg3d-rewrite
OutputDir=..\..
OutputBaseFilename=pg3d-rewrite-setup-x86_64
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=force
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked

[Files]
Source: "{#Source}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[UninstallDelete]
Type: filesandordirs; Name: "{app}\exporter\private"
Type: filesandordirs; Name: "{app}\exporter\tools"

[Icons]
Name: "{userprograms}\pg3d-rewrite"; Filename: "{app}\pg3d-rewrite.exe"; WorkingDir: "{app}"; Comment: "Pixel Gun 3D rewrite"
Name: "{userdesktop}\pg3d-rewrite"; Filename: "{app}\pg3d-rewrite.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\pg3d-rewrite.exe"; Description: "Play now"; WorkingDir: "{app}"; Flags: nowait postinstall skipifsilent
