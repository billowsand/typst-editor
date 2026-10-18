; Compiled by package-windows.ps1 after release staging and smoke testing.
#ifndef AppVersion
  #error AppVersion is required
#endif
#ifndef StageDir
  #error StageDir is required
#endif
#ifndef OutputPath
  #error OutputPath is required
#endif
#ifndef OutputName
  #define OutputName "Zijian-Setup"
#endif

[Setup]
AppId={{65A338F8-9058-4477-BD7B-E21BA3D8CE04}
AppName=字笺 · Typst
AppVersion={#AppVersion}
AppPublisher=Zijian Contributors
AppPublisherURL=https://github.com/billowsand/typst-editor
AppSupportURL=https://github.com/billowsand/typst-editor/issues
DefaultDirName={localappdata}\Programs\Zijian
DefaultGroupName=字笺 · Typst
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputPath}
OutputBaseFilename={#OutputName}
SetupIconFile=..\assets\icons\zijian.ico
UninstallDisplayIcon={app}\typst-editor-gpui.exe
UninstallDisplayName=字笺 · Typst
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no
SetupLogging=yes
ChangesAssociations=yes

[Languages]
Name: "chinesesimplified"; MessagesFile: "languages\ChineseSimplified.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "创建桌面快捷方式"; GroupDescription: "快捷方式："; Flags: unchecked
Name: "fileassociation"; Description: "在 .typ 文件的“打开方式”中添加字笺"; GroupDescription: "文件关联："; Flags: unchecked

[Files]
Source: "{#StageDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\字笺 · Typst"; Filename: "{app}\typst-editor-gpui.exe"; WorkingDir: "{app}"
Name: "{group}\用户指南"; Filename: "{app}\docs\用户指南.md"
Name: "{autodesktop}\字笺 · Typst"; Filename: "{app}\typst-editor-gpui.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Classes\Zijian.Typst"; ValueType: string; ValueName: ""; ValueData: "Typst 文稿"; Flags: uninsdeletekey; Tasks: fileassociation
Root: HKCU; Subkey: "Software\Classes\Zijian.Typst\DefaultIcon"; ValueType: string; ValueData: "{app}\typst-editor-gpui.exe,0"; Tasks: fileassociation
Root: HKCU; Subkey: "Software\Classes\Zijian.Typst\shell\open\command"; ValueType: string; ValueData: """{app}\typst-editor-gpui.exe"" ""%1"""; Tasks: fileassociation
Root: HKCU; Subkey: "Software\Classes\.typ\OpenWithProgids"; ValueType: none; ValueName: "Zijian.Typst"; Flags: uninsdeletevalue; Tasks: fileassociation

[Run]
Filename: "{app}\typst-editor-gpui.exe"; Description: "启动字笺 · Typst"; Flags: nowait postinstall skipifsilent

; User settings, recovery drafts and documents are deliberately retained.
