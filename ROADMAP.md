# Roadmap Posturografox

Plan de trabajo salido de la revisión completa del programa (app Rust,
firmware ESP32, repo y CI). Cada punto es un commit.

Estado: `[ ]` pendiente · `[x]` hecho

---

## Fase 0 — Base del repo

- [x] **R1 · Limpiar el repo.** Borrar la app Python (`app/`, `requirements.txt`),
  que quedó duplicada y muerta tras el port a Rust, y reescribir el README
  para documentar la app Rust (build, ejecución, firmware).
- [x] **R2 · CI unificado.** `cargo test` + `clippy` + `fmt --check` en cada push
  y PR; release de binarios Rust (Linux/Windows) en tags `v*.*.*`. Hoy el
  workflow de release sigue siendo el de Python y el de Rust solo corre en
  una rama.
- [x] **R3 · Versión única.** El archivo `VERSION` y `Cargo.toml` divergen:
  dejar `Cargo.toml` como fuente de verdad y que la app muestre su versión.
- [x] **R4 · Licencia, privacidad y herramientas.** `LICENSE`, nota de manejo de
  datos de pacientes, `rustfmt.toml` y `rust-toolchain.toml` para builds
  reproducibles.
- [x] **R5 · Avisos de clippy.** Los 12 avisos actuales (ifs colapsables, deref
  redundante, función con 13 argumentos).

## Fase 1 — Configuración y persistencia

- [x] **R6 · Zona de configuración.** Un único lugar (`config.rs` + panel
  "Configuración") con *todas* las opciones del programa: geometría de la
  plataforma, calibración, umbral de detección, trazo, duración del ensayo
  clínico y **duración de la partida del modo juego**. Hoy están repartidas
  entre tarjetas de la barra superior y constantes compiladas.
- [x] **R7 · Persistencia.** Guardar y restaurar toda la configuración entre
  sesiones con `eframe::App::save` + `serde`. Hoy cada arranque vuelve a
  ganancias 1.0 y plataforma 40×40.
- [x] **R8 · Exportación fuera del directorio de trabajo.** Escribir en el
  directorio de datos del usuario (o diálogo de guardado) en vez de
  `./sesiones`, que falla en Windows o con permisos de solo lectura.
- [x] **R9 · Tests de exportación aislados.** Usar directorio temporal para que
  los tests no escriban dentro del repo.

## Fase 2 — Adquisición robusta

- [x] **R10 · Parser de línea testeable.** Extraer el parseo del hilo lector a
  una función pura con tests (encabezado, comentarios, línea truncada,
  campos de más) y **rechazar NaN/Inf**, que hoy se cuelan y envenenan
  todas las métricas en silencio.
- [x] **R11 · Firmware: secuencia y marca de tiempo.** Agregar número de muestra
  y `micros()` a cada línea CSV.
- [x] **R12 · Host: base de tiempo del dispositivo.** Usar el reloj del firmware
  en vez de `Instant::now()` del host (que agrupa muestras por el buffering
  del USB CDC y sesga la velocidad media), y reportar muestras perdidas.
- [x] **R13 · Firmware: calibración en NVS.** Guardar tara y calibración por
  celda en memoria no volátil, con comandos para setearlas desde la app, en
  vez de `#define` que obligan a recompilar.
- [x] **R14 · Firmware: estado consultable y salida no bloqueante.** Comando que
  informa modo (crudo/calibrado), frecuencia y calibración vigente, y
  chequeo de `availableForWrite()` para que el host lento no frene el
  muestreo.
- [x] **R15 · Reconexión automática.** Reintentar la conexión cuando el USB se
  desenchufa y vuelve, sin tener que apretar nada.
- [x] **R16 · Capa de transporte y simulador.** Trait `Transporte` (serie /
  simulado / reproducción de CSV) y modo `--simular` para desarrollar,
  demostrar y testear sin la plataforma física. Además deja preparada la
  migración a Bluetooth descrita en `firmware/BLUETOOTH.md`.

## Fase 3 — Validez clínica

- [x] **R17 · Filtrado del COP.** Pasabajos Butterworth de fase cero (filtfilt),
  corte configurable 5–10 Hz. Sin esto, longitud de trazo y velocidad media
  —las métricas más usadas— quedan infladas por el ruido del HX711.
- [x] **R18 · Ensayo de duración fija.** Ventana de registro fija (30 s por
  defecto) con descarte de los primeros segundos de acomodación y cuenta
  regresiva en pantalla. Hoy la sesión dura lo que la persona esté parada,
  así que las métricas no son comparables entre ensayos.
- [x] **R19 · Calibración a kilogramos.** Rutina guiada con masa conocida por
  celda, resultado persistente. Sin ella las ganancias son arbitrarias y el
  COP queda sesgado si las celdas difieren entre sí.
- [x] **R20 · Umbral de detección en kg.** Reemplazar el umbral mágico en
  cuentas crudas (20000) por un umbral en kilogramos una vez calibrado.
- [x] **R21 · Elipse coherente.** La elipse dibujada se ajusta hoy sobre la
  ventana del trazo y la reportada sobre la sesión completa: unificar para
  que el área95 del panel corresponda al dibujo.
- [x] **R22 · Métricas nuevas.** Velocidad media ML y AP por separado (la más
  reproducible test-retest), análisis frecuencial (frecuencia mediana y
  F80), **peso corporal** y asimetrías izquierda/derecha y anterior/posterior
  en kg — información que hoy se descarta.
- [x] **R23 · Historial por paciente.** Almacén local de sesiones con listado,
  comparación y gráfico de evolución, en vez de un CSV suelto por ensayo.
- [x] **R24 · Informe imprimible.** Reporte con datos del paciente, trazo,
  elipse, tabla de métricas y cocientes CTSIB, listo para la ficha clínica.

## Fase 4 — Rendimiento

- [x] **R25 · Degradé del trazo sin HashMap.** Hoy se reconstruye un mapa de
  hasta 20.000 entradas por frame, y dos puntos idénticos colisionan y toman
  el color equivocado.
- [x] **R26 · Métricas incrementales.** Acumuladores O(1) por muestra en vez de
  recalcular toda la sesión en cada frame.
- [x] **R27 · Repintado por evento.** No repintar a 30 fps fijos cuando no llega
  ninguna muestra.
- [x] **R28 · Audio decodificado una vez.** El loop de música vuelve a decodificar
  el `.ogg` completo en cada vuelta.

## Fase 5 — Modo juego

- [x] **R29 · El juego exige plataforma ocupada.** Hoy arranca con solo estar
  conectado: el reloj corre y el zorro queda centrado aunque no haya nadie.
- [x] **R30 · Simulación pura y tests.** La colisión vive dentro de la función de
  dibujo, así que la física depende del tamaño de ventana y no se puede
  testear. Separar simulación (coordenadas normalizadas) de dibujo.
- [x] **R31 · Recursos agrupados.** `dibujar_partida` recibe 13 argumentos.

## Fase 6 — UX clínica

- [x] **R32 · Navegación por secciones.** Las siete tarjetas en una fila se
  desbordan en ventanas chicas: pestañas Examen / Configuración / Ejercicios
  / Historial, con los gráficos grandes.
- [x] **R33 · Controles en su lugar.** "Espaciado" (cosmético del trazo) está en
  la tarjeta de detección automática; queda reubicado en Configuración.
  *(Resuelto junto con R6: ahora vive en la sección Gráficos.)*
- [x] **R34 · Modo paciente.** Pantalla completa con solo el COP, sin controles,
  para que el paciente vea su biofeedback sin distracciones.
- [x] **R35 · Tema oscuro y alto contraste.** Hoy la app fuerza tema claro.
- [x] **R36 · Límites de estabilidad: resultados.** Guardar distancia alcanzada y
  déficits por dirección, mostrarlos y exportarlos; hoy solo se mide el
  tiempo por objetivo y se pierde al salir.

## Fase 7 — Juego calibrado y medible

El modo juego hoy escala el COP al semieje físico de la plataforma
(`juego.rs:1058`, 20 cm por defecto). Nadie desplaza su COP 20 cm: la
excursión voluntaria sana ronda los ±5–8 cm y la de un hemiparético los
±2–3 cm, y además descentrada. Resultado: el zorro usa una fracción de la
pista, las rocas del borde son inesquivables por escala y no por déficit, y
el puntaje no se puede comparar entre pacientes ni entre sesiones.

La fase cambia el eje de la cosa: **el juego es el estímulo, no la medición**.
El puntaje es motivación; el dato clínico es el COP que ya se está grabando
mientras el paciente juega (`app.rs:820`), cruzado con los eventos del juego.
Cada roca es un ensayo de weight-shifting con dirección conocida, así que de
una partida salen decenas de maniobras medidas en vez de los 8 alcances del
ejercicio de límites.

- [x] **R37 · Rango calibrado del paciente.** Centro de reposo `x0` y alcances
  `x_izq`/`x_der` (más el par AP), y mapeo lineal por tramo y asimétrico en
  lugar de la división por el semieje. Tramos separados porque la carga
  asimétrica es la regla en rehabilitación y un rango simétrico deja al
  zorro corrido de forma permanente. Con rango degenerado (<2 cm) la
  calibración se declara inválida y cae al default: si no, el ruido de un
  milímetro manda al zorro de punta a punta. Default poblacional cuando no
  hay calibración (ML ±7 cm, AP +8/−4 cm, recortado a la plataforma), nunca
  el semieje, y marcado como "sin calibrar" para que el dato no se lea como
  clínico.
- [x] **R38 · Botón "Calibrar".** Calibración por sesión, manual, disponible en
  la pantalla de espera del juego y en la tarjeta JUEGO de la vista clínica.
  Reposo 3–5 s para `x0` y alcance sostenido a cada lado para los extremos,
  presentado con el zorro y una recompensa que se corre al borde: la
  calibración es también el tutorial de controles. Vive en `EstadoJuego`, no
  en `Partida`, para sobrevivir a "Reintentar".
- [x] **R39 · Calibración heredada del ejercicio de límites.** Si la sesión ya
  corrió el ejercicio (`limites.rs`), tomar los alcances de E/O y N/S en vez
  de pedir una calibración nueva. Cascada completa: límites → calibración en
  juego → default.
- [x] **R40 · Exigencia configurable.** Fracción del límite alcanzado que hay
  que cubrir para llegar al borde de la pista (0.4–0.9, slider en
  Configuración junto a duración y volúmenes). Es el parámetro de
  dosificación —el que un fisio sube sesión a sesión—, así que no puede ser
  una constante compilada. Fija dentro de la partida: si cambiara mientras
  juega, la partida deja de ser una condición medible y "aguantó 180 s"
  pierde sentido. La velocidad sigue rampeando como hoy; son dos ejes
  distintos (velocidad del cambio de carga vs. amplitud del desplazamiento).
- [x] **R41 · Reloj común.** Pasar el instante de muestra del firmware (`m.t`)
  dentro de `EntradaJuego`. Hoy el juego solo conoce `dt` de frame, y estampar
  los eventos con el reloj de render le mete ±8–16 ms de jitter de vsync a
  cada latencia. A 80 SPS el reloj del firmware da 12.5 ms de resolución.
- [x] **R42 · Registro de eventos del juego.** Por cada roca: instante de
  aparición, lado que exige, instante en que entra en zona de reacción y
  resultado (esquivó / golpeó). La zona se define por **tiempo al contacto**
  (~1.2 s a la velocidad de ese instante), no por distancia fija: con la
  rampa de velocidad, una distancia fija achica la ventana de reacción a lo
  largo de la partida y contamina la latencia con la aceleración.
- [x] **R43 · Métricas de maniobra.** Cruzar eventos y COP para obtener, por
  maniobra: latencia de reacción (del estímulo al primer desplazamiento ML
  sobre umbral, haya salido hacia donde haya salido: si solo contara los
  arranques correctos, las maniobras mal dirigidas desaparecerían del
  registro justo por estar mal), velocidad pico, amplitud alcanzada en
  cm y en % del límite calibrado, y control direccional. Agregado por
  partida: medianas por lado e índice de asimetría. Son las cuatro
  dimensiones del test de límites de estabilidad, medidas decenas de veces
  por sesión. Se descartan las maniobras que empiezan con el paciente ya en
  movimiento, las que caen en el congelamiento por golpe, las que se solapan
  con otra roca y las de latencia fuera de 100–1500 ms (anticipación o falta
  de respuesta).
- [x] **R44 · La partida en el historial.** Guardar la sesión de juego en el
  mismo historial, con marca de juego y fuera de los cocientes CTSIB: sus
  métricas de bipedestación quieta no son comparables con las de un ensayo
  estático (el área 95% durante una partida mide cuánto jugó, no cuánto
  oscila). El informe imprime la definición de la latencia y cuántas
  maniobras válidas hubo sobre el total, para que la métrica no sea una caja
  negra.
- [ ] **R45 · Sugerencia de exigencia.** Al terminar la partida, proponer el
  valor siguiente a partir de las métricas —no del puntaje, que sube solo con
  la velocidad—. Solo en la ventana clínica, nunca en la pantalla del
  paciente, y solo con datos suficientes: calibración real y un mínimo de
  maniobras válidas por lado. Se sugiere, no se aplica: si la dificultad se
  moviera sola, dos sesiones dejarían de ser comparables y se perdería
  justamente lo que la fase viene a ganar. Sin datos suficientes se dice por
  qué no hay sugerencia.
