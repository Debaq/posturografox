<p align="center"><img src="logo.jpeg" alt="Posturografox" width="320"></p>

# Posturografox

Posturógrafo de 4 celdas de carga: firmware para ESP32-C3 + HX711 y una
aplicación de escritorio en Rust (egui) para el análisis del centro de
presión (COP).

La app muestra el COP en un gráfico cartesiano con trazo y elipse de
confianza 95%, el desplazamiento medio-lateral / antero-posterior en el
tiempo, métricas clásicas de estabilometría, el examen guiado CTSIB, un
ejercicio de límites de estabilidad y un modo juego para rehabilitación.

Los pacientes y sus exámenes viven en la **misma base que vHIT**: las dos
aplicaciones son una suite y comparten una ficha por persona. Ver
[Pacientes](#pacientes-la-base-compartida-de-la-suite).

## Compilar y ejecutar

```bash
cd rust
cargo run --release
```

Dependencias de sistema en Linux (Debian/Ubuntu):

```bash
sudo apt-get install -y libx11-dev libxi-dev libxcursor-dev libxrandr-dev \
  libxinerama-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev \
  libudev-dev pkg-config
```

Sin la plataforma a mano:

```bash
cargo run --release -- --simular
```

Genera muestras sintéticas con el mismo formato que el firmware. La app lo
marca como "⚠ Simulado": no sirve como registro clínico.

### Segunda pantalla

Si hay más de un monitor conectado, el modo juego se abre en su propia ventana
sobre la pantalla que **no** está usando el evaluador: el paciente ve el juego a
pantalla completa mientras la ventana principal sigue mostrando el COP y las
métricas. Un botón abajo a la izquierda del juego lo pasa al otro monitor, y la
tarjeta "JUEGO" de la vista clínica dice en cuál está y permite cerrarlo.

Con un solo monitor el juego ocupa la ventana principal, como siempre.

```bash
cargo run --release -- --simular --juego   # arranca directo en el modo juego
POSTUROGRAFOX_PANTALLAS=2 cargo run --release -- --simular --juego
```

La variable `POSTUROGRAFOX_PANTALLAS` parte la pantalla real en esa cantidad de
monitores de mentira. Sirve para probar el modo de dos pantallas en una máquina
que tiene una sola.

Para desarrollo:

```bash
cargo test          # tests de estabilometría, límites y exportación
cargo clippy --all-targets
cargo fmt
```

## Pacientes: la base compartida de la suite

El botón **👥 Pacientes** abre la ficha, el historial y la evolución del
paciente. Es el mismo sistema de gestión que usa
[vHIT](../vhit-wout-google), conectado a la **misma base de datos**: un
paciente dado de alta en un equipo aparece en el otro, con su ficha, su fecha
de nacimiento y sus notas. Lo que no se mezcla son los exámenes: vHIT lista
sus impulsos y Posturografox sus ensayos de equilibrio, colgados de la misma
persona.

- **Dónde está la base.** `vhit.sqlite`, por defecto en
  `~/.local/share/vhit` (`%APPDATA%\vhit` en Windows), que es la carpeta de
  vHIT. Se puede cambiar desde la ventana (⚙ → *Dónde está la base*); lo
  único que hay que hacer es que los dos programas apunten al mismo lugar.
  La elección queda en `almacenamiento.ron`, en la carpeta de datos de la
  app.
- **Cifrado.** SQLCipher AES-256 por defecto. La frase de paso se pide al
  abrir la base, no al arrancar el programa: probar el equipo, calibrar
  celdas o jugar un rato no deberían exigir escribirla. No se guarda en
  ningún lado, así que si se pierde los datos no se recuperan. Es la frase de
  la base: la misma que abre vHIT.
- **Qué se archiva.** Cada ensayo cerrado, cada partida y cada examen de
  límites se guardan solos en la ficha del paciente elegido, con **la
  configuración exacta con la que se midieron** (tamaño de plataforma,
  filtro, duración), la versión de la app y el **COP crudo**. Por eso un
  examen viejo se puede volver a mirar tal como se vio ese día, sin
  repetírselo a nadie.
- **Sin base abierta el programa funciona igual.** Sigue midiendo, exportando
  CSV, imprimiendo informes y archivando en el historial local
  (`historial.ronl`). La base es el registro clínico; el historial local es el
  archivo que no necesita frase de paso.

Las reglas de la convivencia de las dos aplicaciones en un mismo archivo
—qué tabla es un contrato, qué pasa al borrar un paciente, qué versión de
SQLCipher— están en [`vhit-wout-google/SUITE.md`](../vhit-wout-google/SUITE.md).

## Firmware

El ESP32 imprime por serie, a 115200 baudios, líneas CSV `fd,fi,bd,bi`
(frontal/posterior, derecha/izquierda). Ver `firmware/posturografox` para
el sketch y `firmware/BLUETOOTH.md` para el plan de transmisión inalámbrica.

Al conectar, la app busca sola el puerto: le manda `i` a cada puerto USB y
se queda con el que responde `# POSTUROGRAFOX,1`.

## Releases

Al pushear un tag `vX.Y.Z` se compilan los binarios de Windows y Linux y se
publica un release con ese nombre (ver Actions).

## Estado

El plan de trabajo vigente está en [ROADMAP.md](ROADMAP.md).
