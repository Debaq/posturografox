//! Ventana de pacientes: alta, búsqueda, ficha, historial y evolución.
//!
//! Es el mismo sistema de gestión que usa vHIT, traído acá y conectado a la
//! **misma base** (ver [`crate::almacenamiento`] y [`crate::pacientes`]). Lo que
//! cambia es qué se guarda de cada visita: allá pulsos de impulso cefálico, acá
//! exámenes de posturografía con su COP crudo.
//!
//! # Ventana y no modal
//!
//! En vHIT esta pantalla es modal: mientras se elige a quién se le hace el
//! estudio, nada de lo de atrás importa. Acá no puede serlo: la plataforma sigue
//! entregando muestras a 100 Hz y el operador tiene que poder mirar el COP
//! mientras busca la ficha. Así que es una ventana como las de configuración,
//! calibración e historial, y se mueve al costado.
//!
//! # La base se abre desde acá, y no al arrancar
//!
//! Pedir la frase de paso antes de poder ver el COP obligaría a escribirla para
//! cualquier uso: probar el equipo, calibrar celdas, jugar un rato con alguien
//! que no es un paciente. Si nadie abre la base, el programa funciona igual y
//! archiva como siempre en `historial.ronl`.
//!
//! # Lo que se ve de un examen guardado es lo que se midió ese día
//!
//! El detalle dibuja el COP crudo que está en la base con **la configuración con
//! la que se midió**, que también está guardada. No se recalcula nada con los
//! valores de hoy: dos áreas distintas para el mismo examen, según cuándo se lo
//! abrió, sería lo peor que podría hacer un registro clínico.

mod ficha;
mod lista;

use egui::{Color32, RichText};

use crate::almacenamiento::Almacenamiento;
use crate::app::{AMARILLO, CORAL, LILA, VERDE};
use crate::pacientes::{Base, DetalleExamen, ExamenNuevo, Fila, Orden, Paciente, ResumenExamen};

/// Cuántos pacientes trae la búsqueda.
///
/// Es un tope de la consulta, no de la tabla: con más que esto en pantalla nadie
/// encuentra a nadie leyendo, se escribe en el buscador. El tope existe para que
/// abrir la ventana con una base grande no lea la base entera.
const MAX_RESULTADOS: usize = 200;

/// Cuántos exámenes trae el historial de un paciente.
const MAX_HISTORIAL: usize = 100;

/// Ancho útil de la tarjeta que pide la frase de paso o configura la base.
const ANCHO_PORTAL: f32 = 420.0;

/// Último mensaje de la ventana, con su gravedad.
///
/// La gravedad no es decoración: "examen guardado" y "no pude guardar" tienen
/// que distinguirse de un vistazo.
#[derive(Default)]
struct Estado {
    texto: String,
    malo: bool,
}

impl Estado {
    fn ok(&mut self, texto: impl Into<String>) {
        self.texto = texto.into();
        self.malo = false;
    }

    fn error(&mut self, texto: impl std::fmt::Display) {
        self.texto = texto.to_string();
        self.malo = true;
    }

    fn limpiar(&mut self) {
        self.texto.clear();
        self.malo = false;
    }

    fn ui(&self, ui: &mut egui::Ui) {
        if self.texto.is_empty() {
            return;
        }
        let color = if self.malo { CORAL } else { ui.visuals().weak_text_color() };
        let icono = if self.malo { "⚠" } else { "✓" };
        ui.label(RichText::new(format!("{icono} {}", self.texto)).color(color));
    }
}

/// Pestaña del panel de detalle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Pestana {
    #[default]
    Ficha,
    Historial,
    Evolucion,
}

/// El formulario de alta.
#[derive(Default)]
struct Alta {
    abierto: bool,
    nombre: String,
    ficha: String,
    /// `None` es "no la sé", que es un estado legítimo y distinto de una fecha
    /// puesta al azar para llenar el campo.
    nacimiento: Option<jiff::civil::Date>,
}

/// Un examen guardado, ya leído de la base y listo para dibujar.
///
/// Se guarda entero porque el detalle se redibuja en cada frame y el COP de un
/// examen son miles de muestras: releer y decodificar el BLOB a 30 Hz contra una
/// base cifrada no es una opción.
struct ExamenCargado {
    detalle: DetalleExamen,
    /// Notas editables del examen, y si hay algo sin guardar.
    notas: String,
    notas_sucias: bool,
}

#[derive(Default)]
pub struct Pacientes {
    base: Option<Base>,
    frase: String,
    frase_nueva: String,
    estado: Estado,

    /// Dónde está la base y si está cifrada. Se lee del disco al arrancar.
    almacen: Almacenamiento,
    /// Copia en edición mientras el asistente está abierto. `None` = cerrado.
    asistente: Option<Almacenamiento>,

    busqueda: String,
    orden: Orden,
    filas: Vec<Fila>,

    /// El paciente elegido, **como se lo está editando**.
    elegido: Option<Paciente>,
    /// El mismo, como está en la base. La diferencia entre los dos es lo que
    /// falta guardar, y es lo que permite avisar antes de perderlo.
    guardado: Option<Paciente>,
    historial: Vec<ResumenExamen>,

    alta: Alta,

    /// Quién opera. Es de la jornada y no del paciente: se escribe una vez al
    /// empezar el turno y vale para todos los exámenes que siguen.
    pub operador: String,

    pestana: Pestana,
    examen_elegido: Option<i64>,
    cargado: Option<ExamenCargado>,

    confirmar_borrar_paciente: bool,
    confirmar_borrar_examen: bool,
}

impl Pacientes {
    pub fn nueva() -> Self {
        Self { almacen: Almacenamiento::cargar(), ..Default::default() }
    }

    // --- lo que la aplicación necesita saber ------------------------------

    /// El paciente elegido, si hay base abierta y alguien elegido en ella.
    pub fn paciente(&self) -> Option<&Paciente> {
        self.elegido.as_ref()
    }

    /// `true` si un examen que termine ahora se puede archivar en la base.
    pub fn puede_archivar(&self) -> bool {
        self.base.is_some() && self.elegido.is_some()
    }

    /// El id del paciente elegido, para llenar un [`ExamenNuevo`].
    pub fn paciente_id(&self) -> Option<i64> {
        self.elegido.as_ref().map(|p| p.id)
    }

    /// Archiva un examen en la base de la suite.
    ///
    /// Devuelve el texto para la barra de estado —bien o mal—, porque esto se
    /// llama justo cuando el paciente se baja de la plataforma y el operador
    /// está mirando esa línea y no esta ventana.
    pub fn archivar(&mut self, examen: &ExamenNuevo) -> Result<i64, String> {
        let Some(base) = &self.base else { return Err("la base de pacientes no está abierta".to_string()) };
        let id = base.guardar_examen(examen).map_err(|e| e.to_string())?;
        // Recién guardado, seleccionado y a la vista: es lo que se quiere ver a
        // continuación, y confirma que quedó bien guardado.
        self.refrescar();
        self.examen_elegido = Some(id);
        self.pestana = Pestana::Historial;
        self.cargar_examen(id);
        self.estado.ok(format!("Examen #{id} archivado en la base"));
        Ok(id)
    }

    // --- consultas --------------------------------------------------------

    fn refrescar(&mut self) {
        let Some(base) = &self.base else { return };
        match base.buscar_pacientes(&self.busqueda, self.orden, MAX_RESULTADOS) {
            Ok(filas) => self.filas = filas,
            Err(e) => self.estado.error(e),
        }
        self.refrescar_historial();
    }

    fn refrescar_historial(&mut self) {
        let Some(base) = &self.base else { return };
        let Some(p) = &self.elegido else {
            self.historial.clear();
            return;
        };
        match base.examenes_de(p.id, MAX_HISTORIAL) {
            Ok(h) => self.historial = h,
            Err(e) => self.estado.error(e),
        }
    }

    /// Elige un paciente y deja el panel de detalle en un estado coherente.
    fn elegir(&mut self, p: Paciente) {
        if self.elegido.as_ref().is_some_and(|s| s.id == p.id) {
            return;
        }
        self.elegido = Some(p.clone());
        self.guardado = Some(p);
        // Lo que se estaba mirando era de OTRO paciente: mostrarlo bajo el
        // nombre nuevo sería atribuirle un examen que no es suyo.
        self.examen_elegido = None;
        self.cargado = None;
        self.confirmar_borrar_paciente = false;
        self.confirmar_borrar_examen = false;
        self.refrescar_historial();
    }

    /// `true` si la ficha abierta tiene cambios que no están en la base.
    fn sucia(&self) -> bool {
        match (&self.elegido, &self.guardado) {
            (Some(a), Some(b)) => a != b,
            _ => false,
        }
    }

    /// Trae un examen guardado y lo deja listo para dibujar.
    fn cargar_examen(&mut self, id: i64) {
        let Some(base) = &self.base else { return };
        if self.cargado.as_ref().is_some_and(|c| c.detalle.resumen.id == id) {
            return;
        }
        match base.detalle_examen(id) {
            Ok(Some(detalle)) => {
                self.cargado = Some(ExamenCargado { notas: detalle.notas.clone(), notas_sucias: false, detalle })
            }
            Ok(None) => self.estado.error(format!("el examen #{id} ya no está en la base")),
            Err(e) => self.estado.error(e),
        }
    }

    // --- ventana ----------------------------------------------------------

    pub fn ui(&mut self, ctx: &egui::Context, abierta: &mut bool) {
        if !*abierta {
            return;
        }
        let mut visible = true;
        egui::Window::new("👥 Pacientes")
            .open(&mut visible)
            .default_width(940.0)
            .default_height(560.0)
            .resizable(true)
            .collapsible(false)
            .show(ctx, |ui| self.contenido(ui));
        *abierta = visible;
    }

    fn contenido(&mut self, ui: &mut egui::Ui) {
        // El asistente y el portal se quedan con la ventana entera: mientras no
        // haya base abierta no hay nada que listar, y media pantalla de tabla
        // vacía al lado de un campo de frase de paso solo distrae.
        if self.asistente.is_some() {
            self.asistente_ui(ui);
            return;
        }
        if self.base.is_none() {
            self.portal_ui(ui);
            return;
        }

        self.barra_ui(ui);
        ui.separator();

        // El cuerpo se reparte a mano en dos columnas de rectángulo fijo: la
        // tabla de pacientes pide todo el alto que se le ofrezca, y adentro de
        // una ventana redimensionable eso se realimenta con el alto de la
        // ventana hasta dejarla en blanco.
        let alto = (ui.available_height() - 26.0).max(220.0);
        let ancho = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ancho, alto), egui::Sense::hover());
        let ancho_lista = (ancho * 0.38).clamp(280.0, 440.0);
        let rect_lista = egui::Rect::from_min_size(rect.min, egui::vec2(ancho_lista, alto));
        let rect_detalle = egui::Rect::from_min_max(egui::pos2(rect_lista.right() + 8.0, rect.top()), rect.max);
        let columna =
            |r: egui::Rect| egui::UiBuilder::new().max_rect(r).layout(egui::Layout::top_down(egui::Align::Min));

        let mut ui_lista = ui.new_child(columna(rect_lista));
        self.lista_ui(&mut ui_lista);
        let mut ui_detalle = ui.new_child(columna(rect_detalle));
        self.detalle_ui(&mut ui_detalle);
        // La línea divisoria, dibujada y no insertada: entre dos hijos con
        // rectángulo propio no hay un hueco donde meter un `separator`.
        ui.painter().vline(rect_lista.right() + 4.0, rect.y_range(), ui.visuals().widgets.noninteractive.bg_stroke);

        ui.separator();
        self.estado.ui(ui);
    }

    /// La pantalla previa a tener base abierta.
    ///
    /// Con cifrado pide la frase. **Sin cifrado no pide nada**: no hay ningún
    /// secreto que verificar, así que un botón "Abrir" sería un trámite
    /// inventado. Se abre sola y se avisa, arriba y en rojo, que está en claro.
    fn portal_ui(&mut self, ui: &mut egui::Ui) {
        // Sin configurar, lo primero es decir dónde está la base de la suite.
        if !self.almacen.configurado {
            self.asistente = Some(self.almacen.clone());
            return;
        }
        if !self.almacen.cifrada && self.estado.texto.is_empty() {
            // Un intento por frame mientras no abra. Si falla —permisos, disco
            // lleno, una base que en realidad sí está cifrada— el error queda a
            // la vista y no se reintenta en bucle silencioso.
            self.abrir_base(None);
        }

        tarjeta_centrada(ui, ANCHO_PORTAL, |ui| {
            if self.almacen.cifrada {
                encabezado(
                    ui,
                    "🔒",
                    ui.visuals().weak_text_color(),
                    "Frase de paso",
                    &[
                        "La base de pacientes de la suite está cifrada.",
                        "Si la frase se pierde, los datos NO se recuperan.",
                    ],
                );
                ui.add_space(14.0);
                let campo = ui.add(
                    egui::TextEdit::singleline(&mut self.frase)
                        .password(true)
                        .desired_width(f32::INFINITY)
                        .hint_text("Frase de paso"),
                );
                let con_enter = campo.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(8.0);
                let puede = !self.frase.is_empty();
                let boton = egui::Button::new(RichText::new("🔓 Abrir la base").strong())
                    .min_size(egui::vec2(ui.available_width(), 28.0));
                if ui.add_enabled(puede, boton).clicked() || (con_enter && puede) {
                    let frase = self.frase.clone();
                    self.abrir_base(Some(&frase));
                }
            } else {
                encabezado(
                    ui,
                    "⚠",
                    CORAL,
                    "La base NO está cifrada",
                    &[
                        "Un examen de equilibrio es un dato de salud.",
                        "Cualquiera que llegue al archivo lo abre con un visor de SQLite.",
                    ],
                );
            }
            self.estado.ui(ui);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("⚙ Dónde está la base").clicked() {
                    self.asistente = Some(self.almacen.clone());
                    self.estado.limpiar();
                }
                ui.label(RichText::new(self.almacen.ruta_base().display().to_string()).small().weak());
            });
        });
    }

    /// Dónde vive la base y si se cifra. Es el asistente del primer arranque, y
    /// también la forma de mudar la base después.
    fn asistente_ui(&mut self, ui: &mut egui::Ui) {
        let Some(mut editado) = self.asistente.clone() else { return };
        let primera_vez = !self.almacen.configurado;
        let mut cerrar = false;
        let mut confirmar = false;

        tarjeta_centrada(ui, 520.0, |ui| {
            encabezado(
                ui,
                "👥",
                VERDE,
                "Base de pacientes de la suite",
                &[
                    "La misma base que usa vHIT: una ficha por persona para los dos equipos.",
                    "Basta con que los dos programas apunten a esta carpeta.",
                ],
            );
            ui.add_space(12.0);

            ui.label(RichText::new("Carpeta de la base").strong());
            ui.add(egui::TextEdit::singleline(&mut editado.carpeta).desired_width(f32::INFINITY));
            ui.label(RichText::new(format!("El archivo será {}", editado.ruta_base().display())).small().weak());
            ui.add_space(12.0);

            let existe = editado.base_existe();
            if existe {
                // Con una base ya hecha, la política de cifrado no se elige: es
                // la que tiene el archivo. Ofrecerlo igual haría creer que se
                // puede cifrar una base en claro cambiando una casilla, y no se
                // puede (ver `Base::recifrar`).
                ui.label(RichText::new("Ya hay una base ahí. Se va a usar tal como está.").color(AMARILLO));
                ui.label(
                    RichText::new("Si la creó vHIT cifrada, pedirá su frase de paso; si está en claro, abre sola.")
                        .small()
                        .weak(),
                );
                // La política real la dice el archivo, no esta pantalla: si es
                // un SQLite legible, está en claro.
                editado.cifrada = !archivo_sqlite_en_claro(&editado.ruta_base());
            } else {
                ui.label(RichText::new("Cifrado").strong());
                ui.radio_value(&mut editado.cifrada, true, "Cifrar la base (recomendado)");
                ui.label(
                    RichText::new(
                        "SQLCipher AES-256. La frase se pide al abrir y no se guarda en ningún lado: \
                         si se pierde, los datos no se recuperan.",
                    )
                    .small()
                    .weak(),
                );
                ui.radio_value(&mut editado.cifrada, false, "Dejarla sin cifrar");
                ui.label(
                    RichText::new(
                        "Los datos de salud piden cifrado en reposo (ley 19.628, GDPR, HIPAA). \
                         Sin cifrar, el archivo lo abre cualquiera que llegue al disco.",
                    )
                    .small()
                    .color(CORAL),
                );
            }

            ui.add_space(14.0);
            ui.horizontal(|ui| {
                let puede = !editado.carpeta.trim().is_empty();
                if ui.add_enabled(puede, egui::Button::new(RichText::new("✓ Confirmar").strong())).clicked() {
                    confirmar = true;
                }
                if !primera_vez && ui.button("Cancelar").clicked() {
                    cerrar = true;
                }
            });
            self.estado.ui(ui);
        });

        if confirmar {
            let cifrada_antes = self.almacen.cifrada;
            match editado.crear_carpeta() {
                Ok(()) => {
                    // La constancia de la política se fecha solo si CAMBIÓ: una
                    // mudanza de carpeta no es una decisión sobre el cifrado.
                    if editado.cifrada != cifrada_antes || self.almacen.cifrado_decidido_en.is_empty() {
                        editado.fijar_cifrado(editado.cifrada);
                    }
                    editado.configurado = true;
                    if let Err(e) = editado.guardar() {
                        self.estado.error(e);
                    } else {
                        self.estado.limpiar();
                    }
                    self.almacen = editado;
                    // Cambiar de base invalida la que estuviera abierta: sus
                    // pacientes son otros.
                    self.cerrar_base();
                    self.asistente = None;
                }
                Err(e) => {
                    self.estado.error(e);
                    self.asistente = Some(editado);
                }
            }
        } else if cerrar {
            self.asistente = None;
            self.estado.limpiar();
        } else {
            self.asistente = Some(editado);
        }
    }

    fn abrir_base(&mut self, frase: Option<&str>) {
        match Base::abrir(&self.almacen.ruta_base(), frase) {
            Ok(base) => {
                self.base = Some(base);
                // La frase no se conserva en memoria más de lo necesario.
                self.frase.clear();
                self.estado.limpiar();
                self.refrescar();
            }
            Err(e) => self.estado.error(e),
        }
    }

    /// Cierra la base: deja de guardar y olvida la frase.
    ///
    /// Es lo que hay que hacer al terminar con un paciente, y por eso está a la
    /// vista y no escondido. El programa sigue midiendo.
    fn cerrar_base(&mut self) {
        self.base = None;
        self.elegido = None;
        self.guardado = None;
        self.filas.clear();
        self.historial.clear();
        self.examen_elegido = None;
        self.cargado = None;
        self.confirmar_borrar_paciente = false;
        self.confirmar_borrar_examen = false;
        self.frase.clear();
        self.frase_nueva.clear();
        self.estado.limpiar();
    }

    /// Barra de arriba: quién opera, el aviso de base en claro y cerrar la base.
    fn barra_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Operador");
            ui.add(egui::TextEdit::singleline(&mut self.operador).desired_width(150.0).hint_text("quién examina"));

            // Mientras la base esté en claro se dice acá, cada vez que se abre
            // la ventana. Una decisión de este tamaño no puede quedar tomada una
            // vez en el asistente y no volver a verse nunca.
            if !self.almacen.cifrada {
                ui.separator();
                ui.label(RichText::new("⚠ Base sin cifrar").color(CORAL).strong()).on_hover_text(
                    "Los datos de salud piden cifrado en reposo. Este archivo lo abre \
                     cualquiera que llegue al disco.",
                );
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button("🔒 Cerrar base")
                    .on_hover_text("Deja de guardar y olvida la frase. El programa sigue midiendo.")
                    .clicked()
                {
                    self.cerrar_base();
                }
                self.menu_base_ui(ui);
            });
        });
    }

    /// Operaciones de la base —no del paciente—, detrás de un menú: cambiar la
    /// frase de paso y mudar la base.
    fn menu_base_ui(&mut self, ui: &mut egui::Ui) {
        let cifrada = self.base.as_ref().is_some_and(|b| b.esta_cifrada());
        ui.menu_button("⚙", |ui| {
            if ui.button("⚙ Dónde está la base").clicked() {
                self.asistente = Some(self.almacen.clone());
                ui.close();
            }
            ui.label(RichText::new(self.almacen.ruta_base().display().to_string()).small().weak());
            // Sobre una base sin cifrar no hay frase que cambiar. Ofrecerlo
            // igual daría a entender que hay un secreto donde no lo hay.
            if !cifrada {
                return;
            }
            ui.separator();
            ui.label(RichText::new("Cambiar la frase de paso").strong());
            ui.label(
                RichText::new(
                    "Vale para toda la suite: es la misma base que abre vHIT. \
                     Si se olvida la nueva, los datos no se recuperan.",
                )
                .small()
                .weak(),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.frase_nueva)
                    .password(true)
                    .desired_width(220.0)
                    .hint_text("frase nueva"),
            );
            let puede = !self.frase_nueva.is_empty();
            if ui.add_enabled(puede, egui::Button::new("🔒 Confirmar")).clicked() {
                if let Some(base) = &self.base {
                    match base.recifrar(&self.frase_nueva) {
                        Ok(()) => self.estado.ok("frase de paso cambiada"),
                        Err(e) => self.estado.error(e),
                    }
                }
                self.frase_nueva.clear();
                ui.close();
            }
        });
    }
}

/// Una tarjeta centrada, del ancho pedido: es la forma que tienen el portal y el
/// asistente. Antes eran widgets sueltos centrados uno por uno, cada uno con su
/// ancho, y el conjunto se leía desordenado.
fn tarjeta_centrada(ui: &mut egui::Ui, ancho: f32, contenido: impl FnOnce(&mut egui::Ui)) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.12).clamp(8.0, 48.0));
            let v = ui.visuals().clone();
            egui::Frame::new()
                .fill(v.window_fill)
                .stroke(egui::Stroke::new(1.2, LILA.gamma_multiply(0.55)))
                .corner_radius(10.0)
                .inner_margin(egui::Margin::same(20))
                .show(ui, |ui| {
                    ui.set_width(ancho);
                    contenido(ui);
                });
        });
    });
}

/// Icono, título y las líneas de explicación de una tarjeta.
fn encabezado(ui: &mut egui::Ui, icono: &str, color: Color32, titulo: &str, ayuda: &[&str]) {
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(icono).size(32.0).color(color));
        ui.add_space(6.0);
        ui.label(RichText::new(titulo).strong().size(17.0).color(color));
        ui.add_space(4.0);
        for linea in ayuda {
            ui.label(RichText::new(*linea).weak().small());
        }
    });
}

/// Si el archivo del disco es un SQLite **sin cifrar**.
///
/// Se pregunta para no dejar que el asistente diga "cifrada" sobre una base que
/// vHIT creó en claro: la política real la tiene el archivo.
fn archivo_sqlite_en_claro(ruta: &std::path::Path) -> bool {
    use std::io::Read;
    let Ok(mut archivo) = std::fs::File::open(ruta) else { return false };
    let mut cabecera = [0u8; 16];
    archivo.read_exact(&mut cabecera).is_ok() && &cabecera == b"SQLite format 3\0"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sin_base_abierta_no_se_archiva_nada() {
        // Y el mensaje tiene que decir por qué, porque esto se lee en la barra
        // de estado justo cuando el paciente se bajó de la plataforma.
        let mut p = Pacientes::default();
        assert!(!p.puede_archivar());
        let examen = ExamenNuevo {
            paciente_id: 1,
            operador: String::new(),
            tipo: crate::pacientes::TipoExamen::Estatica,
            superficie: Default::default(),
            condicion: Default::default(),
            metricas: None,
            config: Default::default(),
            juego: None,
            limites: Vec::new(),
            notas: String::new(),
            registro: Vec::new(),
        };
        let error = p.archivar(&examen).expect_err("sin base no hay dónde guardar");
        assert!(error.contains("no está abierta"), "{error}");
    }

    #[test]
    fn la_ficha_sucia_es_la_que_no_coincide_con_la_base() {
        let mut p = Pacientes::default();
        assert!(!p.sucia(), "sin paciente no hay nada sucio");
        let guardado = Paciente { id: 1, nombre: "Ana".into(), ..Default::default() };
        p.elegido = Some(guardado.clone());
        p.guardado = Some(guardado);
        assert!(!p.sucia());
        p.elegido.as_mut().unwrap().nombre = "Ana Torres".into();
        assert!(p.sucia(), "el nombre cambiado tiene que avisar antes de perderse");
    }

    #[test]
    fn elegir_otro_paciente_no_le_deja_puesto_el_examen_del_anterior() {
        let mut p = Pacientes::default();
        p.elegido = Some(Paciente { id: 1, nombre: "Ana".into(), ..Default::default() });
        p.guardado = p.elegido.clone();
        p.examen_elegido = Some(77);
        p.elegir(Paciente { id: 2, nombre: "Luis".into(), ..Default::default() });
        assert_eq!(p.paciente_id(), Some(2));
        assert!(p.examen_elegido.is_none(), "atribuirle el examen de otro es el error a hacer imposible");
    }

    #[test]
    fn un_archivo_que_no_existe_no_es_un_sqlite_en_claro() {
        assert!(!archivo_sqlite_en_claro(std::path::Path::new("/no/existe/base.sqlite")));
    }
}
