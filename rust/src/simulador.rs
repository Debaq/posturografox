//! Posturógrafo simulado: genera muestras como las del firmware real, sin
//! hardware. Sirve para desarrollar, mostrar la app y probar el modo juego
//! cuando la plataforma no está a mano.
//!
//! El sway sintético es la suma de unas pocas sinusoides lentas (0.2–1.2 Hz,
//! la banda donde vive el balanceo humano) más ruido, y arranca con la
//! plataforma vacía unos segundos para que se vea funcionar la detección
//! automática de subida.

use std::f64::consts::TAU;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use crate::serial_link::{EventoSerie, Muestra};
use crate::transporte::Transporte;

/// Muestras por segundo, igual que el firmware a 80 SPS.
const FRECUENCIA_HZ: f64 = 80.0;
/// Cuentas del ADC por kilogramo (valor plausible de una celda con HX711).
const CUENTAS_POR_KG: f64 = 10_000.0;
/// Peso de la persona simulada.
const PESO_KG: f64 = 70.0;
/// Segundos con la plataforma vacía antes de que "se suba" alguien.
const SEGUNDOS_VACIO: f64 = 2.0;
/// Geometría asumida para repartir el peso entre las 4 celdas.
const ANCHO_CM: f64 = 40.0;
const PROF_CM: f64 = 40.0;

pub struct Simulador {
    eventos: Receiver<EventoSerie>,
    comandos: Sender<u8>,
    detener: Arc<AtomicBool>,
}

impl Simulador {
    pub fn iniciar() -> Self {
        let (tx_eventos, rx_eventos) = mpsc::channel();
        let (tx_comandos, rx_comandos) = mpsc::channel();
        let detener = Arc::new(AtomicBool::new(false));
        let detener_hilo = Arc::clone(&detener);

        thread::spawn(move || hilo(tx_eventos, rx_comandos, detener_hilo));

        Self { eventos: rx_eventos, comandos: tx_comandos, detener }
    }
}

impl Drop for Simulador {
    fn drop(&mut self) {
        self.detener.store(true, Ordering::SeqCst);
    }
}

impl Transporte for Simulador {
    fn eventos(&self) -> &Receiver<EventoSerie> {
        &self.eventos
    }

    fn enviar_comando(&mut self, comando: u8) {
        let _ = self.comandos.send(comando);
    }

    fn descripcion(&self) -> String {
        "Simulador".to_string()
    }

    fn es_simulado(&self) -> bool {
        true
    }
}

fn hilo(tx: Sender<EventoSerie>, comandos: Receiver<u8>, detener: Arc<AtomicBool>) {
    let _ = tx.send(EventoSerie::Conectado);
    let _ = tx.send(EventoSerie::MensajeFirmware("Simulador: datos sintéticos, no es una medición".into()));

    let periodo = Duration::from_secs_f64(1.0 / FRECUENCIA_HZ);
    let mut n: u64 = 0;
    let mut ruido = Ruido::nuevo(0x5EED_1234_ABCD_0001);

    while !detener.load(Ordering::SeqCst) {
        // Los comandos se contestan como lo haría el firmware, para que la app
        // se comporte igual contra el simulador que contra la plataforma.
        while let Ok(comando) = comandos.try_recv() {
            let respuesta = match comando {
                b'p' => Some("Modo: calibrado".to_string()),
                b't' => Some("Tara lista: 0,0,0,0".to_string()),
                b's' => Some("Resincronizado".to_string()),
                b'i' => Some("POSTUROGRAFOX,2 (simulado)".to_string()),
                _ => None,
            };
            if let Some(texto) = respuesta {
                let _ = tx.send(EventoSerie::MensajeFirmware(texto));
            }
        }

        let t = n as f64 / FRECUENCIA_HZ;
        let crudos = muestra_en(t, &mut ruido);
        let muestra = Muestra { t, crudos, perdidas: 0 };
        if tx.send(EventoSerie::Muestra(muestra)).is_err() {
            break; // la UI se cerró
        }
        n += 1;
        thread::sleep(periodo);
    }
    let _ = tx.send(EventoSerie::Desconectado);
}

/// Las 4 lecturas (fd, fi, bd, bi) en el instante `t`.
pub fn muestra_en(t: f64, ruido: &mut Ruido) -> [f64; 4] {
    if t < SEGUNDOS_VACIO {
        // Plataforma vacía: solo ruido alrededor del cero, como en la realidad.
        return std::array::from_fn(|_| ruido.rango(-40.0, 40.0));
    }

    let ts = t - SEGUNDOS_VACIO;
    // Balanceo típico: más amplitud en antero-posterior que en medio-lateral.
    let ml_cm = 0.6 * (TAU * 0.31 * ts).sin() + 0.25 * (TAU * 0.83 * ts + 1.1).sin();
    let ap_cm = 0.9 * (TAU * 0.23 * ts).sin() + 0.35 * (TAU * 1.17 * ts + 0.4).sin();

    let peso = PESO_KG * CUENTAS_POR_KG;
    repartir_en_celdas(ml_cm, ap_cm, peso).map(|v| v + ruido.rango(-60.0, 60.0))
}

/// Reparte `peso` entre las 4 celdas de modo que el COP resultante caiga
/// exactamente en (`ml_cm`, `ap_cm`). Es la inversa del cálculo del COP que
/// hace la app, así que sirve de verificación cruzada de esa fórmula.
pub fn repartir_en_celdas(ml_cm: f64, ap_cm: f64, peso: f64) -> [f64; 4] {
    let fx = 2.0 * ml_cm / ANCHO_CM; // -1.0 (izquierda) .. 1.0 (derecha)
    let fy = 2.0 * ap_cm / PROF_CM; // -1.0 (atrás) .. 1.0 (adelante)
    let cuarto = peso / 4.0;
    [
        cuarto * (1.0 + fx) * (1.0 + fy), // fd
        cuarto * (1.0 - fx) * (1.0 + fy), // fi
        cuarto * (1.0 + fx) * (1.0 - fy), // bd
        cuarto * (1.0 - fx) * (1.0 - fy), // bi
    ]
}

/// Ruido pseudoaleatorio reproducible (xorshift64), sin dependencias.
pub struct Ruido(u64);

impl Ruido {
    pub fn nuevo(semilla: u64) -> Self {
        Self(semilla | 1)
    }

    fn siguiente(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn rango(&mut self, min: f64, max: f64) -> f64 {
        let unidad = (self.siguiente() >> 11) as f64 / (1u64 << 53) as f64;
        min + unidad * (max - min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mismo cálculo que hace la app sobre las 4 celdas.
    fn cop_de(crudos: [f64; 4]) -> (f64, f64) {
        let suma: f64 = crudos.iter().sum();
        let ml = ((crudos[0] + crudos[2]) - (crudos[1] + crudos[3])) / suma * (ANCHO_CM / 2.0);
        let ap = ((crudos[0] + crudos[1]) - (crudos[2] + crudos[3])) / suma * (PROF_CM / 2.0);
        (ml, ap)
    }

    #[test]
    fn el_reparto_entre_celdas_reproduce_el_cop_pedido() {
        for (ml, ap) in [(0.0, 0.0), (5.0, -3.0), (-8.5, 7.25)] {
            let (ml_calc, ap_calc) = cop_de(repartir_en_celdas(ml, ap, 700_000.0));
            assert!((ml_calc - ml).abs() < 1e-9, "ML: esperaba {ml}, dio {ml_calc}");
            assert!((ap_calc - ap).abs() < 1e-9, "AP: esperaba {ap}, dio {ap_calc}");
        }
    }

    #[test]
    fn centrado_reparte_el_peso_en_partes_iguales() {
        let celdas = repartir_en_celdas(0.0, 0.0, 800_000.0);
        for celda in celdas {
            assert!((celda - 200_000.0).abs() < 1e-9);
        }
    }

    #[test]
    fn la_plataforma_arranca_vacia_y_despues_se_sube_alguien() {
        let mut ruido = Ruido::nuevo(7);
        let vacia: f64 = muestra_en(0.5, &mut ruido).iter().sum();
        let ocupada: f64 = muestra_en(SEGUNDOS_VACIO + 1.0, &mut ruido).iter().sum();
        assert!(vacia.abs() < 1_000.0, "con nadie arriba la suma debe rondar el cero, dio {vacia}");
        assert!(ocupada > 600_000.0, "70 kg a 10000 cuentas/kg, dio {ocupada}");
    }

    #[test]
    fn el_sway_simulado_se_mueve_pero_sin_salirse_de_la_plataforma() {
        let mut ruido = Ruido::nuevo(99);
        let mut min_ml: f64 = f64::MAX;
        let mut max_ml: f64 = f64::MIN;
        for i in 0..800 {
            let t = SEGUNDOS_VACIO + i as f64 / FRECUENCIA_HZ;
            let (ml, _) = cop_de(muestra_en(t, &mut ruido));
            min_ml = min_ml.min(ml);
            max_ml = max_ml.max(ml);
        }
        assert!(max_ml - min_ml > 0.5, "el COP simulado debería oscilar, rango {}", max_ml - min_ml);
        assert!(max_ml < ANCHO_CM / 2.0 && min_ml > -ANCHO_CM / 2.0, "el COP se fue de la plataforma");
    }
}
