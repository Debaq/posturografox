//! Análisis frecuencial del COP.
//!
//! Dos personas pueden tener la misma área de elipse oscilando de maneras muy
//! distintas: una con vaivenes lentos y amplios, otra con temblor rápido. Eso
//! no se ve en las métricas de amplitud, pero sí en el espectro; por eso la
//! frecuencia mediana y la F80 (la frecuencia por debajo de la cual está el
//! 80% de la potencia) son parte del set clásico de posturografía.

use std::f64::consts::TAU;

/// Número complejo mínimo, lo justo para la FFT.
#[derive(Clone, Copy)]
struct Complejo {
    re: f64,
    im: f64,
}

impl Complejo {
    fn nuevo(re: f64, im: f64) -> Self {
        Self { re, im }
    }

    fn potencia(self) -> f64 {
        self.re * self.re + self.im * self.im
    }
}

/// FFT iterativa de Cooley-Tukey, in-place. `datos.len()` debe ser potencia
/// de dos (lo garantiza `espectro_potencia`).
fn fft(datos: &mut [Complejo]) {
    let n = datos.len();
    if n <= 1 {
        return;
    }

    // Permutación por inversión de bits.
    let mut objetivo = 0usize;
    for origen in 1..n {
        let mut bit = n >> 1;
        while objetivo & bit != 0 {
            objetivo ^= bit;
            bit >>= 1;
        }
        objetivo |= bit;
        if origen < objetivo {
            datos.swap(origen, objetivo);
        }
    }

    let mut largo = 2;
    while largo <= n {
        let angulo = -TAU / largo as f64;
        for bloque in (0..n).step_by(largo) {
            for k in 0..largo / 2 {
                let (sin, cos) = (angulo * k as f64).sin_cos();
                let par = datos[bloque + k];
                let impar = datos[bloque + k + largo / 2];
                let giro = Complejo::nuevo(impar.re * cos - impar.im * sin, impar.re * sin + impar.im * cos);
                datos[bloque + k] = Complejo::nuevo(par.re + giro.re, par.im + giro.im);
                datos[bloque + k + largo / 2] = Complejo::nuevo(par.re - giro.re, par.im - giro.im);
            }
        }
        largo <<= 1;
    }
}

/// Espectro de potencia de la serie, junto con el paso de frecuencia entre
/// bins. Le quita la media (el continuo no es oscilación) y aplica ventana de
/// Hann para que el corte abrupto de los extremos no ensucie el espectro.
fn espectro_potencia(serie: &[f64], muestreo_hz: f64) -> Option<(Vec<f64>, f64)> {
    if serie.len() < 8 || !muestreo_hz.is_finite() || muestreo_hz <= 0.0 {
        return None;
    }
    let media = serie.iter().sum::<f64>() / serie.len() as f64;

    let n = serie.len().next_power_of_two();
    let mut datos = vec![Complejo::nuevo(0.0, 0.0); n];
    let largo = serie.len() as f64;
    for (i, &v) in serie.iter().enumerate() {
        let hann = 0.5 - 0.5 * (TAU * i as f64 / largo).cos();
        datos[i] = Complejo::nuevo((v - media) * hann, 0.0);
    }

    fft(&mut datos);

    // Solo la mitad útil (el espectro de una señal real es simétrico).
    let potencias: Vec<f64> = datos[..n / 2].iter().map(|c| c.potencia()).collect();
    Some((potencias, muestreo_hz / n as f64))
}

/// Frecuencia por debajo de la cual queda la fracción `fraccion` de la
/// potencia total. Con `fraccion = 0.5` da la frecuencia mediana; con `0.8`,
/// la F80.
pub fn frecuencia_de_potencia(serie: &[f64], muestreo_hz: f64, fraccion: f64) -> Option<f64> {
    let (potencias, paso_hz) = espectro_potencia(serie, muestreo_hz)?;
    // El bin 0 es el continuo, que ya se restó: no aporta.
    let total: f64 = potencias.iter().skip(1).sum();
    if total <= 0.0 {
        return None;
    }

    let objetivo = total * fraccion.clamp(0.0, 1.0);
    let mut acumulado = 0.0;
    for (i, potencia) in potencias.iter().enumerate().skip(1) {
        let previo = acumulado;
        acumulado += potencia;
        if acumulado >= objetivo {
            // Interpolación lineal dentro del bin, para no quedar pegado a la
            // resolución del espectro.
            let sobrante = if *potencia > 0.0 { (objetivo - previo) / potencia } else { 0.0 };
            return Some((i as f64 - 1.0 + sobrante) * paso_hz);
        }
    }
    Some((potencias.len() - 1) as f64 * paso_hz)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frecuencia mediana: la mitad de la potencia está por debajo.
    fn frecuencia_mediana(serie: &[f64], muestreo_hz: f64) -> Option<f64> {
        frecuencia_de_potencia(serie, muestreo_hz, 0.5)
    }

    fn f80(serie: &[f64], muestreo_hz: f64) -> Option<f64> {
        frecuencia_de_potencia(serie, muestreo_hz, 0.8)
    }

    fn senoidal(frecuencia_hz: f64, muestreo_hz: f64, n: usize) -> Vec<f64> {
        (0..n).map(|i| (TAU * frecuencia_hz * i as f64 / muestreo_hz).sin()).collect()
    }

    #[test]
    fn una_senoidal_pura_tiene_su_frecuencia_como_mediana() {
        let serie = senoidal(1.0, 80.0, 1024);
        let mediana = frecuencia_mediana(&serie, 80.0).unwrap();
        assert!((mediana - 1.0).abs() < 0.15, "esperaba ~1 Hz, dio {mediana}");
    }

    #[test]
    fn el_balanceo_lento_da_frecuencias_mas_bajas_que_el_temblor() {
        let lento = frecuencia_mediana(&senoidal(0.3, 80.0, 2048), 80.0).unwrap();
        let rapido = frecuencia_mediana(&senoidal(3.0, 80.0, 2048), 80.0).unwrap();
        assert!(lento < rapido, "lento {lento} debería ser menor que rápido {rapido}");
    }

    #[test]
    fn la_f80_nunca_queda_por_debajo_de_la_mediana() {
        // Mezcla de un vaivén lento con un temblor chico encima.
        let serie: Vec<f64> = (0..2048)
            .map(|i| {
                let t = i as f64 / 80.0;
                (TAU * 0.4 * t).sin() + 0.2 * (TAU * 5.0 * t).sin()
            })
            .collect();
        let mediana = frecuencia_mediana(&serie, 80.0).unwrap();
        let f80 = f80(&serie, 80.0).unwrap();
        assert!(f80 >= mediana, "f80 {f80} < mediana {mediana}");
        assert!(mediana < 1.0, "la mayor parte de la potencia es del vaivén lento, dio {mediana}");
    }

    #[test]
    fn el_continuo_no_cuenta_como_oscilacion() {
        // Serie constante: no hay oscilación de la que hablar.
        assert!(frecuencia_mediana(&vec![3.0; 512], 80.0).is_none());
    }

    #[test]
    fn una_serie_muy_corta_no_da_espectro() {
        assert!(frecuencia_mediana(&[1.0, 2.0, 3.0], 80.0).is_none());
        assert!(frecuencia_mediana(&senoidal(1.0, 80.0, 64), 0.0).is_none());
    }
}
