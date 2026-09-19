//! La lista de pacientes: buscador, orden, alta y tabla.
//!
//! Lo que hace falta saber de un paciente ANTES de abrirlo es la edad —un
//! examen de equilibrio se lee distinto a los 20 que a los 80—, cuántos
//! estudios tiene y cuándo fue el último. Las cuatro cosas salen de la misma
//! consulta (ver [`crate::pacientes::Fila`]) y se muestran en columnas.

use egui::RichText;
use egui_extras::{Column, TableBuilder};

use super::{Alta, Pacientes, Pestana};
use crate::fecha;
use crate::pacientes::{Orden, Paciente};

impl Pacientes {
    pub(super) fn lista_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label("🔍");
            // El botón primero y de derecha a izquierda, y el buscador con lo
            // que sobre. Al revés —restándole al ancho un número fijo para el
            // botón— el botón sale cortado en cuanto su texto no mide lo que se
            // supuso.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("＋ Nuevo").clicked() {
                    self.alta.abierto = !self.alta.abierto;
                }
                // Solo consulta cuando el texto cambió, no en cada frame.
                let campo = ui.add(
                    egui::TextEdit::singleline(&mut self.busqueda)
                        .desired_width(ui.available_width())
                        .hint_text("nombre o ficha"),
                );
                if campo.changed() {
                    self.refrescar();
                }
            });
        });

        ui.horizontal(|ui| {
            ui.label(RichText::new("Orden").weak().small());
            let mut orden = self.orden;
            for (modo, etiqueta) in [
                (Orden::UltimoExamen, "último examen"),
                (Orden::Nombre, "nombre"),
                (Orden::Ficha, "ficha"),
                (Orden::Alta, "alta"),
            ] {
                if ui.selectable_label(orden == modo, RichText::new(etiqueta).small()).clicked() {
                    orden = modo;
                }
            }
            if orden != self.orden {
                self.orden = orden;
                self.refrescar();
            }
        });

        if self.alta.abierto {
            self.alta_ui(ui);
        }

        ui.separator();
        self.tabla_ui(ui);
    }

    /// El formulario de alta.
    fn alta_ui(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            egui::Grid::new("alta_paciente").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                ui.label("Nombre");
                ui.add(egui::TextEdit::singleline(&mut self.alta.nombre).desired_width(180.0));
                ui.end_row();
                ui.label("Ficha");
                ui.add(egui::TextEdit::singleline(&mut self.alta.ficha).desired_width(120.0));
                ui.end_row();
                ui.label("Nacimiento");
                fecha_ui(ui, &mut self.alta.nacimiento, "alta_nacimiento");
                ui.end_row();
            });
            // Sin nombre no hay paciente: una ficha con el número solo no se
            // puede buscar ni verificar contra nada.
            let puede = !self.alta.nombre.trim().is_empty();
            if ui.add_enabled(puede, egui::Button::new("✓ Dar de alta")).clicked() {
                self.crear_paciente();
            }
        });
    }

    fn crear_paciente(&mut self) {
        let Some(base) = &self.base else { return };
        let nacimiento = self.alta.nacimiento.map(fecha::fmt_dia).unwrap_or_default();
        let nombre = self.alta.nombre.trim().to_string();
        let ficha = self.alta.ficha.trim().to_string();
        match base.crear_paciente(&ficha, &nombre, &nacimiento, "") {
            Ok(id) => {
                let nuevo = Paciente { id, ficha, nombre, nacimiento, notas: String::new() };
                self.alta = Alta::default();
                self.estado.limpiar();
                self.refrescar();
                // Recién dado de alta es el que se va a examinar ahora.
                self.elegir(nuevo);
                self.pestana = Pestana::Ficha;
            }
            Err(e) => self.estado.error(e),
        }
    }

    fn tabla_ui(&mut self, ui: &mut egui::Ui) {
        if self.filas.is_empty() {
            // "No hay pacientes" y "no hay ninguno que coincida" son dos cosas
            // distintas, y la segunda se arregla borrando el buscador.
            let texto = if self.busqueda.trim().is_empty() {
                "La base no tiene pacientes todavía. El alta también se ve desde vHIT."
            } else {
                "Ningún paciente coincide con la búsqueda."
            };
            ui.add_space(10.0);
            ui.label(RichText::new(texto).weak());
            return;
        }

        let hoy = fecha::hoy();
        let id_elegido = self.elegido.as_ref().map(|p| p.id);
        let mut elegido: Option<Paciente> = None;

        // Sin texto seleccionable dentro de la tabla, y no es estético: una
        // etiqueta seleccionable **sensa arrastre** para poder seleccionar su
        // texto, y ese arrastre le gana al clic de la fila entera, así que la
        // fila deja de ser clickeable. Nadie quiere seleccionar el nombre de un
        // paciente con el mouse: quiere abrirlo.
        ui.style_mut().interaction.selectable_labels = false;

        TableBuilder::new(ui)
            .striped(true)
            // Sin columnas redimensionables y sin encogerse a lo ancho: la tabla
            // ocupa la columna y ni una pizca más.
            .resizable(false)
            .auto_shrink([false, false])
            // Toda la fila responde al clic, no solo el texto del nombre.
            .sense(egui::Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            // Anchos fijos para los campos cortos y el nombre con lo que sobre:
            // el contenido es conocido —una ficha, una edad, una cuenta y una
            // fecha— así que no hay nada que medir y la tabla queda quieta.
            .column(Column::exact(58.0))
            .column(Column::remainder().at_least(90.0).clip(true))
            .column(Column::exact(34.0))
            .column(Column::exact(28.0))
            .column(Column::exact(76.0))
            .min_scrolled_height(100.0)
            .header(20.0, |mut h| {
                for titulo in ["Ficha", "Nombre", "Edad", "n", "Último"] {
                    h.col(|ui| {
                        ui.label(RichText::new(titulo).strong().small());
                    });
                }
            })
            .body(|body| {
                let filas = self.filas.len();
                body.rows(21.0, filas, |mut row| {
                    let fila = &self.filas[row.index()];
                    let p = &fila.paciente;
                    row.set_selected(id_elegido == Some(p.id));

                    row.col(|ui| {
                        ui.label(RichText::new(&p.ficha).monospace().small());
                    });
                    row.col(|ui| {
                        ui.label(&p.nombre);
                    });
                    row.col(|ui| {
                        // Sin fecha de nacimiento no hay edad, y un guion dice
                        // eso; un "0" diría que es un recién nacido.
                        match fecha::edad(&p.nacimiento, hoy) {
                            Some(a) => ui.label(RichText::new(format!("{a}")).small()),
                            None => ui.label(RichText::new("—").weak().small()),
                        };
                    });
                    row.col(|ui| {
                        if fila.examenes == 0 {
                            ui.label(RichText::new("—").weak().small());
                        } else {
                            ui.label(RichText::new(format!("{}", fila.examenes)).small());
                        }
                    });
                    row.col(|ui| match &fila.ultimo_examen {
                        Some(iso) => {
                            ui.label(RichText::new(fecha::local_dia(iso)).monospace().small());
                        }
                        None => {
                            // En una suite es el caso normal: el alta la hizo
                            // vHIT y acá todavía no vino.
                            ui.label(RichText::new("nunca").weak().small());
                        }
                    });

                    if row.response().clicked() {
                        elegido = Some(p.clone());
                    }
                });
            });

        if let Some(p) = elegido {
            self.elegir(p);
        }
    }
}

/// Selector de fecha con "sin fecha" como estado de primera clase.
///
/// El selector de `egui_extras` trabaja sobre una fecha y no sobre una opción,
/// así que no tiene forma de expresar "no la sé". Y hace falta: una fecha de
/// nacimiento inventada para llenar el campo es peor que un campo vacío, porque
/// después alguien calcula una edad con ella.
///
/// Devuelve `true` si el valor cambió en este frame.
pub(super) fn fecha_ui(ui: &mut egui::Ui, valor: &mut Option<jiff::civil::Date>, salt: &str) -> bool {
    let mut cambio = false;
    ui.horizontal(|ui| match valor {
        Some(d) => {
            let mut dia = *d;
            if ui.add(egui_extras::DatePickerButton::new(&mut dia).id_salt(salt).format("%Y-%m-%d")).changed() {
                *valor = Some(dia);
                cambio = true;
            }
            if ui.small_button("✖").on_hover_text("Sin fecha de nacimiento").clicked() {
                *valor = None;
                cambio = true;
            }
        }
        None => {
            if ui.button("📅 Fecha").on_hover_text("Se puede dejar sin fecha: es distinto de inventarla").clicked() {
                *valor = Some(fecha::hoy_menos_40());
                cambio = true;
            }
        }
    });
    cambio
}
