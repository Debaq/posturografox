//! Matemática de estabilometría: elipse de confianza 95%, métricas clásicas
//! de sway (longitud, área, velocidad, RMS, rango) y los cocientes del CTSIB
//! (Clinical Test of Sensory Interaction on Balance). Todo pensado como
//! funciones puras sobre datos crudos (sin depender de `PosturografoxApp`)
//! para que sea fácil de testear y de reusar.

/// χ² al 95% con 2 grados de libertad: escala los semiejes de la elipse de
/// confianza y su área (Prieto et al. 1996, métrica estándar en posturografía).
const CHI2_95_2GL: f64 = 5.991_46;

/// Condición visual del examen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Condicion {
    #[default]
    OjosAbiertos,
    OjosCerrados,
}

impl Condicion {
    pub fn etiqueta(&self) -> &'static str {
        match self {
            Condicion::OjosAbiertos => "Ojos abiertos",
            Condicion::OjosCerrados => "Ojos cerrados",
        }
    }

    pub fn slug(&self) -> &'static str {
        match self {
            Condicion::OjosAbiertos => "ojos_abiertos",
            Condicion::OjosCerrados => "ojos_cerrados",
        }
    }
}

/// Superficie de apoyo del examen (firme = piso normal, espuma = colchoneta
/// que quita referencia propioceptiva precisa). Junto con `Condicion` arma
/// las 4 condiciones del CTSIB.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Superficie {
    #[default]
    Firme,
    Espuma,
}

impl Superficie {
    pub fn etiqueta(&self) -> &'static str {
        match self {
            Superficie::Firme => "Firme",
            Superficie::Espuma => "Espuma",
        }
    }

    pub fn slug(&self) -> &'static str {
        match self {
            Superficie::Firme => "firme",
            Superficie::Espuma => "espuma",
        }
    }
}

/// Métricas clásicas de estabilometría sobre el trazo COP de una sesión.
#[derive(Clone, Copy, Default, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MetricasBalance {
    pub longitud_cm: f64,
    pub area95_cm2: f64,
    pub velocidad_media_cms: f64,
    pub duracion_s: f64,
    pub rms_ml_cm: f64,
    pub rms_ap_cm: f64,
    pub rango_ml_cm: f64,
    pub rango_ap_cm: f64,
    /// Velocidad media por eje. Se informan por separado porque un déficit
    /// suele castigar un eje y no el otro (vestibular tiende a ML,
    /// propioceptivo a AP), y eso se pierde en la velocidad resultante.
    pub velocidad_ml_cms: f64,
    pub velocidad_ap_cms: f64,
    /// Frecuencia mediana de la oscilación por eje: dos personas con la misma
    /// área pueden oscilar lento y amplio o rápido y chico.
    pub frec_mediana_ml_hz: f64,
    pub frec_mediana_ap_hz: f64,
    /// F80: por debajo de esta frecuencia está el 80% de la potencia.
    pub f80_ml_hz: f64,
    pub f80_ap_hz: f64,
}

impl MetricasBalance {
    pub fn texto(&self) -> String {
        format!(
            "Longitud: {:.1} cm · Área 95%: {:.1} cm² · Vel. media: {:.2} cm/s · Duración: {:.1} s · \
             RMS ML/AP: {:.2}/{:.2} cm · Rango ML/AP: {:.1}/{:.1} cm",
            self.longitud_cm,
            self.area95_cm2,
            self.velocidad_media_cms,
            self.duracion_s,
            self.rms_ml_cm,
            self.rms_ap_cm,
            self.rango_ml_cm,
            self.rango_ap_cm
        )
    }

    /// Segunda línea con las métricas por eje y las frecuenciales.
    pub fn texto_avanzado(&self) -> String {
        format!(
            "Vel. ML/AP: {:.2}/{:.2} cm/s · Frec. mediana ML/AP: {:.2}/{:.2} Hz · F80 ML/AP: {:.2}/{:.2} Hz",
            self.velocidad_ml_cms,
            self.velocidad_ap_cms,
            self.frec_mediana_ml_hz,
            self.frec_mediana_ap_hz,
            self.f80_ml_hz,
            self.f80_ap_hz
        )
    }
}

/// Calcula todas las métricas a partir de una serie `[t_s, cop_ml_cm, cop_ap_cm]`
/// en orden cronológico. `None` si hay menos de 3 muestras (no alcanza para
/// ajustar la elipse de confianza).
pub fn calcular_metricas(muestras: &[[f64; 3]]) -> Option<MetricasBalance> {
    let n = muestras.len();
    if n < 3 {
        return None;
    }
    let xs: Vec<f64> = muestras.iter().map(|m| m[1]).collect();
    let ys: Vec<f64> = muestras.iter().map(|m| m[2]).collect();

    let mut longitud_cm = 0.0;
    let mut recorrido_ml_cm = 0.0;
    let mut recorrido_ap_cm = 0.0;
    for i in 1..n {
        let dx = xs[i] - xs[i - 1];
        let dy = ys[i] - ys[i - 1];
        longitud_cm += (dx * dx + dy * dy).sqrt();
        recorrido_ml_cm += dx.abs();
        recorrido_ap_cm += dy.abs();
    }

    let media_x = xs.iter().sum::<f64>() / n as f64;
    let media_y = ys.iter().sum::<f64>() / n as f64;
    let suma_sq = |vs: &[f64], media: f64| vs.iter().map(|v| (v - media).powi(2)).sum::<f64>();
    let rms_ml_cm = (suma_sq(&xs, media_x) / n as f64).sqrt();
    let rms_ap_cm = (suma_sq(&ys, media_y) / n as f64).sqrt();
    let rango = |vs: &[f64]| {
        let min = vs.iter().copied().fold(f64::INFINITY, f64::min);
        let max = vs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        max - min
    };

    let area95_cm2 = ajustar_elipse95_xy(&xs, &ys).map_or(0.0, |e| e.area());
    let duracion_s = muestras.last().unwrap()[0] - muestras.first().unwrap()[0];
    let por_segundo = |recorrido: f64| if duracion_s > 0.0 { recorrido / duracion_s } else { 0.0 };
    let velocidad_media_cms = por_segundo(longitud_cm);

    // Las frecuenciales necesitan saber a qué ritmo se muestreó; si el
    // registro no alcanza para estimarlo, quedan en 0 y no se informan.
    let muestreo_hz = crate::filtro::frecuencia_muestreo(muestras);
    let frecuencia = |serie: &[f64], fraccion: f64| {
        muestreo_hz.and_then(|fs| crate::espectro::frecuencia_de_potencia(serie, fs, fraccion)).unwrap_or(0.0)
    };

    Some(MetricasBalance {
        velocidad_ml_cms: por_segundo(recorrido_ml_cm),
        velocidad_ap_cms: por_segundo(recorrido_ap_cm),
        frec_mediana_ml_hz: frecuencia(&xs, 0.5),
        frec_mediana_ap_hz: frecuencia(&ys, 0.5),
        f80_ml_hz: frecuencia(&xs, 0.8),
        f80_ap_hz: frecuencia(&ys, 0.8),
        longitud_cm,
        area95_cm2,
        velocidad_media_cms,
        duracion_s,
        rms_ml_cm,
        rms_ap_cm,
        rango_ml_cm: rango(&xs),
        rango_ap_cm: rango(&ys),
    })
}

/// Cociente de área entre dos condiciones (`comparado`/`base`): cuánto crece
/// el sway al pasar de una a otra. ~1.0 = sin cambio; valores altos = esa
/// condición exige mucho más al sistema de equilibrio. Sirve tanto para el
/// cociente de Romberg clásico (base=ojos abiertos, comparado=ojos cerrados,
/// misma superficie) como para comparar superficies o el "ratio vestibular"
/// del CTSIB completo (base=firme+ojos abiertos, comparado=espuma+ojos cerrados).
pub fn cociente_area(base: &MetricasBalance, comparado: &MetricasBalance) -> Option<f64> {
    if base.area95_cm2 > 0.0 { Some(comparado.area95_cm2 / base.area95_cm2) } else { None }
}

/// Semiejes + ángulo de la elipse de confianza al 95% de una nube de puntos 2D,
/// vía descomposición espectral cerrada de la matriz de covarianza 2x2.
pub struct Elipse {
    pub centro: (f64, f64),
    pub semi_mayor: f64,
    pub semi_menor: f64,
    pub angulo: f64,
}

/// Elipse de confianza 95% de un registro `[t_s, cop_ml_cm, cop_ap_cm]`.
/// Se toma el registro entero (y no una ventana del trazo) para que la elipse
/// que se dibuja sea exactamente la que se informa como área 95%.
pub fn ajustar_elipse95(registro: &[[f64; 3]]) -> Option<Elipse> {
    let xs: Vec<f64> = registro.iter().map(|m| m[1]).collect();
    let ys: Vec<f64> = registro.iter().map(|m| m[2]).collect();
    ajustar_elipse95_xy(&xs, &ys)
}

pub fn ajustar_elipse95_xy(xs: &[f64], ys: &[f64]) -> Option<Elipse> {
    let n = xs.len();
    if n < 3 {
        return None;
    }
    let nf = n as f64;
    let media_x = xs.iter().sum::<f64>() / nf;
    let media_y = ys.iter().sum::<f64>() / nf;

    let (mut var_x, mut var_y, mut cov_xy) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let dx = xs[i] - media_x;
        let dy = ys[i] - media_y;
        var_x += dx * dx;
        var_y += dy * dy;
        cov_xy += dx * dy;
    }
    let gl = nf - 1.0;
    var_x /= gl;
    var_y /= gl;
    cov_xy /= gl;

    let tr = var_x + var_y;
    let det = var_x * var_y - cov_xy * cov_xy;
    let disc = (tr * tr / 4.0 - det).max(0.0).sqrt();
    let lambda1 = (tr / 2.0 + disc).max(0.0);
    let lambda2 = (tr / 2.0 - disc).max(0.0);
    let angulo = if cov_xy.abs() < 1e-9 && var_x >= var_y { 0.0 } else { 0.5 * (2.0 * cov_xy).atan2(var_x - var_y) };

    Some(Elipse {
        centro: (media_x, media_y),
        semi_mayor: (lambda1 * CHI2_95_2GL).sqrt(),
        semi_menor: (lambda2 * CHI2_95_2GL).sqrt(),
        angulo,
    })
}

impl Elipse {
    pub fn area(&self) -> f64 {
        std::f64::consts::PI * self.semi_mayor * self.semi_menor
    }

    pub fn contorno(&self, segmentos: usize) -> Vec<[f64; 2]> {
        let (cx, cy) = self.centro;
        let (sin_a, cos_a) = self.angulo.sin_cos();
        (0..=segmentos)
            .map(|i| {
                let t = i as f64 / segmentos as f64 * std::f64::consts::TAU;
                let (ex, ey) = (self.semi_mayor * t.cos(), self.semi_menor * t.sin());
                [cx + ex * cos_a - ey * sin_a, cy + ex * sin_a + ey * cos_a]
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elipse_de_puntos_alineados_en_x_no_gira() {
        // Todo el sway es medio-lateral puro: el eje mayor debe quedar sobre X (ángulo 0)
        let xs = vec![-2.0, -1.0, 0.0, 1.0, 2.0];
        let ys = vec![0.0, 0.0, 0.0, 0.0, 0.0];
        let e = ajustar_elipse95_xy(&xs, &ys).unwrap();
        assert!(e.angulo.abs() < 1e-6, "ángulo esperado 0, dio {}", e.angulo);
        assert!(e.semi_mayor > e.semi_menor);
        assert!(e.semi_menor.abs() < 1e-6, "sin varianza en Y, semi-menor debe ser ~0");
    }

    #[test]
    fn elipse_circular_no_favorece_ningun_eje() {
        // Nube simétrica en ambos ejes: los semiejes deben salir prácticamente iguales
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for i in 0..360 {
            let t = (i as f64).to_radians();
            xs.push(t.cos());
            ys.push(t.sin());
        }
        let e = ajustar_elipse95_xy(&xs, &ys).unwrap();
        assert!((e.semi_mayor - e.semi_menor).abs() < 1e-3, "mayor={} menor={}", e.semi_mayor, e.semi_menor);
    }

    #[test]
    fn menos_de_tres_puntos_no_ajusta_elipse() {
        assert!(ajustar_elipse95_xy(&[0.0, 1.0], &[0.0, 1.0]).is_none());
    }

    #[test]
    fn longitud_de_camino_recto_es_la_distancia_esperada() {
        // Path recto de 3 tramos de 1cm en X: longitud total = 3cm exactos
        let muestras = [[0.0, 0.0, 0.0], [1.0, 1.0, 0.0], [2.0, 2.0, 0.0], [3.0, 3.0, 0.0]];
        let m = calcular_metricas(&muestras).unwrap();
        assert!((m.longitud_cm - 3.0).abs() < 1e-9);
        assert!((m.duracion_s - 3.0).abs() < 1e-9);
    }

    #[test]
    fn rango_y_rms_de_oscilacion_simetrica() {
        // Oscila entre -1 y 1 en X, quieto en Y: rango X = 2, rango Y = 0
        let muestras = [[0.0, -1.0, 0.0], [1.0, 1.0, 0.0], [2.0, -1.0, 0.0], [3.0, 1.0, 0.0]];
        let m = calcular_metricas(&muestras).unwrap();
        assert!((m.rango_ml_cm - 2.0).abs() < 1e-9);
        assert!(m.rango_ap_cm.abs() < 1e-9);
        assert!(m.rms_ml_cm > 0.0);
    }

    #[test]
    fn menos_de_tres_muestras_no_da_metricas() {
        assert!(calcular_metricas(&[[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]]).is_none());
    }

    #[test]
    fn la_velocidad_por_eje_separa_el_movimiento_de_cada_direccion() {
        // Movimiento puramente medio-lateral: 1 cm por segundo en ML, nada en AP.
        let muestras: Vec<[f64; 3]> = (0..=10).map(|i| [i as f64, if i % 2 == 0 { 0.0 } else { 1.0 }, 0.0]).collect();
        let m = calcular_metricas(&muestras).unwrap();
        assert!((m.velocidad_ml_cms - 1.0).abs() < 1e-9, "dio {}", m.velocidad_ml_cms);
        assert!(m.velocidad_ap_cms.abs() < 1e-9);
    }

    #[test]
    fn el_balanceo_lento_da_frecuencia_mediana_baja() {
        // 0.5 Hz muestreado a 80 Hz, como un vaivén humano típico.
        let muestras: Vec<[f64; 3]> = (0..1600)
            .map(|i| {
                let t = i as f64 / 80.0;
                [t, (std::f64::consts::TAU * 0.5 * t).sin(), 0.0]
            })
            .collect();
        let m = calcular_metricas(&muestras).unwrap();
        assert!((m.frec_mediana_ml_hz - 0.5).abs() < 0.2, "esperaba ~0.5 Hz, dio {}", m.frec_mediana_ml_hz);
        assert!(m.f80_ml_hz >= m.frec_mediana_ml_hz);
    }

    #[test]
    fn cociente_area_mayor_a_uno_cuando_empeora_la_condicion_comparada() {
        let oa = MetricasBalance { area95_cm2: 2.0, ..Default::default() };
        let oc = MetricasBalance { area95_cm2: 6.0, ..Default::default() };
        assert!((cociente_area(&oa, &oc).unwrap() - 3.0).abs() < 1e-9);
    }

    #[test]
    fn cociente_area_none_si_la_base_tiene_area_cero() {
        let oa = MetricasBalance::default();
        let oc = MetricasBalance { area95_cm2: 1.0, ..Default::default() };
        assert!(cociente_area(&oa, &oc).is_none());
    }
}
