#ifndef AppVersion
  #error AppVersion must be provided
#endif
[Setup]
AppId={{61AF00B6-E92A-4519-A0A3-382347FA4051}
AppName=Zi DevTools
AppVersion={#AppVersion}
AppPublisher=ZiCode
AppPublisherURL=https://devtools.zicode.com/
AppSupportURL=https://github.com/ax2/zi-devtools/issues
DefaultDirName={localappdata}\Programs\ZiDevTools
DefaultGroupName=Zi DevTools
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\release
OutputBaseFilename=ZiDevTools-{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
LicenseFile=..\LICENSE
UninstallDisplayIcon={app}\ZiDevTools.exe
CloseApplications=yes
RestartApplications=no
[Files]
Source: "..\target\release\ZiDevTools.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"
Source: "..\README.md"; DestDir: "{app}"
[Icons]
Name: "{group}\Zi DevTools"; Filename: "{app}\ZiDevTools.exe"
[Run]
Filename: "{app}\ZiDevTools.exe"; Description: "Launch Zi DevTools"; Flags: nowait postinstall skipifsilent
