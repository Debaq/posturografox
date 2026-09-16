//! Lectura del puerto serie del posturógrafo en un hilo aparte.
//!
//! El hilo posee el puerto y bloquea en `read_line`; la UI solo recibe
//! eventos por canal y puede pedir el envío de un comando de un byte
//! (tara/resync del firmware) a través de un `Box<dyn SerialPort>` clonado.

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serialport::{SerialPort, SerialPortType};

const BAUDIOS: u32 = 115_200;

#[derive(Debug, Clone)]
pub struct Muestra {
    pub t: f64,
    pub crudos: [f64; 4], // fd, fi, bd, bi
}

pub enum EventoSerie {
    Conectado,
    Desconectado,
    Muestra(Muestra),
    MensajeFirmware(String),
    Error(String),
}

/// Puertos con un dispositivo USB real detrás: descarta los ttyS*/COM*
/// fantasma del chipset (sin VID/PID) y el Bluetooth virtual, que solo
/// ensucian la lista de selección.
pub fn puertos_usables() -> Vec<String> {
    serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| matches!(p.port_type, SerialPortType::UsbPort(_)))
        .map(|p| p.port_name)
        .collect()
}

pub struct ConexionSerie {
    escritor: Box<dyn SerialPort>,
    detener: Arc<AtomicBool>,
    pub eventos: Receiver<EventoSerie>,
}

impl ConexionSerie {
    pub fn conectar(puerto: &str) -> Result<Self, String> {
        let lector = serialport::new(puerto, BAUDIOS)
            .timeout(Duration::from_millis(200))
            .open()
            .map_err(|e| e.to_string())?;
        let escritor = lector.try_clone().map_err(|e| e.to_string())?;

        let (tx, rx) = mpsc::channel();
        let detener = Arc::new(AtomicBool::new(false));
        let detener_hilo = Arc::clone(&detener);

        thread::spawn(move || hilo_lectura(lector, tx, detener_hilo));

        Ok(Self {
            escritor,
            detener,
            eventos: rx,
        })
    }

    pub fn enviar_comando(&mut self, c: u8) {
        let _ = self.escritor.write_all(&[c]);
    }
}

impl Drop for ConexionSerie {
    fn drop(&mut self) {
        self.detener.store(true, Ordering::SeqCst);
    }
}

fn hilo_lectura(puerto: Box<dyn SerialPort>, tx: mpsc::Sender<EventoSerie>, detener: Arc<AtomicBool>) {
    let _ = tx.send(EventoSerie::Conectado);
    let t0 = Instant::now();
    let mut lector = BufReader::new(puerto);
    let mut linea = String::new();

    while !detener.load(Ordering::SeqCst) {
        linea.clear();
        match lector.read_line(&mut linea) {
            Ok(0) => break, // puerto cerrado del otro lado
            Ok(_) => {
                let texto = linea.trim();
                if texto.is_empty() {
                    continue;
                }
                if let Some(msg) = texto.strip_prefix('#') {
                    let _ = tx.send(EventoSerie::MensajeFirmware(msg.trim().to_string()));
                    continue;
                }
                let partes: Vec<&str> = texto.split(',').collect();
                if partes.len() != 4 {
                    continue;
                }
                let valores: Result<Vec<f64>, _> = partes.iter().map(|p| p.trim().parse::<f64>()).collect();
                let Ok(v) = valores else { continue }; // encabezado "fd,fi,bd,bi" u otra línea no numérica
                let muestra = Muestra {
                    t: t0.elapsed().as_secs_f64(),
                    crudos: [v[0], v[1], v[2], v[3]],
                };
                if tx.send(EventoSerie::Muestra(muestra)).is_err() {
                    break; // la UI se cerró
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(e) => {
                let _ = tx.send(EventoSerie::Error(e.to_string()));
                break;
            }
        }
    }
    let _ = tx.send(EventoSerie::Desconectado);
}
