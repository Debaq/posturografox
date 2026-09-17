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
use crate::config::{self, Config};
use crate::descubrimiento::{self, EventoDescubrimiento};
use crate::estabilometria::{
    Condicion, MetricasBalance, Superficie, ajustar_elipse95, calcular_metricas, cociente_area,
};
use crate::exportar::exportar_csv;
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

// ── Superficies: fondo tipo "dashboard" + tarjetas blancas con sombra ───────
const LIENZO: Color32 = Color32::from_rgb(235, 238, 242);
const TARJETA_BG: Color32 = Color32::from_rgb(252, 253, 254);

fn sombra_tarjeta() -> egui::Shadow {
    egui::Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha(22) }
}

/// Tarjeta con acento de color por categoría: agrupa controles relacionados
/// en vez de tirarlos todos en una única fila (look "tablero de instrumentos"
/// en lugar de una barra de widgets sin jerarquía visual).
fn tarjeta(ui: &mut egui::Ui, titulo: &str, acento: Color32, contenido: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(TARJETA_BG)
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
            self.estado = "Elegí un puerto primero".to_string();
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
            asistente.alimentar(m.crudos);
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

        ui.horizontal(|ui| {
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

            // Un solo acceso a todas las opciones del programa: geometría,
            // ganancias, umbrales, gráficos y modo juego (ver src/config.rs).
            tarjeta(ui, "AJUSTES", VERDE, |ui| {
                if ui
                    .button("⚙ Configuración")
                    .on_hover_text("Plataforma, calibración, detección, gráficos y modo juego")
                    .clicked()
                {
                    self.mostrar_config = !self.mostrar_config;
                }
                ui.label(
                    egui::RichText::new(format!("{:.0}×{:.0} cm", self.config.ancho_cm, self.config.prof_cm))
                        .small()
                        .color(Color32::from_gray(140)),
                );
            });

            tarjeta(ui, "JUEGO", ROSA_JUEGO, |ui| {
                if ui.button("🎮 Modo juego").clicked() {
                    self.modo_juego = true;
                }
            });
        });

        ui.add_space(10.0);

        ui.horizontal(|ui| {
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
                    "Examen guiado de 4 condiciones. Elegí un paso y apretá 'Iniciar \
                     prueba': recién ahí cuenta pararse en la plataforma como resultado \
                     del CTSIB (sin armarlo, pararse solo muestra el COP en vivo, no \
                     graba nada acá). Al bajarte se guarda ese paso y salta sola al \
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
                        let estado = if self.ocupado { "grabando..." } else { "subite a la plataforma" };
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

            tarjeta(ui, "LÍMITES DE ESTABILIDAD", AMARILLO, |ui| {
                if self.ejercicio.activo() {
                    if self.ejercicio.completo() {
                        if let Some(resumen) = self.ejercicio.resumen() {
                            ui.label(resumen);
                        }
                        if ui.button("Reiniciar").clicked() {
                            self.ejercicio.iniciar();
                        }
                    } else {
                        ui.label(format!("Objetivo {}/{}", self.ejercicio.indice_actual() + 1, limites::DIRECCIONES));
                        if ui.button("Detener").clicked() {
                            self.ejercicio.detener();
                        }
                    }
                } else if ui
                    .add_enabled(self.ocupado, egui::Button::new("Iniciar ejercicio"))
                    .on_hover_text("Parate en la plataforma primero")
                    .clicked()
                {
                    self.ejercicio.iniciar();
                }
            });
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
        let Some(asistente) = &mut self.asistente else { return };
        let masa = self.config.masa_calibracion_kg;
        let lado = self.config.lado_patron_cm;
        let mut cerrar = false;
        let mut abierta = true;

        egui::Window::new("⚖ Calibración con masa conocida").open(&mut abierta).default_width(460.0).show(ctx, |ui| {
            ui.label(
                egui::RichText::new(
                    "El peso total sobre la plataforma es el mismo esté donde esté la masa: con \
                         una captura por celda queda un sistema de 4 ecuaciones que da la ganancia \
                         de cada una, incluida la parte de carga que se reparte a las vecinas.",
                )
                .small()
                .color(Color32::from_gray(120)),
            );
            ui.add_space(8.0);

            ui.label(egui::RichText::new(asistente.paso.instruccion(lado, masa)).heading());
            ui.add_space(6.0);

            match asistente.paso {
                calibracion::Paso::Resultado => {
                    match asistente.resultado {
                        Some(ganancias) => {
                            ui.label("Ganancias resueltas (kg por cuenta):");
                            for (i, g) in ganancias.iter().enumerate() {
                                ui.monospace(format!("  {:<28} {g:.8}", calibracion::ETIQUETAS[i]));
                            }
                            if let Some(peso) = asistente.peso_verificacion(masa) {
                                ui.add_space(4.0);
                                ui.label(format!(
                                    "Verificación: la última captura pesa {peso:.3} kg (el patrón es {masa:.3} kg)"
                                ));
                            }
                            ui.add_space(8.0);
                            if ui.button("Guardar calibración").clicked() {
                                self.config.ganancia = ganancias;
                                self.config.calibrado_en_kg = true;
                                self.estado = "Calibración guardada: las lecturas están en kilogramos".to_string();
                                cerrar = true;
                            }
                        }
                        None => {
                            ui.colored_label(CORAL, "No se pudo resolver la calibración.");
                        }
                    }
                    if ui.button("Empezar de nuevo").clicked() {
                        *asistente = Asistente::default();
                    }
                }
                _ => {
                    ui.label(format!("Muestras promediadas: {}", asistente.muestras_acumuladas()));
                    let hay_muestras = asistente.muestras_acumuladas() > 0;
                    if ui
                        .add_enabled(hay_muestras, egui::Button::new("Capturar"))
                        .on_hover_text("Promedia todas las muestras que llegaron desde el paso anterior")
                        .clicked()
                    {
                        asistente.capturar(masa);
                    }
                }
            }

            if !asistente.aviso.is_empty() {
                ui.add_space(6.0);
                ui.colored_label(CORAL, &asistente.aviso);
            }
        });

        if cerrar || !abierta {
            self.asistente = None;
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
                ui.label("Escribí el identificador del paciente para ver su historial.");
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
        if self.ocupado && !self.ensayo_cerrado { calcular_metricas(&self.sesion_actual) } else { self.ultima_sesion }
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
            match calcular_metricas(&self.sesion_actual) {
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
        egui::Frame::new()
            .fill(TARJETA_BG)
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

    /// El registro que se está midiendo (o el último cerrado si nadie está
    /// arriba). Es la fuente única de las métricas y de la elipse dibujada.
    fn registro_medido(&self) -> &[[f64; 3]] {
        if self.ocupado && !self.ensayo_cerrado { &self.sesion_actual } else { &self.ultimo_registro }
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
        let elipse = if self.config.mostrar_elipse { ajustar_elipse95(self.registro_medido()) } else { None };

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
        .fill(TARJETA_BG)
        .stroke(egui::Stroke::new(1.0, Color32::from_gray(224)))
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
        let eventos: Vec<EventoSerie> = match &self.conexion {
            Some(c) => c.eventos().try_iter().collect(),
            None => Vec::new(),
        };
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
                        self.estado = "No se encontró el posturógrafo: elegí el puerto a mano".to_string();
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
                dt: ui.input(|i| i.stable_dt),
                duracion_partida_s: self.config.duracion_partida_s,
                volumen_musica: self.config.volumen_musica,
                volumen_efectos: self.config.volumen_efectos,
            };
            egui::CentralPanel::default().frame(egui::Frame::new().fill(TARJETA_BG)).show(ui, |ui| {
                if juego::mostrar(ui, &mut self.estado_juego, entrada) {
                    self.modo_juego = false;
                }
            });
            ui.ctx().request_repaint_after(Duration::from_millis(16));
            return;
        }

        if self.ocupado {
            let dt = ui.input(|i| i.stable_dt);
            self.ejercicio.actualizar(self.ultimo_ml, self.ultimo_ap, self.config.ancho_cm, self.config.prof_cm, dt);
        }

        let fondo = |margen| egui::Frame::new().fill(LIENZO).inner_margin(margen);

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

        ui.ctx().request_repaint_after(Duration::from_millis(33));
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
