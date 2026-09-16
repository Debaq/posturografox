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
use std::sync::Arc;
use std::time::Duration;

use egui::Color32;
use egui_plot::{HLine, Legend, Line, MarkerShape, Plot, PlotBounds, PlotPoint, PlotPoints, Points, Polygon, VLine};

use crate::juego;
use crate::serial_link::{puertos_usables, ConexionSerie, EventoSerie, Muestra};

const MAX_MUESTRAS_TIEMPO: usize = 8_000;
const MAX_PUNTOS_TRAZO: usize = 20_000;
const VENTANA_TIEMPO_S: f64 = 20.0;
const MUESTRAS_TARA_SW: usize = 20;
const DEBOUNCE_DETECCION: u32 = 5;
const ESPACIADO_PUNTOS_DEFAULT: usize = 8;
const ETIQUETAS: [&str; 4] = ["fd", "fi", "bd", "bi"];
/// χ² al 95% con 2 grados de libertad: escala los semiejes de la elipse de
/// confianza y su área (Prieto et al. 1996, métrica estándar en posturografía).
const CHI2_95_2GL: f64 = 5.991_46;

// ── Paleta: pasteles contrastantes sobre fondo claro (look clínico) ─────────
const AZUL: Color32 = Color32::from_rgb(90, 149, 210); // trazo COP / curva ML / conexión
const NARANJA: Color32 = Color32::from_rgb(240, 165, 100); // curva AP / firmware
const CORAL: Color32 = Color32::from_rgb(222, 118, 112); // punto COP actual / desconectar
const LILA: Color32 = Color32::from_rgb(168, 146, 214); // elipse de confianza 95% / detección
const VERDE: Color32 = Color32::from_rgb(120, 178, 140); // plataforma / calibración
const GUIA: Color32 = Color32::from_gray(180); // líneas de referencia en 0,0
const ROSA_JUEGO: Color32 = Color32::from_rgb(214, 130, 176); // acento del modo juego

// ── Superficies: fondo tipo "dashboard" + tarjetas blancas con sombra ───────
const LIENZO: Color32 = Color32::from_rgb(235, 238, 242);
const TARJETA_BG: Color32 = Color32::from_rgb(252, 253, 254);

fn sombra_tarjeta() -> egui::Shadow {
    egui::Shadow {
        offset: [0, 2],
        blur: 10,
        spread: 0,
        color: Color32::from_black_alpha(22),
    }
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

/// Métricas clásicas de estabilometría, calculadas sobre el trazo COP de una sesión.
#[derive(Clone, Copy, Default)]
struct MetricasBalance {
    longitud_cm: f64,
    area95_cm2: f64,
    velocidad_media_cms: f64,
    duracion_s: f64,
}

impl MetricasBalance {
    fn texto(&self) -> String {
        format!(
            "Longitud: {:.1} cm · Área 95%: {:.1} cm² · Vel. media: {:.2} cm/s · Duración: {:.1} s",
            self.longitud_cm, self.area95_cm2, self.velocidad_media_cms, self.duracion_s
        )
    }
}

/// Semiejes + ángulo de la elipse de confianza al 95% de una nube de puntos 2D,
/// via descomposición espectral cerrada de la matriz de covarianza 2x2.
struct Elipse {
    centro: (f64, f64),
    semi_mayor: f64,
    semi_menor: f64,
    angulo: f64,
}

fn ajustar_elipse95(xs: &[f64], ys: &[f64]) -> Option<Elipse> {
    let n = xs.len();
    if n < 3 {
        return None;
    }
    let nf = n as f64;
    let media_x = xs.iter().sum::<f64>() / nf;
    let media_y = ys.iter().sum::<f64>() / nf;

    let (mut var_x, mut var_y, mut cov_xy) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let dx = xs[i] - media_x;
        let dy = ys[i] - media_y;
        var_x += dx * dx;
        var_y += dy * dy;
        cov_xy += dx * dy;
    }
    let gl = nf - 1.0;
    var_x /= gl;
    var_y /= gl;
    cov_xy /= gl;

    let tr = var_x + var_y;
    let det = var_x * var_y - cov_xy * cov_xy;
    let disc = (tr * tr / 4.0 - det).max(0.0).sqrt();
    let lambda1 = (tr / 2.0 + disc).max(0.0);
    let lambda2 = (tr / 2.0 - disc).max(0.0);
    let angulo = if cov_xy.abs() < 1e-9 && var_x >= var_y {
        0.0
    } else {
        0.5 * (2.0 * cov_xy).atan2(var_x - var_y)
    };

    Some(Elipse {
        centro: (media_x, media_y),
        semi_mayor: (lambda1 * CHI2_95_2GL).sqrt(),
        semi_menor: (lambda2 * CHI2_95_2GL).sqrt(),
        angulo,
    })
}

impl Elipse {
    fn area(&self) -> f64 {
        std::f64::consts::PI * self.semi_mayor * self.semi_menor
    }

    fn contorno(&self, segmentos: usize) -> Vec<[f64; 2]> {
        let (cx, cy) = self.centro;
        let (sin_a, cos_a) = self.angulo.sin_cos();
        (0..=segmentos)
            .map(|i| {
                let t = i as f64 / segmentos as f64 * std::f64::consts::TAU;
                let (ex, ey) = (self.semi_mayor * t.cos(), self.semi_menor * t.sin());
                [cx + ex * cos_a - ey * sin_a, cy + ex * sin_a + ey * cos_a]
            })
            .collect()
    }
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

fn empujar_acotado(buf: &mut VecDeque<f64>, valor: f64, max: usize) {
    buf.push_back(valor);
    if buf.len() > max {
        buf.pop_front();
    }
}

pub struct PosturografoxApp {
    // Conexión
    puertos: Vec<String>,
    puerto_seleccionado: Option<String>,
    conexion: Option<ConexionSerie>,
    estado: String,

    // Calibración: offset (tara por software) y ganancia por canal (fd,fi,bd,bi)
    offset: [f64; 4],
    ganancia: [f64; 4],
    buffer_crudo: VecDeque<[f64; 4]>,
    buffer_arriba: VecDeque<[f64; 4]>, // solo muestras ya sobre el umbral

    // Geometría de la plataforma
    ancho_cm: f64,
    prof_cm: f64,

    // Detección automática de subida/bajada
    umbral: f64,
    ocupado: bool,
    contador_arriba: u32,
    contador_abajo: u32,

    // Buffers de graficado
    t_buf: VecDeque<f64>,
    ml_buf: VecDeque<f64>,
    ap_buf: VecDeque<f64>,
    trazo_x: VecDeque<f64>,
    trazo_y: VecDeque<f64>,
    espaciado_puntos: usize,
    ultimo_ml: f64,
    ultimo_ap: f64,

    ultima_sesion: Option<MetricasBalance>,

    // Modo juego (ver src/juego.rs)
    modo_juego: bool,
    estado_juego: juego::EstadoJuego,
}

impl Default for PosturografoxApp {
    fn default() -> Self {
        let puertos = puertos_usables();
        let puerto_seleccionado = puertos.first().cloned();
        Self {
            puertos,
            puerto_seleccionado,
            conexion: None,
            estado: "Desconectado".to_string(),

            offset: [0.0; 4],
            ganancia: [1.0; 4],
            buffer_crudo: VecDeque::with_capacity(MUESTRAS_TARA_SW),
            buffer_arriba: VecDeque::with_capacity(MUESTRAS_TARA_SW),

            ancho_cm: 40.0,
            prof_cm: 40.0,

            umbral: 20_000.0,
            ocupado: false,
            contador_arriba: 0,
            contador_abajo: 0,

            t_buf: VecDeque::with_capacity(MAX_MUESTRAS_TIEMPO),
            ml_buf: VecDeque::with_capacity(MAX_MUESTRAS_TIEMPO),
            ap_buf: VecDeque::with_capacity(MAX_MUESTRAS_TIEMPO),
            trazo_x: VecDeque::with_capacity(MAX_PUNTOS_TRAZO),
            trazo_y: VecDeque::with_capacity(MAX_PUNTOS_TRAZO),
            espaciado_puntos: ESPACIADO_PUNTOS_DEFAULT,
            ultimo_ml: 0.0,
            ultimo_ap: 0.0,

            ultima_sesion: None,

            modo_juego: false,
            estado_juego: juego::EstadoJuego::default(),
        }
    }
}

impl PosturografoxApp {
    fn alternar_conexion(&mut self) {
        if self.conexion.is_some() {
            self.conexion = None; // Drop detiene el hilo lector
            self.estado = "Desconectado".to_string();
        } else if let Some(puerto) = self.puerto_seleccionado.clone() {
            match ConexionSerie::conectar(&puerto) {
                Ok(c) => {
                    self.conexion = Some(c);
                    self.estado = format!("Conectando a {puerto}...");
                }
                Err(e) => self.estado = format!("Error al conectar: {e}"),
            }
        } else {
            self.estado = "Elegí un puerto primero".to_string();
        }
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
        for i in 0..4 {
            self.offset[i] = suma[i] / n;
        }
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
    }

    /// Métricas clásicas de estabilometría sobre el trazo COP actual:
    /// longitud del camino recorrido, área de la elipse de confianza al 95%
    /// (Prieto et al. 1996) y velocidad media = longitud / duración.
    fn calcular_metricas(&self) -> Option<MetricasBalance> {
        let n = self.trazo_x.len();
        if n < 3 {
            return None;
        }
        let xs: Vec<f64> = self.trazo_x.iter().copied().collect();
        let ys: Vec<f64> = self.trazo_y.iter().copied().collect();

        let mut longitud_cm = 0.0;
        for i in 1..n {
            let dx = xs[i] - xs[i - 1];
            let dy = ys[i] - ys[i - 1];
            longitud_cm += (dx * dx + dy * dy).sqrt();
        }

        let area95_cm2 = ajustar_elipse95(&xs, &ys).map_or(0.0, |e| e.area());
        let duracion_s = self.t_buf.back().copied().unwrap_or(0.0) - self.t_buf.front().copied().unwrap_or(0.0);
        let velocidad_media_cms = if duracion_s > 0.0 { longitud_cm / duracion_s } else { 0.0 };

        Some(MetricasBalance {
            longitud_cm,
            area95_cm2,
            velocidad_media_cms,
            duracion_s,
        })
    }

    /// Detecta cuándo alguien sube o baja de la plataforma por la suma cruda.
    /// Al subir: tara automática (solo con muestras ya cargadas, sin mezclar
    /// con las de plataforma vacía) + sesión nueva. Al bajar: sesión nueva,
    /// lista para el siguiente. Debounce de N muestras contra ruido puntual.
    fn procesar_deteccion(&mut self, crudos: [f64; 4]) {
        let suma_cruda: f64 = crudos.iter().sum();
        if suma_cruda.abs() >= self.umbral {
            self.buffer_arriba.push_back(crudos);
            if self.buffer_arriba.len() > MUESTRAS_TARA_SW {
                self.buffer_arriba.pop_front();
            }
            self.contador_arriba += 1;
            self.contador_abajo = 0;
            if !self.ocupado && self.contador_arriba >= DEBOUNCE_DETECCION {
                self.ocupado = true;
                self.tara_software(true);
                self.reiniciar_sesion();
                self.estado = "Persona detectada: tara automática".to_string();
            }
        } else {
            self.buffer_arriba.clear();
            self.contador_abajo += 1;
            self.contador_arriba = 0;
            if self.ocupado && self.contador_abajo >= DEBOUNCE_DETECCION {
                self.ocupado = false;
                self.ultima_sesion = self.calcular_metricas();
                self.reiniciar_sesion();
                self.estado = "Plataforma libre: sesión reiniciada".to_string();
            }
        }
    }

    fn procesar_muestra(&mut self, m: Muestra) {
        self.buffer_crudo.push_back(m.crudos);
        if self.buffer_crudo.len() > MUESTRAS_TARA_SW {
            self.buffer_crudo.pop_front();
        }
        self.procesar_deteccion(m.crudos);

        let vals: [f64; 4] = std::array::from_fn(|i| (m.crudos[i] - self.offset[i]) * self.ganancia[i]);
        let suma: f64 = vals.iter().sum();
        let (cop_ml, cop_ap) = if suma == 0.0 {
            (0.0, 0.0)
        } else {
            let ml = ((vals[0] + vals[2]) - (vals[1] + vals[3])) / suma * (self.ancho_cm / 2.0);
            let ap = ((vals[0] + vals[1]) - (vals[2] + vals[3])) / suma * (self.prof_cm / 2.0);
            (ml, ap)
        };

        if self.ocupado {
            empujar_acotado(&mut self.trazo_x, cop_ml, MAX_PUNTOS_TRAZO);
            empujar_acotado(&mut self.trazo_y, cop_ap, MAX_PUNTOS_TRAZO);
            empujar_acotado(&mut self.t_buf, m.t, MAX_MUESTRAS_TIEMPO);
            empujar_acotado(&mut self.ml_buf, cop_ml, MAX_MUESTRAS_TIEMPO);
            empujar_acotado(&mut self.ap_buf, cop_ap, MAX_MUESTRAS_TIEMPO);
        }
        self.ultimo_ml = cop_ml;
        self.ultimo_ap = cop_ap;
    }

    fn procesar_evento(&mut self, evento: EventoSerie) {
        match evento {
            EventoSerie::Conectado => self.estado = "Conectado".to_string(),
            EventoSerie::Desconectado => {
                self.conexion = None;
                self.estado = "Desconectado".to_string();
            }
            EventoSerie::MensajeFirmware(m) => self.estado = m,
            EventoSerie::Error(e) => {
                self.conexion = None;
                self.estado = format!("Error: {e}");
            }
            EventoSerie::Muestra(m) => self.procesar_muestra(m),
        }
    }

    fn barra_controles(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);

        ui.horizontal_wrapped(|ui| {
            let conectado = self.conexion.is_some();

            tarjeta(ui, "CONEXIÓN", AZUL, |ui| {
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
                if ui.button(if conectado { "Desconectar" } else { "Conectar" }).clicked() {
                    self.alternar_conexion();
                }
            });

            tarjeta(ui, "FIRMWARE", NARANJA, |ui| {
                if ui.add_enabled(conectado, egui::Button::new("Tara")).clicked() {
                    if let Some(c) = &mut self.conexion {
                        c.enviar_comando(b't');
                    }
                }
                if ui.add_enabled(conectado, egui::Button::new("Resincronizar")).clicked() {
                    if let Some(c) = &mut self.conexion {
                        c.enviar_comando(b's');
                    }
                }
            });

            tarjeta(ui, "PLATAFORMA", VERDE, |ui| {
                ui.label("Ancho");
                ui.add(egui::DragValue::new(&mut self.ancho_cm).range(1.0..=500.0).speed(0.5).suffix(" cm"));
                ui.label("Prof.");
                ui.add(egui::DragValue::new(&mut self.prof_cm).range(1.0..=500.0).speed(0.5).suffix(" cm"));
            });

            tarjeta(ui, "CALIBRACIÓN", VERDE, |ui| {
                if ui
                    .button("Tara")
                    .on_hover_text(format!(
                        "Promedia las últimas {MUESTRAS_TARA_SW} muestras crudas y las fija como cero"
                    ))
                    .clicked()
                {
                    self.tara_software(false);
                }
                for (i, etq) in ETIQUETAS.iter().enumerate() {
                    ui.label(etq.to_uppercase());
                    ui.add(
                        egui::DragValue::new(&mut self.ganancia[i])
                            .range(0.0001..=1000.0)
                            .speed(0.01)
                            .fixed_decimals(3),
                    );
                }
            });

            tarjeta(ui, "DETECCIÓN AUTOMÁTICA", LILA, |ui| {
                ui.label("Umbral");
                ui.add(egui::DragValue::new(&mut self.umbral).range(0.0..=10_000_000.0).speed(100.0));
                ui.label("Espaciado");
                ui.add(egui::DragValue::new(&mut self.espaciado_puntos).range(1..=200));
            });

            tarjeta(ui, "TRAZO", CORAL, |ui| {
                if ui.button("Limpiar").clicked() {
                    self.limpiar_trazo();
                }
            });

            tarjeta(ui, "JUEGO", ROSA_JUEGO, |ui| {
                if ui.button("🎮 Modo juego").clicked() {
                    self.modo_juego = true;
                }
            });
        });
    }

    fn panel_metricas(&self, ui: &mut egui::Ui) {
        let (etiqueta, acento, texto) = if self.ocupado {
            match self.calcular_metricas() {
                Some(m) => ("EN VIVO", VERDE, m.texto()),
                None => ("EN VIVO", VERDE, "Recolectando datos...".to_string()),
            }
        } else if let Some(m) = &self.ultima_sesion {
            ("ÚLTIMA SESIÓN", AZUL, m.texto())
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
            });
    }

    fn plot_cop(&self, ui: &mut egui::Ui, altura: f32) {
        let margen = 1.2;
        let x_lim = self.ancho_cm / 2.0 * margen;
        let y_lim = self.prof_cm / 2.0 * margen;

        let xs: Vec<f64> = self.trazo_x.iter().copied().collect();
        let ys: Vec<f64> = self.trazo_y.iter().copied().collect();
        let n = xs.len();

        // Color por antigüedad: cola desvanecida -> cabeza (más reciente) saturada,
        // como un "cometa" que deja ver hacia dónde se mueve el COP ahora mismo.
        let color_vieja = Color32::from_rgba_unmultiplied(AZUL.r(), AZUL.g(), AZUL.b(), 25);
        let color_nueva = AZUL;
        let fraccion: HashMap<(u64, u64), f32> = xs
            .iter()
            .zip(ys.iter())
            .enumerate()
            .map(|(i, (&x, &y))| ((x.to_bits(), y.to_bits()), i as f32 / n.max(1) as f32))
            .collect();
        let fraccion = Arc::new(fraccion);

        let trazo: PlotPoints = xs.iter().zip(ys.iter()).map(|(&x, &y)| [x, y]).collect();
        let paso = self.espaciado_puntos.max(1);
        let puntos: PlotPoints = xs.iter().zip(ys.iter()).step_by(paso).map(|(&x, &y)| [x, y]).collect();
        let actual: PlotPoints = vec![[self.ultimo_ml, self.ultimo_ap]].into();
        let elipse = ajustar_elipse95(&xs, &ys);

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
                            .stroke(egui::Stroke::new(1.5, Color32::from_rgba_unmultiplied(LILA.r(), LILA.g(), LILA.b(), 180)))
                            .fill_color(Color32::from_rgba_unmultiplied(LILA.r(), LILA.g(), LILA.b(), 35)),
                    );
                }

                plot_ui.line(
                    Line::new("Trazo", trazo)
                        .width(1.5)
                        .gradient_color(
                            Arc::new(move |p: PlotPoint| {
                                let t = fraccion.get(&(p.x.to_bits(), p.y.to_bits())).copied().unwrap_or(1.0);
                                lerp_color(color_vieja, color_nueva, t)
                            }),
                            false,
                        ),
                );
                plot_ui.points(
                    Points::new("", puntos)
                        .color(Color32::from_rgba_unmultiplied(AZUL.r(), AZUL.g(), AZUL.b(), 190))
                        .radius(2.5),
                );
                plot_ui.points(
                    Points::new("COP", actual)
                        .shape(MarkerShape::Circle)
                        .filled(true)
                        .radius(7.0)
                        .color(CORAL),
                );
            });
    }

    fn plot_tiempo(&self, ui: &mut egui::Ui, altura: f32) {
        let limite = self.ancho_cm.max(self.prof_cm) / 2.0 * 1.2;
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
                    [t_ultimo - VENTANA_TIEMPO_S, -limite],
                    [t_ultimo.max(VENTANA_TIEMPO_S), limite],
                ));
                plot_ui.line(Line::new("ML", ml).color(AZUL).width(1.5));
                plot_ui.line(Line::new("AP", ap).color(NARANJA).width(1.5));
            });
    }
}

/// Tarjeta blanca con encabezado, usada para enmarcar cada gráfico principal
/// (mismo lenguaje visual que `tarjeta`, pero pensada para contenido alto).
fn tarjeta_plot(ui: &mut egui::Ui, titulo: &str, acento: Color32, alto: f32, contenido: impl FnOnce(&mut egui::Ui, f32)) {
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

impl eframe::App for PosturografoxApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let eventos: Vec<EventoSerie> = match &self.conexion {
            Some(c) => c.eventos.try_iter().collect(),
            None => Vec::new(),
        };
        for evento in eventos {
            self.procesar_evento(evento);
        }

        if self.modo_juego {
            let entrada = juego::EntradaJuego {
                cop_ml: self.ultimo_ml,
                cop_ap: self.ultimo_ap,
                ancho_cm: self.ancho_cm,
                prof_cm: self.prof_cm,
                conectado: self.conexion.is_some(),
                dt: ui.input(|i| i.stable_dt),
            };
            egui::CentralPanel::default().frame(egui::Frame::new().fill(TARJETA_BG)).show(ui, |ui| {
                if juego::mostrar(ui, &mut self.estado_juego, entrada) {
                    self.modo_juego = false;
                }
            });
            ui.ctx().request_repaint_after(Duration::from_millis(16));
            return;
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
            });
        });

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
    fn elipse_de_puntos_alineados_en_x_no_gira() {
        // Todo el sway es medio-lateral puro: el eje mayor debe quedar sobre X (ángulo 0)
        let xs = vec![-2.0, -1.0, 0.0, 1.0, 2.0];
        let ys = vec![0.0, 0.0, 0.0, 0.0, 0.0];
        let e = ajustar_elipse95(&xs, &ys).unwrap();
        assert!(e.angulo.abs() < 1e-6, "ángulo esperado 0, dio {}", e.angulo);
        assert!(e.semi_mayor > e.semi_menor);
        assert!(e.semi_menor.abs() < 1e-6, "sin varianza en Y, semi-menor debe ser ~0");
    }

    #[test]
    fn elipse_circular_no_favorece_ningun_eje() {
        // Nube simétrica en ambos ejes: los semiejes deben salir prácticamente iguales
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for i in 0..360 {
            let t = (i as f64).to_radians();
            xs.push(t.cos());
            ys.push(t.sin());
        }
        let e = ajustar_elipse95(&xs, &ys).unwrap();
        assert!((e.semi_mayor - e.semi_menor).abs() < 1e-3, "mayor={} menor={}", e.semi_mayor, e.semi_menor);
    }

    #[test]
    fn menos_de_tres_puntos_no_ajusta_elipse() {
        assert!(ajustar_elipse95(&[0.0, 1.0], &[0.0, 1.0]).is_none());
    }

    #[test]
    fn longitud_de_camino_recto_es_la_distancia_esperada() {
        // Path recto de 3 tramos de 1cm en X: longitud total = 3cm exactos
        let xs: [f64; 4] = [0.0, 1.0, 2.0, 3.0];
        let ys: [f64; 4] = [0.0, 0.0, 0.0, 0.0];
        let mut longitud: f64 = 0.0;
        for i in 1..xs.len() {
            let dx = xs[i] - xs[i - 1];
            let dy = ys[i] - ys[i - 1];
            longitud += (dx * dx + dy * dy).sqrt();
        }
        assert!((longitud - 3.0).abs() < 1e-9);
    }

    #[test]
    fn lerp_color_en_extremos_devuelve_los_colores_originales() {
        let a = Color32::from_rgba_unmultiplied(0, 150, 255, 25);
        let b = Color32::from_rgb(0, 150, 255);
        assert_eq!(lerp_color(a, b, 0.0), a);
        assert_eq!(lerp_color(a, b, 1.0), b);
    }
}
