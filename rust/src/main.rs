mod app;
mod calibracion;
mod config;
mod datos;
mod descubrimiento;
mod espectro;
mod estabilometria;
mod exportar;
mod filtro;
mod historial;
mod informe;
mod juego;
mod limites;
mod pantallas;
mod precarga;
mod serial_link;
mod simulador;
mod transporte;

fn cargar_icono() -> egui::IconData {
    let bytes = include_bytes!("../../logo.jpeg");
    let imagen = image::load_from_memory(bytes).expect("logo.jpeg inválido").into_rgba8();
    let (ancho, alto) = imagen.dimensions();
    egui::IconData { rgba: imagen.into_raw(), width: ancho, height: alto }
}

/// Ayuda de la línea de comandos.
const AYUDA: &str = "\
Posturografox: análisis de centro de presión para posturógrafo de 4 celdas.

Uso: posturografox [OPCIONES]

Opciones:
  --simular   Arranca con el posturógrafo simulado, sin hardware. Sirve para
              desarrollar, mostrar la app o probar el modo juego. Los datos
              son sintéticos y no valen como registro clínico.
  --juego     Abre directamente el modo juego, sin pasar por la vista clínica.
              Si hay una segunda pantalla, el juego se abre ahí.
  --ayuda     Muestra esta ayuda.
";

fn main() -> eframe::Result<()> {
    let argumentos: Vec<String> = std::env::args().skip(1).collect();
    if argumentos.iter().any(|a| a == "--ayuda" || a == "-h" || a == "--help") {
        print!("{AYUDA}");
        return Ok(());
    }
    let simular = argumentos.iter().any(|a| a == "--simular");
    let juego = argumentos.iter().any(|a| a == "--juego");

    let opciones = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 900.0])
            .with_min_inner_size([600.0, 600.0])
            .with_maximized(true)
            .with_icon(cargar_icono()),
        ..Default::default()
    };

    eframe::run_native(
        &format!("Posturografox {}", app::VERSION),
        opciones,
        Box::new(|cc| {
            // El tema lo aplica la app según la configuración guardada.
            let mut aplicacion = app::PosturografoxApp::nueva(cc);
            if simular {
                aplicacion.conectar_simulador();
            }
            if juego {
                aplicacion.empezar_en_modo_juego();
            }
            Ok(Box::new(aplicacion))
        }),
    )
}
