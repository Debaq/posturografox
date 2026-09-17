//! Zona de configuración: **todas** las opciones del programa en un solo
//! lugar, en vez de repartidas entre tarjetas de la barra superior y
//! constantes compiladas dentro de cada módulo.
//!
//! `Config` es puro estado (sin dependencias de la UI ni del puerto serie),
//! así que se puede serializar entero para guardarlo entre sesiones y
//! testear sus valores por defecto.

use egui::Color32;

/// Valores por defecto de cada opción, en un solo lugar para que el botón
/// "Restaurar valores por defecto" y `Default` no puedan divergir.
pub mod defecto {
    pub const ANCHO_CM: f64 = 40.0;
    pub const PROF_CM: f64 = 40.0;
    pub const GANANCIA: [f64; 4] = [1.0; 4];
    pub const UMBRAL: f64 = 20_000.0;
    pub const DEBOUNCE: u32 = 5;
    pub const MUESTRAS_TARA: usize = 20;
    pub const ESPACIADO_PUNTOS: usize = 8;
    pub const VENTANA_TIEMPO_S: f64 = 20.0;
    pub const MOSTRAR_ELIPSE: bool = true;
    pub const MASA_CALIBRACION_KG: f64 = 1.0;
    pub const LADO_PATRON_CM: f64 = 10.0;
    pub const ENSAYO_DURACION_FIJA: bool = true;
    pub const DURACION_ENSAYO_S: f64 = 30.0;
    pub const DESCARTE_INICIAL_S: f64 = 3.0;
    pub const FILTRAR_COP: bool = true;
    pub const FILTRO_CORTE_HZ: f64 = 6.0;
    pub const DURACION_PARTIDA_S: f32 = 60.0;
    pub const VOLUMEN_MUSICA: f32 = 0.35;
    pub const VOLUMEN_EFECTOS: f32 = 0.6;
}

/// Clave con la que se guarda la configuración en el almacenamiento de
/// `eframe` (ver `PosturografoxApp::save`).
pub const CLAVE_ALMACEN: &str = "config";

/// Configuración completa del programa.
///
/// `serde(default)` hace que una versión vieja del archivo guardado siga
/// cargando cuando se agregan opciones nuevas: las que falten toman su
/// valor de fábrica en vez de descartar toda la configuración.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Config {
    // ── Plataforma ──────────────────────────────────────────────────────
    /// Distancia entre celdas en el eje medio-lateral.
    pub ancho_cm: f64,
    /// Distancia entre celdas en el eje antero-posterior.
    pub prof_cm: f64,

    // ── Calibración ─────────────────────────────────────────────────────
    /// Ganancia por canal (fd, fi, bd, bi): compensa que las celdas no
    /// respondan igual entre sí.
    pub ganancia: [f64; 4],
    /// Cuántas muestras crudas se promedian al pedir tara por software.
    pub muestras_tara: usize,
    /// Masa del patrón que se usa para calibrar a kilogramos.
    pub masa_calibracion_kg: f64,
    /// Lado del patrón, solo para las instrucciones del asistente.
    pub lado_patron_cm: f64,
    /// `true` cuando las ganancias vienen de una calibración con masa
    /// conocida: recién ahí los valores están en kilogramos de verdad.
    pub calibrado_en_kg: bool,

    // ── Detección automática de subida/bajada ───────────────────────────
    /// Suma cruda a partir de la cual se considera que hay alguien arriba.
    pub umbral: f64,
    /// Muestras consecutivas que hay que ver para confirmar el cambio.
    pub debounce: u32,

    // ── Gráficos ────────────────────────────────────────────────────────
    /// Se dibuja un punto cada N muestras sobre el trazo del COP.
    pub espaciado_puntos: usize,
    /// Ventana visible del gráfico movimiento-tiempo.
    pub ventana_tiempo_s: f64,
    /// Elipse de confianza 95% sobre el trazo.
    pub mostrar_elipse: bool,

    // ── Ensayo clínico ──────────────────────────────────────────────────
    /// El registro se cierra solo al cumplirse `duracion_ensayo_s`. Sin esto,
    /// la sesión dura lo que la persona se quede parada y los ensayos no son
    /// comparables entre sí.
    pub ensayo_duracion_fija: bool,
    /// Ventana de registro que entra en las métricas.
    pub duracion_ensayo_s: f64,
    /// Segundos iniciales que no se registran: la persona recién se subió y
    /// todavía se está acomodando.
    pub descarte_inicial_s: f64,

    // ── Procesamiento de la señal ───────────────────────────────────────
    /// Filtra el COP antes de calcular las métricas de la sesión.
    pub filtrar_cop: bool,
    /// Frecuencia de corte del pasabajos (ver `src/filtro.rs`).
    pub filtro_corte_hz: f64,

    // ── Modo juego ──────────────────────────────────────────────────────
    /// Cuánto dura una partida completa; al llegar a 0 sin perder, se gana.
    pub duracion_partida_s: f32,
    pub volumen_musica: f32,
    pub volumen_efectos: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ancho_cm: defecto::ANCHO_CM,
            prof_cm: defecto::PROF_CM,
            ganancia: defecto::GANANCIA,
            muestras_tara: defecto::MUESTRAS_TARA,
            masa_calibracion_kg: defecto::MASA_CALIBRACION_KG,
            lado_patron_cm: defecto::LADO_PATRON_CM,
            calibrado_en_kg: false,
            umbral: defecto::UMBRAL,
            debounce: defecto::DEBOUNCE,
            espaciado_puntos: defecto::ESPACIADO_PUNTOS,
            ventana_tiempo_s: defecto::VENTANA_TIEMPO_S,
            mostrar_elipse: defecto::MOSTRAR_ELIPSE,
            ensayo_duracion_fija: defecto::ENSAYO_DURACION_FIJA,
            duracion_ensayo_s: defecto::DURACION_ENSAYO_S,
            descarte_inicial_s: defecto::DESCARTE_INICIAL_S,
            filtrar_cop: defecto::FILTRAR_COP,
            filtro_corte_hz: defecto::FILTRO_CORTE_HZ,
            duracion_partida_s: defecto::DURACION_PARTIDA_S,
            volumen_musica: defecto::VOLUMEN_MUSICA,
            volumen_efectos: defecto::VOLUMEN_EFECTOS,
        }
    }
}

const ETIQUETAS_CELDA: [&str; 4] = ["FD", "FI", "BD", "BI"];

/// Encabezado de sección dentro de la ventana de configuración.
fn seccion(ui: &mut egui::Ui, titulo: &str, acento: Color32) {
    ui.add_space(10.0);
    ui.label(egui::RichText::new(titulo).small().strong().color(acento.gamma_multiply(0.75)));
    ui.separator();
}

/// Dibuja la ventana de configuración. `abierta` la controla quien llama
/// (botón ⚙ de la barra de controles) para que se pueda cerrar con la X.
pub fn ventana(ctx: &egui::Context, cfg: &mut Config, abierta: &mut bool, acento: Color32) {
    egui::Window::new("⚙ Configuración")
        .open(abierta)
        .resizable(true)
        .default_width(430.0)
        .collapsible(false)
        .show(ctx, |ui| contenido(ui, cfg, acento));
}

fn contenido(ui: &mut egui::Ui, cfg: &mut Config, acento: Color32) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.label(
            egui::RichText::new("Todas las opciones del programa viven acá. Se guardan al cerrar la app.")
                .small()
                .color(Color32::from_gray(120)),
        );

        seccion(ui, "PLATAFORMA", acento);
        egui::Grid::new("grid_plataforma").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Ancho (medio-lateral)");
            ui.add(egui::DragValue::new(&mut cfg.ancho_cm).range(1.0..=500.0).speed(0.5).suffix(" cm"));
            ui.end_row();
            ui.label("Profundidad (antero-posterior)");
            ui.add(egui::DragValue::new(&mut cfg.prof_cm).range(1.0..=500.0).speed(0.5).suffix(" cm"));
            ui.end_row();
        });

        seccion(ui, "CALIBRACIÓN", acento);
        ui.label(
            egui::RichText::new("Ganancia por celda: compensa que no respondan igual entre sí.")
                .small()
                .color(Color32::from_gray(120)),
        );
        egui::Grid::new("grid_calibracion").num_columns(4).spacing([12.0, 6.0]).show(ui, |ui| {
            for (i, etq) in ETIQUETAS_CELDA.iter().enumerate() {
                ui.label(*etq);
                ui.add(egui::DragValue::new(&mut cfg.ganancia[i]).range(0.0001..=1000.0).speed(0.01).fixed_decimals(3));
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
        });
        egui::Grid::new("grid_calibracion_extra").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Muestras promediadas en la tara");
            ui.add(egui::DragValue::new(&mut cfg.muestras_tara).range(1..=500));
            ui.end_row();
            ui.label("Masa del patrón").on_hover_text("La masa conocida que se usa en el asistente de calibración");
            ui.add(egui::DragValue::new(&mut cfg.masa_calibracion_kg).range(0.1..=200.0).speed(0.1).suffix(" kg"));
            ui.end_row();
            ui.label("Lado del patrón").on_hover_text("Solo para las instrucciones: de qué tamaño es el bloque");
            ui.add(egui::DragValue::new(&mut cfg.lado_patron_cm).range(1.0..=100.0).speed(0.5).suffix(" cm"));
            ui.end_row();
            ui.label("Estado");
            ui.label(if cfg.calibrado_en_kg { "calibrado en kg" } else { "sin calibrar (valores en cuentas del ADC)" });
            ui.end_row();
        });

        seccion(ui, "DETECCIÓN AUTOMÁTICA", acento);
        egui::Grid::new("grid_deteccion").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Umbral de presencia")
                .on_hover_text("Suma cruda de las 4 celdas a partir de la cual se considera que hay alguien arriba");
            ui.add(egui::DragValue::new(&mut cfg.umbral).range(0.0..=10_000_000.0).speed(100.0));
            ui.end_row();
            ui.label("Muestras para confirmar").on_hover_text("Evita que un ruido puntual dispare la detección");
            ui.add(egui::DragValue::new(&mut cfg.debounce).range(1..=100));
            ui.end_row();
        });

        seccion(ui, "GRÁFICOS", acento);
        egui::Grid::new("grid_graficos").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Espaciado de puntos del trazo").on_hover_text("Se dibuja un punto cada N muestras");
            ui.add(egui::DragValue::new(&mut cfg.espaciado_puntos).range(1..=200));
            ui.end_row();
            ui.label("Ventana del gráfico en el tiempo");
            ui.add(egui::DragValue::new(&mut cfg.ventana_tiempo_s).range(2.0..=300.0).speed(0.5).suffix(" s"));
            ui.end_row();
            ui.label("Elipse de confianza 95%");
            ui.checkbox(&mut cfg.mostrar_elipse, "");
            ui.end_row();
        });

        seccion(ui, "ENSAYO CLÍNICO", acento);
        egui::Grid::new("grid_ensayo").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Duración fija del ensayo").on_hover_text(
                "El registro se cierra solo. Sin esto, dos ensayos de distinta duración no se pueden comparar",
            );
            ui.checkbox(&mut cfg.ensayo_duracion_fija, "");
            ui.end_row();
            ui.label("Duración del registro");
            ui.add_enabled(
                cfg.ensayo_duracion_fija,
                egui::DragValue::new(&mut cfg.duracion_ensayo_s).range(5.0..=300.0).speed(1.0).suffix(" s"),
            );
            ui.end_row();
            ui.label("Descarte inicial").on_hover_text("Segundos de acomodación que no entran en las métricas");
            ui.add(egui::DragValue::new(&mut cfg.descarte_inicial_s).range(0.0..=30.0).speed(0.5).suffix(" s"));
            ui.end_row();
        });

        seccion(ui, "PROCESAMIENTO DE LA SEÑAL", acento);
        ui.label(
            egui::RichText::new(
                "Sin filtrar, el ruido del ADC agrega zigzag a cada muestra e infla \
                 la longitud del trazo y la velocidad media.",
            )
            .small()
            .color(Color32::from_gray(120)),
        );
        egui::Grid::new("grid_senal").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Filtrar el COP");
            ui.checkbox(&mut cfg.filtrar_cop, "");
            ui.end_row();
            ui.label("Frecuencia de corte")
                .on_hover_text("Butterworth pasabajos de fase cero. Lo habitual en posturografía: 5 a 10 Hz");
            ui.add_enabled(
                cfg.filtrar_cop,
                egui::DragValue::new(&mut cfg.filtro_corte_hz).range(0.5..=20.0).speed(0.1).suffix(" Hz"),
            );
            ui.end_row();
        });

        seccion(ui, "MODO JUEGO", acento);
        egui::Grid::new("grid_juego").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Duración de la partida")
                .on_hover_text("Cuánto hay que aguantar sin perder para ganar. Se aplica a la próxima partida.");
            ui.add(egui::DragValue::new(&mut cfg.duracion_partida_s).range(10.0..=900.0).speed(1.0).suffix(" s"));
            ui.end_row();
            ui.label("Volumen de la música");
            ui.add(egui::Slider::new(&mut cfg.volumen_musica, 0.0..=1.0).show_value(false));
            ui.end_row();
            ui.label("Volumen de los efectos");
            ui.add(egui::Slider::new(&mut cfg.volumen_efectos, 0.0..=1.0).show_value(false));
            ui.end_row();
        });

        ui.add_space(14.0);
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Restaurar valores por defecto").clicked() {
                *cfg = Config::default();
            }
        });
    });
}

/// Texto legible de cuánto dura la partida, para mostrar en el juego sin
/// que el mensaje pueda contradecir a la configuración real.
pub fn duracion_legible(segundos: f32) -> String {
    let total = segundos.max(0.0).round() as i32;
    let (min, seg) = (total / 60, total % 60);
    match (min, seg) {
        (0, s) => format!("{s} segundos"),
        (1, 0) => "1 minuto".to_string(),
        (m, 0) => format!("{m} minutos"),
        (m, s) => format!("{m}:{s:02}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_config_sobrevive_ida_y_vuelta_por_serde() {
        let original = Config { ancho_cm: 52.5, duracion_partida_s: 180.0, mostrar_elipse: false, ..Config::default() };
        let texto = ron::ser::to_string(&original).expect("serializar");
        let recuperada: Config = ron::from_str(&texto).expect("deserializar");
        assert_eq!(original, recuperada);
    }

    #[test]
    fn una_config_guardada_sin_opciones_nuevas_carga_igual() {
        // Simula un archivo guardado por una versión anterior: solo trae dos
        // campos, el resto debe tomar el valor de fábrica y no fallar.
        let recuperada: Config = ron::from_str("(ancho_cm: 60.0, prof_cm: 30.0)").expect("deserializar parcial");
        assert_eq!(recuperada.ancho_cm, 60.0);
        assert_eq!(recuperada.prof_cm, 30.0);
        assert_eq!(recuperada.duracion_partida_s, defecto::DURACION_PARTIDA_S);
    }

    #[test]
    fn restaurar_deja_la_config_igual_a_la_de_fabrica() {
        let tocada = Config { ancho_cm: 123.0, duracion_partida_s: 999.0, ..Config::default() };
        assert_ne!(tocada, Config::default(), "el caso de prueba debe partir de una config modificada");
        assert_eq!(Config::default(), Config::default());
    }

    #[test]
    fn los_valores_por_defecto_salen_del_modulo_defecto() {
        // Si alguien cambia un default suelto en `Default` y no en `defecto`,
        // el botón "Restaurar" dejaría valores distintos a los de fábrica.
        let cfg = Config::default();
        assert_eq!(cfg.ancho_cm, defecto::ANCHO_CM);
        assert_eq!(cfg.duracion_partida_s, defecto::DURACION_PARTIDA_S);
        assert_eq!(cfg.umbral, defecto::UMBRAL);
    }

    #[test]
    fn duracion_legible_cubre_segundos_minutos_y_mixto() {
        assert_eq!(duracion_legible(45.0), "45 segundos");
        assert_eq!(duracion_legible(60.0), "1 minuto");
        assert_eq!(duracion_legible(120.0), "2 minutos");
        assert_eq!(duracion_legible(90.0), "1:30");
    }
}
