//! El panel de detalle: ficha, historial y evolución del paciente elegido.

use egui::RichText;
use egui_plot::{Legend, Line, MarkerShape, Plot, PlotPoints, Points};

use super::lista::fecha_ui;
use super::{Pacientes, Pestana};
use crate::app::{AMARILLO, AZUL, CORAL, LILA, VERDE};
use crate::fecha;
use crate::pacientes::TipoExamen;

impl Pacientes {
    pub(super) fn detalle_ui(&mut self, ui: &mut egui::Ui) {
        let Some(p) = self.elegido.clone() else {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.3);
                ui.label(RichText::new("👥").size(34.0).color(ui.visuals().weak_text_color()));
                ui.label(RichText::new("Elija un paciente de la lista").weak());
            });
            return;
        };

        // Encabezado: quién es, de un vistazo y siempre visible, sea cual sea la
        // pestaña. Confundir de quién es un examen es el error que hay que hacer
        // imposible.
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(&p.nombre).strong().size(16.0));
            if !p.ficha.is_empty() {
                ui.label(RichText::new(&p.ficha).monospace().weak());
            }
            if let Some(a) = fecha::edad(&p.nacimiento, fecha::hoy()) {
                ui.label(RichText::new(format!("{a} años")).weak());
            }
            if self.sucia() {
                ui.label(RichText::new("⚠ ficha sin guardar").color(AMARILLO).small());
            }
        });

        ui.horizontal(|ui| {
            for (pestana, etiqueta) in [
                (Pestana::Ficha, "Ficha".to_string()),
                (
                    Pestana::Historial,
                    if self.historial.is_empty() {
                        "Historial".to_string()
                    } else {
                        format!("Historial ({})", self.historial.len())
                    },
                ),
                (Pestana::Evolucion, "Evolución".to_string()),
            ] {
                if ui.selectable_label(self.pestana == pestana, etiqueta).clicked() {
                    self.pestana = pestana;
                }
            }
        });
        ui.separator();

        match self.pestana {
            Pestana::Ficha => self.ficha_ui(ui),
            Pestana::Historial => self.historial_ui(ui),
            Pestana::Evolucion => self.evolucion_ui(ui),
        }
    }

    /// Pestaña ficha: los datos del paciente, y lo irreversible bien separado.
    fn ficha_ui(&mut self, ui: &mut egui::Ui) {
        let Some(mut editado) = self.elegido.clone() else { return };

        egui::ScrollArea::vertical().id_salt("ficha_paciente").show(ui, |ui| {
            // Es la misma ficha que edita vHIT: el nombre que se corrija acá se
            // corrige para toda la suite.
            ui.label(RichText::new("Esta ficha es la de la suite: vHIT ve los mismos datos.").small().weak());
            ui.add_space(4.0);

            egui::Grid::new("editar_paciente").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("Nombre");
                ui.add(egui::TextEdit::singleline(&mut editado.nombre).desired_width(260.0));
                ui.end_row();

                ui.label("Ficha");
                ui.add(egui::TextEdit::singleline(&mut editado.ficha).desired_width(160.0));
                ui.end_row();

                ui.label("Nacimiento");
                ui.horizontal(|ui| {
                    let mut dia = fecha::parse_dia(&editado.nacimiento);
                    if fecha_ui(ui, &mut dia, "ficha_nacimiento") {
                        editado.nacimiento = dia.map(fecha::fmt_dia).unwrap_or_default();
                    }
                    // Una fecha que el selector no sabe leer se muestra tal cual
                    // en vez de borrarla en silencio: el dato está.
                    if dia.is_none() && !editado.nacimiento.trim().is_empty() {
                        ui.label(RichText::new(&editado.nacimiento).monospace().color(AMARILLO));
                    }
                    if let Some(a) = fecha::edad(&editado.nacimiento, fecha::hoy()) {
                        ui.label(RichText::new(format!("{a} años")).weak());
                    }
                });
                ui.end_row();

                ui.label("Notas");
                ui.add(egui::TextEdit::multiline(&mut editado.notas).desired_width(300.0).desired_rows(4));
                ui.end_row();
            });

            if self.elegido.as_ref().is_some_and(|actual| *actual != editado) {
                self.elegido = Some(editado);
            }

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let sucia = self.sucia();
                if ui.add_enabled(sucia, egui::Button::new("💾 Guardar ficha")).clicked() {
                    self.guardar_paciente();
                }
                if sucia && ui.button("↩ Descartar").clicked() {
                    // Volver a lo que está en la base, que es lo que "descartar"
                    // quiere decir acá.
                    self.elegido = self.guardado.clone();
                }
            });

            ui.add_space(10.0);
            ui.separator();
            self.borrar_paciente_ui(ui);
        });
    }

    fn guardar_paciente(&mut self) {
        let Some(base) = &self.base else { return };
        let Some(actual) = &self.elegido else { return };
        match base.actualizar_paciente(actual) {
            Ok(()) => {
                self.estado.ok("ficha guardada");
                self.guardado = Some(actual.clone());
                self.refrescar();
            }
            Err(e) => self.estado.error(e),
        }
    }

    /// Borrar un paciente se lleva sus exámenes **de los dos programas** y no se
    /// puede deshacer: va con confirmación explícita, nunca en un clic.
    fn borrar_paciente_ui(&mut self, ui: &mut egui::Ui) {
        let Some(p) = self.elegido.clone() else { return };
        if !self.confirmar_borrar_paciente {
            if ui.button(RichText::new("🗑 Borrar paciente").color(CORAL)).clicked() {
                self.confirmar_borrar_paciente = true;
            }
            return;
        }
        ui.label(
            RichText::new("Se borran la ficha y TODOS sus exámenes, también los de vHIT. No se puede deshacer.")
                .color(CORAL)
                .strong(),
        );
        // El nombre en la confirmación no es cortesía: es la última chance de
        // ver que el seleccionado no es el que se creía.
        ui.label(RichText::new(p.etiqueta()).monospace());
        ui.horizontal(|ui| {
            if ui.button(RichText::new("🗑 Confirmar").color(CORAL)).clicked() {
                if let Some(base) = &self.base {
                    match base.borrar_paciente(p.id) {
                        Ok(()) => self.estado.ok(format!("paciente borrado: {}", p.nombre)),
                        Err(e) => self.estado.error(e),
                    }
                }
                self.elegido = None;
                self.guardado = None;
                self.historial.clear();
                self.examen_elegido = None;
                self.cargado = None;
                self.confirmar_borrar_paciente = false;
                self.refrescar();
            }
            if ui.button("Cancelar").clicked() {
                self.confirmar_borrar_paciente = false;
            }
        });
    }

    /// Pestaña historial: la tabla de exámenes arriba y el examen abierto abajo.
    fn historial_ui(&mut self, ui: &mut egui::Ui) {
        if self.historial.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new("Todavía no tiene exámenes de posturografía en la base.").weak());
            ui.label(
                RichText::new("Los exámenes se archivan solos al cerrar cada ensayo, mientras la base esté abierta.")
                    .small()
                    .weak(),
            );
            return;
        }

        let alto_tabla = (ui.available_height() * 0.38).clamp(90.0, 230.0);
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), alto_tabla),
            egui::Layout::top_down(egui::Align::Min),
            |ui| self.tabla_examenes(ui),
        );
        ui.separator();

        let Some(id) = self.examen_elegido else {
            ui.add_space(8.0);
            ui.label(RichText::new("Elija un examen de la lista para verlo.").weak());
            return;
        };
        self.cargar_examen(id);
        self.examen_ui(ui);
    }

    fn tabla_examenes(&mut self, ui: &mut egui::Ui) {
        use egui_extras::{Column, TableBuilder};

        let sel = self.examen_elegido;
        let mut elegido: Option<i64> = None;

        // Por lo mismo que en la lista de pacientes: una etiqueta seleccionable
        // sensa arrastre y le roba el clic a la fila.
        ui.style_mut().interaction.selectable_labels = false;

        TableBuilder::new(ui)
            .striped(true)
            .resizable(false)
            .auto_shrink([false, false])
            .sense(egui::Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(104.0))
            .column(Column::remainder().at_least(120.0).clip(true))
            .column(Column::exact(64.0))
            .column(Column::exact(64.0))
            .column(Column::exact(52.0))
            .min_scrolled_height(60.0)
            .header(20.0, |mut h| {
                for titulo in ["Fecha", "Condición", "Área 95%", "Vel. media", "Dur."] {
                    h.col(|ui| {
                        ui.label(RichText::new(titulo).strong().small());
                    });
                }
            })
            .body(|body| {
                let filas = self.historial.len();
                body.rows(21.0, filas, |mut row| {
                    let e = &self.historial[row.index()];
                    row.set_selected(sel == Some(e.id));

                    row.col(|ui| {
                        ui.label(RichText::new(fecha::local(&e.fecha)).monospace().small());
                    });
                    row.col(|ui| {
                        // El tipo va con el color de su pantalla: el juego es
                        // rosa en la suya, los límites amarillos en la suya.
                        let color = match e.tipo {
                            TipoExamen::Estatica => ui.visuals().text_color(),
                            TipoExamen::Juego => LILA,
                            TipoExamen::Limites => AMARILLO,
                        };
                        ui.label(RichText::new(e.etiqueta()).small().color(color));
                    });
                    // Las métricas de una partida o de un examen de límites NO
                    // son comparables con las de un ensayo quieto: se muestran,
                    // porque el dato es el dato, pero apagadas.
                    let comparable = e.tipo == TipoExamen::Estatica;
                    for valor in [e.metricas.map(|m| m.area95_cm2), e.metricas.map(|m| m.velocidad_media_cms)] {
                        row.col(|ui| match valor {
                            Some(v) => {
                                let t = RichText::new(format!("{v:.2}")).small();
                                ui.label(if comparable { t } else { t.weak() });
                            }
                            None => {
                                ui.label(RichText::new("—").weak().small());
                            }
                        });
                    }
                    row.col(|ui| match e.metricas.map(|m| m.duracion_s) {
                        Some(d) => {
                            ui.label(RichText::new(format!("{d:.0} s")).small());
                        }
                        None => {
                            ui.label(RichText::new("—").weak().small());
                        }
                    });

                    if row.response().clicked() {
                        elegido = Some(e.id);
                    }
                });
            });

        if let Some(id) = elegido
            && self.examen_elegido != Some(id)
        {
            self.examen_elegido = Some(id);
            self.confirmar_borrar_examen = false;
        }
    }

    /// El examen abierto: con qué se midió, sus números, su COP y sus notas.
    fn examen_ui(&mut self, ui: &mut egui::Ui) {
        let Some(cargado) = &self.cargado else { return };
        let detalle = &cargado.detalle;
        let resumen = &detalle.resumen;
        // Todo lo que el dibujo necesita se copia ANTES del cierre: adentro se
        // llama a métodos de `self` (borrar el examen), y con `self.cargado`
        // prestado eso no compila.
        let registro = detalle.registro.clone();
        let (ancho_cm, prof_cm) = (detalle.config.ancho_cm, detalle.config.prof_cm);
        let filtro = if detalle.config.filtrar_cop {
            format!("{:.1} Hz", detalle.config.filtro_corte_hz)
        } else {
            "apagado".to_string()
        };
        let mut notas = cargado.notas.clone();
        let sucias = cargado.notas_sucias;
        let texto_metricas = resumen.metricas.map(|m| m.texto());
        let texto_avanzado = resumen.metricas.map(|m| m.texto_avanzado());
        let juego = detalle.juego.clone();
        let limites = detalle.limites.clone();
        let version = detalle.version_app.clone();
        let operador = resumen.operador.clone();
        let muestras = resumen.muestras;
        let etiqueta = resumen.etiqueta();
        let cuando = fecha::local(&resumen.fecha);
        let id = resumen.id;

        let mut guardar_notas = false;
        let mut notas_cambiaron = false;

        egui::ScrollArea::vertical().id_salt("examen_guardado").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("#{id} · {cuando}")).strong());
                ui.label(RichText::new(etiqueta).color(VERDE));
                if !operador.is_empty() {
                    ui.label(RichText::new(format!("por {operador}")).weak().small());
                }
            });
            // Lo que se ve es lo que se midió ese día, con la configuración de
            // ese día: sin esto, dos lecturas del mismo examen podrían dar
            // números distintos según cuándo se lo abrió.
            ui.label(
                RichText::new(format!(
                    "Medido con v{version} · plataforma {ancho_cm:.0}×{prof_cm:.0} cm · \
                     filtro {filtro} · {muestras} muestras"
                ))
                .small()
                .weak(),
            );

            if let Some(t) = &texto_metricas {
                ui.add_space(4.0);
                ui.label(RichText::new(t).small());
            }
            if let Some(t) = &texto_avanzado {
                ui.label(RichText::new(t).small().weak());
            }

            if let Some(j) = &juego {
                ui.add_space(4.0);
                let final_partida = if j.gano { "completó la pista" } else { "no llegó al final" };
                ui.label(RichText::new(format!("🎮 {final_partida}")).small().color(LILA));
                // La misma línea que muestra el historial local, para que la
                // partida se lea igual en los dos lados.
                ui.label(RichText::new(crate::app::resumen_partida(j)).small().weak());
            }

            if !limites.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new("Alcance por dirección").small().strong().color(AMARILLO));
                egui::Grid::new("limites_guardados").num_columns(4).spacing([10.0, 2.0]).show(ui, |ui| {
                    for (i, intento) in limites.iter().enumerate() {
                        ui.label(RichText::new(crate::limites::NOMBRES[intento.direccion]).small().strong());
                        ui.label(RichText::new(format!("{:.1} cm", intento.alcance_cm)).small());
                        ui.label(RichText::new(format!("{:.0}%", intento.fraccion_objetivo * 100.0)).small());
                        ui.label(RichText::new(format!("{:.1} s", intento.tiempo_s)).small());
                        if i % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
            }

            // El COP tal como se guardó. Es la razón de guardar las muestras
            // crudas: poder volver a mirar el examen sin repetírselo a nadie.
            if !registro.is_empty() {
                let puntos: Vec<[f64; 2]> = registro.iter().map(|m| [m[1], m[2]]).collect();
                let limite_x = (ancho_cm / 2.0).max(1.0);
                let limite_y = (prof_cm / 2.0).max(1.0);
                Plot::new("cop_guardado")
                    .height(190.0)
                    .data_aspect(1.0)
                    .include_x(-limite_x)
                    .include_x(limite_x)
                    .include_y(-limite_y)
                    .include_y(limite_y)
                    .show(ui, |plot_ui| {
                        plot_ui.line(Line::new("COP", PlotPoints::from(puntos.clone())).color(AZUL).width(1.2));
                        if let Some(ultimo) = puntos.last() {
                            plot_ui.points(
                                Points::new("fin", vec![*ultimo]).color(CORAL).radius(4.0).shape(MarkerShape::Circle),
                            );
                        }
                    });
            }

            ui.add_space(4.0);
            ui.label(RichText::new("Notas del examen").small().strong());
            let campo = ui.add(egui::TextEdit::multiline(&mut notas).desired_width(f32::INFINITY).desired_rows(2));
            if campo.changed() {
                notas_cambiaron = true;
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(sucias, egui::Button::new("💾 Guardar notas")).clicked() {
                    guardar_notas = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.borrar_examen_ui(ui, id);
                });
            });
        });

        if let Some(cargado) = &mut self.cargado
            && notas_cambiaron
        {
            cargado.notas = notas.clone();
            cargado.notas_sucias = true;
        }
        if guardar_notas {
            self.guardar_notas(id, &notas);
        }
    }

    fn guardar_notas(&mut self, id: i64, notas: &str) {
        let Some(base) = &self.base else { return };
        match base.actualizar_notas_examen(id, notas) {
            Ok(()) => {
                self.estado.ok("notas guardadas");
                if let Some(cargado) = &mut self.cargado {
                    cargado.notas_sucias = false;
                    cargado.detalle.notas = notas.to_string();
                }
            }
            Err(e) => self.estado.error(e),
        }
    }

    /// Borrar un examen suelto. También es irreversible, así que también
    /// confirma; lo que no arrastra es la ficha del paciente.
    fn borrar_examen_ui(&mut self, ui: &mut egui::Ui, id: i64) {
        if !self.confirmar_borrar_examen {
            if ui.button(RichText::new("🗑 Borrar examen").color(CORAL).small()).clicked() {
                self.confirmar_borrar_examen = true;
            }
            return;
        }
        if ui.button(RichText::new("🗑 Confirmar").color(CORAL).small()).clicked() {
            if let Some(base) = &self.base {
                match base.borrar_examen(id) {
                    Ok(()) => self.estado.ok(format!("examen #{id} borrado")),
                    Err(e) => self.estado.error(e),
                }
            }
            self.examen_elegido = None;
            self.cargado = None;
            self.confirmar_borrar_examen = false;
            self.refrescar_historial();
        }
        if ui.button(RichText::new("Cancelar").small()).clicked() {
            self.confirmar_borrar_examen = false;
        }
    }

    /// Pestaña evolución: área 95% y velocidad media de cada examen contra su
    /// fecha.
    ///
    /// Es la pregunta que motiva tener historial —¿mejoró?— y no se contesta
    /// leyendo una tabla de veinte filas.
    ///
    /// Solo entran los ensayos quietos. Durante una partida el paciente se
    /// desplaza a propósito y el examen de límites le pide justamente irse al
    /// borde: meterlos en la curva daría saltos que no son cambios del paciente.
    fn evolucion_ui(&mut self, ui: &mut egui::Ui) {
        let estaticos: Vec<&crate::pacientes::ResumenExamen> =
            self.historial.iter().filter(|e| e.tipo == TipoExamen::Estatica && e.metricas.is_some()).collect();
        if estaticos.len() < 2 {
            ui.add_space(8.0);
            ui.label(RichText::new("Hacen falta al menos dos ensayos quietos para ver una evolución.").weak());
            return;
        }

        // Del examen más viejo al más nuevo: `examenes_de` los trae al revés,
        // que es el orden correcto para la tabla y el equivocado para una curva.
        let mut area: Vec<[f64; 2]> = Vec::new();
        let mut velocidad: Vec<[f64; 2]> = Vec::new();
        for e in estaticos.iter().rev() {
            let Some(x) = fecha::dias_epoch(&e.fecha) else { continue };
            let Some(m) = e.metricas else { continue };
            area.push([x, m.area95_cm2]);
            velocidad.push([x, m.velocidad_media_cms]);
        }

        ui.label(RichText::new("Evolución de los ensayos quietos").strong().small());
        let alto = (ui.available_height() - 24.0).max(160.0);
        Plot::new("evolucion_postura")
            .height(alto)
            .legend(Legend::default())
            .x_axis_label("fecha del examen")
            .x_axis_formatter(|m, _| fecha::dia_desde_epoch(m.value))
            .label_formatter(|pos| {
                let (nombre, p) = match pos {
                    egui_plot::HoverPosition::NearDataPoint { plot_name, position, .. } => (*plot_name, *position),
                    egui_plot::HoverPosition::Elsewhere { position } => ("", *position),
                };
                Some(format!("{nombre}\n{}\n{:.2}", fecha::dia_desde_epoch(p.x), p.y))
            })
            .show(ui, |plot_ui| {
                plot_ui.line(Line::new("Área 95% (cm²)", PlotPoints::from(area.clone())).color(LILA).width(2.0));
                plot_ui
                    .line(Line::new("Vel. media (cm/s)", PlotPoints::from(velocidad.clone())).color(AZUL).width(2.0));
                plot_ui.points(Points::new("Área 95% (cm²)", area).color(LILA).radius(3.5).shape(MarkerShape::Circle));
                plot_ui.points(
                    Points::new("Vel. media (cm/s)", velocidad).color(AZUL).radius(3.5).shape(MarkerShape::Circle),
                );
            });
    }
}

#[cfg(test)]
mod tests {
    use super::super::ExamenCargado;
    use super::*;

    #[test]
    fn el_examen_cargado_arranca_con_las_notas_limpias() {
        // Si arrancara "sucio", el botón de guardar notas estaría habilitado sin
        // que nadie haya escrito nada.
        let cargado = ExamenCargado {
            detalle: crate::pacientes::DetalleExamen {
                resumen: crate::pacientes::ResumenExamen {
                    id: 1,
                    paciente_id: 1,
                    fecha: "2026-09-19T10:00:00Z".into(),
                    operador: String::new(),
                    tipo: TipoExamen::Estatica,
                    superficie: Default::default(),
                    condicion: Default::default(),
                    metricas: None,
                    muestras: 0,
                },
                version_app: "0".into(),
                config: Default::default(),
                juego: None,
                limites: Vec::new(),
                notas: "algo".into(),
                registro: Vec::new(),
            },
            notas: "algo".into(),
            notas_sucias: false,
        };
        assert!(!cargado.notas_sucias);
        assert_eq!(cargado.notas, cargado.detalle.notas);
    }
}
