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
    /// Segundos desde la primera muestra, reconstruidos con el reloj del
    /// firmware cuando está disponible (ver `SeguimientoMuestras`).
    pub t: f64,
    pub crudos: [f64; 4], // fd, fi, bd, bi
    /// Cuántas muestras se perdieron justo antes de esta.
    pub perdidas: u64,
}

/// Lo que trae una línea de datos del firmware. `numero` y `t_us` solo
/// vienen del firmware nuevo (formato `n,t_us,fd,fi,bd,bi`); con el formato
/// viejo de 4 columnas llegan en `None` y el host cae al reloj propio.
#[derive(Debug, Clone, PartialEq)]
pub struct LecturaCruda {
    pub numero: Option<u64>,
    pub t_us: Option<u64>,
    pub crudos: [f64; 4],
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
    /// Una muestra válida.
    Muestra(LecturaCruda),
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

    let campos: Vec<&str> = texto.split(',').map(str::trim).collect();
    // 6 campos = firmware nuevo (n,t_us,fd,fi,bd,bi); 4 = firmware viejo.
    let (numero, t_us, celdas) = match campos.len() {
        6 => {
            let (Ok(n), Ok(us)) = (campos[0].parse::<u64>(), campos[1].parse::<u64>()) else {
                return LineaSerie::Ignorar; // encabezado "n,t_us,fd,..." o línea rota
            };
            (Some(n), Some(us), &campos[2..])
        }
        4 => (None, None, &campos[..]),
        _ => return LineaSerie::Ignorar,
    };

    let mut crudos = [0.0f64; 4];
    for (i, campo) in celdas.iter().enumerate() {
        // El encabezado y cualquier línea cortada por el reset del ESP32
        // caen acá.
        let Ok(valor) = campo.parse::<f64>() else {
            return LineaSerie::Ignorar;
        };
        if !valor.is_finite() {
            return LineaSerie::Ignorar;
        }
        crudos[i] = valor;
    }
    LineaSerie::Muestra(LecturaCruda { numero, t_us, crudos })
}

/// Una vuelta completa del `micros()` del firmware: es `unsigned long` de 32
/// bits, así que vuelve a cero cada ~71 minutos.
const VUELTA_MICROS: u64 = 1 << 32;

/// Reconstruye la línea de tiempo de la sesión y cuenta las muestras que se
/// perdieron, a partir del número de muestra y el `micros()` del firmware.
///
/// Usar el reloj del dispositivo importa porque el USB CDC entrega las líneas
/// a los tirones: fechadas en el host, varias muestras quedan con casi el
/// mismo instante y después aparece un hueco, lo que distorsiona la velocidad
/// media de oscilación (la métrica más sensible del examen).
#[derive(Default)]
pub struct SeguimientoMuestras {
    ultimo_us: Option<u64>,
    ultimo_numero: Option<u64>,
    t_s: f64,
}

impl SeguimientoMuestras {
    /// Devuelve `(t_s, muestras_perdidas)` para esta lectura. `t_host_s` es el
    /// reloj del host, que solo se usa si el firmware no manda el suyo.
    pub fn registrar(&mut self, lectura: &LecturaCruda, t_host_s: f64) -> (f64, u64) {
        let perdidas = match (self.ultimo_numero, lectura.numero) {
            (Some(previo), Some(actual)) => actual.saturating_sub(previo).saturating_sub(1),
            _ => 0,
        };
        self.ultimo_numero = lectura.numero;

        let Some(us) = lectura.t_us else {
            self.t_s = t_host_s; // firmware viejo: no hay más remedio que el reloj del host
            return (self.t_s, perdidas);
        };

        match self.ultimo_us {
            None => self.t_s = 0.0, // primera muestra: el origen de la sesión
            Some(previo) => {
                // Resta modular, para que la vuelta del contador de micros no
                // produzca un salto de ~71 minutos hacia atrás.
                let delta_us = (us + VUELTA_MICROS - previo) % VUELTA_MICROS;
                self.t_s += delta_us as f64 / 1_000_000.0;
            }
        }
        self.ultimo_us = Some(us);
        (self.t_s, perdidas)
    }
}

fn hilo_lectura(puerto: Box<dyn SerialPort>, tx: mpsc::Sender<EventoSerie>, detener: Arc<AtomicBool>) {
    let _ = tx.send(EventoSerie::Conectado);
    let t0 = Instant::now();
    let mut lector = BufReader::new(puerto);
    let mut linea = String::new();
    let mut seguimiento = SeguimientoMuestras::default();

    while !detener.load(Ordering::SeqCst) {
        linea.clear();
        match lector.read_line(&mut linea) {
            Ok(0) => break, // puerto cerrado del otro lado
            Ok(_) => match parsear_linea(&linea) {
                LineaSerie::Muestra(lectura) => {
                    let (t, perdidas) = seguimiento.registrar(&lectura, t0.elapsed().as_secs_f64());
                    let muestra = Muestra { t, crudos: lectura.crudos, perdidas };
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

    fn lectura(numero: Option<u64>, t_us: Option<u64>, crudos: [f64; 4]) -> LineaSerie {
        LineaSerie::Muestra(LecturaCruda { numero, t_us, crudos })
    }

    #[test]
    fn una_linea_del_firmware_nuevo_trae_numero_y_marca_de_tiempo() {
        assert_eq!(parsear_linea("7,123456,1,2,3,4\n"), lectura(Some(7), Some(123_456), [1.0, 2.0, 3.0, 4.0]));
    }

    #[test]
    fn el_formato_viejo_de_cuatro_columnas_sigue_funcionando() {
        assert_eq!(parsear_linea("1,2,3,4\n"), lectura(None, None, [1.0, 2.0, 3.0, 4.0]));
        assert_eq!(parsear_linea(" -1.50 , 2.25 ,0,4e2 \r\n"), lectura(None, None, [-1.5, 2.25, 0.0, 400.0]));
    }

    #[test]
    fn las_lineas_con_almohadilla_son_mensajes_del_firmware() {
        assert_eq!(parsear_linea("# Tara lista\n"), LineaSerie::Mensaje("Tara lista".to_string()));
        assert_eq!(parsear_linea("#POSTUROGRAFOX,2"), LineaSerie::Mensaje("POSTUROGRAFOX,2".to_string()));
    }

    #[test]
    fn los_encabezados_y_las_lineas_vacias_se_ignoran() {
        assert_eq!(parsear_linea("fd,fi,bd,bi\n"), LineaSerie::Ignorar);
        assert_eq!(parsear_linea("n,t_us,fd,fi,bd,bi\n"), LineaSerie::Ignorar);
        assert_eq!(parsear_linea("\n"), LineaSerie::Ignorar);
        assert_eq!(parsear_linea("   "), LineaSerie::Ignorar);
    }

    #[test]
    fn una_linea_cortada_o_con_campos_de_mas_se_ignora() {
        assert_eq!(parsear_linea("1,2,3"), LineaSerie::Ignorar);
        assert_eq!(parsear_linea("1,2,3,4,5"), LineaSerie::Ignorar);
        assert_eq!(parsear_linea("1,2,3,4,5,6,7"), LineaSerie::Ignorar);
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
        assert_eq!(parsear_linea("0,0,1,2,3,NaN"), LineaSerie::Ignorar);
    }

    fn cruda(numero: u64, t_us: u64) -> LecturaCruda {
        LecturaCruda { numero: Some(numero), t_us: Some(t_us), crudos: [0.0; 4] }
    }

    #[test]
    fn el_tiempo_arranca_en_cero_y_avanza_con_el_reloj_del_firmware() {
        let mut seg = SeguimientoMuestras::default();
        // El firmware ya venía corriendo hace rato: la sesión igual empieza en 0.
        let (t0, _) = seg.registrar(&cruda(0, 5_000_000), 99.0);
        let (t1, _) = seg.registrar(&cruda(1, 5_012_500), 99.0);
        let (t2, _) = seg.registrar(&cruda(2, 5_025_000), 99.0);
        assert!(t0.abs() < 1e-9);
        assert!((t1 - 0.0125).abs() < 1e-9, "12.5 ms = una muestra a 80 SPS, dio {t1}");
        assert!((t2 - 0.025).abs() < 1e-9);
    }

    #[test]
    fn la_vuelta_del_contador_de_micros_no_manda_el_tiempo_para_atras() {
        // micros() es de 32 bits: vuelve a cero cada ~71 minutos.
        let mut seg = SeguimientoMuestras::default();
        seg.registrar(&cruda(0, u32::MAX as u64 - 5_000), 0.0);
        let (t, _) = seg.registrar(&cruda(1, 7_500), 0.0);
        // 5000 µs hasta la vuelta + 7500 µs después, más el µs del cruce por 0.
        assert!((t - 0.0125).abs() < 1e-6, "esperaba ~12.5 ms de salto, dio {t}");
    }

    #[test]
    fn el_salto_en_el_numero_de_muestra_se_reporta_como_perdidas() {
        let mut seg = SeguimientoMuestras::default();
        assert_eq!(seg.registrar(&cruda(10, 0), 0.0).1, 0, "la primera muestra no puede contar pérdidas");
        assert_eq!(seg.registrar(&cruda(11, 12_500).clone(), 0.0).1, 0);
        assert_eq!(seg.registrar(&cruda(15, 62_500), 0.0).1, 3, "de la 11 a la 15 se perdieron 12, 13 y 14");
    }

    #[test]
    fn sin_reloj_del_firmware_se_usa_el_del_host() {
        let mut seg = SeguimientoMuestras::default();
        let vieja = LecturaCruda { numero: None, t_us: None, crudos: [1.0; 4] };
        assert_eq!(seg.registrar(&vieja, 3.25), (3.25, 0));
    }
}
