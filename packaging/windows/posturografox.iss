; Instalador de Windows, con Inno Setup 6.
;
; Por qué un instalador y no el .exe suelto: el .exe pelado no deja acceso
; directo, no se desinstala y no se puede volver a una versión anterior sin
; adivinar qué archivo era. Y en un equipo clínico el que instala no es el que
; usa: tiene que poder hacerlo una vez y que en el menú aparezca el programa.
;
; Todo el arte y el audio del juego están adentro del binario, así que el
; instalador copia UN archivo. Los datos (historial local, CSV, informes y la
; base de pacientes) van a la carpeta del usuario y NO los toca el instalador:
; desinstalar no borra los exámenes de nadie.
;
; La versión llega por la línea de comandos:
;   iscc /DMiVersion=0.2.1 packaging\windows\posturografox.iss
#ifndef MiVersion
  #define MiVersion "0.0.0"
#endif

[Setup]
AppId={{81C0AD55-9693-40AD-A373-41E4C78F63DF}
AppName=Posturografox
AppVersion={#MiVersion}
AppPublisher=Debaq
AppPublisherURL=https://github.com/Debaq/posturografox
DefaultDirName={autopf}\Posturografox
DefaultGroupName=Posturografox
; `lowest` y `autopf`: sin pedir administrador, el programa se instala en la
; carpeta del usuario. Con administrador va a Program Files. Las dos funcionan,
; y no exigir UAC es lo que permite instalarlo en un equipo donde el operador no
; es administrador, que es lo normal en una institución.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=..\..\out
OutputBaseFilename=Posturografox-{#MiVersion}-windows-setup
SetupIconFile=posturografox.ico
UninstallDisplayIcon={app}\posturografox.exe
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
LicenseFile=..\..\LICENSE

[Languages]
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"

[Files]
Source: "..\..\out\posturografox.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "posturografox.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\PRIVACIDAD.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Posturografox"; Filename: "{app}\posturografox.exe"; IconFilename: "{app}\posturografox.ico"
Name: "{group}\Manejo de datos de pacientes"; Filename: "{app}\PRIVACIDAD.md"
Name: "{autodesktop}\Posturografox"; Filename: "{app}\posturografox.exe"; IconFilename: "{app}\posturografox.ico"; Tasks: escritorio

[Tasks]
Name: "escritorio"; Description: "Crear un acceso directo en el escritorio"; GroupDescription: "Accesos directos:"

[Run]
Filename: "{app}\posturografox.exe"; Description: "Abrir Posturografox"; Flags: nowait postinstall skipifsilent
