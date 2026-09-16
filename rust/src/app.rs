//! Estado y UI de Posturografox: gráfico COP (cartesiano, con trazo y
//! puntos espaciados) + gráfico movimiento-tiempo, calibración por canal
//! (offset+ganancia) y detección automática de subida/bajada de la
//! plataforma. Ver `serial_link.rs` para la lectura del puerto serie.
//!
//! El COP de una plataforma rectangular de 4 celdas se calcula como:
//!   valor_i = (crudo_i - offset_i) * ganancia_i        (i = fd, fi, bd, bi)
//!   COP_ml (medio-lateral, + = derecha) = ((fd+bd)-(fi+bi))/suma * ancho/2
//!   COP_ap (antero-posterior, + = frente) = ((fd+fi)-(bd+bi))/suma * profundidad/2

use std::collections::VecDeque;
use std::time::Duration;

use egui::Color32;
use egui_plot::{HLine, Legend, Line, MarkerShape, Plot, PlotBounds, PlotPoints, Points, VLine};

use crate::serial_link::{puertos_usables, ConexionSerie, EventoSerie, Muestra};

const MAX_MUESTRAS_TIEMPO: usize = 8_000;
const MAX_PUNTOS_TRAZO: usize = 20_000;
const VENTANA_TIEMPO_S: f64 = 20.0;
const MUESTRAS_TARA_SW: usize = 20;
const DEBOUNCE_DETECCION: u32 = 5;
const ESPACIADO_PUNTOS_DEFAULT: usize = 8;
const ETIQUETAS: [&str; 4] = ["fd", "fi", "bd", "bi"];

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
        ui.horizontal(|ui| {
            ui.label("Puerto:");
            egui::ComboBox::from_id_salt("combo_puerto")
                .selected_text(self.puerto_seleccionado.clone().unwrap_or_else(|| "—".to_string()))
                .show_ui(ui, |ui| {
                    for p in self.puertos.clone() {
                        ui.selectable_value(&mut self.puerto_seleccionado, Some(p.clone()), p);
                    }
                });
            if ui.button("Actualizar").clicked() {
                self.puertos = puertos_usables();
            }

            let conectado = self.conexion.is_some();
            if ui.button(if conectado { "Desconectar" } else { "Conectar" }).clicked() {
                self.alternar_conexion();
            }
            if ui.add_enabled(conectado, egui::Button::new("Tara firmware")).clicked() {
                if let Some(c) = &mut self.conexion {
                    c.enviar_comando(b't');
                }
            }
            if ui.add_enabled(conectado, egui::Button::new("Resincronizar")).clicked() {
                if let Some(c) = &mut self.conexion {
                    c.enviar_comando(b's');
                }
            }
            if ui.button("Limpiar trazo").clicked() {
                self.limpiar_trazo();
            }
        });

        ui.horizontal(|ui| {
            ui.label("Ancho (cm):");
            ui.add(egui::DragValue::new(&mut self.ancho_cm).range(1.0..=500.0).speed(0.5));
            ui.label("Profundidad (cm):");
            ui.add(egui::DragValue::new(&mut self.prof_cm).range(1.0..=500.0).speed(0.5));
        });

        ui.horizontal(|ui| {
            if ui
                .button("Tara (software)")
                .on_hover_text(format!(
                    "Promedia las últimas {MUESTRAS_TARA_SW} muestras crudas y las fija como cero"
                ))
                .clicked()
            {
                self.tara_software(false);
            }
            for (i, etq) in ETIQUETAS.iter().enumerate() {
                ui.label(format!("Gan. {etq}:"));
                ui.add(
                    egui::DragValue::new(&mut self.ganancia[i])
                        .range(0.0001..=1000.0)
                        .speed(0.01)
                        .fixed_decimals(4),
                );
            }
        });

        ui.horizontal(|ui| {
            ui.label("Umbral detección (cuentas crudas):");
            ui.add(egui::DragValue::new(&mut self.umbral).range(0.0..=10_000_000.0).speed(100.0));
            ui.label("Espaciado puntos (muestras):");
            ui.add(egui::DragValue::new(&mut self.espaciado_puntos).range(1..=200));
        });
    }

    fn plot_cop(&self, ui: &mut egui::Ui, altura: f32) {
        let margen = 1.2;
        let x_lim = self.ancho_cm / 2.0 * margen;
        let y_lim = self.prof_cm / 2.0 * margen;

        let trazo: PlotPoints = self.trazo_x.iter().zip(self.trazo_y.iter()).map(|(&x, &y)| [x, y]).collect();
        let paso = self.espaciado_puntos.max(1);
        let puntos: PlotPoints = self
            .trazo_x
            .iter()
            .zip(self.trazo_y.iter())
            .step_by(paso)
            .map(|(&x, &y)| [x, y])
            .collect();
        let actual: PlotPoints = vec![[self.ultimo_ml, self.ultimo_ap]].into();

        Plot::new("plot_cop")
            .height(altura)
            .data_aspect(1.0)
            .allow_scroll(false)
            .allow_boxed_zoom(false)
            .allow_drag(false)
            .show(ui, |plot_ui| {
                plot_ui.set_plot_bounds(PlotBounds::from_min_max([-x_lim, -y_lim], [x_lim, y_lim]));
                plot_ui.hline(HLine::new("", 0.0).color(Color32::from_gray(140)));
                plot_ui.vline(VLine::new("", 0.0).color(Color32::from_gray(140)));
                plot_ui.line(Line::new("Trazo", trazo).color(Color32::from_rgb(0, 150, 255)).width(1.5));
                plot_ui.points(
                    Points::new("", puntos)
                        .color(Color32::from_rgba_unmultiplied(0, 150, 255, 180))
                        .radius(2.5),
                );
                plot_ui.points(
                    Points::new("COP", actual)
                        .shape(MarkerShape::Circle)
                        .filled(true)
                        .radius(7.0)
                        .color(Color32::from_rgb(255, 60, 60)),
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
                plot_ui.line(Line::new("ML", ml).color(Color32::from_rgb(0, 150, 255)).width(1.5));
                plot_ui.line(Line::new("AP", ap).color(Color32::from_rgb(255, 140, 0)).width(1.5));
            });
    }
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

        egui::Panel::top("controles").show(ui, |ui| {
            self.barra_controles(ui);
        });

        egui::Panel::bottom("estado").show(ui, |ui| {
            ui.label(&self.estado);
        });

        egui::CentralPanel::default().show(ui, |ui| {
            let alto_total = ui.available_height();
            self.plot_cop(ui, alto_total * 0.6);
            ui.separator();
            self.plot_tiempo(ui, alto_total * 0.35);
        });

        ui.ctx().request_repaint_after(Duration::from_millis(33));
    }
}
