//! Modo juego — PENDIENTE DE IMPLEMENTAR. Este archivo es el espacio
//! reservado para otro agente/sesión: acá va toda la lógica y el arte del
//! juego. No lo implementes vos si estás trabajando en otra parte de la app;
//! coordiná con quien esté en este archivo.
//!
//! ## Qué es
//! Versión gamificada del ejercicio de equilibrio: un zorrito corre solo
//! (auto-scroll) y el jugador lo esquiva de lado a lado inclinándose sobre
//! el posturógrafo real. Reemplaza toda la vista clínica por un juego a
//! pantalla completa (sin barra de controles, sin barra de estado, sin
//! los gráficos COP/tiempo).
//!
//! ## Cómo está enganchado con el resto de la app (ya funciona, no tocar)
//! - `app.rs` tiene un campo `modo_juego: bool` y `estado_juego: EstadoJuego`.
//! - El botón "🎮 Modo juego" (tarjeta rosa en la barra de controles) hace
//!   `self.modo_juego = true`.
//! - Mientras `modo_juego` es `true`, `PosturografoxApp::ui()` NO dibuja los
//!   paneles clínicos: en cambio llama a `juego::mostrar(ui, &mut
//!   self.estado_juego, entrada)` dentro de un `CentralPanel` que ocupa toda
//!   la ventana. Si `mostrar(...)` devuelve `true`, `app.rs` vuelve solo al
//!   modo clínico (`modo_juego = false`).
//! - La lectura del puerto serie sigue corriendo igual en modo juego (no se
//!   pausa), así que `entrada.cop_ml/cop_ap` están siempre actualizados.
//!
//! ## Qué te llega en cada frame (`EntradaJuego`)
//! - `cop_ml: f64` — posición medio-lateral del COP, en cm, 0 = centro,
//!   negativo = izquierda, positivo = derecha. Rango físico real:
//!   `±ancho_cm/2`. Para normalizar a -1.0..1.0: `cop_ml / (ancho_cm/2.0)`.
//! - `cop_ap: f64` — antero-posterior, cm, 0 = centro, + = inclinarse
//!   adelante. Rango: `±prof_cm/2`. Libre para usar (ej. saltar/agachar) o
//!   ignorar y jugar solo con `cop_ml`.
//! - `ancho_cm`, `prof_cm`: dimensiones configuradas de la plataforma (las
//!   edita el usuario en la tarjeta "Plataforma", no son constantes).
//! - `conectado: bool` — si es `false` no hay posturógrafo enchufado y
//!   `cop_ml`/`cop_ap` quedan clavados en 0.0. Mostrar algo como "Conectá el
//!   posturógrafo" en vez de arrancar a jugar en ese caso.
//! - `dt: f32` — segundos desde el frame anterior (`egui`'s `stable_dt`).
//!   Usalo para animar independiente del framerate; la app pide repintado
//!   cada ~33 ms pero no asumas que es exacto.
//!
//! ## Qué tenés que hacer vos
//! - Dibujar todo dentro del `ui: &mut egui::Ui` que te pasan (ya ocupa toda
//!   la ventana — pantalla completa real, no hace falta pedir fullscreen del
//!   sistema operativo).
//! - Guardar el estado del juego (posición del zorro, obstáculos, puntaje,
//!   velocidad, semilla de aleatoriedad, lo que sea) en `EstadoJuego`. No
//!   toques `PosturografoxApp` en `app.rs` más que para lo que ya está.
//! - Mapeo sugerido (podés cambiarlo): `cop_ml` mueve al zorro
//!   izquierda/derecha, ya sea por carriles discretos o posición continua.
//!   Los obstáculos aparecen del lado opuesto (arriba/derecha, a tu criterio
//!   de dirección de scroll) y el jugador esquiva moviéndose.
//! - Devolvé `true` desde `mostrar(...)` cuando el jugador pide salir (botón
//!   propio y/o tecla ESC vía `ui.input(|i| i.key_pressed(egui::Key::Escape))`)
//!   para volver al modo clínico.
//! - Sprite/mascota: `logo.jpeg` en la raíz del repo tiene al zorro de
//!   Posturografox si querés arrancar de ahí; para animación probablemente
//!   necesites dibujar formas simples con `egui::Painter` o cargar tus
//!   propios assets (`egui_extras` + `include_bytes!` es una opción, no está
//!   agregado como dependencia todavía — sumalo en `Cargo.toml` si lo usás).
//! - Dificultad/puntaje/game over: a tu criterio, no hay nada definido.
//!
//! ## Qué NO está implementado (a propósito, es todo tuyo)
//! El zorro, los obstáculos, las colisiones, el puntaje, el fondo, la
//! dificultad progresiva y la pantalla de game over. Lo único que existe es
//! el enganche (`mostrar` se llama, recibe el COP en vivo, y puede volver al
//! modo clínico). El cuerpo de `mostrar` de abajo es un placeholder: reemplazalo.

use egui::{Color32, Key, Ui};

/// Lo que el posturógrafo le pasa al juego en cada frame.
pub struct EntradaJuego {
    pub cop_ml: f64,
    pub cop_ap: f64,
    pub ancho_cm: f64,
    pub prof_cm: f64,
    pub conectado: bool,
    pub dt: f32,
}

/// Estado propio del juego. Vive mientras `modo_juego` esté activo en
/// `PosturografoxApp`; se reinicia solo si vos lo pedís explícitamente.
#[derive(Default)]
pub struct EstadoJuego {
    // TODO(agente de modo juego): posición del zorro, obstáculos, puntaje,
    // velocidad de scroll, etc. Placeholder vacío por ahora.
}

/// Dibuja el juego a pantalla completa dentro de `ui`.
/// Devuelve `true` si el jugador pidió salir (volver al modo clínico).
pub fn mostrar(ui: &mut Ui, _estado: &mut EstadoJuego, entrada: EntradaJuego) -> bool {
    let mut salir = ui.input(|i| i.key_pressed(Key::Escape));

    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            ui.heading("🦊 Modo juego");
            ui.add_space(8.0);
            ui.label("Todavía no implementado — instrucciones completas en src/juego.rs");
            ui.add_space(8.0);

            if entrada.conectado {
                ui.monospace(format!(
                    "COP en vivo: ML {:+.1} cm · AP {:+.1} cm (dt {:.3}s)",
                    entrada.cop_ml, entrada.cop_ap, entrada.dt
                ));
            } else {
                ui.colored_label(Color32::from_rgb(210, 100, 95), "Conectá el posturógrafo para jugar");
            }

            let _ = entrada.ancho_cm;
            let _ = entrada.prof_cm;

            ui.add_space(16.0);
            if ui.button("Salir (o ESC)").clicked() {
                salir = true;
            }
        });
    });

    salir
}
