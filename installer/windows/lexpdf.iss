; Instalador Windows de LexPDF.
; Se compila en CI (ver ../../.github/workflows/windows.yml) o a mano:
;   1. Arma dist/windows/ con stage.ps1
;   2. iscc installer/windows/lexpdf.iss
; La firma la pone SignPath en CI (ver README.md de esta carpeta).

#define MyAppName "LexPDF"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "LexiSec"
#define StageDir SourcePath + "/../../dist/windows"

[Setup]
AppId={{d318d4c9-5eb4-4837-9a16-654a20e5df26}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\{#MyAppName}
; Ruta corta a propósito: LT tiene paths largos al descomprimir
DefaultGroupName={#MyAppName}
OutputDir={#SourcePath}/../../dist
OutputBaseFilename=lexpdf-setup
Compression=lzma2/max
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
WizardStyle=modern
; Descomenta al tener SignPath configurado (firma el instalador):
; SignTool=signpath $f

[Languages]
Name: "spanish"; MessagesFile: "compiler:Languages/Spanish.isl"

[Tasks]
Name: "desktopicon"; Description: "Crear icono en el escritorio"; GroupDescription: "Accesos directos:"

[Files]
Source: "{#StageDir}\lexpdf.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\pdfium.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\lt\*"; DestDir: "{app}\lt"; Flags: ignoreversion recursesubdirs createallsubdirs
; JRE empaquetado (opcional: si no está, el instalador pedirá Java, ver [Code])
Source: "{#StageDir}\jre\*"; DestDir: "{app}\jre"; Flags: ignoreversion recursesubdirs createallsubdirs skipifsourcedoesntexist; Check: DirExists(ExpandConstant('{#StageDir}\jre'))

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\lexpdf.exe"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\lexpdf.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\lexpdf.exe"; Description: "Abrir {#MyAppName}"; Flags: nowait postinstall skipifsilent

[Code]
const
  TemurinURL = 'https://adoptium.net/temurin/releases/?version=21&os=windows&arch=x64';

function JavaDisponible(): Boolean;
var
  Codigo: Integer;
  Bundled: String;
begin
  { 1. JRE empaquetado junto al exe }
  Bundled := ExpandConstant('{app}\jre\bin\java.exe');
  if FileExists(Bundled) then
  begin
    Result := True;
    Exit;
  end;
  { 2. Java del sistema }
  Result := Exec('java.exe', '-version', '', SW_HIDE, ewWaitUntilTerminated, Codigo)
    and (Codigo = 0);
end;

function InitializeSetup(): Boolean;
var
  Respuesta, Codigo: Integer;
begin
  Result := True;
  if JavaDisponible() then
    Exit;
  Respuesta := MsgBox(
    'No se detectó Java en el sistema.' + #13#10 +
    'La revisión de sintaxis lo necesita (Java 17 o superior).' + #13#10 + #13#10 +
    '¿Abrir la página de descarga de Temurin ahora?' + #13#10 +
    '(Puedes continuar e instalar Java después.)',
    mbConfirmation, MB_YESNOCANCEL);
  if Respuesta = IDYES then
    ShellExec('open', TemurinURL, '', '', SW_SHOW, ewNoWait, Codigo);
  if Respuesta = IDCANCEL then
    Result := False;
end;
