mod app;
mod serial_link;

fn cargar_icono() -> egui::IconData {
    let bytes = include_bytes!("../../logo.jpeg");
    let imagen = image::load_from_memory(bytes)
        .expect("logo.jpeg inválido")
        .into_rgba8();
    let (ancho, alto) = imagen.dimensions();
    egui::IconData {
        rgba: imagen.into_raw(),
        width: ancho,
        height: alto,
    }
}

fn main() -> eframe::Result<()> {
    let opciones = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 900.0])
            .with_min_inner_size([600.0, 600.0])
            .with_icon(cargar_icono()),
        ..Default::default()
    };

    eframe::run_native(
        "Posturografox",
        opciones,
        Box::new(|_cc| Ok(Box::new(app::PosturografoxApp::default()))),
    )
}
