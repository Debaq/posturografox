# Empaquetado

Qué se publica en cada plataforma, y qué está realmente probado.

| Plataforma | Formato | Estado |
|---|---|---|
| Linux | AppImage | Armado por `linux/build-appimage.sh`, **probado** en consola y en CI |
| Windows | Instalador (Inno Setup 6) | Armado en CI, **sin probar en una máquina Windows real** |
| macOS | — | No hay build |

Además del paquete, cada release sube el **binario suelto** de las dos
plataformas. No es un descuido: sirve para probar una versión sin instalar nada,
y es la red por si el instalador o el AppImage fallan en un equipo particular.

## Lo que NO hay que empaquetar

Todo el arte y el audio del juego están **adentro del binario**
(`include_bytes!` en `rust/src/juego.rs`), igual que el logo. Así que el paquete
copia un archivo y nada más. Si algún día un recurso pasa a leerse del disco, hay
que tocar los dos empaquetados.

## Linux

```bash
packaging/linux/build-appimage.sh          # con linuxdeploy en el PATH
LINUXDEPLOY=/ruta/linuxdeploy packaging/linux/build-appimage.sh
```

El paquete queda en `out/Posturografox-<version>-x86_64.AppImage`.

Se elige AppImage y no `.deb`/`.rpm` por una razón concreta: el binario que
compila CI en Ubuntu queda atado a la glibc de esa versión, y en una distro más
vieja —el PC que hay en el box y que nadie actualiza— falla con
`GLIBC_2.xx not found`. El AppImage lleva adentro lo que hace falta y corre sin
instalar nada.

El `AppRun` ejecuta desde `~/.local/share/posturografox`: el AppImage es de solo
lectura y el historial local, los CSV y los informes tienen que caer en algún
lado que se pueda escribir. La base de pacientes tiene su propia ruta (ver
`PRIVACIDAD.md`) y no depende de esto.

Notas de CI, aprendidas a golpes:

- `APPIMAGE_EXTRACT_AND_RUN=1`: el runner no tiene FUSE, así que un AppImage no
  se puede montar y hay que extraerlo para ejecutarlo. Vale para linuxdeploy y
  para el `appimagetool` que usa por dentro.
- `NO_STRIP=1`: el `strip` que trae linuxdeploy es más viejo que las bibliotecas
  de Mesa actuales y falla con ``unknown type [0x13] section `.relr.dyn'``. Lo
  único que se pierde es tamaño de paquete.

## Windows

```
iscc /DMiVersion=0.2.1 packaging\windows\posturografox.iss
```

Espera el binario en `out\posturografox.exe` y deja el instalador en
`out\Posturografox-<version>-windows-setup.exe`.

Decisiones del instalador:

- **Sin exigir administrador** (`PrivilegesRequired=lowest`): se instala en la
  carpeta del usuario, y con administrador va a Program Files. En una institución
  el operador casi nunca es administrador, y pedir UAC sería pedirle que llame a
  informática para poder trabajar.
- **Desinstalar no borra datos.** El historial local, los CSV, los informes y la
  base de pacientes viven en la carpeta del usuario; el instalador no los toca.
- Accesos directos en el menú y, opcional, en el escritorio. Y un acceso a
  `PRIVACIDAD.md`, que es lo que hay que leer antes de cargar el primer paciente.

Lo que **falta** y se sabe: el ejecutable **no está firmado**, así que Windows
muestra la advertencia de SmartScreen la primera vez. Firmarlo necesita un
certificado de firma de código, que es una compra y un trámite, no una línea de
código. El ícono del acceso directo sale de `posturografox.ico`, que el
instalador copia; el `.exe` en sí todavía no lleva el ícono embebido.

## Los iconos

`linux/posturografox.png` (256×256) y `windows/posturografox.ico` (7 tamaños)
salen de `logo.jpeg`, recortado al centro para que quede cuadrado sin deformarse.
Están commiteados a propósito: generarlos en CI agregaría una dependencia de
ImageMagick o Pillow para producir siempre el mismo archivo.
