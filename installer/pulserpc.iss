; ═══════════════════════════════════════════════════════════════════════════
;  PulseRPC installer — Inno Setup 6
;
;  Build:        installer\build.cmd
;  or directly:  ISCC.exe /DAppVersion=1.1.0 installer\pulserpc.iss
;
;  Per-user install (no UAC): files go to %LOCALAPPDATA%\Programs\PulseRPC,
;  autostart is a plain HKCU Run value, and uninstall is handled by Windows.
;  No PowerShell is used at any point.
; ═══════════════════════════════════════════════════════════════════════════

#ifndef AppVersion
  #define AppVersion "1.1.0"
#endif

#define AppName "PulseRPC"
#define AppExe "pulserpc.exe"
#define AppPublisher "PulseRPC"
#define AppUrl "https://github.com/pulserpc/pulserpc"
#define RunKey "Software\Microsoft\Windows\CurrentVersion\Run"
; Left behind by the portable `pulserpc --install` command.
#define LegacyUninstallKey "Software\Microsoft\Windows\CurrentVersion\Uninstall\PulseRPC"

[Setup]
AppId={{2F6E9B4A-7C31-4D8E-9A52-8B1F0C6D7E93}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}
AppUpdatesURL={#AppUrl}
VersionInfoVersion={#AppVersion}
VersionInfoCompany={#AppPublisher}
VersionInfoDescription={#AppName} setup
DefaultDirName={localappdata}\Programs\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
DisableWelcomePage=no
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir=..\dist
OutputBaseFilename=PulseRPC-Setup-{#AppVersion}
SetupIconFile=..\assets\pulserpc.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName} {#AppVersion}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=no
RestartApplications=no
AllowNoIcons=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "autostart"; Description: "Start {#AppName} when I sign in"; GroupDescription: "Startup:"; Flags: checkedonce
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion isreadme
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
; Default configuration. Existing user files are never overwritten, and they
; survive uninstall (the uninstaller asks whether to remove them).
Source: "..\config\config.toml"; DestDir: "{app}\config"; Flags: onlyifdoesntexist uninsneveruninstall
Source: "..\config\apps\*.toml"; DestDir: "{app}\config\apps"; Flags: onlyifdoesntexist uninsneveruninstall

[Dirs]
; Keep the config folder writable for the current user.
Name: "{app}\config"; Permissions: users-modify

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; Comment: "Automatic Discord Rich Presence"
Name: "{autoprograms}\{#AppName} settings"; Filename: "{app}\config"; Comment: "Edit rules and settings"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Comment: "Automatic Discord Rich Presence"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "{#RunKey}"; ValueType: string; ValueName: "{#AppName}"; ValueData: """{app}\{#AppExe}"" --tray"; Tasks: autostart; Flags: uninsdeletevalue
Root: HKCU; Subkey: "{#LegacyUninstallKey}"; ValueType: none; ValueName: ""; Flags: deletekey
; Entries created by the pre-rename builds.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Uninstall\AutoPresence"; ValueType: none; ValueName: ""; Flags: deletekey
Root: HKCU; Subkey: "{#RunKey}"; ValueType: none; ValueName: "AutoPresence"; Flags: deletevalue

[Run]
; Store the id chosen in the wizard. `--set-id` rewrites only the client_id
; line, so all comments in config.toml stay intact.
Filename: "{app}\{#AppExe}"; Parameters: "--set-id ""{code:GetDiscordId}"""; Flags: runhidden; Check: DiscordIdProvided
Filename: "{app}\{#AppExe}"; Description: "Launch {#AppName} now"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/IM {#AppExe} /F"; Flags: runhidden; RunOnceId: "StopPulseRPC"

[Code]
var
  DiscordPage: TInputQueryWizardPage;
  DiscordId: String;

function IsDiscordId(const Value: String): Boolean;
var
  I: Integer;
begin
  Result := (Length(Value) >= 17) and (Length(Value) <= 20);
  if not Result then
    Exit;
  for I := 1 to Length(Value) do
    if (Value[I] < '0') or (Value[I] > '9') then
    begin
      Result := False;
      Exit;
    end;
end;

procedure InitializeWizard;
begin
  DiscordPage := CreateInputQueryPage(wpSelectTasks,
    'Discord application',
    'Which Discord application should show your status?',
    'PulseRPC needs an Application ID — not a bot token and not your Discord user id.' + #13#10 +
    'Create one at discord.com/developers/applications, open General Information,' + #13#10 +
    'and copy the Application ID.' + #13#10#13#10 +
    'Leave this blank to fill in presence.client_id in config.toml later.');
  DiscordPage.Add('Application ID:', False);
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if CurPageID = DiscordPage.ID then
  begin
    DiscordId := Trim(DiscordPage.Values[0]);
    if (DiscordId <> '') and not IsDiscordId(DiscordId) then
    begin
      MsgBox('A Discord application ID has 17 to 20 digits, for example 123456789012345678.' + #13#10 +
             'Copy it from the General Information page of your Discord application.',
             mbError, MB_OK);
      Result := False;
    end;
  end;
end;

function DiscordIdProvided: Boolean;
begin
  Result := DiscordId <> '';
end;

function GetDiscordId(Param: String): String;
begin
  Result := DiscordId;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ResultCode: Integer;
begin
  { Stop a running copy so the executable can be replaced without a reboot. }
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/IM {#AppExe} /F', '',
       SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  ConfigDir: String;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    ConfigDir := ExpandConstant('{app}\config');
    if DirExists(ConfigDir) and not UninstallSilent then
      if MsgBox('Keep your PulseRPC configuration?' + #13#10 + ConfigDir,
                mbConfirmation, MB_YESNO) = IDNO then
        DelTree(ConfigDir, True, True, True);
  end;
end;
