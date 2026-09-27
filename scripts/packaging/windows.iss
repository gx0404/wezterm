; GX installer. Its identity and payload are independent of the upstream release.
[Setup]
AppId={{734DC47D-4799-46A6-A286-4A64B802370A}
AppName=WezTerm GX
AppVersion={#PackageVersion}
AppPublisher=gx0404
AppPublisherURL=https://github.com/gx0404/wezterm
DefaultDirName={autopf}\WezTerm GX
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
DisableProgramGroupPage=yes
OutputBaseFilename=WezTerm-GX-{#PackageVersion}-Setup-x64
SetupIconFile={#RepoDir}\assets\windows\terminal.ico
UninstallDisplayIcon={app}\wezterm-gui.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked

[Files]
Source: "{#StageDir}\app\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
#include StageDir + "\fonts.iss"

[Icons]
Name: "{autoprograms}\WezTerm GX"; Filename: "{app}\wezterm-gx.exe"; IconFilename: "{app}\wezterm-gui.exe"
Name: "{autodesktop}\WezTerm GX"; Filename: "{app}\wezterm-gx.exe"; IconFilename: "{app}\wezterm-gui.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\wezterm-gx.exe"; Description: "{cm:LaunchProgram,WezTerm GX}"; Flags: nowait postinstall skipifsilent runasoriginaluser

[Registry]
Root: HKA; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\wezterm-gx.exe"; ValueType: string; ValueData: "{app}\wezterm-gx.exe"; Flags: uninsdeletekey

[Code]
procedure MigrateLegacyShortcut();
var
  LinkPath, TargetPath, LegacyRoot: String;
  Shell, Shortcut: Variant;
begin
  { In all-users mode the elevated identity need not be the interactive user. }
  if IsAdminInstallMode then exit;
  LinkPath := ExpandConstant('{userprograms}\WezTerm (gx).lnk');
  if not FileExists(LinkPath) then exit;
  try
    Shell := CreateOleObject('WScript.Shell');
    Shortcut := Shell.CreateShortcut(LinkPath);
    TargetPath := Shortcut.TargetPath;
    TargetPath := Lowercase(TargetPath);
    LegacyRoot := Lowercase(ExpandConstant('{localappdata}\Programs\wezterm-gx\'));
    if (Pos(LegacyRoot, TargetPath) = 1) and
       (Lowercase(ExtractFileName(TargetPath)) = 'wezterm-gui.exe') then begin
      Shortcut.TargetPath := ExpandConstant('{app}\wezterm-gx.exe');
      Shortcut.IconLocation := ExpandConstant('{app}\wezterm-gui.exe') + ',0';
      Shortcut.Save();
    end;
  except
    Log('Legacy shortcut was not changed: ' + GetExceptionMessage);
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then MigrateLegacyShortcut();
end;
