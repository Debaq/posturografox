<p align="center"><img src="logo.jpeg" alt="Posturografox" width="320"></p>

# Posturografox

Posturógrafo de 4 celdas de carga: firmware para ESP32-C3 + HX711 y una
aplicación de escritorio en Rust (egui) para el análisis del centro de
presión (COP).

La app muestra el COP en un gráfico cartesiano con trazo y elipse de
confianza 95%, el desplazamiento medio-lateral / antero-posterior en el
tiempo, métricas clásicas de estabilometría, el examen guiado CTSIB, un
ejercicio de límites de estabilidad y un modo juego para rehabilitación.

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

Para desarrollo:

```bash
cargo test          # tests de estabilometría, límites y exportación
cargo clippy --all-targets
cargo fmt
```

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
