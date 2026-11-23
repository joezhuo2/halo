#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef Exe
  #define Exe "..\target\release\halo-ide.exe"
#endif

[Setup]
AppId={{DC872D12-77AC-49A3-8C7F-FF6D04CA0E19}
AppName=Halo
AppVersion={#AppVersion}
AppPublisherURL=https://github.com/joezhuo2/halo
DefaultDirName={autopf}\Halo
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=halo-{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
SetupIconFile=..\assets\halo.ico
UninstallDisplayIcon={app}\halo-ide.exe
ChangesAssociations=yes
ChangesEnvironment=yes

[Files]
Source: "{#Exe}"; DestDir: "{app}"; DestName: "halo-ide.exe"; Flags: ignoreversion
Source: "halo.cmd"; DestDir: "{app}\bin"; Flags: ignoreversion

[InstallDelete]
Type: files; Name: "{app}\halo.exe"

[Icons]
Name: "{autoprograms}\Halo"; Filename: "{app}\halo-ide.exe"

[Registry]
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}\bin"; Check: NeedsPath(ExpandConstant('{app}\bin'))
Root: HKA; Subkey: "Software\Classes\*\shell\Halo"; ValueType: string; ValueData: "Open with Halo"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\*\shell\Halo"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\halo-ide.exe"
Root: HKA; Subkey: "Software\Classes\*\shell\Halo\command"; ValueType: string; ValueData: """{app}\halo-ide.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Directory\shell\Halo"; ValueType: string; ValueData: "Open with Halo"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Directory\shell\Halo"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\halo-ide.exe"
Root: HKA; Subkey: "Software\Classes\Directory\shell\Halo\command"; ValueType: string; ValueData: """{app}\halo-ide.exe"" ""%V"""
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\Halo"; ValueType: string; ValueData: "Open with Halo"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\Halo"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\halo-ide.exe"
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\Halo\command"; ValueType: string; ValueData: """{app}\halo-ide.exe"" ""%V"""
Root: HKA; Subkey: "Software\Classes\Halo.File"; ValueType: string; ValueData: "Halo file"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Halo.File\DefaultIcon"; ValueType: string; ValueData: "{app}\halo-ide.exe,0"
Root: HKA; Subkey: "Software\Classes\Halo.File\shell\open\command"; ValueType: string; ValueData: """{app}\halo-ide.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\halo-ide.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "Halo"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\halo-ide.exe\shell\open\command"; ValueType: string; ValueData: """{app}\halo-ide.exe"" ""%1"""
Root: HKA; Subkey: "Software\Halo"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Halo\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "Halo"
Root: HKA; Subkey: "Software\Halo\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "Lightweight code editor"
Root: HKA; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "Halo"; ValueData: "Software\Halo\Capabilities"; Flags: uninsdeletevalue

[Run]
Filename: "{app}\halo-ide.exe"; Description: "Launch Halo"; Flags: nowait postinstall skipifsilent

[Code]
const
  Types = '.txt .md .markdown .json .jsonc .yaml .yml .toml .ini .cfg .conf .xml .csv .log .env .diff .patch .rs .py .js .mjs .cjs .ts .tsx .jsx .c .h .cpp .hpp .cc .cs .go .java .kt .rb .php .lua .sh .bash .zsh .ps1 .psm1 .bat .cmd .sql .html .htm .css .scss .less .vue .svelte .swift .dart .tex .gitignore .gitattributes .editorconfig ';

procedure RegisterTypes(Install: Boolean);
var
  List, Ext: string;
  At: Integer;
begin
  List := Types;
  At := Pos(' ', List);
  while At > 0 do
  begin
    Ext := Copy(List, 1, At - 1);
    Delete(List, 1, At);
    At := Pos(' ', List);
    if Install then
    begin
      RegWriteStringValue(HKCU, 'Software\Classes\' + Ext + '\OpenWithProgids', 'Halo.File', '');
      RegWriteStringValue(HKCU, 'Software\Classes\Applications\halo-ide.exe\SupportedTypes', Ext, '');
      RegWriteStringValue(HKCU, 'Software\Halo\Capabilities\FileAssociations', Ext, 'Halo.File');
    end
    else
      RegDeleteValue(HKCU, 'Software\Classes\' + Ext + '\OpenWithProgids', 'Halo.File');
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    RegisterTypes(True);
end;

function NeedsPath(Dir: string): Boolean;
var
  Path: string;
begin
  if not RegQueryStringValue(HKCU, 'Environment', 'Path', Path) then
    Path := '';
  Result := Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Path) + ';') = 0;
end;

procedure CurUninstallStepChanged(Step: TUninstallStep);
var
  Path, Dir: string;
  At: Integer;
begin
  if Step = usUninstall then
    RegisterTypes(False);
  if (Step = usPostUninstall) and RegQueryStringValue(HKCU, 'Environment', 'Path', Path) then
  begin
    Dir := ExpandConstant('{app}\bin');
    At := Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Path) + ';');
    if At > 0 then
    begin
      if At = 1 then
        Delete(Path, 1, Length(Dir) + 1)
      else
        Delete(Path, At - 1, Length(Dir) + 1);
      RegWriteExpandStringValue(HKCU, 'Environment', 'Path', Path);
    end;
  end;
end;
