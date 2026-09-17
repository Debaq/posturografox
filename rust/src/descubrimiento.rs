//! Autodetección del puerto: prueba cada puerto USB real hasta encontrar
//! uno que responda con el saludo de identificación del firmware
//! (comando 'i' -> "# POSTUROGRAFOX,1"), para no tener que elegirlo a mano.

use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use crate::serial_link::{BAUDIOS, puertos_usables};

const ID_FIRMWARE: &str = "POSTUROGRAFOX";
const TIEMPO_POR_PUERTO: Duration = Duration::from_millis(1500);

pub enum EventoDescubrimiento {
    Encontrado(String),
    Terminado,
}

/// Lanza la búsqueda en un hilo aparte y devuelve el canal donde avisa el
/// resultado. No bloquea: la UI sigue andando mientras prueba los puertos.
pub fn iniciar() -> Receiver<EventoDescubrimiento> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for puerto in puertos_usables() {
            if identificar(&puerto) {
                let _ = tx.send(EventoDescubrimiento::Encontrado(puerto));
                return;
            }
        }
        let _ = tx.send(EventoDescubrimiento::Terminado);
    });
    rx
}

/// Abre el puerto, pide identificación un par de veces (por si el reset del
/// ESP32 al abrir el puerto se come el primer saludo) y espera la respuesta
/// hasta `TIEMPO_POR_PUERTO`. Cualquier error simplemente descarta el puerto.
fn identificar(puerto: &str) -> bool {
    let Ok(dispositivo) = serialport::new(puerto, BAUDIOS).timeout(Duration::from_millis(200)).open() else {
        return false;
    };
    let Ok(mut escritor) = dispositivo.try_clone() else {
        return false;
    };
    let mut lector = BufReader::new(dispositivo);

    let inicio = Instant::now();
    let mut linea = String::new();
    while inicio.elapsed() < TIEMPO_POR_PUERTO {
        let _ = escritor.write_all(b"i");
        linea.clear();
        if lector.read_line(&mut linea).is_ok() && linea.contains(ID_FIRMWARE) {
            return true;
        }
    }
    false
}
