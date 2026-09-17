//! Estado y UI de Posturografox: gráfico COP (cartesiano, con trazo y
//! puntos espaciados) + gráfico movimiento-tiempo, calibración por canal
//! (offset+ganancia) y detección automática de subida/bajada de la
//! plataforma. Ver `serial_link.rs` para la lectura del puerto serie.
//!
//! El COP de una plataforma rectangular de 4 celdas se calcula como:
//!   valor_i = (crudo_i - offset_i) * ganancia_i        (i = fd, fi, bd, bi)
//!   COP_ml (medio-lateral, + = derecha) = ((fd+bd)-(fi+bi))/suma * ancho/2
//!   COP_ap (antero-posterior, + = frente) = ((fd+fi)-(bd+bi))/suma * profundidad/2

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use egui::Color32;
use egui_plot::{HLine, Legend, Line, MarkerShape, Plot, PlotBounds, PlotPoints, Points, Polygon, VLine};

use crate::calibracion::{self, Asistente};
use crate::config::{self, Config, Tema};
use crate::descubrimiento::{self, EventoDescubrimiento};
use crate::estabilometria::{
    Acumulador, Condicion, MetricasBalance, Superficie, ajustar_elipse95, calcular_metricas, cociente_area,
};
use crate::exportar::{exportar_csv, exportar_limites};
use crate::filtro::filtrar_registro;
use crate::historial;
use crate::informe;
use crate::juego;
use crate::limites;
use crate::serial_link::{ConexionSerie, EventoSerie, Muestra, puertos_usables};
use crate::simulador::Simulador;
use crate::transporte::Transporte;

/// Única fuente de verdad de la versión: la de `Cargo.toml`. Se muestra en el
/// título de la ventana y en la barra de estado.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Cuánto se espera entre reintentos de reconexión.
const ESPERA_RECONEXION: Duration = Duration::from_millis(1500);
/// Tras estos reintentos fallidos en el mismo puerto, se vuelve a buscar.
const INTENTOS_ANTES_DE_BUSCAR: u32 = 4;

/// Reconexión pendiente tras una caída que nadie pidió.
struct Reconexion {
    puerto: String,
    intentos: u32,
    proximo: Instant,
}

/// Ritmo de repintado según lo que esté pasando: 30 fps mientras llegan
/// muestras, más lento cuando no hay nada que mostrar.
const ESPERA_REPINTADO_ACTIVO: Duration = Duration::from_millis(33);
const ESPERA_REPINTADO_CONECTADO: Duration = Duration::from_millis(200);
const ESPERA_REPINTADO_OCIOSO: Duration = Duration::from_millis(500);

const MAX_MUESTRAS_TIEMPO: usize = 8_000;
const MAX_PUNTOS_TRAZO: usize = 20_000;

// ── Paleta: pasteles contrastantes sobre fondo claro (look clínico) ─────────
const AZUL: Color32 = Color32::from_rgb(90, 149, 210); // trazo COP / curva ML / conexión
const NARANJA: Color32 = Color32::from_rgb(240, 165, 100); // curva AP / firmware
const CORAL: Color32 = Color32::from_rgb(222, 118, 112); // punto COP actual / desconectar
const LILA: Color32 = Color32::from_rgb(168, 146, 214); // elipse de confianza 95% / detección
const VERDE: Color32 = Color32::from_rgb(120, 178, 140); // plataforma / calibración
const GUIA: Color32 = Color32::from_gray(180); // líneas de referencia en 0,0
const ROSA_JUEGO: Color32 = Color32::from_rgb(214, 130, 176); // acento del modo juego
const AMARILLO: Color32 = Color32::from_rgb(216, 186, 90); // acento del ejercicio de límites de estabilidad

/// Las 4 condiciones del CTSIB en el orden clásico en que se suelen tomar.
const PASOS_CTSIB: [(Superficie, Condicion); 4] = [
    (Superficie::Firme, Condicion::OjosAbiertos),
    (Superficie::Firme, Condicion::OjosCerrados),
    (Superficie::Espuma, Condicion::OjosAbiertos),
    (Superficie::Espuma, Condicion::OjosCerrados),
];

// ── Superficies: fondo tipo "dashboard" + tarjetas con sombra ───────────────
// Los acentos (azul, naranja, ...) se leen bien sobre cualquiera de los tres
// temas; lo que cambia son las superficies y los `Visuals` de egui.
const LIENZO_CLARO: Color32 = Color32::from_rgb(235, 238, 242);
const TARJETA_CLARA: Color32 = Color32::from_rgb(252, 253, 254);
const LIENZO_OSCURO: Color32 = Color32::from_rgb(24, 27, 32);
const TARJETA_OSCURA: Color32 = Color32::from_rgb(34, 38, 45);

/// Colores de fondo (lienzo, tarjeta) de cada tema.
fn superficies(tema: Tema) -> (Color32, Color32) {
    match tema {
        Tema::Claro => (LIENZO_CLARO, TARJETA_CLARA),
        Tema::Oscuro => (LIENZO_OSCURO, TARJETA_OSCURA),
        // Alto contraste: negro puro contra los acentos, sin grises intermedios
        // que se pierdan en una pantalla mala o con poca visión.
        Tema::AltoContraste => (Color32::BLACK, Color32::from_rgb(12, 12, 12)),
    }
}

/// `Visuals` de egui que acompañan al tema (texto, botones, bordes).
fn visuales(tema: Tema) -> egui::Visuals {
    let mut visuals = if tema.es_oscuro() { egui::Visuals::dark() } else { egui::Visuals::light() };
    let (lienzo, tarjeta) = superficies(tema);
    visuals.panel_fill = lienzo;
    visuals.window_fill = tarjeta;
    visuals.extreme_bg_color = tarjeta;
    if tema == Tema::AltoContraste {
        visuals.override_text_color = Some(Color32::WHITE);
        visuals.widgets.noninteractive.bg_stroke.color = Color32::from_gray(160);
    }
    visuals
}

fn sombra_tarjeta() -> egui::Shadow {
    egui::Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha(22) }
}

/// Tarjeta con acento de color por categoría: agrupa controles relacionados
/// en vez de tirarlos todos en una única fila (look "tablero de instrumentos"
/// en lugar de una barra de widgets sin jerarquía visual).
fn tarjeta(ui: &mut egui::Ui, titulo: &str, acento: Color32, contenido: impl FnOnce(&mut egui::Ui)) {
    let fondo = ui.visuals().window_fill;
    egui::Frame::new()
        .fill(fondo)
        .stroke(egui::Stroke::new(1.2, acento.gamma_multiply(0.55)))
        .corner_radius(10.0)
        .shadow(sombra_tarjeta())
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(titulo).small().strong().color(acento.gamma_multiply(0.7)));
                ui.add_space(3.0);
                ui.horizontal(|ui| contenido(ui));
            });
        });
}

/// Interpola en sRGB sin premultiplicar: `Color32` guarda alpha premultiplicado
/// internamente, así que interpolar `.r()/.g()/.b()` directo daría un degradé
/// incorrecto (mezclaría color y opacidad). `to_srgba_unmultiplied` separa ambos.
fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let [ar, ag, ab, aa] = a.to_srgba_unmultiplied();
    let [br, bg, bb, ba] = b.to_srgba_unmultiplied();
    let ch = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(ch(ar, br), ch(ag, bg), ch(ab, bb), ch(aa, ba))
}

/// Escala en la que el firmware manda los datos.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModoFirmware {
    /// Cuentas del ADC tal cual salen del HX711.
    Crudo,
    /// Ya con la tara y la calibración del firmware aplicadas.
    Calibrado,
}

impl ModoFirmware {
    /// Lee el modo del mensaje `# Modo: <crudo|calibrado>` que manda el
    /// firmware al conectar y cada vez que se alterna con el comando 'c'.
    pub fn desde_mensaje(mensaje: &str) -> Option<Self> {
        let resto = mensaje.trim().strip_prefix("Modo:")?.trim();
        match resto {
            "crudo" => Some(ModoFirmware::Crudo),
            "calibrado" => Some(ModoFirmware::Calibrado),
            _ => None,
        }
    }

    pub fn etiqueta(self) -> &'static str {
        match self {
            ModoFirmware::Crudo => "crudo",
            ModoFirmware::Calibrado => "calibrado",
        }
    }
}

/// En qué etapa está el ensayo que se está tomando.
enum ProgresoEnsayo {
    /// Descarte inicial: la persona se acaba de subir y se acomoda.
    Acomodando(f64),
    /// Registrando con duración fija.
    Grabando { restante_s: f64, fraccion: f64 },
    /// Registrando sin corte automático (duración fija desactivada).
    Libre(f64),
    /// Se cumplió la duración: el resultado ya está guardado.
    Completo,
}

/// Fecha legible a partir de un epoch en segundos, sin depender de una
/// biblioteca de calendario: conversión civil desde días de Unix.
fn fecha_legible(epoch_s: u64) -> String {
    let dias = (epoch_s / 86_400) as i64;
    let segundos_del_dia = epoch_s % 86_400;
    // Algoritmo de Howard Hinnant (civil_from_days).
    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let anio = if m <= 2 { y + 1 } else { y };
    format!("{anio:04}-{m:02}-{d:02} {:02}:{:02}", segundos_del_dia / 3600, (segundos_del_dia % 3600) / 60)
}

/// Abre un archivo con la aplicación que el sistema tenga asociada. Si no se
/// puede (sin entorno gráfico, sin handler), no pasa nada: el archivo quedó
/// escrito igual y la barra de estado muestra su ruta.
fn abrir_en_el_sistema(ruta: &std::path::Path) {
    let comando = if cfg!(target_os = "windows") {
        ("cmd", vec!["/C", "start", ""])
    } else if cfg!(target_os = "macos") {
        ("open", vec![])
    } else {
        ("xdg-open", vec![])
    };
    let mut proceso = std::process::Command::new(comando.0);
    proceso.args(comando.1);
    proceso.arg(ruta);
    let _ = proceso.spawn();
}

/// En cuántos tramos se parte el trazo para el degradé de antigüedad.
const TRAMOS_TRAZO: usize = 24;
/// Color de la cola del trazo (lo más viejo), casi transparente.
const COLOR_TRAZO_VIEJO: Color32 = Color32::from_rgba_premultiplied(9, 15, 21, 25);

/// Parte el trazo en `tramos` pedazos consecutivos, repitiendo el último
/// punto de cada uno al principio del siguiente para que no queden huecos.
fn segmentar_trazo(xs: &[f64], ys: &[f64], tramos: usize) -> Vec<Vec<[f64; 2]>> {
    let n = xs.len().min(ys.len());
    if n < 2 || tramos == 0 {
        return Vec::new();
    }
    let por_tramo = n.div_ceil(tramos).max(2);
    let mut salida = Vec::new();
    let mut inicio = 0;
    while inicio + 1 < n {
        let fin = (inicio + por_tramo).min(n);
        salida.push((inicio..fin).map(|i| [xs[i], ys[i]]).collect());
        inicio = fin - 1; // comparte el punto de unión con el tramo siguiente
    }
    salida
}

/// Dibuja la plataforma vista desde arriba, con la celda del paso actual
/// resaltada y una marca en las ya capturadas. Ver dónde va la masa evita
/// tener que traducir mentalmente "BD" a una esquina.
fn dibujar_plataforma(ui: &mut egui::Ui, asistente: &Asistente) {
    const LADO: f32 = 190.0;
    let (respuesta, painter) = ui.allocate_painter(egui::vec2(LADO, LADO), egui::Sense::hover());
    let rect = respuesta.rect;
    let celda = rect.width() / 2.0;
    let objetivo = match asistente.paso {
        calibracion::Paso::Celda(i) => Some(i),
        _ => None,
    };
    let capturadas = asistente.capturadas();

    // Orden en pantalla: frontal arriba, derecha a la derecha.
    // índices: 0=FD, 1=FI, 2=BD, 3=BI
    for (indice, (fila, columna)) in [(0, 1), (0, 0), (1, 1), (1, 0)].into_iter().enumerate() {
        let esquina = egui::pos2(rect.left() + columna as f32 * celda, rect.top() + fila as f32 * celda);
        let caja = egui::Rect::from_min_size(esquina, egui::vec2(celda, celda)).shrink(3.0);
        let es_objetivo = objetivo == Some(indice);
        let fondo = if es_objetivo {
            AMARILLO.gamma_multiply(0.35)
        } else if capturadas[indice] {
            VERDE.gamma_multiply(0.2)
        } else {
            ui.visuals().faint_bg_color
        };
        let borde = if es_objetivo { AMARILLO } else { Color32::from_gray(150) };
        painter.rect_filled(caja, 6.0, fondo);
        painter.rect_stroke(
            caja,
            6.0,
            egui::Stroke::new(if es_objetivo { 2.5 } else { 1.0 }, borde),
            egui::StrokeKind::Inside,
        );

        let marca = if capturadas[indice] { "✓ " } else { "" };
        painter.text(
            caja.center(),
            egui::Align2::CENTER_CENTER,
            format!("{marca}{}", calibracion::ETIQUETAS_CORTAS[indice]),
            egui::FontId::proportional(17.0),
            ui.visuals().text_color(),
        );
        if es_objetivo {
            // El patrón, a escala aproximada, sobre la celda que toca.
            painter.circle_filled(caja.center() + egui::vec2(0.0, 22.0), 13.0, AMARILLO);
            painter.text(
                caja.center() + egui::vec2(0.0, 22.0),
                egui::Align2::CENTER_CENTER,
                "kg",
                egui::FontId::proportional(11.0),
                Color32::BLACK,
            );
        }
    }
    painter.text(
        egui::pos2(rect.center().x, rect.top() - 2.0),
        egui::Align2::CENTER_BOTTOM,
        "frente",
        egui::FontId::proportional(11.0),
        Color32::from_gray(140),
    );
}

/// Lecturas en vivo de las 4 celdas: valor crudo y, si ya hay cero, cuánto
/// subió respecto de él. Es lo que permite ver que la masa está haciendo algo
/// antes de capturar.
fn tabla_celdas(ui: &mut egui::Ui, asistente: &Asistente) {
    let lectura = asistente.lectura();
    let delta = asistente.delta_en_vivo();
    ui.label(egui::RichText::new("LECTURA EN VIVO").small().strong().color(Color32::from_gray(130)));
    egui::Grid::new("grid_celdas_calibracion").num_columns(3).spacing([14.0, 4.0]).show(ui, |ui| {
        ui.label("");
        ui.label(egui::RichText::new("cuentas").small());
        ui.label(egui::RichText::new("vs. cero").small());
        ui.end_row();
        for i in 0..calibracion::N_CELDAS {
            ui.label(egui::RichText::new(calibracion::ETIQUETAS_CORTAS[i]).strong());
            ui.monospace(format!("{:>10.0}", lectura[i]));
            match delta {
                Some(d) => {
                    let color = if d[i] > 0.0 { VERDE } else { Color32::from_gray(140) };
                    ui.colored_label(color, egui::RichText::new(format!("{:+.0}", d[i])).monospace());
                }
                None => {
                    ui.label(egui::RichText::new("—").monospace());
                }
            }
            ui.end_row();
        }
        if let Some(d) = delta {
            ui.label(egui::RichText::new("total").strong());
            ui.label("");
            let total: f64 = d.iter().sum();
            ui.colored_label(
                if total > 0.0 { VERDE } else { Color32::from_gray(140) },
                egui::RichText::new(format!("{total:+.0}")).monospace().strong(),
            );
            ui.end_row();
        }
    });
}

/// Grupos de controles de la barra superior.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pestana {
    /// Paciente, CTSIB y reparto de peso: lo que se usa mientras se evalúa.
    Examen,
    /// Conexión, firmware, calibración y trazo.
    Dispositivo,
    /// Límites de estabilidad y modo juego.
    Ejercicios,
}

fn empujar_acotado(buf: &mut VecDeque<f64>, valor: f64, max: usize) {
    buf.push_back(valor);
    if buf.len() > max {
        buf.pop_front();
    }
}

pub struct PosturografoxApp {
    // Todas las opciones ajustables del programa (ver src/config.rs)
    config: Config,
    mostrar_config: bool,

    // Conexión
    puertos: Vec<String>,
    puerto_seleccionado: Option<String>,
    conexion: Option<Box<dyn Transporte>>,
    estado: String,
    // Búsqueda automática del puerto al arrancar (ver src/descubrimiento.rs)
    descubrimiento: Option<mpsc::Receiver<EventoDescubrimiento>>,
    /// Reintento automático tras una desconexión no pedida (ver `reconectar`).
    reconexion: Option<Reconexion>,

    // Calibración: offset (tara por software); la ganancia por canal vive en `config`
    offset: [f64; 4],
    /// Asistente de calibración con masa conocida, mientras está abierto.
    asistente: Option<Asistente>,
    buffer_crudo: VecDeque<[f64; 4]>,
    buffer_arriba: VecDeque<[f64; 4]>, // solo muestras ya sobre el umbral

    /// En qué escala está mandando los datos el firmware: crudo (cuentas del
    /// HX711) o calibrado. Lo informa el propio firmware al conectar, así un
    /// cambio de modo no pasa desapercibido y deja los umbrales sin sentido.
    modo_firmware: Option<ModoFirmware>,

    /// Muestras que el firmware generó y nunca llegaron (línea corrupta o
    /// buffer lleno). Se muestra en la barra de estado: si sube, la señal
    /// no está completa y las métricas de velocidad quedan subestimadas.
    muestras_perdidas: u64,

    // Detección automática de subida/bajada
    ocupado: bool,
    /// Tiempo (del reloj del dispositivo) en que la persona se subió. Con él
    /// se descuenta el descarte inicial y se mide la duración del ensayo.
    t_subida: Option<f64>,
    /// El ensayo de duración fija ya se cerró: no se vuelve a grabar hasta
    /// que la persona se baje y se suba otra vez.
    ensayo_cerrado: bool,
    contador_arriba: u32,
    contador_abajo: u32,

    // Buffers de graficado
    t_buf: VecDeque<f64>,
    ml_buf: VecDeque<f64>,
    ap_buf: VecDeque<f64>,
    trazo_x: VecDeque<f64>,
    trazo_y: VecDeque<f64>,
    ultimo_ml: f64,
    ultimo_ap: f64,
    ultimos_pct: [f64; 4], // % de carga por celda (fd,fi,bd,bi), para biofeedback en vivo
    /// Peso medido sobre la plataforma. Solo tiene sentido con calibración en
    /// kg; sin ella queda en 0 y no se muestra.
    peso_kg: f64,

    // Registro de sesión: sin límite mientras dura (a diferencia de los
    // buffers de arriba, que son ventanas acotadas solo para dibujar).
    sesion_actual: Vec<[f64; 3]>, // [t_s, cop_ml_cm, cop_ap_cm]
    /// Métricas de la sesión en curso, actualizadas muestra a muestra: el
    /// panel en vivo se repinta a 30 fps y recalcular todo el registro en cada
    /// frame es O(n) sobre una serie que no para de crecer.
    acumulador: Acumulador,
    ultimo_registro: Vec<[f64; 3]>,
    ultima_sesion: Option<MetricasBalance>,
    ultima_condicion: Condicion,
    ultima_superficie: Superficie,

    // Examen: identificación + condición/superficie (CTSIB: las 4 combinaciones)
    paciente: String,
    condicion: Condicion,
    superficie: Superficie,
    resultados_ctsib: HashMap<(Superficie, Condicion), MetricasBalance>,
    // Solo se graba como resultado del CTSIB si esto está armado explícitamente
    // (botón "Iniciar prueba"): pararse en la plataforma sin armar nada sigue
    // funcionando para mirar el COP en vivo, pero no pisa el paso del examen.
    ctsib_armado: bool,

    /// Historial de sesiones en disco (ver src/historial.rs). Se carga al
    /// abrir la ventana y se refresca al archivar una sesión nueva.
    historial: Vec<historial::Sesion>,
    mostrar_historial: bool,

    // Ejercicio de límites de estabilidad (ver src/limites.rs)
    ejercicio: limites::EjercicioLimites,

    /// Qué grupo de tarjetas se está mostrando.
    pestana: Pestana,
    /// Vista para el paciente: solo el COP, a pantalla completa, sin
    /// controles ni números que distraigan del biofeedback.
    modo_paciente: bool,
    /// Último tema aplicado a egui, para no reconstruir los `Visuals` en
    /// cada frame.
    tema_aplicado: Option<Tema>,

    // Modo juego (ver src/juego.rs)
    modo_juego: bool,
    estado_juego: juego::EstadoJuego,
}

impl Default for PosturografoxApp {
    fn default() -> Self {
        let puertos = puertos_usables();
        let puerto_seleccionado = puertos.first().cloned();
        let config = Config::default();
        Self {
            mostrar_config: false,

            puertos,
            puerto_seleccionado,
            conexion: None,
            estado: "Buscando posturógrafo...".to_string(),
            descubrimiento: Some(descubrimiento::iniciar()),
            reconexion: None,

            offset: [0.0; 4],
            asistente: None,
            buffer_crudo: VecDeque::with_capacity(config.muestras_tara),
            buffer_arriba: VecDeque::with_capacity(config.muestras_tara),

            modo_firmware: None,
            muestras_perdidas: 0,

            ocupado: false,
            t_subida: None,
            ensayo_cerrado: false,
            contador_arriba: 0,
            contador_abajo: 0,

            t_buf: VecDeque::with_capacity(MAX_MUESTRAS_TIEMPO),
            ml_buf: VecDeque::with_capacity(MAX_MUESTRAS_TIEMPO),
            ap_buf: VecDeque::with_capacity(MAX_MUESTRAS_TIEMPO),
            trazo_x: VecDeque::with_capacity(MAX_PUNTOS_TRAZO),
            trazo_y: VecDeque::with_capacity(MAX_PUNTOS_TRAZO),
            config,
            ultimo_ml: 0.0,
            ultimo_ap: 0.0,
            ultimos_pct: [25.0; 4],
            peso_kg: 0.0,

            sesion_actual: Vec::new(),
            acumulador: Acumulador::nuevo(),
            ultimo_registro: Vec::new(),
            ultima_sesion: None,
            ultima_condicion: Condicion::default(),
            ultima_superficie: Superficie::default(),

            paciente: String::new(),
            condicion: Condicion::default(),
            superficie: Superficie::default(),
            resultados_ctsib: HashMap::new(),
            ctsib_armado: false,

            historial: Vec::new(),
            mostrar_historial: false,

            ejercicio: limites::EjercicioLimites::default(),

            pestana: Pestana::Examen,
            modo_paciente: false,
            tema_aplicado: None,
            modo_juego: false,
            estado_juego: juego::EstadoJuego::default(),
        }
    }
}

impl PosturografoxApp {
    fn alternar_conexion(&mut self) {
        if self.conexion.is_some() {
            self.conexion = None; // Drop detiene el hilo lector
            self.reconexion = None; // desconexión pedida: no reintentar sola
            self.estado = "Desconectado".to_string();
        } else if let Some(puerto) = self.puerto_seleccionado.clone() {
            self.reconexion = None;
            match ConexionSerie::conectar(&puerto) {
                Ok(c) => {
                    self.conexion = Some(Box::new(c));
                    self.estado = format!("Conectando a {puerto}...");
                }
                Err(e) => self.estado = format!("Error al conectar: {e}"),
            }
        } else {
            self.estado = "Seleccione un puerto primero".to_string();
        }
    }

    /// Conecta el posturógrafo simulado (ver src/simulador.rs). No hay
    /// reintentos ni descubrimiento: no hay hardware detrás.
    pub fn conectar_simulador(&mut self) {
        self.reconexion = None;
        self.descubrimiento = None;
        self.conexion = Some(Box::new(Simulador::iniciar()));
        self.estado = "Simulador conectado".to_string();
    }

    /// Programa el reintento automático tras una caída no pedida (el cable se
    /// soltó, el ESP32 se reinició). `motivo` se muestra en la barra de estado.
    fn programar_reconexion(&mut self, motivo: &str) {
        let Some(puerto) = self.puerto_seleccionado.clone() else { return };
        let intentos = self.reconexion.as_ref().map_or(0, |r| r.intentos);
        self.reconexion = Some(Reconexion { puerto, intentos, proximo: Instant::now() + ESPERA_RECONEXION });
        self.estado = format!("{motivo}: reintentando conexión...");
    }

    /// Reintenta la conexión cuando toca. Tras varios fallos seguidos vuelve a
    /// buscar el puerto: al reenumerar el USB, el dispositivo puede aparecer
    /// con otro nombre (ttyACM0 -> ttyACM1, COM3 -> COM4).
    fn reconectar(&mut self) {
        let Some(pendiente) = &self.reconexion else { return };
        if Instant::now() < pendiente.proximo {
            return;
        }
        let puerto = pendiente.puerto.clone();
        let intentos = pendiente.intentos + 1;

        if let Ok(conexion) = ConexionSerie::conectar(&puerto) {
            self.conexion = Some(Box::new(conexion));
            self.reconexion = None;
            self.estado = format!("Reconectado a {puerto}");
            return;
        }

        self.puertos = puertos_usables();
        if intentos >= INTENTOS_ANTES_DE_BUSCAR {
            self.reconexion = None;
            self.estado = "No responde en el mismo puerto: buscando de nuevo...".to_string();
            self.descubrimiento = Some(descubrimiento::iniciar());
            return;
        }
        self.reconexion = Some(Reconexion { puerto, intentos, proximo: Instant::now() + ESPERA_RECONEXION });
    }

    fn tara_software(&mut self, usar_muestras_de_carga: bool) {
        let datos: Vec<[f64; 4]> = if usar_muestras_de_carga {
            self.buffer_arriba.iter().copied().collect()
        } else {
            self.buffer_crudo.iter().copied().collect()
        };
        if datos.is_empty() {
            self.estado = "Todavía no llegaron muestras".to_string();
            return;
        }
        let n = datos.len() as f64;
        let mut suma = [0.0f64; 4];
        for c in &datos {
            for i in 0..4 {
                suma[i] += c[i];
            }
        }
        self.offset = suma.map(|s| s / n);
        self.estado = "Tara por software aplicada".to_string();
    }

    fn limpiar_trazo(&mut self) {
        self.trazo_x.clear();
        self.trazo_y.clear();
    }

    fn reiniciar_sesion(&mut self) {
        self.limpiar_trazo();
        self.t_buf.clear();
        self.ml_buf.clear();
        self.ap_buf.clear();
        self.sesion_actual.clear();
        self.acumulador = Acumulador::nuevo();
    }

    /// Cierra la sesión en curso: calcula sus métricas, las guarda como
    /// "última sesión" y también en el casillero de su condición (para el
    /// cociente de Romberg), y se queda con una copia del registro crudo
    /// completo para poder exportarlo a CSV.
    fn cerrar_sesion(&mut self) {
        // Las métricas y el CSV salen de la señal filtrada: el zigzag del ruido
        // del ADC no es balanceo y solo infla longitud y velocidad.
        let registro = if self.config.filtrar_cop {
            filtrar_registro(&self.sesion_actual, self.config.filtro_corte_hz)
        } else {
            std::mem::take(&mut self.sesion_actual)
        };
        let metricas = calcular_metricas(&registro);
        self.ultima_sesion = metricas;
        self.ultima_condicion = self.condicion;
        self.ultima_superficie = self.superficie;
        if self.ctsib_armado {
            if let Some(m) = metricas {
                self.resultados_ctsib.insert((self.superficie, self.condicion), m);
                self.avanzar_paso_ctsib();
            }
            self.ctsib_armado = false;
        }
        if let Some(m) = metricas {
            // Cada ensayo cerrado queda en el historial del paciente, no solo
            // en un CSV suelto que después hay que ir a buscar.
            let sesion = historial::Sesion::nueva(&self.paciente, self.superficie, self.condicion, m);
            match historial::agregar(&sesion) {
                Ok(()) => self.historial.push(sesion),
                Err(e) => self.estado = format!("No se pudo archivar la sesión: {e}"),
            }
        }
        self.ultimo_registro = registro;
        self.reiniciar_sesion();
        self.ejercicio.detener();
    }

    /// Salta a la siguiente condición del CTSIB que todavía no se corrió,
    /// como el "objetivo siguiente" del ejercicio de límites: cierra una
    /// sesión y ya queda listo el próximo paso, sin tocar nada a mano.
    fn avanzar_paso_ctsib(&mut self) {
        let actual = PASOS_CTSIB.iter().position(|&(s, c)| s == self.superficie && c == self.condicion).unwrap_or(0);
        for offset in 1..=PASOS_CTSIB.len() {
            let (s, c) = PASOS_CTSIB[(actual + offset) % PASOS_CTSIB.len()];
            if !self.resultados_ctsib.contains_key(&(s, c)) {
                self.superficie = s;
                self.condicion = c;
                return;
            }
        }
        // Las 4 condiciones ya están hechas: se queda donde está.
    }

    /// Detecta cuándo alguien sube o baja de la plataforma por la suma cruda.
    /// Al subir: tara automática (solo con muestras ya cargadas, sin mezclar
    /// con las de plataforma vacía) + sesión nueva. Al bajar: sesión nueva,
    /// lista para el siguiente. Debounce de N muestras contra ruido puntual.
    /// Carga sobre la plataforma y umbral con el que compararla, en la misma
    /// unidad: kilogramos si hay calibración con masa conocida, y cuentas
    /// crudas del ADC mientras no la haya.
    fn carga_y_umbral(&self, crudos: [f64; 4]) -> (f64, f64) {
        if self.config.calibrado_en_kg {
            (calibracion::peso_kg(&crudos, &self.config.ganancia), self.config.umbral_kg)
        } else {
            (crudos.iter().sum(), self.config.umbral)
        }
    }

    fn procesar_deteccion(&mut self, crudos: [f64; 4]) {
        let (carga, umbral) = self.carga_y_umbral(crudos);
        self.peso_kg = if self.config.calibrado_en_kg { carga } else { 0.0 };
        if carga.abs() >= umbral {
            self.buffer_arriba.push_back(crudos);
            if self.buffer_arriba.len() > self.config.muestras_tara {
                self.buffer_arriba.pop_front();
            }
            self.contador_arriba += 1;
            self.contador_abajo = 0;
            if !self.ocupado && self.contador_arriba >= self.config.debounce {
                self.ocupado = true;
                self.t_subida = None; // lo fija la primera muestra con la persona arriba
                self.ensayo_cerrado = false;
                self.tara_software(true);
                self.reiniciar_sesion();
                self.estado = "Persona detectada: tara automática".to_string();
            }
        } else {
            self.buffer_arriba.clear();
            self.contador_abajo += 1;
            self.contador_arriba = 0;
            if self.ocupado && self.contador_abajo >= self.config.debounce {
                self.ocupado = false;
                self.t_subida = None;
                if self.ensayo_cerrado {
                    // Ya se cerró solo al cumplir la duración: no pisar ese
                    // resultado con el registro vacío de después.
                    self.reiniciar_sesion();
                    self.ensayo_cerrado = false;
                } else {
                    self.cerrar_sesion();
                }
                self.estado = "Plataforma libre: sesión reiniciada".to_string();
            }
        }
    }

    fn procesar_muestra(&mut self, m: Muestra) {
        self.muestras_perdidas += m.perdidas;
        if let Some(asistente) = &mut self.asistente {
            asistente.alimentar(m.t, m.crudos, self.config.masa_calibracion_kg);
        }
        self.buffer_crudo.push_back(m.crudos);
        if self.buffer_crudo.len() > self.config.muestras_tara {
            self.buffer_crudo.pop_front();
        }
        self.procesar_deteccion(m.crudos);

        let vals: [f64; 4] = std::array::from_fn(|i| (m.crudos[i] - self.offset[i]) * self.config.ganancia[i]);
        let suma: f64 = vals.iter().sum();
        // Con la plataforma vacía `suma` es puro ruido cerca de cero: dividir
        // por eso amplifica cualquier ruidito a un COP que salta como loco.
        // Solo calculamos el COP real mientras hay alguien parado (`ocupado`).
        let (cop_ml, cop_ap) = if self.ocupado && suma != 0.0 {
            let ml = ((vals[0] + vals[2]) - (vals[1] + vals[3])) / suma * (self.config.ancho_cm / 2.0);
            let ap = ((vals[0] + vals[1]) - (vals[2] + vals[3])) / suma * (self.config.prof_cm / 2.0);
            self.ultimos_pct = vals.map(|v| v / suma * 100.0);
            (ml, ap)
        } else {
            (0.0, 0.0)
        };

        if self.ocupado {
            // Los gráficos muestran todo desde que se subió; el registro que
            // se mide empieza después del descarte de acomodación.
            empujar_acotado(&mut self.trazo_x, cop_ml, MAX_PUNTOS_TRAZO);
            empujar_acotado(&mut self.trazo_y, cop_ap, MAX_PUNTOS_TRAZO);
            empujar_acotado(&mut self.t_buf, m.t, MAX_MUESTRAS_TIEMPO);
            empujar_acotado(&mut self.ml_buf, cop_ml, MAX_MUESTRAS_TIEMPO);
            empujar_acotado(&mut self.ap_buf, cop_ap, MAX_MUESTRAS_TIEMPO);

            let inicio = *self.t_subida.get_or_insert(m.t);
            let transcurrido = m.t - inicio;
            if !self.ensayo_cerrado && transcurrido >= self.config.descarte_inicial_s {
                self.sesion_actual.push([m.t, cop_ml, cop_ap]);
                self.acumulador.agregar([m.t, cop_ml, cop_ap]);
                if self.config.ensayo_duracion_fija
                    && transcurrido >= self.config.descarte_inicial_s + self.config.duracion_ensayo_s
                {
                    self.cerrar_sesion();
                    self.ensayo_cerrado = true;
                    self.estado =
                        format!("Ensayo completo ({:.0} s): ya se puede bajar", self.config.duracion_ensayo_s);
                }
            }
        }
        self.ultimo_ml = cop_ml;
        self.ultimo_ap = cop_ap;
    }

    /// Cómo va el ensayo en curso, para mostrarlo en pantalla.
    fn progreso_ensayo(&self) -> Option<ProgresoEnsayo> {
        if !self.ocupado {
            return None;
        }
        if self.ensayo_cerrado {
            return Some(ProgresoEnsayo::Completo);
        }
        let inicio = self.t_subida?;
        let ahora = self.t_buf.back().copied()?;
        let transcurrido = ahora - inicio;
        if transcurrido < self.config.descarte_inicial_s {
            return Some(ProgresoEnsayo::Acomodando(self.config.descarte_inicial_s - transcurrido));
        }
        if !self.config.ensayo_duracion_fija {
            return Some(ProgresoEnsayo::Libre(transcurrido - self.config.descarte_inicial_s));
        }
        let registrado = transcurrido - self.config.descarte_inicial_s;
        Some(ProgresoEnsayo::Grabando {
            restante_s: (self.config.duracion_ensayo_s - registrado).max(0.0),
            fraccion: (registrado / self.config.duracion_ensayo_s).clamp(0.0, 1.0),
        })
    }

    fn procesar_evento(&mut self, evento: EventoSerie) {
        match evento {
            EventoSerie::Conectado => {
                self.estado = "Conectado".to_string();
                self.muestras_perdidas = 0;
                // Preguntar el estado del firmware: modo, calibración y tara.
                if let Some(c) = &mut self.conexion {
                    c.enviar_comando(b'p');
                }
            }
            EventoSerie::Desconectado => {
                self.conexion = None;
                self.programar_reconexion("Se desconectó");
            }
            EventoSerie::MensajeFirmware(m) => {
                if let Some(modo) = ModoFirmware::desde_mensaje(&m) {
                    self.modo_firmware = Some(modo);
                }
                self.estado = m;
            }
            EventoSerie::Error(e) => {
                self.conexion = None;
                self.programar_reconexion(&format!("Error: {e}"));
            }
            EventoSerie::Muestra(m) => self.procesar_muestra(m),
        }
    }

    fn barra_controles(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
        self.barra_pestanas(ui);
        ui.add_space(8.0);

        // Scroll horizontal: en una ventana angosta las tarjetas ya no se
        // desbordan fuera de la pantalla, se pueden recorrer.
        egui::ScrollArea::horizontal().id_salt("tarjetas").show(ui, |ui| {
            ui.horizontal(|ui| match self.pestana {
                Pestana::Examen => self.tarjetas_examen(ui),
                Pestana::Dispositivo => self.tarjetas_dispositivo(ui),
                Pestana::Ejercicios => self.tarjetas_ejercicios(ui),
            });
        });
    }

    /// Pestañas: en vez de nueve tarjetas apiladas a la vez, se muestra el
    /// grupo que corresponde a lo que se está haciendo.
    fn barra_pestanas(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (pestana, etiqueta) in [
                (Pestana::Examen, "Examen"),
                (Pestana::Dispositivo, "Dispositivo"),
                (Pestana::Ejercicios, "Ejercicios"),
            ] {
                if ui.selectable_label(self.pestana == pestana, etiqueta).clicked() {
                    self.pestana = pestana;
                }
            }
            ui.separator();
            if ui
                .button("⚙ Configuración")
                .on_hover_text("Plataforma, calibración, detección, gráficos, ensayo y modo juego")
                .clicked()
            {
                self.mostrar_config = !self.mostrar_config;
            }
            if ui.button("📈 Historial").clicked() {
                self.historial = historial::cargar();
                self.mostrar_historial = !self.mostrar_historial;
            }
            if ui
                .button("👁 Modo paciente")
                .on_hover_text("Solo el COP a pantalla completa, para que la persona se vea. ESC para volver.")
                .clicked()
            {
                self.modo_paciente = true;
            }
        });
    }

    /// Vista de biofeedback para el paciente: el gráfico COP ocupando todo,
    /// sin tarjetas ni métricas. La toma sigue funcionando igual por detrás.
    fn vista_paciente(&mut self, ui: &mut egui::Ui) {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.modo_paciente = false;
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(ui.visuals().window_fill).inner_margin(egui::Margin::symmetric(16, 12)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if let Some(progreso) = self.progreso_ensayo() {
                        let texto = match progreso {
                            ProgresoEnsayo::Acomodando(restante) => format!("Acomódese... {restante:.0} s"),
                            ProgresoEnsayo::Grabando { restante_s, .. } => format!("Quedan {restante_s:.0} s"),
                            ProgresoEnsayo::Libre(transcurrido) => format!("{transcurrido:.0} s"),
                            ProgresoEnsayo::Completo => "Listo, ya puede bajar".to_string(),
                        };
                        ui.label(egui::RichText::new(texto).size(22.0).strong().color(VERDE.gamma_multiply(0.85)));
                    } else if self.conexion.is_some() {
                        ui.label(
                            egui::RichText::new("Súbase a la plataforma").size(22.0).color(Color32::from_gray(120)),
                        );
                    } else {
                        ui.label(egui::RichText::new("Sin conexión").size(22.0).color(CORAL));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Salir").clicked() {
                            self.modo_paciente = false;
                        }
                        ui.label(egui::RichText::new("ESC para volver").small().color(Color32::from_gray(150)));
                    });
                });
                let alto = ui.available_height();
                self.plot_cop(ui, alto);
            });
    }

    fn tarjetas_examen(&mut self, ui: &mut egui::Ui) {
        tarjeta(ui, "PACIENTE", LILA, |ui| {
            ui.add(egui::TextEdit::singleline(&mut self.paciente).hint_text("Paciente / ID").desired_width(140.0));
            let hay_datos = !self.ultimo_registro.is_empty();
            if ui.add_enabled(hay_datos, egui::Button::new("Exportar CSV")).clicked() {
                self.exportar_sesion();
            }
            if ui.add_enabled(hay_datos, egui::Button::new("🖨 Informe")).clicked() {
                self.generar_informe();
            }
            if ui.button("📈 Historial").clicked() {
                self.historial = historial::cargar();
                self.mostrar_historial = true;
            }
        });
        tarjeta(ui, "CTSIB", LILA, |ui| {
            ui.label("ℹ").on_hover_text(
                "Examen guiado de 4 condiciones. Seleccione un paso y presione 'Iniciar \
                     prueba': recién ahí cuenta subirse a la plataforma como resultado \
                     del CTSIB (sin armarlo, subirse solo muestra el COP en vivo, no \
                     graba nada aquí). Al bajar se guarda ese paso y salta sola al \
                     siguiente pendiente. El ✓ marca los pasos ya hechos.",
            );
            ui.vertical(|ui| {
                for (i, &(sup, cond)) in PASOS_CTSIB.iter().enumerate() {
                    let hecho = self.resultados_ctsib.contains_key(&(sup, cond));
                    let activo = self.superficie == sup && self.condicion == cond;
                    let marca = if hecho { "✓" } else { "○" };
                    let etiqueta = format!("{marca} {}. {} + {}", i + 1, sup.etiqueta(), cond.etiqueta());
                    let respuesta = ui.selectable_label(activo, etiqueta);
                    if !self.ctsib_armado && respuesta.clicked() {
                        self.superficie = sup;
                        self.condicion = cond;
                    }
                }

                ui.add_space(4.0);
                if self.ctsib_armado {
                    let estado = if self.ocupado { "grabando..." } else { "súbase a la plataforma" };
                    ui.label(format!(
                        "Prueba armada: {} + {} — {estado}",
                        self.superficie.etiqueta(),
                        self.condicion.etiqueta()
                    ));
                    if ui.button("Cancelar").clicked() {
                        self.ctsib_armado = false;
                    }
                } else if ui.button("Iniciar prueba").clicked() {
                    self.ctsib_armado = true;
                }
            });
        });
        tarjeta(ui, "PESO POR CELDA", NARANJA, |ui| {
            let barra = |ui: &mut egui::Ui, etq: &str, idx: usize| {
                ui.label(etq);
                let pct = self.ultimos_pct[idx].clamp(0.0, 100.0);
                ui.add(egui::ProgressBar::new((pct / 100.0) as f32).desired_width(56.0).text(format!("{pct:.0}%")));
            };
            // Grilla 2x2 como la plataforma real: frontal arriba, posterior abajo.
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    barra(ui, "FI", 1);
                    barra(ui, "FD", 0);
                });
                ui.horizontal(|ui| {
                    barra(ui, "BI", 3);
                    barra(ui, "BD", 2);
                });

                // La plataforma es una balanza: el peso y el reparto entre
                // lados son datos clínicos que antes se descartaban.
                let derecha = self.ultimos_pct[0] + self.ultimos_pct[2];
                let frente = self.ultimos_pct[0] + self.ultimos_pct[1];
                ui.label(
                    egui::RichText::new(format!(
                        "I/D {:.0}/{:.0}%  ·  Post/Ant {:.0}/{:.0}%",
                        100.0 - derecha,
                        derecha,
                        100.0 - frente,
                        frente
                    ))
                    .small(),
                );
                if self.config.calibrado_en_kg && self.peso_kg.abs() > 0.5 {
                    ui.label(
                        egui::RichText::new(format!("Peso: {:.1} kg", self.peso_kg))
                            .strong()
                            .color(NARANJA.gamma_multiply(0.8)),
                    );
                }
            });
        });
    }

    fn tarjetas_dispositivo(&mut self, ui: &mut egui::Ui) {
        let conectado = self.conexion.is_some();
        tarjeta(ui, "CONEXIÓN", AZUL, |ui| {
            if conectado {
                let simulado = self.conexion.as_ref().is_some_and(|c| c.es_simulado());
                let descripcion = self.conexion.as_ref().map(|c| c.descripcion()).unwrap_or_default();
                if simulado {
                    // Que nadie confunda una demo con una medición real.
                    ui.colored_label(AMARILLO, "⚠ Simulado")
                        .on_hover_text("Datos sintéticos generados por la app: no sirven como registro clínico");
                } else {
                    ui.colored_label(VERDE, "🟢 Online").on_hover_text(descripcion);
                }
                if ui.button("Desconectar").clicked() {
                    self.alternar_conexion();
                }
            } else {
                egui::ComboBox::from_id_salt("combo_puerto")
                    .width(150.0)
                    .selected_text(self.puerto_seleccionado.clone().unwrap_or_else(|| "Sin puerto".to_string()))
                    .show_ui(ui, |ui| {
                        for p in self.puertos.clone() {
                            ui.selectable_value(&mut self.puerto_seleccionado, Some(p.clone()), p);
                        }
                    });
                if ui.button("⟳").on_hover_text("Actualizar lista de puertos").clicked() {
                    self.puertos = puertos_usables();
                }
                if ui.button("Conectar").clicked() {
                    self.alternar_conexion();
                }
                if ui
                    .button("🔍 Buscar")
                    .on_hover_text("Probar los puertos USB hasta encontrar el posturógrafo")
                    .clicked()
                {
                    self.estado = "Buscando posturógrafo...".to_string();
                    self.descubrimiento = Some(descubrimiento::iniciar());
                }
                if ui
                    .button("Simulador")
                    .on_hover_text("Datos sintéticos, sin plataforma: para probar la app o el modo juego")
                    .clicked()
                {
                    self.conectar_simulador();
                }
            }
        });
        tarjeta(ui, "FIRMWARE", NARANJA, |ui| {
            if ui.add_enabled(conectado, egui::Button::new("Tara")).clicked()
                && let Some(c) = &mut self.conexion
            {
                c.enviar_comando(b't');
            }
            if ui.add_enabled(conectado, egui::Button::new("Resincronizar")).clicked()
                && let Some(c) = &mut self.conexion
            {
                c.enviar_comando(b's');
            }
            let etiqueta = match self.modo_firmware {
                Some(modo) => modo.etiqueta(),
                None => "modo ?",
            };
            if ui
                .add_enabled(conectado, egui::Button::new(etiqueta))
                .on_hover_text(
                    "En qué escala manda los datos el firmware. Cambiarla cambia \
                         la escala del umbral de detección y de las ganancias.",
                )
                .clicked()
                && let Some(c) = &mut self.conexion
            {
                c.enviar_comando(b'c');
            }
        });
        tarjeta(ui, "CALIBRACIÓN", VERDE, |ui| {
            if ui
                .button("Tara")
                .on_hover_text(format!(
                    "Promedia las últimas {} muestras crudas y las fija como cero",
                    self.config.muestras_tara
                ))
                .clicked()
            {
                self.tara_software(false);
            }
            if ui
                .button("⚖ Calibrar")
                .on_hover_text("Asistente con masa conocida: deja las lecturas en kilogramos")
                .clicked()
            {
                self.asistente = Some(Asistente::default());
            }
            if self.config.calibrado_en_kg {
                ui.label(egui::RichText::new("en kg").small().color(VERDE));
            }
        });
        tarjeta(ui, "TRAZO", CORAL, |ui| {
            if ui.button("Limpiar").clicked() {
                self.limpiar_trazo();
            }
        });
    }

    fn tarjetas_ejercicios(&mut self, ui: &mut egui::Ui) {
        tarjeta(ui, "LÍMITES DE ESTABILIDAD", AMARILLO, |ui| {
            if self.ejercicio.activo() {
                if self.ejercicio.completo() {
                    ui.vertical(|ui| {
                        if let Some(resumen) = self.ejercicio.resumen() {
                            ui.label(resumen);
                        }
                        // Alcance por dirección: es el resultado clínico del
                        // ejercicio, y antes se perdía al salir de la pantalla.
                        egui::Grid::new("grid_limites").num_columns(4).spacing([10.0, 2.0]).show(ui, |ui| {
                            for (i, intento) in self.ejercicio.intentos.iter().enumerate() {
                                ui.label(egui::RichText::new(limites::NOMBRES[intento.direccion]).small().strong());
                                ui.label(egui::RichText::new(format!("{:.1} cm", intento.alcance_cm)).small());
                                ui.label(
                                    egui::RichText::new(format!("{:.0}%", intento.fraccion_objetivo * 100.0)).small(),
                                );
                                ui.label(egui::RichText::new(format!("{:.1} s", intento.tiempo_s)).small());
                                if i % 2 == 1 {
                                    ui.end_row();
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Reiniciar").clicked() {
                                self.ejercicio.iniciar();
                            }
                            if ui.button("Exportar CSV").clicked() {
                                match exportar_limites(&self.paciente, &self.ejercicio.intentos) {
                                    Ok(ruta) => self.estado = format!("Límites exportados: {}", ruta.display()),
                                    Err(e) => self.estado = format!("Error al exportar: {e}"),
                                }
                            }
                        });
                    });
                } else {
                    ui.label(format!("Objetivo {}/{}", self.ejercicio.indice_actual() + 1, limites::DIRECCIONES));
                    if ui.button("Detener").clicked() {
                        self.ejercicio.detener();
                    }
                }
            } else if ui
                .add_enabled(self.ocupado, egui::Button::new("Iniciar ejercicio"))
                .on_hover_text("Primero súbase a la plataforma")
                .clicked()
            {
                self.ejercicio.iniciar();
            }
        });
        tarjeta(ui, "JUEGO", ROSA_JUEGO, |ui| {
            if ui.button("🎮 Modo juego").clicked() {
                self.modo_juego = true;
            }
        });
    }

    fn exportar_sesion(&mut self) {
        let Some(metricas) = self.ultima_sesion else {
            self.estado = "No hay una sesión completa para exportar".to_string();
            return;
        };
        match exportar_csv(
            &self.paciente,
            self.ultima_condicion,
            self.ultima_superficie,
            self.config.ancho_cm,
            self.config.prof_cm,
            &metricas,
            &self.ultimo_registro,
        ) {
            Ok(ruta) => self.estado = format!("Exportado: {}", ruta.display()),
            Err(e) => self.estado = format!("Error al exportar: {e}"),
        }
    }

    /// Asistente de calibración a kilogramos con una masa conocida
    /// (ver src/calibracion.rs).
    fn ventana_calibracion(&mut self, ctx: &egui::Context) {
        if self.asistente.is_none() {
            return;
        }
        let masa = self.config.masa_calibracion_kg;
        let lado = self.config.lado_patron_cm;
        let conectado = self.conexion.is_some();
        let mut cerrar = false;
        let mut abierta = true;
        let mut asistente = self.asistente.take().expect("recién se comprobó que está");

        egui::Window::new("⚖ Calibración con masa conocida").open(&mut abierta).default_width(520.0).show(ctx, |ui| {
            ui.label(
                egui::RichText::new(
                    "El peso total sobre la plataforma es el mismo esté donde esté la masa: con \
                     una captura por celda queda un sistema de 4 ecuaciones que da la ganancia \
                     de cada una, incluida la parte de carga que se reparte a las vecinas.",
                )
                .small()
                .color(Color32::from_gray(120)),
            );

            if !conectado {
                ui.add_space(8.0);
                ui.colored_label(CORAL, "Sin conexión: conecte el posturógrafo para poder capturar.");
            }

            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(format!(
                    "Paso {} de {} — {}",
                    asistente.paso.numero(),
                    calibracion::N_CELDAS + 2,
                    asistente.paso.instruccion(lado, masa)
                ))
                .heading(),
            );
            ui.add_space(8.0);

            ui.horizontal_top(|ui| {
                dibujar_plataforma(ui, &asistente);
                ui.add_space(12.0);
                ui.vertical(|ui| tabla_celdas(ui, &asistente));
            });

            ui.add_space(10.0);

            if asistente.paso == calibracion::Paso::Resultado {
                self.panel_resultado_calibracion(ui, &mut asistente, masa, &mut cerrar);
            } else {
                ui.horizontal(|ui| {
                    if let Some((texto, avance)) = asistente.progreso() {
                        ui.add(egui::ProgressBar::new(avance).desired_width(280.0).text(texto));
                        if ui.button("Cancelar").clicked() {
                            asistente.cancelar_captura();
                        }
                    } else {
                        if ui
                            .add_enabled(conectado, egui::Button::new("Capturar"))
                            .on_hover_text(format!(
                                "Descarta {:.1} s para que la lectura se asiente y después promedia {:.1} s",
                                calibracion::ESTABILIZACION_S,
                                calibracion::MEDICION_S
                            ))
                            .clicked()
                        {
                            asistente.iniciar_captura();
                        }
                        if asistente.paso != calibracion::Paso::Vacia && ui.button("Repetir paso anterior").clicked() {
                            asistente.repetir_paso();
                        }
                    }
                });
            }

            if !asistente.aviso.is_empty() {
                ui.add_space(6.0);
                let color = if asistente.error.is_some() { CORAL } else { Color32::from_gray(110) };
                ui.colored_label(color, &asistente.aviso);
            }
        });

        if cerrar || !abierta {
            self.asistente = None;
        } else {
            self.asistente = Some(asistente);
        }
    }

    /// Último paso: ganancias resueltas, verificación y guardado; o el error
    /// con su explicación y las salidas posibles.
    fn panel_resultado_calibracion(
        &mut self,
        ui: &mut egui::Ui,
        asistente: &mut Asistente,
        masa: f64,
        cerrar: &mut bool,
    ) {
        match asistente.resultado {
            Some(ganancias) => {
                if asistente.es_respaldo {
                    ui.colored_label(AMARILLO, "Calibración de respaldo: una sola escala para las 4 celdas.");
                }
                ui.label("Verificación: cada captura tiene que pesar lo que pesa el patrón.");
                egui::Grid::new("grid_verificacion").num_columns(3).spacing([14.0, 4.0]).show(ui, |ui| {
                    let pesos = calibracion::verificacion(asistente.deltas(), &ganancias);
                    for (i, peso) in pesos.iter().enumerate() {
                        let error_pct = (peso - masa).abs() / masa * 100.0;
                        ui.label(egui::RichText::new(calibracion::ETIQUETAS_CORTAS[i]).strong());
                        ui.label(format!("{peso:.3} kg"));
                        let color = if error_pct < 2.0 { VERDE } else { CORAL };
                        ui.colored_label(color, format!("{error_pct:+.1} %"));
                        ui.end_row();
                    }
                });
                ui.add_space(6.0);
                ui.collapsing("Ganancias resueltas", |ui| {
                    for (i, g) in ganancias.iter().enumerate() {
                        ui.monospace(format!("{:<26} {g:.8} kg/cuenta", calibracion::ETIQUETAS[i]));
                    }
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Guardar calibración").clicked() {
                        self.config.ganancia = ganancias;
                        self.config.calibrado_en_kg = true;
                        self.estado = "Calibración guardada: las lecturas están en kilogramos".to_string();
                        *cerrar = true;
                    }
                    if ui.button("Repetir último paso").clicked() {
                        asistente.repetir_paso();
                    }
                    if ui.button("Empezar de nuevo").clicked() {
                        *asistente = Asistente::default();
                    }
                });
            }
            None => {
                ui.horizontal(|ui| {
                    if ui
                        .button("Usar escala global")
                        .on_hover_text(
                            "Una sola escala para las 4 celdas: deja el peso bien medido, pero no corrige \
                             las diferencias entre celdas",
                        )
                        .clicked()
                    {
                        asistente.usar_escala_global(masa);
                    }
                    if ui.button("Repetir último paso").clicked() {
                        asistente.repetir_paso();
                    }
                    if ui.button("Empezar de nuevo").clicked() {
                        *asistente = Asistente::default();
                    }
                });
            }
        }
    }

    /// Historial del paciente: lista de sesiones y evolución de las dos
    /// métricas que mejor resumen el examen (área 95% y velocidad media).
    fn ventana_historial(&mut self, ctx: &egui::Context) {
        if !self.mostrar_historial {
            return;
        }
        let paciente = self.paciente.clone();
        let sesiones: Vec<historial::Sesion> =
            historial::de_paciente(&self.historial, &paciente).into_iter().cloned().collect();
        let mut abierta = true;

        egui::Window::new("📈 Historial del paciente").open(&mut abierta).default_width(620.0).show(ctx, |ui| {
            if paciente.trim().is_empty() {
                ui.label("Escriba el identificador del paciente para ver su historial.");
                let otros = historial::pacientes(&self.historial);
                if !otros.is_empty() {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("Con sesiones guardadas:").small());
                    for p in otros {
                        if ui.selectable_label(false, &p).clicked() {
                            self.paciente = p;
                        }
                    }
                }
                return;
            }

            if sesiones.is_empty() {
                ui.label(format!("Todavía no hay sesiones archivadas de {paciente}."));
                return;
            }

            ui.label(format!("{} sesiones archivadas", sesiones.len()));
            ui.add_space(6.0);

            // Evolución: de la más vieja a la más nueva, por número de sesión.
            let mut area: Vec<[f64; 2]> = Vec::new();
            let mut velocidad: Vec<[f64; 2]> = Vec::new();
            for (i, s) in sesiones.iter().rev().enumerate() {
                area.push([i as f64 + 1.0, s.metricas.area95_cm2]);
                velocidad.push([i as f64 + 1.0, s.metricas.velocidad_media_cms]);
            }
            Plot::new("plot_historial").height(200.0).legend(Legend::default()).show(ui, |plot_ui| {
                plot_ui.line(Line::new("Área 95% (cm²)", PlotPoints::from(area)).color(LILA).width(2.0));
                plot_ui.line(Line::new("Vel. media (cm/s)", PlotPoints::from(velocidad)).color(AZUL).width(2.0));
            });

            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                for s in &sesiones {
                    ui.label(format!(
                        "{} · {} · área {:.1} cm² · vel {:.2} cm/s · {:.0} s",
                        fecha_legible(s.epoch_s),
                        s.etiqueta(),
                        s.metricas.area95_cm2,
                        s.metricas.velocidad_media_cms,
                        s.metricas.duracion_s
                    ));
                }
            });
        });

        self.mostrar_historial = abierta;
    }

    /// Barra de progreso del ensayo: cuánto falta para que empiece a contar
    /// y cuánto queda de registro. Sin esto, la duración fija sería una regla
    /// invisible que corta la toma cuando menos se espera.
    fn barra_ensayo(&self, ui: &mut egui::Ui) {
        let Some(progreso) = self.progreso_ensayo() else { return };
        let (texto, fraccion, color) = match progreso {
            ProgresoEnsayo::Acomodando(restante) => {
                (format!("Acomodándose... el registro empieza en {restante:.0} s"), None, AMARILLO)
            }
            ProgresoEnsayo::Grabando { restante_s, fraccion } => {
                (format!("Grabando ensayo · quedan {restante_s:.0} s"), Some(fraccion as f32), VERDE)
            }
            ProgresoEnsayo::Libre(transcurrido) => (format!("Grabando · {transcurrido:.0} s"), None, VERDE),
            ProgresoEnsayo::Completo => ("Ensayo completo: ya se puede bajar".to_string(), Some(1.0), AZUL),
        };
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(texto).small().strong().color(color.gamma_multiply(0.8)));
            if let Some(fraccion) = fraccion {
                ui.add(egui::ProgressBar::new(fraccion).desired_width(180.0).fill(color.gamma_multiply(0.8)));
            }
        });
    }

    /// Las métricas que se están mostrando: las de la toma en curso o las de
    /// la última sesión cerrada.
    fn metricas_mostradas(&self) -> Option<MetricasBalance> {
        if self.ocupado && !self.ensayo_cerrado { self.acumulador.metricas() } else { self.ultima_sesion }
    }

    /// Genera el informe imprimible de la última sesión y lo abre con el
    /// navegador del sistema, que es donde se imprime o se guarda como PDF.
    fn generar_informe(&mut self) {
        let Some(metricas) = self.ultima_sesion else {
            self.estado = "No hay una sesión completa para informar".to_string();
            return;
        };
        let cocientes = self.cocientes_ctsib();
        let fecha = fecha_legible(
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default(),
        );
        let datos = informe::DatosInforme {
            paciente: &self.paciente,
            fecha: &fecha,
            superficie: self.ultima_superficie,
            condicion: self.ultima_condicion,
            ancho_cm: self.config.ancho_cm,
            prof_cm: self.config.prof_cm,
            metricas: &metricas,
            registro: &self.ultimo_registro,
            cocientes: &cocientes,
            version: VERSION,
        };
        match informe::escribir(&datos) {
            Ok(ruta) => {
                abrir_en_el_sistema(&ruta);
                self.estado = format!("Informe generado: {}", ruta.display());
            }
            Err(e) => self.estado = format!("Error al generar el informe: {e}"),
        }
    }

    fn panel_metricas(&self, ui: &mut egui::Ui) {
        let (etiqueta, acento, texto) = if self.ocupado {
            match self.acumulador.metricas() {
                Some(m) => ("EN VIVO", VERDE, m.texto()),
                None => ("EN VIVO", VERDE, "Recolectando datos...".to_string()),
            }
        } else if let Some(m) = &self.ultima_sesion {
            (
                "ÚLTIMA SESIÓN",
                AZUL,
                format!("{} · {} · {}", self.ultima_superficie.etiqueta(), self.ultima_condicion.etiqueta(), m.texto()),
            )
        } else {
            return;
        };
        ui.add_space(10.0);
        let fondo = ui.visuals().window_fill;
        egui::Frame::new()
            .fill(fondo)
            .stroke(egui::Stroke::new(1.2, acento.gamma_multiply(0.55)))
            .corner_radius(10.0)
            .shadow(sombra_tarjeta())
            .inner_margin(egui::Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(etiqueta).small().strong().color(acento.gamma_multiply(0.7)));
                    ui.separator();
                    ui.label(texto);
                });
                if let Some(m) = self.metricas_mostradas() {
                    ui.label(egui::RichText::new(m.texto_avanzado()).small().color(Color32::from_gray(110)));
                }
                self.barra_ensayo(ui);
                if let Some(resumen) = self.resumen_ctsib() {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("CTSIB").small().strong().color(LILA.gamma_multiply(0.7)));
                        ui.separator();
                        ui.label(resumen);
                    });
                }
            });
    }

    /// Cocientes del CTSIB disponibles con las sesiones ya registradas: solo
    /// se muestra cada uno cuando ambas condiciones que compara ya se corrieron.
    fn cocientes_ctsib(&self) -> Vec<(String, f64)> {
        let buscar = |s, c| self.resultados_ctsib.get(&(s, c));
        let firme_oa = buscar(Superficie::Firme, Condicion::OjosAbiertos);
        let firme_oc = buscar(Superficie::Firme, Condicion::OjosCerrados);
        let espuma_oa = buscar(Superficie::Espuma, Condicion::OjosAbiertos);
        let espuma_oc = buscar(Superficie::Espuma, Condicion::OjosCerrados);

        let mut cocientes = Vec::new();
        for (nombre, base, comparado) in [
            ("Romberg firme", firme_oa, firme_oc),
            ("Romberg espuma", espuma_oa, espuma_oc),
            ("Ratio vestibular", firme_oa, espuma_oc),
        ] {
            if let (Some(a), Some(b)) = (base, comparado)
                && let Some(c) = cociente_area(a, b)
            {
                cocientes.push((nombre.to_string(), c));
            }
        }
        cocientes
    }

    fn resumen_ctsib(&self) -> Option<String> {
        let cocientes = self.cocientes_ctsib();
        if cocientes.is_empty() {
            return None;
        }
        Some(cocientes.iter().map(|(n, v)| format!("{n} {v:.2}x")).collect::<Vec<_>>().join(" · "))
    }

    fn plot_cop(&self, ui: &mut egui::Ui, altura: f32) {
        let margen = 1.2;
        let x_lim = self.config.ancho_cm / 2.0 * margen;
        let y_lim = self.config.prof_cm / 2.0 * margen;

        let xs: Vec<f64> = self.trazo_x.iter().copied().collect();
        let ys: Vec<f64> = self.trazo_y.iter().copied().collect();

        // Color por antigüedad: cola desvanecida -> cabeza (más reciente)
        // saturada, como un "cometa" que deja ver hacia dónde se mueve el COP
        // ahora mismo. Se arma por tramos, cada uno con su color: antes había
        // un degradé punto por punto que reconstruía un HashMap de hasta 20.000
        // entradas en cada frame y, como la clave eran los bits de (x, y), dos
        // puntos idénticos compartían entrada y tomaban el color equivocado.
        let tramos = segmentar_trazo(&xs, &ys, TRAMOS_TRAZO);
        let paso = self.config.espaciado_puntos.max(1);
        let puntos: PlotPoints = xs.iter().zip(ys.iter()).step_by(paso).map(|(&x, &y)| [x, y]).collect();
        let actual: PlotPoints = vec![[self.ultimo_ml, self.ultimo_ap]].into();
        // La elipse se ajusta sobre el mismo registro del que salen las
        // métricas, no sobre la ventana del trazo: si se ajustara sobre el
        // trazo (una ventana de otro largo, y sin filtrar), el área dibujada
        // no sería la que informa el panel.
        let elipse = if !self.config.mostrar_elipse {
            None
        } else if self.ocupado && !self.ensayo_cerrado {
            self.acumulador.elipse() // en vivo: sale de las sumas acumuladas
        } else {
            ajustar_elipse95(&self.ultimo_registro)
        };

        Plot::new("plot_cop")
            .height(altura)
            .data_aspect(1.0)
            .allow_scroll(false)
            .allow_boxed_zoom(false)
            .allow_drag(false)
            .show(ui, |plot_ui| {
                plot_ui.set_plot_bounds(PlotBounds::from_min_max([-x_lim, -y_lim], [x_lim, y_lim]));
                plot_ui.hline(HLine::new("", 0.0).color(GUIA));
                plot_ui.vline(VLine::new("", 0.0).color(GUIA));

                if let Some(e) = &elipse {
                    let contorno: PlotPoints = e.contorno(64).into();
                    plot_ui.polygon(
                        Polygon::new("Elipse 95%", contorno)
                            .stroke(egui::Stroke::new(
                                1.5,
                                Color32::from_rgba_unmultiplied(LILA.r(), LILA.g(), LILA.b(), 180),
                            ))
                            .fill_color(Color32::from_rgba_unmultiplied(LILA.r(), LILA.g(), LILA.b(), 35)),
                    );
                }

                for (i, tramo) in tramos.into_iter().enumerate() {
                    let antiguedad = if TRAMOS_TRAZO > 1 { i as f32 / (TRAMOS_TRAZO - 1) as f32 } else { 1.0 };
                    let color = lerp_color(COLOR_TRAZO_VIEJO, AZUL, antiguedad);
                    let nombre = if i + 1 == TRAMOS_TRAZO { "Trazo" } else { "" };
                    plot_ui.line(Line::new(nombre, PlotPoints::from(tramo)).width(1.5).color(color));
                }
                plot_ui.points(
                    Points::new("", puntos)
                        .color(Color32::from_rgba_unmultiplied(AZUL.r(), AZUL.g(), AZUL.b(), 190))
                        .radius(2.5),
                );
                plot_ui.points(
                    Points::new("COP", actual).shape(MarkerShape::Circle).filled(true).radius(7.0).color(CORAL),
                );

                if let Some(obj) = self.ejercicio.objetivo_actual(self.config.ancho_cm, self.config.prof_cm) {
                    let anillo: PlotPoints = vec![[obj.x, obj.y]].into();
                    plot_ui.points(
                        Points::new("Objetivo", anillo)
                            .shape(MarkerShape::Circle)
                            .filled(false)
                            .radius(12.0)
                            .color(AMARILLO),
                    );
                    let progreso = self.ejercicio.progreso_hold();
                    if progreso > 0.0 {
                        let relleno: PlotPoints = vec![[obj.x, obj.y]].into();
                        plot_ui.points(
                            Points::new("", relleno)
                                .shape(MarkerShape::Circle)
                                .filled(true)
                                .radius(12.0 * progreso)
                                .color(AMARILLO),
                        );
                    }
                }
            });
    }

    fn plot_tiempo(&self, ui: &mut egui::Ui, altura: f32) {
        let limite = self.config.ancho_cm.max(self.config.prof_cm) / 2.0 * 1.2;
        let t_ultimo = self.t_buf.back().copied().unwrap_or(0.0);

        let ml: PlotPoints = self.t_buf.iter().zip(self.ml_buf.iter()).map(|(&t, &v)| [t, v]).collect();
        let ap: PlotPoints = self.t_buf.iter().zip(self.ap_buf.iter()).map(|(&t, &v)| [t, v]).collect();

        Plot::new("plot_tiempo")
            .height(altura)
            .legend(Legend::default())
            .allow_scroll(false)
            .allow_boxed_zoom(false)
            .allow_drag(false)
            .show(ui, |plot_ui| {
                plot_ui.set_plot_bounds(PlotBounds::from_min_max(
                    [t_ultimo - self.config.ventana_tiempo_s, -limite],
                    [t_ultimo.max(self.config.ventana_tiempo_s), limite],
                ));
                plot_ui.line(Line::new("ML", ml).color(AZUL).width(1.5));
                plot_ui.line(Line::new("AP", ap).color(NARANJA).width(1.5));
            });
    }
}

/// Tarjeta blanca con encabezado, usada para enmarcar cada gráfico principal
/// (mismo lenguaje visual que `tarjeta`, pero pensada para contenido alto).
fn tarjeta_plot(
    ui: &mut egui::Ui,
    titulo: &str,
    acento: Color32,
    alto: f32,
    contenido: impl FnOnce(&mut egui::Ui, f32),
) {
    egui::Frame::new()
        .fill(ui.visuals().window_fill)
        .stroke(egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color))
        .corner_radius(10.0)
        .shadow(sombra_tarjeta())
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(titulo).small().strong().color(acento.gamma_multiply(0.7)));
            contenido(ui, (alto - 26.0).max(50.0));
        });
}

impl PosturografoxApp {
    /// Arranca restaurando la configuración guardada en la sesión anterior.
    /// Si no hay nada guardado (primer arranque) o el archivo no se puede
    /// leer, se queda con los valores de fábrica en vez de fallar.
    pub fn nueva(cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self::default();
        if let Some(almacen) = cc.storage
            && let Some(guardada) = eframe::get_value::<Config>(almacen, config::CLAVE_ALMACEN)
        {
            app.buffer_crudo = VecDeque::with_capacity(guardada.muestras_tara);
            app.buffer_arriba = VecDeque::with_capacity(guardada.muestras_tara);
            app.config = guardada;
        }
        app
    }
}

impl eframe::App for PosturografoxApp {
    /// `eframe` la llama al cerrar y cada `auto_save_interval`.
    fn save(&mut self, almacen: &mut dyn eframe::Storage) {
        eframe::set_value(almacen, config::CLAVE_ALMACEN, &self.config);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.tema_aplicado != Some(self.config.tema) {
            ui.ctx().set_visuals(visuales(self.config.tema));
            self.tema_aplicado = Some(self.config.tema);
        }

        let eventos: Vec<EventoSerie> = match &self.conexion {
            Some(c) => c.eventos().try_iter().collect(),
            None => Vec::new(),
        };
        let llegaron_datos = !eventos.is_empty();
        for evento in eventos {
            self.procesar_evento(evento);
        }

        if self.conexion.is_none() {
            self.reconectar();
        }

        if let Some(rx) = &self.descubrimiento
            && let Ok(evento) = rx.try_recv()
        {
            match evento {
                EventoDescubrimiento::Encontrado(puerto) => {
                    if self.conexion.is_none() {
                        self.puerto_seleccionado = Some(puerto);
                        self.alternar_conexion();
                    }
                }
                EventoDescubrimiento::Terminado => {
                    if self.conexion.is_none() {
                        self.estado = "No se encontró el posturógrafo: seleccione el puerto a mano".to_string();
                    }
                }
            }
            self.descubrimiento = None;
        }

        if self.modo_juego {
            let entrada = juego::EntradaJuego {
                cop_ml: self.ultimo_ml,
                cop_ap: self.ultimo_ap,
                ancho_cm: self.config.ancho_cm,
                prof_cm: self.config.prof_cm,
                conectado: self.conexion.is_some(),
                en_plataforma: self.ocupado,
                dt: ui.input(|i| i.stable_dt),
                duracion_partida_s: self.config.duracion_partida_s,
                volumen_musica: self.config.volumen_musica,
                volumen_efectos: self.config.volumen_efectos,
            };
            egui::CentralPanel::default().frame(egui::Frame::new().fill(ui.visuals().window_fill)).show(ui, |ui| {
                if juego::mostrar(ui, &mut self.estado_juego, entrada) {
                    self.modo_juego = false;
                }
            });
            ui.ctx().request_repaint_after(Duration::from_millis(16));
            return;
        }

        if self.modo_paciente {
            self.vista_paciente(ui);
            ui.ctx().request_repaint_after(ESPERA_REPINTADO_ACTIVO);
            return;
        }

        if self.ocupado {
            let dt = ui.input(|i| i.stable_dt);
            self.ejercicio.actualizar(self.ultimo_ml, self.ultimo_ap, self.config.ancho_cm, self.config.prof_cm, dt);
        }

        let lienzo = ui.visuals().panel_fill;
        let fondo = move |margen| egui::Frame::new().fill(lienzo).inner_margin(margen);

        egui::Panel::top("controles").frame(fondo(egui::Margin::symmetric(12, 10))).show(ui, |ui| {
            self.barra_controles(ui);
            self.panel_metricas(ui);
        });

        egui::Panel::bottom("estado").frame(fondo(egui::Margin::symmetric(14, 7))).show(ui, |ui| {
            ui.horizontal(|ui| {
                let color = if self.conexion.is_some() {
                    VERDE
                } else if self.estado.starts_with("Error") {
                    CORAL
                } else {
                    Color32::from_gray(170)
                };
                let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 4.0, color);
                ui.label(&self.estado);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(format!("v{VERSION}")).small().color(Color32::from_gray(150)));
                    if self.muestras_perdidas > 0 {
                        ui.label(
                            egui::RichText::new(format!("⚠ {} muestras perdidas", self.muestras_perdidas))
                                .small()
                                .color(CORAL),
                        )
                        .on_hover_text(
                            "El firmware numera cada muestra: estas nunca llegaron. \
                             Con muchas perdidas, la velocidad media queda subestimada.",
                        );
                    }
                });
            });
        });

        crate::config::ventana(ui.ctx(), &mut self.config, &mut self.mostrar_config, VERDE);
        self.ventana_calibracion(ui.ctx());
        self.ventana_historial(ui.ctx());

        egui::CentralPanel::default().frame(fondo(egui::Margin::symmetric(12, 10))).show(ui, |ui| {
            let alto_total = ui.available_height();
            tarjeta_plot(ui, "CENTRO DE PRESIÓN (COP)", AZUL, alto_total * 0.62, |ui, alto| self.plot_cop(ui, alto));
            ui.add_space(10.0);
            tarjeta_plot(ui, "MOVIMIENTO EN EL TIEMPO", NARANJA, alto_total * 0.34, |ui, alto| {
                self.plot_tiempo(ui, alto)
            });
        });

        // Repintar a 30 fps constantes gasta CPU (y batería) aunque no esté
        // pasando nada. Solo se mantiene ese ritmo mientras entran muestras.
        let espera = if llegaron_datos {
            ESPERA_REPINTADO_ACTIVO
        } else if self.conexion.is_some() {
            ESPERA_REPINTADO_CONECTADO
        } else {
            ESPERA_REPINTADO_OCIOSO
        };
        ui.ctx().request_repaint_after(espera);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn los_tramos_del_trazo_cubren_todo_sin_dejar_huecos() {
        let xs: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let ys = xs.clone();
        let tramos = segmentar_trazo(&xs, &ys, 8);

        assert!(!tramos.is_empty());
        assert_eq!(tramos.first().unwrap().first().unwrap()[0], 0.0);
        assert_eq!(tramos.last().unwrap().last().unwrap()[0], 99.0);
        // El final de cada tramo es el comienzo del siguiente: sin eso, la
        // línea quedaría cortada entre tramo y tramo.
        for par in tramos.windows(2) {
            assert_eq!(par[0].last().unwrap(), par[1].first().unwrap());
        }
    }

    #[test]
    fn un_trazo_sin_dos_puntos_no_genera_tramos() {
        assert!(segmentar_trazo(&[], &[], 8).is_empty());
        assert!(segmentar_trazo(&[1.0], &[1.0], 8).is_empty());
        assert!(segmentar_trazo(&[1.0, 2.0], &[1.0, 2.0], 0).is_empty());
    }

    #[test]
    fn la_fecha_legible_convierte_bien_epochs_conocidos() {
        assert_eq!(fecha_legible(0), "1970-01-01 00:00");
        assert_eq!(fecha_legible(1_000_000_000), "2001-09-09 01:46");
        // 2024 fue bisiesto: el 29 de febrero tiene que existir.
        assert_eq!(fecha_legible(1_709_208_000), "2024-02-29 12:00");
    }

    #[test]
    fn el_modo_del_firmware_sale_de_su_mensaje_de_estado() {
        assert_eq!(ModoFirmware::desde_mensaje("Modo: crudo"), Some(ModoFirmware::Crudo));
        assert_eq!(ModoFirmware::desde_mensaje("Modo: calibrado"), Some(ModoFirmware::Calibrado));
        assert_eq!(ModoFirmware::desde_mensaje("Tara lista: 1,2,3,4"), None);
        assert_eq!(ModoFirmware::desde_mensaje("Modo: otra cosa"), None);
    }

    #[test]
    fn lerp_color_en_extremos_devuelve_los_colores_originales() {
        let a = Color32::from_rgba_unmultiplied(0, 150, 255, 25);
        let b = Color32::from_rgb(0, 150, 255);
        assert_eq!(lerp_color(a, b, 0.0), a);
        assert_eq!(lerp_color(a, b, 1.0), b);
    }
}
