//! Modo juego: el zorrito de Posturografox esquiva obstáculos que caen,
//! moviéndose de lado a lado según el COP medio-lateral en vivo. Ver
//! `EntradaJuego` para lo que llega del posturógrafo real en cada frame.

use egui::{Align2, Color32, ColorImage, Image, Key, Pos2, Rect, RichText, TextureHandle, TextureOptions, Ui, Vec2};
use rodio::Source;

const ZORRO_BYTES: &[u8] = include_bytes!("../assets/fox.png");
const ZORRO_COLUMNAS: u32 = 8;
const ZORRO_FILAS: u32 = 1;

const GALLINA_BYTES: &[u8] = include_bytes!("../assets/gallina.png");
const GALLINA_COLUMNAS: u32 = 4;
const GALLINA_FILAS: u32 = 1;

const CONEJO_BYTES: &[u8] = include_bytes!("../assets/conejo.png");
const CONEJO_COLUMNAS: u32 = 4;
const CONEJO_FILAS: u32 = 1;

// No es una animación: cada celda es un diseño de roca distinto, se elige
// uno al azar por obstáculo (variedad visual sin subir la dificultad).
// TODO(assets): rocas.png está en muy mala resolución (se nota pixelado
// incluso achicado). Reemplazar por una versión más nítida antes de sumar
// más obstáculos/coleccionables nuevos.
const ROCAS_BYTES: &[u8] = include_bytes!("../assets/rocas.png");
const ROCAS_COLUMNAS: u32 = 12;
const ROCAS_FILAS: u32 = 1;

// Íconos de "vida" para el contador del HUD: índice 0 = conejo, 1 = gallina.
const CONTADOR_BYTES: &[u8] = include_bytes!("../assets/contador.png");
const CONTADOR_COLUMNAS: u32 = 2;
const CONTADOR_FILAS: u32 = 1;

// Animación de tropiezo del zorro al chocar con una roca (mientras dura
// `Partida::pausa`), grilla 2x2.
const CAIDA_BYTES: &[u8] = include_bytes!("../assets/caida.png");
const CAIDA_COLUMNAS: u32 = 2;
const CAIDA_FILAS: u32 = 2;

// Celebración de la pantalla de victoria: zorro + gallina + conejo de la
// mano, grilla 2x2, se anima en loop mientras dura la pantalla.
const WINWIN_BYTES: &[u8] = include_bytes!("../assets/winwin.png");
const WINWIN_COLUMNAS: u32 = 2;
const WINWIN_FILAS: u32 = 2;

// Fondo de la partida: alterna día/noche cada METROS_POR_CICLO metros
// recorridos (usa el puntaje, que ya se muestra en "m" en el HUD).
const FONDO_DIA_BYTES: &[u8] = include_bytes!("../assets/fondodia.png");
const FONDO_NOCHE_BYTES: &[u8] = include_bytes!("../assets/fondonoche.jpeg");
const FONDO_HALLOWEEN_BYTES: &[u8] = include_bytes!("../assets/fondohalloween.png");
const METROS_POR_CICLO: f32 = 600.0;
// Secuencia que se repite cada 4 tramos de METROS_POR_CICLO:
// día → noche → día → halloween → día → noche → ...
const SECUENCIA_FONDOS: [Fondo; 4] = [Fondo::Dia, Fondo::Noche, Fondo::Dia, Fondo::Halloween];

#[derive(Clone, Copy)]
enum Fondo {
    Dia,
    Noche,
    Halloween,
}

// Franja de pasto/tierra por donde corre el zorro, tileable horizontalmente.
const PLATAFORMA_BYTES: &[u8] = include_bytes!("../assets/plataforma.png");

/// Lo que el posturógrafo le pasa al juego en cada frame.
pub struct EntradaJuego {
    pub cop_ml: f64,
    // Libres para usar (ej. saltar/agachar con AP) o ignorar; ver src/juego.rs.
    #[allow(dead_code)]
    pub cop_ap: f64,
    pub ancho_cm: f64,
    #[allow(dead_code)]
    pub prof_cm: f64,
    pub conectado: bool,
    /// Hay alguien parado sobre la plataforma. Sin esto el juego arrancaba con
    /// solo estar conectado: el reloj corría y el zorro quedaba clavado en el
    /// centro porque el COP de una plataforma vacía es (0, 0).
    pub en_plataforma: bool,
    pub dt: f32,
    // Opciones del juego, definidas en la zona de configuración (src/config.rs).
    pub duracion_partida_s: f32,
    pub volumen_musica: f32,
    pub volumen_efectos: f32,
}

// Paleta propia del modo juego (independiente de la vista clínica).
const CIELO_ARRIBA: Color32 = Color32::from_rgb(255, 236, 214);
const CIELO_ABAJO: Color32 = Color32::from_rgb(255, 214, 224);
const TEXTO: Color32 = Color32::from_rgb(70, 55, 60);
const ROJO_GOLPE: Color32 = Color32::from_rgb(210, 90, 85);

const VELOCIDAD_INICIAL: f32 = 230.0;
const VELOCIDAD_MAX: f32 = 680.0;
const ACELERACION: f32 = 12.0; // px/s por cada segundo jugado
const INTERVALO_SPAWN_INICIAL: f32 = 1.25;
const INTERVALO_SPAWN_MIN: f32 = 0.45;
const MARGEN_PISTA: f32 = 24.0;

// --- audio (ver assets/musica/CREDITOS.txt por licencias) ---
const MUSICA_MENU: &[u8] = include_bytes!("../assets/musica/menu.ogg");
const MUSICA_JUGANDO: &[u8] = include_bytes!("../assets/musica/jugando.ogg");
const MUSICA_HALLOWEEN: &[u8] = include_bytes!("../assets/musica/halloween.ogg");
const SONIDO_COMER: &[u8] = include_bytes!("../assets/musica/comer.ogg");
const SONIDO_CAIDA: &[u8] = include_bytes!("../assets/musica/caida.ogg");
const SONIDO_VICTORIA: &[u8] = include_bytes!("../assets/musica/victoria.ogg");
const SONIDO_DERROTA: &[u8] = include_bytes!("../assets/musica/derrota.ogg");

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pista {
    Menu,
    Jugando,
    Halloween,
}

/// Sale del audio del juego. Si no hay dispositivo de sonido disponible
/// (`Audio::nueva()` devuelve `None`), el juego sigue andando mudo.
struct Audio {
    _flujo: rodio::OutputStream, // hay que mantenerlo vivo o se corta el sonido
    salida: rodio::OutputStreamHandle,
    musica: Option<rodio::Sink>,
    pista_actual: Option<Pista>,
    // Volúmenes vigentes, sincronizados cada frame con la configuración.
    volumen_musica: f32,
    volumen_efectos: f32,
    /// Audio ya decodificado, indexado por la dirección de sus bytes. Decodificar
    /// un .ogg entero cada vez que hay que repetirlo (o cada vez que suena un
    /// efecto) es caro y se notaba como tironeo en el juego.
    decodificados: std::collections::HashMap<usize, rodio::buffer::SamplesBuffer<i16>>,
}

impl Audio {
    fn nueva() -> Option<Self> {
        let (flujo, salida) = rodio::OutputStream::try_default().ok()?;
        Some(Self {
            _flujo: flujo,
            salida,
            musica: None,
            pista_actual: None,
            volumen_musica: crate::config::defecto::VOLUMEN_MUSICA,
            volumen_efectos: crate::config::defecto::VOLUMEN_EFECTOS,
            decodificados: std::collections::HashMap::new(),
        })
    }

    /// Decodifica una vez y guarda el resultado. `SamplesBuffer` sí es `Clone`,
    /// así que a partir de acá reproducir o repetir el audio es copiar memoria,
    /// sin volver a pasar por el decodificador de Vorbis.
    fn buffer_de(&mut self, bytes: &'static [u8]) -> Option<rodio::buffer::SamplesBuffer<i16>> {
        let clave = bytes.as_ptr() as usize;
        if let Some(buffer) = self.decodificados.get(&clave) {
            return Some(buffer.clone());
        }
        let decodificador = rodio::Decoder::new(std::io::Cursor::new(bytes)).ok()?;
        let canales = decodificador.channels();
        let tasa = decodificador.sample_rate();
        let muestras: Vec<i16> = decodificador.collect();
        let buffer = rodio::buffer::SamplesBuffer::new(canales, tasa, muestras);
        self.decodificados.insert(clave, buffer.clone());
        Some(buffer)
    }

    /// Toma los volúmenes de la configuración y los aplica también a la
    /// pista que ya está sonando, para que el cambio se escuche al toque.
    fn ajustar_volumenes(&mut self, musica: f32, efectos: f32) {
        self.volumen_musica = musica.clamp(0.0, 1.0);
        self.volumen_efectos = efectos.clamp(0.0, 1.0);
        if let Some(sink) = &self.musica {
            sink.set_volume(self.volumen_musica);
        }
    }

    fn bytes_de(pista: Pista) -> &'static [u8] {
        match pista {
            Pista::Menu => MUSICA_MENU,
            Pista::Jugando => MUSICA_JUGANDO,
            Pista::Halloween => MUSICA_HALLOWEEN,
        }
    }

    /// Pone a sonar `pista` en loop si no es ya la que está sonando. El loop
    /// lo maneja rodio (`repeat_infinite`), así que no hay que estar
    /// vigilando cada frame si la pista terminó para volver a ponerla.
    fn poner_pista(&mut self, pista: Pista) {
        if self.pista_actual == Some(pista) && self.musica.as_ref().is_some_and(|s| !s.empty()) {
            return;
        }
        let Some(fuente) = self.buffer_de(Self::bytes_de(pista)) else { return };
        if let Ok(sink) = rodio::Sink::try_new(&self.salida) {
            sink.set_volume(self.volumen_musica);
            sink.append(fuente.repeat_infinite());
            self.musica = Some(sink); // dropea el sink anterior, que corta esa pista solo
            self.pista_actual = Some(pista);
        }
    }

    fn detener_musica(&mut self) {
        self.musica = None; // dropear el sink corta el sonido
        self.pista_actual = None;
    }

    /// Sonido suelto (no-loop) que se reproduce solo y se limpia sola.
    fn reproducir_efecto(&mut self, bytes: &'static [u8]) {
        let Some(fuente) = self.buffer_de(bytes) else { return };
        if let Ok(sink) = rodio::Sink::try_new(&self.salida) {
            sink.set_volume(self.volumen_efectos);
            sink.append(fuente);
            sink.detach();
        }
    }
}

/// Carga (o reutiliza) el dispositivo de audio. `None` si ya se intentó y
/// no hay salida de sonido disponible; en ese caso el juego sigue mudo.
fn obtener_audio<'a>(cache: &'a mut Option<Audio>, intentado: &mut bool) -> Option<&'a mut Audio> {
    if !*intentado {
        *cache = Audio::nueva();
        *intentado = true;
    }
    cache.as_mut()
}

/// El mejor puntaje vive en la carpeta de datos del usuario, la misma que
/// usan las sesiones exportadas (ver `src/datos.rs`). Antes se armaba a mano
/// con `$HOME`, así que en Windows no se guardaba nunca.
fn ruta_mejor_puntaje() -> std::path::PathBuf {
    crate::datos::carpeta_datos().join("mejor_puntaje.txt")
}

fn cargar_mejor_puntaje() -> f32 {
    std::fs::read_to_string(ruta_mejor_puntaje())
        .ok()
        .and_then(|texto| texto.trim().parse::<f32>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0)
        .unwrap_or(0.0)
}

fn guardar_mejor_puntaje(valor: f32) {
    let ruta = ruta_mejor_puntaje();
    let Some(dir) = ruta.parent() else { return };
    if std::fs::create_dir_all(dir).is_ok() {
        let _ = std::fs::write(&ruta, format!("{valor}"));
    }
}

/// Un obstáculo cayendo. `x` y `ancho_frac` están normalizados a la pista
/// jugable (0.0 = borde izquierdo, 1.0 = borde derecho) para que el juego
/// se adapte solo si se redimensiona la ventana.
struct Obstaculo {
    x_frac: f32,
    ancho_frac: f32,
    y_px: f32,
    esquivado: bool,
    variante: usize,
}

/// Bicho atrapable: qué sprite usa, cuántos puntos suma y a qué tamaño
/// relativo al zorro se dibuja.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TipoRecompensa {
    Gallina,
    Conejo,
}

impl TipoRecompensa {
    fn puntos(self) -> f32 {
        match self {
            TipoRecompensa::Gallina => 150.0,
            TipoRecompensa::Conejo => 220.0,
        }
    }

    fn escala_alto(self) -> f32 {
        match self {
            TipoRecompensa::Gallina => 0.85,
            TipoRecompensa::Conejo => 0.8,
        }
    }

    /// Índice del ícono correspondiente en `contador.png`.
    fn indice_contador(self) -> usize {
        match self {
            TipoRecompensa::Conejo => 0,
            TipoRecompensa::Gallina => 1,
        }
    }
}

/// Una recompensa cayendo (gallina o conejo), atrapable para sumar puntos.
/// A diferencia de los obstáculos, no termina la partida: si el zorro la
/// toca, desaparece y suma `tipo.puntos()`.
struct Recompensa {
    tipo: TipoRecompensa,
    x_frac: f32,
    y_px: f32,
    desfase_animacion: f32,
}

/// Todos los sprites del juego, ya cargados. Agruparlos evita una función de
/// dibujo con una docena de parámetros donde es fácil cruzar dos por error.
struct Recursos {
    zorro: SpriteSheet,
    caida: SpriteSheet,
    gallina: SpriteSheet,
    conejo: SpriteSheet,
    rocas: SpriteSheet,
    fondo_dia: SpriteSheet,
    fondo_noche: SpriteSheet,
    fondo_halloween: SpriteSheet,
    plataforma: SpriteSheet,
    contador: SpriteSheet,
    winwin: SpriteSheet,
}

/// Hoja de sprites genérica (grilla `columnas` x `filas`), con carga
/// perezosa y dibujado que preserva la relación de aspecto real de cada
/// celda (una celda no cuadrada no debe "achatar" el personaje).
#[derive(Clone)]
struct SpriteSheet {
    textura: TextureHandle,
    columnas: u32,
    filas: u32,
    celda_px: Vec2,
}

impl SpriteSheet {
    fn frames(&self) -> usize {
        (self.columnas * self.filas) as usize
    }

    fn uv(&self, indice: usize) -> Rect {
        let indice = indice % self.frames();
        let col = (indice as u32 % self.columnas) as f32;
        let fila = (indice as u32 / self.columnas) as f32;
        Rect::from_min_size(
            Pos2::new(col / self.columnas as f32, fila / self.filas as f32),
            Vec2::new(1.0 / self.columnas as f32, 1.0 / self.filas as f32),
        )
    }

    /// Relación ancho/alto de una celda.
    fn aspecto(&self) -> f32 {
        self.celda_px.x / self.celda_px.y
    }

    /// Tamaño en pantalla (ancho, alto) que respeta el aspecto real de la
    /// celda para una altura visual deseada `alto_deseado`.
    fn tamano_para_alto(&self, alto_deseado: f32) -> Vec2 {
        let aspecto = self.celda_px.x / self.celda_px.y;
        Vec2::new(alto_deseado * aspecto, alto_deseado)
    }

    fn dibujar(&self, ui: &Ui, centro: Pos2, alto_deseado: f32, indice: usize, rotacion: f32) {
        self.dibujar_en(ui, centro, self.tamano_para_alto(alto_deseado), indice, rotacion);
    }

    fn dibujar_en(&self, ui: &Ui, centro: Pos2, tamano: Vec2, indice: usize, rotacion: f32) {
        let rect = Rect::from_center_size(centro, tamano);
        Image::from_texture(&self.textura).uv(self.uv(indice)).rotate(rotacion, Vec2::splat(0.5)).paint_at(ui, rect);
    }

    /// Dibuja la celda `indice` cubriendo todo `rect` (recorta el sobrante,
    /// sin deformar), como el `background-size: cover` de CSS.
    fn dibujar_cubriendo(&self, ui: &Ui, rect: Rect, indice: usize) {
        let aspecto_img = self.celda_px.x / self.celda_px.y;
        let aspecto_rect = rect.width() / rect.height();
        let tamano = if aspecto_rect > aspecto_img {
            Vec2::new(rect.width(), rect.width() / aspecto_img)
        } else {
            Vec2::new(rect.height() * aspecto_img, rect.height())
        };
        self.dibujar_en(ui, rect.center(), tamano, indice, 0.0);
    }

    /// Dibuja la celda `indice` en mosaico horizontal (sin estirar cada
    /// baldosa) apoyada en el borde inferior de `rect`, con altura `alto`.
    fn dibujar_tileado(&self, ui: &Ui, rect: Rect, indice: usize, alto: f32) {
        let aspecto = self.celda_px.x / self.celda_px.y;
        let ancho_tile = alto * aspecto;
        if ancho_tile <= 0.5 {
            return;
        }
        let y = rect.bottom() - alto / 2.0;
        let n = (rect.width() / ancho_tile).ceil() as i32 + 1;
        for i in 0..n {
            let x = rect.left() + ancho_tile / 2.0 + i as f32 * ancho_tile;
            self.dibujar_en(ui, Pos2::new(x, y), Vec2::new(ancho_tile, alto), indice, 0.0);
        }
    }
}

/// Carga (o reutiliza del caché) una hoja de sprites.
fn obtener_sprite(
    ui: &Ui,
    cache: &mut Option<SpriteSheet>,
    nombre: &'static str,
    bytes: &[u8],
    columnas: u32,
    filas: u32,
) -> SpriteSheet {
    cache
        .get_or_insert_with(|| {
            let imagen = image::load_from_memory(bytes).expect("sprite del juego inválido").into_rgba8();
            let (ancho, alto) = imagen.dimensions();
            let color_image = ColorImage::from_rgba_unmultiplied([ancho as usize, alto as usize], imagen.as_raw());
            let textura = ui.ctx().load_texture(nombre, color_image, TextureOptions::LINEAR);
            SpriteSheet {
                textura,
                columnas,
                filas,
                celda_px: Vec2::new(ancho as f32 / columnas as f32, alto as f32 / filas as f32),
            }
        })
        .clone()
}

/// Generador pseudoaleatorio mínimo (xorshift64), sin depender de `rand`.
struct Rng(u64);

impl Rng {
    fn nueva() -> Self {
        let semilla = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E3779B97F4A7C15);
        Self(semilla | 1)
    }

    fn siguiente_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn rango(&mut self, min: f32, max: f32) -> f32 {
        let unidad = (self.siguiente_u64() >> 11) as f32 / (1u64 << 53) as f32;
        min + unidad * (max - min)
    }
}

/// Partida en curso. Existe solo mientras se está jugando o mostrando la
/// pantalla de game over; se descarta al reiniciar o al desconectar.
struct Partida {
    fox_x: f32, // -1.0 (izquierda) .. 1.0 (derecha)
    obstaculos: Vec<Obstaculo>,
    recompensas: Vec<Recompensa>,
    tiempo: f32,
    velocidad: f32,
    temporizador_spawn: f32,
    temporizador_recompensa: f32,
    puntaje: f32,
    game_over: bool,
    temporizador_reinicio: f32,
    /// Cuántas veces sonó ya el jingle de ganar/perder (tope REPETICIONES_SONIDO_FIN).
    repeticiones_sonido_fin: u8,
    /// Cuenta regresiva para la próxima repetición de ese jingle.
    temporizador_sonido_fin: f32,
    /// Gallinas/conejos comidos que hacen de "escudo": cada roca que golpea
    /// consume el último de la pila en vez de terminar la partida.
    vidas: Vec<TipoRecompensa>,
    /// Segundos restantes de congelamiento tras perder una vida contra una
    /// roca (placeholder hasta que haya una animación de golpe).
    pausa: f32,
    /// Cuenta regresiva de la partida completa; en 0 se gana.
    tiempo_restante: f32,
    /// Cuánto duraba esta partida al empezar (de la configuración). Se guarda
    /// acá para que cambiar la opción a mitad de partida no altere la que ya
    /// está en curso, y para que la pantalla de victoria diga el tiempo real.
    duracion_s: f32,
    gano: bool,
    /// Polvito bajo las patas del zorro mientras corre (solo estético).
    particulas: Vec<Particula>,
    temporizador_polvo: f32,
    rng: Rng,
}

/// Una mota de polvo bajo el zorro. `jitter_x` es su posición lateral fija
/// (relativa al ancho del zorro), no se mueve por sí sola: la caída y el
/// desvanecido se calculan a partir de cuánta `vida` le queda.
struct Particula {
    jitter_x: f32,
    vida: f32,
    vida_total: f32,
}

/// Segundos de la cuenta atrás en la pantalla de game over antes de
/// reintentar solo.
const REINICIO_SEGUNDOS: f32 = 6.0;

/// Segundos que se congela el juego al perder una vida contra una roca.
const PAUSA_GOLPE_SEGUNDOS: f32 = 0.8;

/// El jingle de ganar/perder se repite (no queda sonando solo una vez ni en
/// loop infinito) hasta este tope, cada INTERVALO_SONIDO_FIN segundos.
const REPETICIONES_SONIDO_FIN: u8 = 3;
const INTERVALO_SONIDO_FIN: f32 = 2.0;

impl Partida {
    /// `duracion_s` viene de la zona de configuración (ver src/config.rs).
    fn nueva(duracion_s: f32) -> Self {
        Self {
            fox_x: 0.0,
            obstaculos: Vec::new(),
            recompensas: Vec::new(),
            tiempo: 0.0,
            velocidad: VELOCIDAD_INICIAL,
            temporizador_spawn: INTERVALO_SPAWN_INICIAL,
            temporizador_recompensa: 2.0,
            puntaje: 0.0,
            game_over: false,
            temporizador_reinicio: REINICIO_SEGUNDOS,
            repeticiones_sonido_fin: 0,
            temporizador_sonido_fin: 0.0,
            vidas: Vec::new(),
            pausa: 0.0,
            tiempo_restante: duracion_s,
            duracion_s,
            gano: false,
            particulas: Vec::new(),
            temporizador_polvo: 0.0,
            rng: Rng::nueva(),
        }
    }
}

/// Estado propio del juego. Vive mientras `modo_juego` esté activo en
/// `PosturografoxApp`; se reinicia solo si el jugador pide "Reintentar".
#[derive(Default)]
pub struct EstadoJuego {
    partida: Option<Partida>,
    puntaje_maximo: f32,
    puntaje_maximo_cargado: bool,
    sprite_zorro: Option<SpriteSheet>,
    sprite_gallina: Option<SpriteSheet>,
    sprite_conejo: Option<SpriteSheet>,
    sprite_rocas: Option<SpriteSheet>,
    sprite_fondo_dia: Option<SpriteSheet>,
    sprite_fondo_noche: Option<SpriteSheet>,
    sprite_fondo_halloween: Option<SpriteSheet>,
    sprite_plataforma: Option<SpriteSheet>,
    sprite_contador: Option<SpriteSheet>,
    sprite_caida: Option<SpriteSheet>,
    sprite_winwin: Option<SpriteSheet>,
    audio: Option<Audio>,
    audio_intentado: bool,
}

/// Dibuja el juego a pantalla completa dentro de `ui`.
/// Devuelve `true` si el jugador pidió salir (volver al modo clínico).
pub fn mostrar(ui: &mut Ui, estado: &mut EstadoJuego, entrada: EntradaJuego) -> bool {
    if !estado.puntaje_maximo_cargado {
        estado.puntaje_maximo = cargar_mejor_puntaje();
        estado.puntaje_maximo_cargado = true;
    }
    let salir_tecla = ui.input(|i| i.key_pressed(Key::Escape));
    let sprites = Recursos {
        zorro: obtener_sprite(ui, &mut estado.sprite_zorro, "zorro_sprite", ZORRO_BYTES, ZORRO_COLUMNAS, ZORRO_FILAS),
        caida: obtener_sprite(ui, &mut estado.sprite_caida, "caida_sprite", CAIDA_BYTES, CAIDA_COLUMNAS, CAIDA_FILAS),
        gallina: obtener_sprite(
            ui,
            &mut estado.sprite_gallina,
            "gallina_sprite",
            GALLINA_BYTES,
            GALLINA_COLUMNAS,
            GALLINA_FILAS,
        ),
        conejo: obtener_sprite(
            ui,
            &mut estado.sprite_conejo,
            "conejo_sprite",
            CONEJO_BYTES,
            CONEJO_COLUMNAS,
            CONEJO_FILAS,
        ),
        rocas: obtener_sprite(ui, &mut estado.sprite_rocas, "rocas_sprite", ROCAS_BYTES, ROCAS_COLUMNAS, ROCAS_FILAS),
        fondo_dia: obtener_sprite(ui, &mut estado.sprite_fondo_dia, "fondo_dia_sprite", FONDO_DIA_BYTES, 1, 1),
        fondo_noche: obtener_sprite(ui, &mut estado.sprite_fondo_noche, "fondo_noche_sprite", FONDO_NOCHE_BYTES, 1, 1),
        fondo_halloween: obtener_sprite(
            ui,
            &mut estado.sprite_fondo_halloween,
            "fondo_halloween_sprite",
            FONDO_HALLOWEEN_BYTES,
            1,
            1,
        ),
        plataforma: obtener_sprite(ui, &mut estado.sprite_plataforma, "plataforma_sprite", PLATAFORMA_BYTES, 1, 1),
        contador: obtener_sprite(
            ui,
            &mut estado.sprite_contador,
            "contador_sprite",
            CONTADOR_BYTES,
            CONTADOR_COLUMNAS,
            CONTADOR_FILAS,
        ),
        winwin: obtener_sprite(
            ui,
            &mut estado.sprite_winwin,
            "winwin_sprite",
            WINWIN_BYTES,
            WINWIN_COLUMNAS,
            WINWIN_FILAS,
        ),
    };
    let mut audio = obtener_audio(&mut estado.audio, &mut estado.audio_intentado);
    if let Some(audio) = audio.as_deref_mut() {
        audio.ajustar_volumenes(entrada.volumen_musica, entrada.volumen_efectos);
    }

    if !entrada.conectado || !entrada.en_plataforma {
        estado.partida = None; // evita que arranque con velocidad "gratis" mientras no hay lecturas
        if let Some(audio) = audio.as_deref_mut() {
            if salir_tecla {
                audio.detener_musica(); // se sale al modo clínico, no dejar sonando
            } else {
                audio.poner_pista(Pista::Menu);
            }
        }
        let motivo = if entrada.conectado {
            ("Súbase a la plataforma para jugar", "El zorro se mueve con su peso")
        } else {
            ("Conecte el posturógrafo para jugar", "En cuanto detecte señal, arranca solo")
        };
        dibujar_espera(ui, &sprites.zorro, motivo.0, motivo.1);
        return salir_tecla;
    }

    let partida = estado.partida.get_or_insert_with(|| Partida::nueva(entrada.duracion_partida_s));

    if partida.game_over {
        if let Some(audio) = audio.as_deref_mut() {
            repetir_sonido_fin(audio, partida, SONIDO_DERROTA, entrada.dt.clamp(0.0, 0.1));
        }
        partida.temporizador_reinicio -= entrada.dt.clamp(0.0, 0.1);
        let (salir_boton, reintentar) = dibujar_game_over(
            ui,
            &sprites.caida,
            partida.puntaje,
            estado.puntaje_maximo,
            partida.temporizador_reinicio.max(0.0),
        );
        if reintentar || partida.temporizador_reinicio <= 0.0 {
            estado.partida = Some(Partida::nueva(entrada.duracion_partida_s));
        }
        ui.ctx().request_repaint();
        return salir_tecla || salir_boton;
    }

    if let Some(audio) = audio.as_deref_mut() {
        let tramo = (partida.puntaje / METROS_POR_CICLO) as usize % SECUENCIA_FONDOS.len();
        let pista = match SECUENCIA_FONDOS[tramo] {
            Fondo::Halloween => Pista::Halloween,
            Fondo::Dia | Fondo::Noche => Pista::Jugando,
        };
        audio.poner_pista(pista);
    }

    // La geometría del área de juego se arma una vez por frame y se le pasa a
    // la simulación; así el movimiento y las colisiones no dependen de egui.
    let area = ui.available_rect_before_wrap();
    let escenario = Escenario {
        ancho: area.width(),
        alto: area.height(),
        aspecto_roca: sprites.rocas.aspecto(),
        aspecto_gallina: sprites.gallina.aspecto(),
        aspecto_conejo: sprites.conejo.aspecto(),
    };

    for sonido in actualizar(partida, &entrada, &escenario) {
        if let Some(audio) = audio.as_deref_mut() {
            audio.reproducir_efecto(match sonido {
                Sonido::Golpe => SONIDO_CAIDA,
                Sonido::Comer => SONIDO_COMER,
            });
        }
    }

    if partida.gano {
        if let Some(audio) = audio.as_deref_mut() {
            repetir_sonido_fin(audio, partida, SONIDO_VICTORIA, entrada.dt.clamp(0.0, 0.1));
        }
        if partida.puntaje > estado.puntaje_maximo {
            estado.puntaje_maximo = partida.puntaje;
            guardar_mejor_puntaje(estado.puntaje_maximo);
        }
        partida.temporizador_reinicio -= entrada.dt.clamp(0.0, 0.1);
        let (salir_boton, reintentar) = dibujar_victoria(
            ui,
            &sprites.winwin,
            partida.puntaje,
            estado.puntaje_maximo,
            partida.temporizador_reinicio.max(0.0),
            partida.duracion_s,
        );
        if reintentar || partida.temporizador_reinicio <= 0.0 {
            estado.partida = Some(Partida::nueva(entrada.duracion_partida_s));
        }
        return salir_tecla || salir_boton;
    }

    let salir_boton = dibujar_partida(ui, &sprites, &escenario, partida);
    if partida.game_over && partida.puntaje > estado.puntaje_maximo {
        estado.puntaje_maximo = partida.puntaje;
        guardar_mejor_puntaje(estado.puntaje_maximo);
    }
    if (salir_tecla || salir_boton)
        && let Some(audio) = audio
    {
        audio.detener_musica(); // se sale al modo clínico, no dejar sonando
    }

    salir_tecla || salir_boton
}

/// Corta la música de fondo (solo la primera vez) y hace sonar `sonido`
/// hasta REPETICIONES_SONIDO_FIN veces, separadas por INTERVALO_SONIDO_FIN
/// segundos, mientras dura la pantalla de game over o victoria.
fn repetir_sonido_fin(audio: &mut Audio, partida: &mut Partida, sonido: &'static [u8], dt: f32) {
    if partida.repeticiones_sonido_fin == 0 {
        audio.detener_musica();
    }
    if partida.repeticiones_sonido_fin >= REPETICIONES_SONIDO_FIN {
        return;
    }
    partida.temporizador_sonido_fin -= dt;
    if partida.temporizador_sonido_fin <= 0.0 {
        audio.reproducir_efecto(sonido);
        partida.repeticiones_sonido_fin += 1;
        partida.temporizador_sonido_fin = INTERVALO_SONIDO_FIN;
    }
}

/// Geometría del área de juego y proporciones de los sprites, en píxeles
/// relativos al borde superior izquierdo de esa área.
///
/// Existe para que la simulación (movimiento y colisiones) no dependa del
/// `Rect` de egui ni de las texturas: con un `Escenario` armado a mano se
/// puede correr una partida entera en un test.
#[derive(Clone, Copy)]
struct Escenario {
    ancho: f32,
    alto: f32,
    /// ancho/alto de una celda del sprite, para calcular el alto a partir del ancho.
    aspecto_roca: f32,
    aspecto_gallina: f32,
    aspecto_conejo: f32,
}

impl Escenario {
    fn ancho_pista(&self) -> f32 {
        (self.ancho - 2.0 * MARGEN_PISTA).max(1.0)
    }

    fn x_de_frac(&self, f: f32) -> f32 {
        MARGEN_PISTA + f * self.ancho_pista()
    }

    fn alto_zorro(&self) -> f32 {
        (self.ancho * 0.14).clamp(60.0, 168.0)
    }

    fn y_zorro(&self) -> f32 {
        self.alto - self.alto_zorro() * 1.4 + 70.0
    }

    fn x_zorro(&self, fox_x: f32) -> f32 {
        self.ancho / 2.0 + fox_x * (self.ancho / 2.0 - MARGEN_PISTA - self.alto_zorro() / 2.0)
    }

    fn radio_zorro(&self) -> f32 {
        self.alto_zorro() * 0.4
    }

    /// Rectángulo que ocupa una roca.
    fn rect_obstaculo(&self, obstaculo: &Obstaculo) -> Rect {
        let ancho = obstaculo.ancho_frac * self.ancho_pista();
        let tamano = Vec2::new(ancho, ancho / self.aspecto_roca);
        Rect::from_center_size(Pos2::new(self.x_de_frac(obstaculo.x_frac), obstaculo.y_px), tamano)
    }

    /// Alto y aspecto con que se dibuja una recompensa.
    fn tamano_recompensa(&self, tipo: TipoRecompensa) -> Vec2 {
        let alto = self.alto_zorro() * tipo.escala_alto();
        let aspecto = match tipo {
            TipoRecompensa::Gallina => self.aspecto_gallina,
            TipoRecompensa::Conejo => self.aspecto_conejo,
        };
        Vec2::new(alto * aspecto, alto)
    }

    fn rect_recompensa(&self, recompensa: &Recompensa) -> Rect {
        let centro = Pos2::new(self.x_de_frac(recompensa.x_frac), recompensa.y_px);
        Rect::from_center_size(centro, self.tamano_recompensa(recompensa.tipo))
    }
}

/// Lo que la simulación quiere que suene en este frame. La simulación no
/// toca el audio: solo dice qué pasó.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sonido {
    Golpe,
    Comer,
}

fn actualizar(partida: &mut Partida, entrada: &EntradaJuego, escenario: &Escenario) -> Vec<Sonido> {
    let dt = entrada.dt.clamp(0.0, 0.1);
    let mut sonidos = Vec::new();

    if partida.pausa > 0.0 {
        partida.pausa = (partida.pausa - dt).max(0.0);
        return sonidos; // congelado: nada se mueve ni spawnea mientras dura el golpe
    }

    partida.tiempo_restante = (partida.tiempo_restante - dt).max(0.0);
    if partida.tiempo_restante <= 0.0 {
        partida.gano = true;
        return sonidos;
    }

    // Normaliza el COP ML al rango de la pista (-1.0 izq .. 1.0 der).
    let mitad_ancho = (entrada.ancho_cm / 2.0).max(1.0);
    let objetivo = ((entrada.cop_ml / mitad_ancho) as f32).clamp(-1.0, 1.0);
    let suavizado = (dt * 10.0).min(1.0);
    partida.fox_x += (objetivo - partida.fox_x) * suavizado;

    partida.tiempo += dt;
    partida.velocidad = (VELOCIDAD_INICIAL + partida.tiempo * ACELERACION).min(VELOCIDAD_MAX);
    partida.puntaje += dt * (10.0 + partida.velocidad * 0.05);

    // Spawn de obstáculos.
    partida.temporizador_spawn -= dt;
    if partida.temporizador_spawn <= 0.0 {
        let ancho_frac = partida.rng.rango(0.06, 0.13);
        let x_frac = partida.rng.rango(ancho_frac / 2.0, 1.0 - ancho_frac / 2.0);
        let variante = (partida.rng.rango(0.0, ROCAS_COLUMNAS as f32) as usize).min(ROCAS_COLUMNAS as usize - 1);
        partida.obstaculos.push(Obstaculo { x_frac, ancho_frac, y_px: -40.0, esquivado: false, variante });
        let intervalo_base = (INTERVALO_SPAWN_INICIAL - partida.tiempo * 0.02).max(INTERVALO_SPAWN_MIN);
        partida.temporizador_spawn = intervalo_base + partida.rng.rango(-0.15, 0.2);
    }

    for obstaculo in &mut partida.obstaculos {
        obstaculo.y_px += partida.velocidad * dt;
    }
    partida.obstaculos.retain(|o| o.y_px < 4000.0);

    // Spawn de recompensas (gallina o conejo, más esporádicas que los obstáculos).
    partida.temporizador_recompensa -= dt;
    if partida.temporizador_recompensa <= 0.0 {
        let tipo = if partida.rng.rango(0.0, 1.0) < 0.5 { TipoRecompensa::Gallina } else { TipoRecompensa::Conejo };
        let x_frac = partida.rng.rango(0.15, 0.85);
        partida.recompensas.push(Recompensa {
            tipo,
            x_frac,
            y_px: -60.0,
            desfase_animacion: partida.rng.rango(0.0, 2.0),
        });
        partida.temporizador_recompensa = partida.rng.rango(2.5, 5.0);
    }

    for recompensa in &mut partida.recompensas {
        recompensa.y_px += partida.velocidad * dt;
    }
    partida.recompensas.retain(|r| r.y_px < 4000.0);

    // Polvito bajo las patas mientras corre (puramente estético).
    partida.temporizador_polvo -= dt;
    if partida.temporizador_polvo <= 0.0 {
        let vida_total = partida.rng.rango(0.35, 0.5);
        partida.particulas.push(Particula { jitter_x: partida.rng.rango(-0.6, 0.6), vida: vida_total, vida_total });
        partida.temporizador_polvo = partida.rng.rango(0.09, 0.16);
    }
    for p in &mut partida.particulas {
        p.vida -= dt;
    }
    partida.particulas.retain(|p| p.vida > 0.0);

    resolver_colisiones(partida, escenario, &mut sonidos);
    sonidos
}

/// Choques con rocas y recolección de recompensas. Antes vivía dentro de la
/// función de dibujo, así que la física dependía del tamaño de la ventana y no
/// se podía testear sin levantar la UI.
fn resolver_colisiones(partida: &mut Partida, escenario: &Escenario, sonidos: &mut Vec<Sonido>) {
    let centro_zorro = Pos2::new(escenario.x_zorro(partida.fox_x), escenario.y_zorro());
    let radio = escenario.radio_zorro();
    let alto_zorro = escenario.alto_zorro();

    let mut golpe_mortal = false;
    let mut vidas_consumidas = 0usize;
    for obstaculo in &mut partida.obstaculos {
        if obstaculo.esquivado {
            continue;
        }
        let roca = escenario.rect_obstaculo(obstaculo);
        if circulo_rect_colisiona(centro_zorro, radio, roca) {
            obstaculo.esquivado = true; // esta roca ya no puede golpear de nuevo
            if partida.vidas.len() > vidas_consumidas {
                vidas_consumidas += 1;
            } else {
                golpe_mortal = true;
            }
        } else if roca.top() > centro_zorro.y + alto_zorro * 0.5 {
            obstaculo.esquivado = true;
            partida.puntaje += 25.0;
        }
    }
    for _ in 0..vidas_consumidas {
        partida.vidas.pop();
        partida.pausa = PAUSA_GOLPE_SEGUNDOS;
        sonidos.push(Sonido::Golpe);
    }
    if golpe_mortal {
        partida.game_over = true;
        partida.temporizador_reinicio = REINICIO_SEGUNDOS;
    }

    let mut i = 0;
    while i < partida.recompensas.len() {
        // La zona de recolección es más chica que el dibujo: el bicho se
        // "come" cuando el zorro lo tapa, no cuando lo roza.
        let caja = escenario.rect_recompensa(&partida.recompensas[i]);
        let zona = Rect::from_center_size(caja.center(), caja.size() * 0.6);
        if circulo_rect_colisiona(centro_zorro, radio, zona) {
            let recompensa = partida.recompensas.remove(i);
            partida.puntaje += recompensa.tipo.puntos();
            partida.vidas.push(recompensa.tipo);
            sonidos.push(Sonido::Comer);
            continue;
        }
        i += 1;
    }
}

/// Dibuja la partida en curso y hace la detección de colisión (que depende
/// del layout real, por eso vive junto al dibujo y no en `actualizar`).
/// Devuelve `true` si se apretó "Salir".
fn dibujar_partida(ui: &mut Ui, sprites: &Recursos, escenario: &Escenario, partida: &Partida) -> bool {
    let Recursos {
        zorro,
        caida: caida_sprite,
        gallina: gallina_sprite,
        conejo: conejo_sprite,
        rocas: rocas_sprite,
        fondo_dia,
        fondo_noche,
        fondo_halloween,
        plataforma,
        contador: contador_sprite,
        ..
    } = sprites;
    let rect = ui.available_rect_before_wrap(); // fijo: el HUD nunca tiembla
    let painter = ui.painter();

    // Sacudida de pantalla mientras dura el tropiezo (se apaga sola con
    // `partida.pausa`). Solo afecta al "mundo" (fondo/pista/personajes),
    // el HUD sigue quieto.
    let sacudida_frac = (partida.pausa / PAUSA_GOLPE_SEGUNDOS).clamp(0.0, 1.0);
    let desplazamiento = if partida.pausa > 0.0 {
        let transcurrido = PAUSA_GOLPE_SEGUNDOS - partida.pausa;
        let amplitud = sacudida_frac * 10.0;
        Vec2::new((transcurrido * 45.0).sin() * amplitud, (transcurrido * 61.0).cos() * amplitud)
    } else {
        Vec2::ZERO
    };
    let mundo = rect.translate(desplazamiento);

    // Ciclo de fondos: día → noche → día → halloween → día → noche → ...
    // (se repite cada 4 tramos de METROS_POR_CICLO metros recorridos, mismo
    // número que ya se muestra en el HUD).
    let tramo = (partida.puntaje / METROS_POR_CICLO) as usize % SECUENCIA_FONDOS.len();
    let fondo_del_tramo = SECUENCIA_FONDOS[tramo];
    let fondo_actual = match fondo_del_tramo {
        Fondo::Dia => fondo_dia,
        Fondo::Noche => fondo_noche,
        Fondo::Halloween => fondo_halloween,
    };
    fondo_actual.dibujar_cubriendo(ui, mundo, 0);

    // Fondos oscuros (noche/halloween) necesitan los números del HUD en
    // blanco para que se lean; de día se mantiene el color oscuro de siempre.
    let color_hud = match fondo_del_tramo {
        Fondo::Dia => TEXTO,
        Fondo::Noche | Fondo::Halloween => Color32::WHITE,
    };

    let alto_plataforma = (rect.height() * 0.14).clamp(50.0, 130.0);
    plataforma.dibujar_tileado(ui, mundo, 0, alto_plataforma);

    // El origen del mundo (con la sacudida aplicada) para llevar las
    // posiciones de la simulación a la pantalla.
    let origen = mundo.min.to_vec2();
    let alto_zorro = escenario.alto_zorro();
    let x_zorro = escenario.x_zorro(partida.fox_x) + origen.x;
    let y_zorro = escenario.y_zorro() + origen.y;

    for obstaculo in &partida.obstaculos {
        let caja = escenario.rect_obstaculo(obstaculo).translate(origen);
        rocas_sprite.dibujar_en(ui, caja.center(), caja.size(), obstaculo.variante, 0.0);
    }

    let fps_recompensa = 10.0;
    for recompensa in &partida.recompensas {
        let sprite = match recompensa.tipo {
            TipoRecompensa::Gallina => gallina_sprite,
            TipoRecompensa::Conejo => conejo_sprite,
        };
        let caja = escenario.rect_recompensa(recompensa).translate(origen);
        let frame = ((partida.tiempo + recompensa.desfase_animacion) * fps_recompensa) as usize;
        sprite.dibujar_en(ui, caja.center(), caja.size(), frame, 0.0);
    }

    // Polvito bajo las patas (dibujado antes que el zorro para que quede detrás).
    for p in &partida.particulas {
        let frac_vida = (p.vida / p.vida_total).clamp(0.0, 1.0); // 1.0 recién nacida, 0.0 apagándose
        let radio = (alto_zorro * 0.09 * frac_vida.sqrt()).max(1.0);
        let caida_px = (1.0 - frac_vida) * alto_zorro * 0.5;
        let x = x_zorro + p.jitter_x * alto_zorro * 0.35;
        let y = y_zorro + alto_zorro * 0.55 + caida_px;
        let alfa = (frac_vida * 130.0) as u8;
        painter.circle_filled(Pos2::new(x, y), radio, Color32::from_rgba_unmultiplied(255, 255, 255, alfa));
    }

    if partida.pausa > 0.0 {
        // Tropiezo: se reemplaza la corrida por la animación de caída,
        // avanzando sus 4 frames a lo largo de toda la pausa.
        let transcurrido = PAUSA_GOLPE_SEGUNDOS - partida.pausa;
        let frame_caida = ((transcurrido / PAUSA_GOLPE_SEGUNDOS) * 4.0) as usize;
        caida_sprite.dibujar(ui, Pos2::new(x_zorro, y_zorro), alto_zorro, frame_caida.min(3), 0.0);
    } else {
        let fps_carrera = 8.0 + partida.velocidad * 0.012;
        let frame = (partida.tiempo * fps_carrera) as usize;
        let inclinacion = (partida.fox_x * 0.28).clamp(-0.4, 0.4);
        zorro.dibujar(ui, Pos2::new(x_zorro, y_zorro), alto_zorro, frame, inclinacion);
    }

    // HUD.
    painter.text(
        Pos2::new(rect.left() + 16.0, rect.top() + 12.0),
        Align2::LEFT_TOP,
        format!("🏃 {} m", partida.puntaje as i32),
        egui::FontId::proportional(26.0),
        color_hud,
    );

    // Vidas: contador "ícono x N" (gallinas arriba, conejos abajo) en vez de
    // un ícono por cada una, para que no se haga una fila interminable.
    let alto_vida = (rect.width() * 0.03).clamp(26.0, 40.0) * 2.0;
    let tamano_vida = contador_sprite.tamano_para_alto(alto_vida);
    let x_vida = rect.left() + 16.0;
    let y_gallinas = rect.top() + 96.0;
    let y_conejos = y_gallinas + alto_vida + 10.0;
    let gallinas_vivas = partida.vidas.iter().filter(|t| matches!(t, TipoRecompensa::Gallina)).count();
    let conejos_vivos = partida.vidas.iter().filter(|t| matches!(t, TipoRecompensa::Conejo)).count();

    for (y, indice, cantidad) in [
        (y_gallinas, TipoRecompensa::Gallina.indice_contador(), gallinas_vivas),
        (y_conejos, TipoRecompensa::Conejo.indice_contador(), conejos_vivos),
    ] {
        contador_sprite.dibujar_en(ui, Pos2::new(x_vida + tamano_vida.x / 2.0, y), tamano_vida, indice, 0.0);
        painter.text(
            Pos2::new(x_vida + tamano_vida.x + 8.0, y),
            Align2::LEFT_CENTER,
            format!("x{cantidad}"),
            egui::FontId::proportional(alto_vida * 0.5),
            color_hud,
        );
    }

    // Reloj de la partida: cuenta regresiva desde DURACION_PARTIDA_SEGUNDOS.
    let segundos_totales = partida.tiempo_restante.ceil().max(0.0) as i32;
    painter.text(
        Pos2::new(rect.right() - 16.0, rect.top() + 12.0),
        Align2::RIGHT_TOP,
        format!("{:02}:{:02}", segundos_totales / 60, segundos_totales % 60),
        egui::FontId::proportional((rect.width() * 0.04).clamp(28.0, 46.0)),
        color_hud,
    );
    painter.text(
        Pos2::new(rect.right() - 16.0, rect.top() + 58.0),
        Align2::RIGHT_TOP,
        "ESC para salir",
        egui::FontId::proportional(14.0),
        color_hud.gamma_multiply(0.7),
    );

    let boton_rect = Rect::from_min_size(Pos2::new(rect.right() - 90.0, rect.bottom() - 44.0), Vec2::new(74.0, 30.0));
    let salir = ui.put(boton_rect, egui::Button::new("Salir")).clicked();

    ui.ctx().request_repaint();
    salir
}

fn circulo_rect_colisiona(centro: Pos2, radio: f32, rect: Rect) -> bool {
    let cx = centro.x.clamp(rect.left(), rect.right());
    let cy = centro.y.clamp(rect.top(), rect.bottom());
    let dx = centro.x - cx;
    let dy = centro.y - cy;
    dx * dx + dy * dy <= radio * radio
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// Usa todo el `rect` disponible (mismo criterio que `dibujar_partida` y
/// `dibujar_game_over`) en vez de apilar todo al medio de la ventana.
fn dibujar_espera(ui: &mut Ui, sprite: &SpriteSheet, titulo: &str, detalle: &str) {
    let rect = ui.available_rect_before_wrap();
    let painter = ui.painter();
    let cx = rect.center().x;
    let alto = rect.height();

    let franjas = 24;
    for i in 0..franjas {
        let t0 = i as f32 / franjas as f32;
        let t1 = (i + 1) as f32 / franjas as f32;
        let color = lerp_color(CIELO_ARRIBA, CIELO_ABAJO, t0);
        let franja = Rect::from_min_max(
            Pos2::new(rect.left(), rect.top() + t0 * alto),
            Pos2::new(rect.right(), rect.top() + t1 * alto),
        );
        painter.rect_filled(franja, 0.0, color);
    }

    let texto_centrado = |texto: &str, y_frac: f32, tamano: f32, color: Color32| {
        painter.text(
            Pos2::new(cx, rect.top() + alto * y_frac),
            Align2::CENTER_CENTER,
            texto,
            egui::FontId::proportional(tamano),
            color,
        );
    };

    let alto_zorro = (alto * 0.32).clamp(140.0, 360.0);
    sprite.dibujar(ui, Pos2::new(cx, rect.top() + alto * 0.38), alto_zorro, 0, 0.0);

    texto_centrado(titulo, 0.63, (alto * 0.042).clamp(20.0, 34.0), TEXTO);
    texto_centrado(detalle, 0.70, (alto * 0.022).clamp(13.0, 17.0), TEXTO.gamma_multiply(0.65));
    texto_centrado(
        "ESC para volver al modo clínico",
        0.90,
        (alto * 0.02).clamp(12.0, 16.0),
        TEXTO.gamma_multiply(0.55),
    );

    ui.ctx().request_repaint();
}

/// Devuelve `(salir, reintentar)`. Usa todo el `rect` disponible (no un
/// bloque apilado al medio) para que se vea bien tanto en ventana chica
/// como maximizada, y muestra una cuenta atrás que reintenta sola.
fn dibujar_game_over(
    ui: &mut Ui,
    caida_sprite: &SpriteSheet,
    puntaje: f32,
    puntaje_maximo: f32,
    segundos_reinicio: f32,
) -> (bool, bool) {
    let rect = ui.available_rect_before_wrap();
    let painter = ui.painter();
    let cx = rect.center().x;
    let alto = rect.height();
    let es_record = puntaje >= puntaje_maximo && puntaje > 0.0;

    // Mismo degradé que la partida, para que la transición no salte.
    let franjas = 24;
    for i in 0..franjas {
        let t0 = i as f32 / franjas as f32;
        let t1 = (i + 1) as f32 / franjas as f32;
        let color = lerp_color(CIELO_ARRIBA, CIELO_ABAJO, t0);
        let franja = Rect::from_min_max(
            Pos2::new(rect.left(), rect.top() + t0 * alto),
            Pos2::new(rect.right(), rect.top() + t1 * alto),
        );
        painter.rect_filled(franja, 0.0, color);
    }

    let texto_centrado = |texto: &str, y_frac: f32, tamano: f32, color: Color32| {
        painter.text(
            Pos2::new(cx, rect.top() + alto * y_frac),
            Align2::CENTER_CENTER,
            texto,
            egui::FontId::proportional(tamano),
            color,
        );
    };

    texto_centrado("💥 PERDIÓ EL EQUILIBRIO", 0.07, (alto * 0.028).clamp(15.0, 22.0), ROJO_GOLPE);

    // Reproduce la caída una vez (los primeros ~0.8s de la pantalla) y se
    // queda en el último frame (tirado, mareado) por el resto de la cuenta
    // atrás.
    let transcurrido = REINICIO_SEGUNDOS - segundos_reinicio;
    let frame_caida = ((transcurrido / 0.8) * 4.0) as usize;
    let alto_zorro = (alto * 0.30).clamp(130.0, 340.0);
    caida_sprite.dibujar(ui, Pos2::new(cx, rect.top() + alto * 0.35), alto_zorro, frame_caida.min(3), 0.0);

    texto_centrado("PUNTAJE", 0.565, (alto * 0.02).clamp(12.0, 16.0), TEXTO.gamma_multiply(0.6));
    texto_centrado(&format!("{} m", puntaje as i32), 0.645, (alto * 0.09).clamp(42.0, 100.0), TEXTO);
    let texto_mejor = if es_record {
        "🏆 ¡Nuevo mejor puntaje!".to_string()
    } else {
        format!("Mejor puntaje: {} m", puntaje_maximo as i32)
    };
    texto_centrado(
        &texto_mejor,
        0.735,
        (alto * 0.026).clamp(15.0, 20.0),
        if es_record { ROJO_GOLPE } else { TEXTO.gamma_multiply(0.7) },
    );

    let (salir, reintentar) = dibujar_pie_fin_partida(ui, rect, cx, alto, segundos_reinicio, ROJO_GOLPE);
    ui.ctx().request_repaint();
    (salir, reintentar)
}

/// Barra de cuenta atrás + botones "Reintentar"/"Salir" que comparten la
/// pantalla de game over y la de victoria.
fn dibujar_pie_fin_partida(
    ui: &mut Ui,
    rect: Rect,
    cx: f32,
    alto: f32,
    segundos_reinicio: f32,
    color_barra: Color32,
) -> (bool, bool) {
    let painter = ui.painter();
    let ancho_barra = (rect.width() * 0.32).clamp(200.0, 420.0);
    let y_barra = rect.top() + alto * 0.83;
    let fraccion = (segundos_reinicio / REINICIO_SEGUNDOS).clamp(0.0, 1.0);
    let fondo_barra = Rect::from_center_size(Pos2::new(cx, y_barra), Vec2::new(ancho_barra, 10.0));
    painter.rect_filled(fondo_barra, 5.0, Color32::from_black_alpha(30));
    let relleno_barra = Rect::from_min_size(fondo_barra.min, Vec2::new(ancho_barra * fraccion, 10.0));
    painter.rect_filled(relleno_barra, 5.0, color_barra.gamma_multiply(0.8));
    painter.text(
        Pos2::new(cx, rect.top() + alto * 0.795),
        Align2::CENTER_CENTER,
        format!("Reintentando en {}s...", segundos_reinicio.ceil().max(0.0) as i32),
        egui::FontId::proportional((alto * 0.02).clamp(12.0, 15.0)),
        TEXTO.gamma_multiply(0.65),
    );

    let boton_ancho = 150.0;
    let boton_alto = 40.0;
    let separacion = 14.0;
    let y_botones = rect.top() + alto * 0.92;
    let rect_reintentar = Rect::from_center_size(
        Pos2::new(cx - (boton_ancho + separacion) / 2.0, y_botones),
        Vec2::new(boton_ancho, boton_alto),
    );
    let rect_salir = Rect::from_center_size(
        Pos2::new(cx + (boton_ancho + separacion) / 2.0, y_botones),
        Vec2::new(boton_ancho * 0.7, boton_alto),
    );

    let reintentar = ui.put(rect_reintentar, egui::Button::new(RichText::new("🔁 Reintentar").size(16.0))).clicked();
    let salir = ui.put(rect_salir, egui::Button::new(RichText::new("Salir").size(15.0))).clicked();
    (salir, reintentar)
}

/// Pantalla de victoria: se llega cuando se acaban los 2 minutos de partida
/// sin perder. Placeholder con el zorro de espaldas (parado) hasta que haya
/// un asset dedicado.
fn dibujar_victoria(
    ui: &mut Ui,
    winwin_sprite: &SpriteSheet,
    puntaje: f32,
    puntaje_maximo: f32,
    segundos_reinicio: f32,
    duracion_partida_s: f32,
) -> (bool, bool) {
    let rect = ui.available_rect_before_wrap();
    let painter = ui.painter();
    let cx = rect.center().x;
    let alto = rect.height();
    let es_record = puntaje >= puntaje_maximo && puntaje > 0.0;

    const DORADO: Color32 = Color32::from_rgb(214, 160, 40);

    let franjas = 24;
    for i in 0..franjas {
        let t0 = i as f32 / franjas as f32;
        let t1 = (i + 1) as f32 / franjas as f32;
        let color = lerp_color(CIELO_ARRIBA, CIELO_ABAJO, t0);
        let franja = Rect::from_min_max(
            Pos2::new(rect.left(), rect.top() + t0 * alto),
            Pos2::new(rect.right(), rect.top() + t1 * alto),
        );
        painter.rect_filled(franja, 0.0, color);
    }

    let texto_centrado = |texto: &str, y_frac: f32, tamano: f32, color: Color32| {
        painter.text(
            Pos2::new(cx, rect.top() + alto * y_frac),
            Align2::CENTER_CENTER,
            texto,
            egui::FontId::proportional(tamano),
            color,
        );
    };

    // El texto sale de la duración real de la partida (configurable), así no
    // puede contradecir a lo que se jugó.
    let titulo = format!("🏆 ¡RESISTIÓ {}!", crate::config::duracion_legible(duracion_partida_s).to_uppercase());
    texto_centrado(&titulo, 0.07, (alto * 0.028).clamp(15.0, 22.0), DORADO);

    // Celebración en loop mientras dura la pantalla.
    let transcurrido = REINICIO_SEGUNDOS - segundos_reinicio;
    let frame_win = (transcurrido * 3.0) as usize;
    let alto_zorro = (alto * 0.30).clamp(130.0, 340.0);
    winwin_sprite.dibujar(ui, Pos2::new(cx, rect.top() + alto * 0.35), alto_zorro, frame_win, 0.0);

    texto_centrado("PUNTAJE", 0.565, (alto * 0.02).clamp(12.0, 16.0), TEXTO.gamma_multiply(0.6));
    texto_centrado(&format!("{} m", puntaje as i32), 0.645, (alto * 0.09).clamp(42.0, 100.0), TEXTO);
    let texto_mejor = if es_record {
        "🏆 ¡Nuevo mejor puntaje!".to_string()
    } else {
        format!("Mejor puntaje: {} m", puntaje_maximo as i32)
    };
    texto_centrado(
        &texto_mejor,
        0.735,
        (alto * 0.026).clamp(15.0, 20.0),
        if es_record { DORADO } else { TEXTO.gamma_multiply(0.7) },
    );

    let (salir, reintentar) = dibujar_pie_fin_partida(ui, rect, cx, alto, segundos_reinicio, DORADO);
    ui.ctx().request_repaint();
    (salir, reintentar)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Escenario de referencia: 1000x700 px y sprites cuadrados, para que las
    /// cuentas de los tests sean fáciles de seguir.
    fn escenario() -> Escenario {
        Escenario { ancho: 1000.0, alto: 700.0, aspecto_roca: 1.0, aspecto_gallina: 1.0, aspecto_conejo: 1.0 }
    }

    fn entrada(dt: f32) -> EntradaJuego {
        EntradaJuego {
            cop_ml: 0.0,
            cop_ap: 0.0,
            ancho_cm: 40.0,
            prof_cm: 40.0,
            conectado: true,
            en_plataforma: true,
            dt,
            duracion_partida_s: 60.0,
            volumen_musica: 0.0,
            volumen_efectos: 0.0,
        }
    }

    /// Pone una roca justo encima del zorro.
    fn roca_sobre_el_zorro(partida: &Partida, esc: &Escenario) -> Obstaculo {
        let x_zorro = esc.x_zorro(partida.fox_x);
        let x_frac = (x_zorro - MARGEN_PISTA) / esc.ancho_pista();
        Obstaculo { x_frac, ancho_frac: 0.1, y_px: esc.y_zorro(), esquivado: false, variante: 0 }
    }

    #[test]
    fn chocar_sin_vidas_termina_la_partida() {
        let esc = escenario();
        let mut partida = Partida::nueva(60.0);
        partida.obstaculos.push(roca_sobre_el_zorro(&partida, &esc));

        let sonidos = actualizar(&mut partida, &entrada(0.016), &esc);

        assert!(partida.game_over, "sin vidas, una roca termina la partida");
        assert!(sonidos.is_empty(), "el jingle de derrota lo maneja la pantalla de game over");
    }

    #[test]
    fn chocar_con_vidas_gasta_una_y_congela_el_juego() {
        let esc = escenario();
        let mut partida = Partida::nueva(60.0);
        partida.vidas.push(TipoRecompensa::Gallina);
        partida.vidas.push(TipoRecompensa::Conejo);
        partida.obstaculos.push(roca_sobre_el_zorro(&partida, &esc));

        let sonidos = actualizar(&mut partida, &entrada(0.016), &esc);

        assert!(!partida.game_over, "con vidas de sobra no se pierde");
        assert_eq!(partida.vidas.len(), 1, "se consume una sola vida");
        assert!(partida.pausa > 0.0, "el golpe congela el juego un momento");
        assert_eq!(sonidos, vec![Sonido::Golpe]);
    }

    #[test]
    fn la_misma_roca_no_puede_golpear_dos_veces() {
        let esc = escenario();
        let mut partida = Partida::nueva(60.0);
        partida.vidas.push(TipoRecompensa::Gallina);
        partida.vidas.push(TipoRecompensa::Gallina);
        partida.obstaculos.push(roca_sobre_el_zorro(&partida, &esc));

        actualizar(&mut partida, &entrada(0.016), &esc);
        partida.pausa = 0.0; // como si ya hubiera pasado el congelamiento
        actualizar(&mut partida, &entrada(0.016), &esc);

        assert_eq!(partida.vidas.len(), 1, "la roca ya golpeó: no puede volver a cobrar");
    }

    #[test]
    fn atrapar_una_recompensa_suma_puntos_y_una_vida() {
        let esc = escenario();
        let mut partida = Partida::nueva(60.0);
        let x_frac = (esc.x_zorro(partida.fox_x) - MARGEN_PISTA) / esc.ancho_pista();
        partida.recompensas.push(Recompensa {
            tipo: TipoRecompensa::Conejo,
            x_frac,
            y_px: esc.y_zorro(),
            desfase_animacion: 0.0,
        });
        let puntaje_previo = partida.puntaje;

        let sonidos = actualizar(&mut partida, &entrada(0.016), &esc);

        assert!(partida.recompensas.is_empty(), "la recompensa atrapada desaparece");
        assert_eq!(partida.vidas, vec![TipoRecompensa::Conejo]);
        assert!(partida.puntaje >= puntaje_previo + TipoRecompensa::Conejo.puntos());
        assert_eq!(sonidos, vec![Sonido::Comer]);
    }

    #[test]
    fn esquivar_una_roca_suma_puntos() {
        let esc = escenario();
        let mut partida = Partida::nueva(60.0);
        // Roca lejos del zorro y ya pasada de largo hacia abajo.
        partida.obstaculos.push(Obstaculo {
            x_frac: 0.05,
            ancho_frac: 0.06,
            y_px: esc.y_zorro() + esc.alto_zorro(),
            esquivado: false,
            variante: 0,
        });
        let puntaje_previo = partida.puntaje;

        actualizar(&mut partida, &entrada(0.016), &esc);

        assert!(partida.obstaculos[0].esquivado);
        assert!(partida.puntaje >= puntaje_previo + 25.0, "esquivarla tiene que premiar");
        assert!(!partida.game_over);
    }

    #[test]
    fn el_juego_queda_congelado_mientras_dura_el_golpe() {
        let esc = escenario();
        let mut partida = Partida::nueva(60.0);
        partida.pausa = PAUSA_GOLPE_SEGUNDOS;
        partida.obstaculos.push(roca_sobre_el_zorro(&partida, &esc));
        let tiempo_previo = partida.tiempo_restante;

        actualizar(&mut partida, &entrada(0.1), &esc);

        assert!(partida.pausa < PAUSA_GOLPE_SEGUNDOS, "la pausa se va agotando");
        assert_eq!(partida.tiempo_restante, tiempo_previo, "el reloj no corre durante el golpe");
        assert!(!partida.game_over, "tampoco se choca mientras está congelado");
    }

    #[test]
    fn el_reloj_en_cero_gana_la_partida() {
        let esc = escenario();
        let mut partida = Partida::nueva(0.05);
        actualizar(&mut partida, &entrada(0.1), &esc);
        assert!(partida.gano);
        assert!(!partida.game_over);
    }

    #[test]
    fn el_zorro_sigue_al_cop_sin_salirse_de_la_pista() {
        let esc = escenario();
        let mut partida = Partida::nueva(60.0);
        let mut e = entrada(0.05);
        e.cop_ml = 100.0; // muy a la derecha, más allá del borde de la plataforma

        for _ in 0..200 {
            actualizar(&mut partida, &e, &esc);
        }

        assert!(partida.fox_x > 0.9, "debería irse hacia la derecha, quedó en {}", partida.fox_x);
        assert!(partida.fox_x <= 1.0, "el movimiento está acotado a la pista");
        let x = esc.x_zorro(partida.fox_x);
        assert!(x < esc.ancho, "el zorro no puede salirse del área de juego");
    }
}
