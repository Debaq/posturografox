//! Lectura del puerto serie del posturógrafo en un hilo aparte.
//!
//! El hilo posee el puerto y bloquea en `read_line`; la UI solo recibe
//! eventos por canal y puede pedir el envío de un comando de un byte
//! (tara/resync del firmware) a través de un `Box<dyn SerialPort>` clonado.

use std::io::{BufRead, BufReader};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use serialport::{SerialPort, SerialPortType};

pub(crate) const BAUDIOS: u32 = 115_200;

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
        let lector =
            serialport::new(puerto, BAUDIOS).timeout(Duration::from_millis(200)).open().map_err(|e| e.to_string())?;
        let escritor = lector.try_clone().map_err(|e| e.to_string())?;

        let (tx, rx) = mpsc::channel();
        let detener = Arc::new(AtomicBool::new(false));
        let detener_hilo = Arc::clone(&detener);

        thread::spawn(move || hilo_lectura(lector, tx, detener_hilo));

        Ok(Self { escritor, detener, eventos: rx })
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

/// Qué resultó ser una línea recibida por el puerto.
#[derive(Debug, PartialEq)]
pub enum LineaSerie {
    /// Las 4 lecturas (fd, fi, bd, bi) de una muestra válida.
    Muestra([f64; 4]),
    /// Texto informativo del firmware (las líneas que empiezan con `#`).
    Mensaje(String),
    /// Línea vacía, encabezado, basura del arranque o datos inválidos.
    Ignorar,
}

/// Interpreta una línea cruda del puerto serie. Función pura, separada del
/// hilo lector para poder testearla.
///
/// Rechaza explícitamente valores no finitos: `"inf"` y `"nan"` parsean sin
/// error como `f64`, y si entraran, la suma de las celdas se volvería inf o
/// NaN y de ahí en más el COP, la elipse y todas las métricas quedarían en
/// NaN sin que nada avise.
pub fn parsear_linea(linea: &str) -> LineaSerie {
    let texto = linea.trim();
    if texto.is_empty() {
        return LineaSerie::Ignorar;
    }
    if let Some(msg) = texto.strip_prefix('#') {
        return LineaSerie::Mensaje(msg.trim().to_string());
    }

    let mut crudos = [0.0f64; 4];
    let mut vistos = 0;
    for parte in texto.split(',') {
        if vistos == 4 {
            return LineaSerie::Ignorar; // más de 4 campos: línea de otro formato
        }
        // El encabezado "fd,fi,bd,bi" y cualquier línea cortada por el reset
        // del ESP32 caen acá.
        let Ok(valor) = parte.trim().parse::<f64>() else {
            return LineaSerie::Ignorar;
        };
        if !valor.is_finite() {
            return LineaSerie::Ignorar;
        }
        crudos[vistos] = valor;
        vistos += 1;
    }
    if vistos == 4 { LineaSerie::Muestra(crudos) } else { LineaSerie::Ignorar }
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
            Ok(_) => match parsear_linea(&linea) {
                LineaSerie::Muestra(crudos) => {
                    let muestra = Muestra { t: t0.elapsed().as_secs_f64(), crudos };
                    if tx.send(EventoSerie::Muestra(muestra)).is_err() {
                        break; // la UI se cerró
                    }
                }
                LineaSerie::Mensaje(msg) => {
                    let _ = tx.send(EventoSerie::MensajeFirmware(msg));
                }
                LineaSerie::Ignorar => continue,
            },
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(e) => {
                let _ = tx.send(EventoSerie::Error(e.to_string()));
                break;
            }
        }
    }
    let _ = tx.send(EventoSerie::Desconectado);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn una_linea_csv_normal_da_las_cuatro_lecturas() {
        assert_eq!(parsear_linea("1,2,3,4\n"), LineaSerie::Muestra([1.0, 2.0, 3.0, 4.0]));
        assert_eq!(parsear_linea(" -1.50 , 2.25 ,0,4e2 \r\n"), LineaSerie::Muestra([-1.5, 2.25, 0.0, 400.0]));
    }

    #[test]
    fn las_lineas_con_almohadilla_son_mensajes_del_firmware() {
        assert_eq!(parsear_linea("# Tara lista\n"), LineaSerie::Mensaje("Tara lista".to_string()));
        assert_eq!(parsear_linea("#POSTUROGRAFOX,1"), LineaSerie::Mensaje("POSTUROGRAFOX,1".to_string()));
    }

    #[test]
    fn el_encabezado_y_las_lineas_vacias_se_ignoran() {
        assert_eq!(parsear_linea("fd,fi,bd,bi\n"), LineaSerie::Ignorar);
        assert_eq!(parsear_linea("\n"), LineaSerie::Ignorar);
        assert_eq!(parsear_linea("   "), LineaSerie::Ignorar);
    }

    #[test]
    fn una_linea_cortada_o_con_campos_de_mas_se_ignora() {
        assert_eq!(parsear_linea("1,2,3"), LineaSerie::Ignorar);
        assert_eq!(parsear_linea("1,2,3,4,5"), LineaSerie::Ignorar);
        assert_eq!(parsear_linea("1,2,3,"), LineaSerie::Ignorar);
        assert_eq!(parsear_linea("23,4"), LineaSerie::Ignorar); // línea partida por un reset
    }

    #[test]
    fn los_valores_no_finitos_no_entran_a_las_metricas() {
        // "inf" y "nan" parsean bien como f64: si pasaran, el COP y todas las
        // métricas quedarían en NaN sin ningún aviso.
        assert_eq!(parsear_linea("1,inf,3,4"), LineaSerie::Ignorar);
        assert_eq!(parsear_linea("nan,2,3,4"), LineaSerie::Ignorar);
        assert_eq!(parsear_linea("1,2,3,-inf"), LineaSerie::Ignorar);
        assert_eq!(parsear_linea("1,2,3,NaN"), LineaSerie::Ignorar);
    }
}
