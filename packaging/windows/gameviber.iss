; GameViber's Windows installer (Inno Setup 6), made by package.sh:
;   iscc -DVersion=<version> -DStage=<staged files> -DDrivers=<driver installers> -DOutput=<dir> -DOutputName=<name> gameviber.iss
; It installs GameViber for every user, opens gameviber:// links with it, and
; offers the drivers the rumble capture uses (ViGEmBus) and the one hiding the
; real gamepad from games (HidHide), unless they are installed already.
; GameViber's updates run it silently (`update::windows`): it then starts
; GameViber again.

#ifndef Version
  #error Version is not defined
#endif

[Setup]
AppId={{8C3B5F3E-6A0D-4E55-9C61-2F0B7D4A9E21}
AppName=GameViber
AppVersion={#Version}
AppPublisher=GameViber
AppPublisherURL=https://github.com/Fyustorm/GameViber
AppSupportURL=https://github.com/Fyustorm/GameViber/issues
AppUpdatesURL=https://github.com/Fyustorm/GameViber/releases
DefaultDirName={autopf}\GameViber
DefaultGroupName=GameViber
DisableProgramGroupPage=yes
LicenseFile={#Stage}\LICENSE
OutputDir={#Output}
OutputBaseFilename={#OutputName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Windows 10 2004: the game's sound (process loopback) and image (Windows.Graphics.Capture).
MinVersion=10.0.19041
PrivilegesRequired=admin
CloseApplications=force
RestartApplications=no
UsedUserAreasWarning=no
UninstallDisplayIcon={app}\gameviber.exe

[Tasks]
Name: "vigembus"; Description: "Install ViGEmBus, which GameViber needs to capture the rumble"; Check: not DriverInstalled('ViGEmBus')
Name: "hidhide"; Description: "Install HidHide, to hide your real gamepad from games (needs a restart)"; Flags: unchecked; Check: not DriverInstalled('HidHide')
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Stage}\gameviber.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\THIRD-PARTY-LICENSES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "distribution-installer"; DestDir: "{app}"; DestName: "distribution"; Flags: ignoreversion
Source: "{#Drivers}\ViGEmBus_setup.exe"; DestDir: "{tmp}"; Flags: deleteafterinstall; Tasks: vigembus
Source: "{#Drivers}\HidHide_setup.exe"; DestDir: "{tmp}"; Flags: deleteafterinstall; Tasks: hidhide

[Icons]
Name: "{group}\GameViber"; Filename: "{app}\gameviber.exe"
Name: "{autodesktop}\GameViber"; Filename: "{app}\gameviber.exe"; Tasks: desktopicon

[Registry]
; gameviber:// links (the website's "Open in GameViber"); GameViber writes the same for the user (`links::windows`).
Root: HKCU; Subkey: "Software\Classes\gameviber"; ValueType: string; ValueName: ""; ValueData: "URL:GameViber"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\gameviber"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\gameviber\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\gameviber.exe"",0"
Root: HKCU; Subkey: "Software\Classes\gameviber\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\gameviber.exe"" ""%1"""

[Run]
; The drivers' own installers, shown: they ask their own questions.
Filename: "{tmp}\ViGEmBus_setup.exe"; StatusMsg: "Installing ViGEmBus..."; Flags: waituntilterminated; Tasks: vigembus
Filename: "{tmp}\HidHide_setup.exe"; StatusMsg: "Installing HidHide..."; Flags: waituntilterminated; Tasks: hidhide
Filename: "{app}\gameviber.exe"; Description: "{cm:LaunchProgram,GameViber}"; Flags: nowait postinstall skipifsilent runasoriginaluser
; After an update GameViber ran silently.
Filename: "{app}\gameviber.exe"; Flags: nowait runasoriginaluser; Check: WizardSilent

[Code]
function DriverInstalled(Name: String): Boolean;
begin
  Result := RegKeyExists(HKLM, 'SYSTEM\CurrentControlSet\Services\' + Name);
end;
