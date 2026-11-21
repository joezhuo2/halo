#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef Exe
  #define Exe "..\target\release\halo.exe"
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
UninstallDisplayIcon={app}\halo.exe
ChangesAssociations=yes

[Files]
Source: "{#Exe}"; DestDir: "{app}"; DestName: "halo.exe"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Halo"; Filename: "{app}\halo.exe"

[Registry]
Root: HKA; Subkey: "Software\Classes\*\shell\Halo"; ValueType: string; ValueData: "Open with Halo"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\*\shell\Halo"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\halo.exe"
Root: HKA; Subkey: "Software\Classes\*\shell\Halo\command"; ValueType: string; ValueData: """{app}\halo.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Directory\shell\Halo"; ValueType: string; ValueData: "Open with Halo"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Directory\shell\Halo"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\halo.exe"
Root: HKA; Subkey: "Software\Classes\Directory\shell\Halo\command"; ValueType: string; ValueData: """{app}\halo.exe"" ""%V"""
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\Halo"; ValueType: string; ValueData: "Open with Halo"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\Halo"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\halo.exe"
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\Halo\command"; ValueType: string; ValueData: """{app}\halo.exe"" ""%V"""

[Run]
Filename: "{app}\halo.exe"; Description: "Launch Halo"; Flags: nowait postinstall skipifsilent
