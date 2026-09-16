//! Modo juego: el zorrito de Posturografox esquiva obstáculos que caen,
//! moviéndose de lado a lado según el COP medio-lateral en vivo. Ver
//! `EntradaJuego` para lo que llega del posturógrafo real en cada frame.

use egui::{
    Align2, Color32, ColorImage, Image, Key, Pos2, Rect, RichText, TextureHandle, TextureOptions,
    Ui, Vec2,
};

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
const ROCAS_BYTES: &[u8] = include_bytes!("../assets/rocas.png");
const ROCAS_COLUMNAS: u32 = 12;
const ROCAS_FILAS: u32 = 1;

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
    pub dt: f32,
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
const SONIDO_FIN: &[u8] = include_bytes!("../assets/musica/fin.ogg");
const VOLUMEN_MUSICA: f32 = 0.35;
const VOLUMEN_EFECTOS: f32 = 0.6;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pista {
    Menu,
    Jugando,
}

/// Sale del audio del juego. Si no hay dispositivo de sonido disponible
/// (`Audio::nueva()` devuelve `None`), el juego sigue andando mudo.
struct Audio {
    _flujo: rodio::OutputStream, // hay que mantenerlo vivo o se corta el sonido
    salida: rodio::OutputStreamHandle,
    musica: Option<rodio::Sink>,
    pista_actual: Option<Pista>,
}

impl Audio {
    fn nueva() -> Option<Self> {
        let (flujo, salida) = rodio::OutputStream::try_default().ok()?;
        Some(Self { _flujo: flujo, salida, musica: None, pista_actual: None })
    }

    fn bytes_de(pista: Pista) -> &'static [u8] {
        match pista {
            Pista::Menu => MUSICA_MENU,
            Pista::Jugando => MUSICA_JUGANDO,
        }
    }

    /// Pone a sonar `pista` en loop si no es ya la que está sonando.
    fn poner_pista(&mut self, pista: Pista) {
        let ya_sonando = self.pista_actual == Some(pista) && self.musica.as_ref().is_some_and(|s| !s.empty());
        if ya_sonando {
            return;
        }
        let intento = (
            rodio::Sink::try_new(&self.salida),
            rodio::Decoder::new(std::io::Cursor::new(Self::bytes_de(pista))),
        );
        if let (Ok(sink), Ok(fuente)) = intento {
            sink.set_volume(VOLUMEN_MUSICA);
            sink.append(fuente);
            self.musica = Some(sink); // dropea el sink anterior, que corta esa pista solo
            self.pista_actual = Some(pista);
        }
    }

    /// Como los `Decoder` no son `Clone` no se puede usar `repeat_infinite`;
    /// en cambio, cada frame se chequea si terminó y se vuelve a poner.
    fn mantener_loop(&mut self) {
        if self.musica.as_ref().is_some_and(|s| s.empty()) {
            if let Some(pista) = self.pista_actual {
                self.pista_actual = None; // fuerza a poner_pista a recargarla
                self.poner_pista(pista);
            }
        }
    }

    fn detener_musica(&mut self) {
        self.musica = None; // dropear el sink corta el sonido
        self.pista_actual = None;
    }

    /// Sonido suelto (no-loop) que se reproduce solo y se limpia sola.
    fn reproducir_efecto(&self, bytes: &'static [u8]) {
        let intento = (rodio::Sink::try_new(&self.salida), rodio::Decoder::new(std::io::Cursor::new(bytes)));
        if let (Ok(sink), Ok(fuente)) = intento {
            sink.set_volume(VOLUMEN_EFECTOS);
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
#[derive(Clone, Copy, PartialEq, Eq)]
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

    /// Tamaño en pantalla (ancho, alto) que respeta el aspecto real de la
    /// celda para una altura visual deseada `alto_deseado`.
    fn tamano_para_alto(&self, alto_deseado: f32) -> Vec2 {
        let aspecto = self.celda_px.x / self.celda_px.y;
        Vec2::new(alto_deseado * aspecto, alto_deseado)
    }

    /// Tamaño en pantalla que respeta el aspecto real de la celda para un
    /// ancho visual deseado `ancho_deseado` (inverso de `tamano_para_alto`).
    fn tamano_para_ancho(&self, ancho_deseado: f32) -> Vec2 {
        let aspecto = self.celda_px.x / self.celda_px.y;
        Vec2::new(ancho_deseado, ancho_deseado / aspecto)
    }

    fn dibujar(&self, ui: &Ui, centro: Pos2, alto_deseado: f32, indice: usize, rotacion: f32) {
        self.dibujar_en(ui, centro, self.tamano_para_alto(alto_deseado), indice, rotacion);
    }

    fn dibujar_en(&self, ui: &Ui, centro: Pos2, tamano: Vec2, indice: usize, rotacion: f32) {
        let rect = Rect::from_center_size(centro, tamano);
        Image::from_texture(&self.textura)
            .uv(self.uv(indice))
            .rotate(rotacion, Vec2::splat(0.5))
            .paint_at(ui, rect);
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
            let imagen = image::load_from_memory(bytes)
                .expect("sprite del juego inválido")
                .into_rgba8();
            let (ancho, alto) = imagen.dimensions();
            let color_image =
                ColorImage::from_rgba_unmultiplied([ancho as usize, alto as usize], imagen.as_raw());
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
    sonido_fin_reproducido: bool,
    rng: Rng,
}

/// Segundos de la cuenta atrás en la pantalla de game over antes de
/// reintentar solo.
const REINICIO_SEGUNDOS: f32 = 6.0;

impl Partida {
    fn nueva() -> Self {
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
            sonido_fin_reproducido: false,
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
    sprite_zorro: Option<SpriteSheet>,
    sprite_gallina: Option<SpriteSheet>,
    sprite_conejo: Option<SpriteSheet>,
    sprite_rocas: Option<SpriteSheet>,
    sprite_fondo_dia: Option<SpriteSheet>,
    sprite_fondo_noche: Option<SpriteSheet>,
    sprite_fondo_halloween: Option<SpriteSheet>,
    sprite_plataforma: Option<SpriteSheet>,
    audio: Option<Audio>,
    audio_intentado: bool,
}

/// Dibuja el juego a pantalla completa dentro de `ui`.
/// Devuelve `true` si el jugador pidió salir (volver al modo clínico).
pub fn mostrar(ui: &mut Ui, estado: &mut EstadoJuego, entrada: EntradaJuego) -> bool {
    let salir_tecla = ui.input(|i| i.key_pressed(Key::Escape));
    let zorro = obtener_sprite(ui, &mut estado.sprite_zorro, "zorro_sprite", ZORRO_BYTES, ZORRO_COLUMNAS, ZORRO_FILAS);
    let gallina = obtener_sprite(
        ui,
        &mut estado.sprite_gallina,
        "gallina_sprite",
        GALLINA_BYTES,
        GALLINA_COLUMNAS,
        GALLINA_FILAS,
    );
    let conejo = obtener_sprite(
        ui,
        &mut estado.sprite_conejo,
        "conejo_sprite",
        CONEJO_BYTES,
        CONEJO_COLUMNAS,
        CONEJO_FILAS,
    );
    let rocas = obtener_sprite(
        ui,
        &mut estado.sprite_rocas,
        "rocas_sprite",
        ROCAS_BYTES,
        ROCAS_COLUMNAS,
        ROCAS_FILAS,
    );
    let fondo_dia = obtener_sprite(ui, &mut estado.sprite_fondo_dia, "fondo_dia_sprite", FONDO_DIA_BYTES, 1, 1);
    let fondo_noche =
        obtener_sprite(ui, &mut estado.sprite_fondo_noche, "fondo_noche_sprite", FONDO_NOCHE_BYTES, 1, 1);
    let fondo_halloween = obtener_sprite(
        ui,
        &mut estado.sprite_fondo_halloween,
        "fondo_halloween_sprite",
        FONDO_HALLOWEEN_BYTES,
        1,
        1,
    );
    let plataforma =
        obtener_sprite(ui, &mut estado.sprite_plataforma, "plataforma_sprite", PLATAFORMA_BYTES, 1, 1);
    let audio = obtener_audio(&mut estado.audio, &mut estado.audio_intentado);

    if !entrada.conectado {
        estado.partida = None; // evita que arranque con velocidad "gratis" mientras no hay lecturas
        if let Some(audio) = audio {
            audio.poner_pista(Pista::Menu);
            audio.mantener_loop();
        }
        dibujar_desconectado(ui, &zorro);
        return salir_tecla;
    }

    let partida = estado.partida.get_or_insert_with(Partida::nueva);

    if partida.game_over {
        if let Some(audio) = audio {
            if !partida.sonido_fin_reproducido {
                audio.detener_musica();
                audio.reproducir_efecto(SONIDO_FIN);
                partida.sonido_fin_reproducido = true;
            }
        }
        partida.temporizador_reinicio -= entrada.dt.clamp(0.0, 0.1);
        let (salir_boton, reintentar) = dibujar_game_over(
            ui,
            &zorro,
            partida.puntaje,
            estado.puntaje_maximo,
            partida.temporizador_reinicio.max(0.0),
        );
        if reintentar || partida.temporizador_reinicio <= 0.0 {
            estado.partida = Some(Partida::nueva());
        }
        ui.ctx().request_repaint();
        return salir_tecla || salir_boton;
    }

    if let Some(audio) = audio {
        audio.poner_pista(Pista::Jugando);
        audio.mantener_loop();
    }

    actualizar(partida, &entrada);
    let salir_boton = dibujar_partida(
        ui, &zorro, &gallina, &conejo, &rocas, &fondo_dia, &fondo_noche, &fondo_halloween, &plataforma,
        partida,
    );
    if partida.game_over && partida.puntaje > estado.puntaje_maximo {
        estado.puntaje_maximo = partida.puntaje;
    }

    salir_tecla || salir_boton
}

fn actualizar(partida: &mut Partida, entrada: &EntradaJuego) {
    let dt = entrada.dt.clamp(0.0, 0.1);

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
        partida.obstaculos.push(Obstaculo {
            x_frac,
            ancho_frac,
            y_px: -40.0,
            esquivado: false,
            variante,
        });
        let intervalo_base =
            (INTERVALO_SPAWN_INICIAL - partida.tiempo * 0.02).max(INTERVALO_SPAWN_MIN);
        partida.temporizador_spawn = intervalo_base + partida.rng.rango(-0.15, 0.2);
    }

    for obstaculo in &mut partida.obstaculos {
        obstaculo.y_px += partida.velocidad * dt;
    }
    partida.obstaculos.retain(|o| o.y_px < 4000.0);

    // Spawn de recompensas (gallina o conejo, más esporádicas que los obstáculos).
    partida.temporizador_recompensa -= dt;
    if partida.temporizador_recompensa <= 0.0 {
        let tipo = if partida.rng.rango(0.0, 1.0) < 0.5 {
            TipoRecompensa::Gallina
        } else {
            TipoRecompensa::Conejo
        };
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
}

/// Dibuja la partida en curso y hace la detección de colisión (que depende
/// del layout real, por eso vive junto al dibujo y no en `actualizar`).
/// Devuelve `true` si se apretó "Salir".
fn dibujar_partida(
    ui: &mut Ui,
    zorro: &SpriteSheet,
    gallina_sprite: &SpriteSheet,
    conejo_sprite: &SpriteSheet,
    rocas_sprite: &SpriteSheet,
    fondo_dia: &SpriteSheet,
    fondo_noche: &SpriteSheet,
    fondo_halloween: &SpriteSheet,
    plataforma: &SpriteSheet,
    partida: &mut Partida,
) -> bool {
    let rect = ui.available_rect_before_wrap();

    // Ciclo de fondos: día → noche → día → halloween → día → noche → ...
    // (se repite cada 4 tramos de METROS_POR_CICLO metros recorridos, mismo
    // número que ya se muestra en el HUD).
    let tramo = (partida.puntaje / METROS_POR_CICLO) as usize % SECUENCIA_FONDOS.len();
    let fondo_actual = match SECUENCIA_FONDOS[tramo] {
        Fondo::Dia => fondo_dia,
        Fondo::Noche => fondo_noche,
        Fondo::Halloween => fondo_halloween,
    };
    fondo_actual.dibujar_cubriendo(ui, rect, 0);

    let alto_plataforma = (rect.height() * 0.14).clamp(50.0, 130.0);
    plataforma.dibujar_tileado(ui, rect, 0, alto_plataforma);

    let painter = ui.painter();

    let ancho_pista = (rect.width() - 2.0 * MARGEN_PISTA).max(1.0);
    let x_de_frac = |f: f32| rect.left() + MARGEN_PISTA + f * ancho_pista;

    let alto_zorro = (rect.width() * 0.14).clamp(60.0, 168.0);
    let y_zorro = rect.bottom() - alto_zorro * 1.4 + 70.0;
    let x_zorro = rect.center().x + partida.fox_x * (rect.width() / 2.0 - MARGEN_PISTA - alto_zorro / 2.0);

    let mut golpe = false;
    for obstaculo in &mut partida.obstaculos {
        let ancho_px = obstaculo.ancho_frac * ancho_pista;
        let x_centro = x_de_frac(obstaculo.x_frac);
        let centro = Pos2::new(x_centro, rect.top() + obstaculo.y_px);
        let tamano = rocas_sprite.tamano_para_ancho(ancho_px);
        let roca_rect = Rect::from_center_size(centro, tamano);
        rocas_sprite.dibujar_en(ui, centro, tamano, obstaculo.variante, 0.0);

        if !obstaculo.esquivado {
            if circulo_rect_colisiona(
                Pos2::new(x_zorro, y_zorro),
                alto_zorro * 0.4,
                roca_rect,
            ) {
                golpe = true;
            } else if roca_rect.top() > y_zorro + alto_zorro * 0.5 {
                obstaculo.esquivado = true;
                partida.puntaje += 25.0;
            }
        }
    }

    // Recompensas: se dibujan, y si el zorro las toca suman puntos y desaparecen.
    let fps_recompensa = 10.0;
    let mut i = 0;
    while i < partida.recompensas.len() {
        let r = &partida.recompensas[i];
        let sprite = match r.tipo {
            TipoRecompensa::Gallina => gallina_sprite,
            TipoRecompensa::Conejo => conejo_sprite,
        };
        let alto = alto_zorro * r.tipo.escala_alto();
        let centro = Pos2::new(x_de_frac(r.x_frac), rect.top() + r.y_px);
        let tamano = sprite.tamano_para_alto(alto);
        let rect_colision = Rect::from_center_size(centro, tamano * 0.6);

        if circulo_rect_colisiona(Pos2::new(x_zorro, y_zorro), alto_zorro * 0.4, rect_colision) {
            partida.puntaje += r.tipo.puntos();
            partida.recompensas.remove(i);
            continue;
        }

        let frame = ((partida.tiempo + r.desfase_animacion) * fps_recompensa) as usize;
        sprite.dibujar(ui, centro, alto, frame, 0.0);
        i += 1;
    }

    let fps_carrera = 8.0 + partida.velocidad * 0.012;
    let frame = (partida.tiempo * fps_carrera) as usize;
    let inclinacion = (partida.fox_x * 0.28).clamp(-0.4, 0.4);
    zorro.dibujar(ui, Pos2::new(x_zorro, y_zorro), alto_zorro, frame, inclinacion);

    // HUD.
    painter.text(
        Pos2::new(rect.left() + 16.0, rect.top() + 12.0),
        Align2::LEFT_TOP,
        format!("🏃 {} m", partida.puntaje as i32),
        egui::FontId::proportional(26.0),
        TEXTO,
    );
    painter.text(
        Pos2::new(rect.right() - 16.0, rect.top() + 12.0),
        Align2::RIGHT_TOP,
        "ESC para salir",
        egui::FontId::proportional(14.0),
        TEXTO.gamma_multiply(0.7),
    );

    let boton_rect = Rect::from_min_size(
        Pos2::new(rect.right() - 90.0, rect.bottom() - 44.0),
        Vec2::new(74.0, 30.0),
    );
    let salir = ui.put(boton_rect, egui::Button::new("Salir")).clicked();

    if golpe {
        partida.game_over = true;
        partida.temporizador_reinicio = REINICIO_SEGUNDOS;
    }

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
fn dibujar_desconectado(ui: &mut Ui, sprite: &SpriteSheet) {
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

    texto_centrado(
        "Conectá el posturógrafo para jugar",
        0.63,
        (alto * 0.042).clamp(20.0, 34.0),
        TEXTO,
    );
    texto_centrado(
        "En cuanto detecte señal, arranca solo",
        0.70,
        (alto * 0.022).clamp(13.0, 17.0),
        TEXTO.gamma_multiply(0.65),
    );
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
    sprite: &SpriteSheet,
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

    texto_centrado("💥 PERDISTE EL EQUILIBRIO", 0.07, (alto * 0.028).clamp(15.0, 22.0), ROJO_GOLPE);

    let alto_zorro = (alto * 0.30).clamp(130.0, 340.0);
    sprite.dibujar(ui, Pos2::new(cx, rect.top() + alto * 0.35), alto_zorro, 2, 0.0);

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

    // Cuenta atrás para reintentar solo.
    let ancho_barra = (rect.width() * 0.32).clamp(200.0, 420.0);
    let y_barra = rect.top() + alto * 0.83;
    let fraccion = (segundos_reinicio / REINICIO_SEGUNDOS).clamp(0.0, 1.0);
    let fondo_barra = Rect::from_center_size(Pos2::new(cx, y_barra), Vec2::new(ancho_barra, 10.0));
    painter.rect_filled(fondo_barra, 5.0, Color32::from_black_alpha(30));
    let relleno_barra = Rect::from_min_size(fondo_barra.min, Vec2::new(ancho_barra * fraccion, 10.0));
    painter.rect_filled(relleno_barra, 5.0, ROJO_GOLPE.gamma_multiply(0.8));
    texto_centrado(
        &format!("Reintentando en {}s...", segundos_reinicio.ceil().max(0.0) as i32),
        0.795,
        (alto * 0.02).clamp(12.0, 15.0),
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

    let reintentar = ui
        .put(rect_reintentar, egui::Button::new(RichText::new("🔁 Reintentar").size(16.0)))
        .clicked();
    let salir = ui.put(rect_salir, egui::Button::new(RichText::new("Salir").size(15.0))).clicked();

    ui.ctx().request_repaint();
    (salir, reintentar)
}
