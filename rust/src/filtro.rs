//! Filtrado del COP antes de calcular métricas.
//!
//! El COP crudo trae el ruido del HX711 encima del balanceo real. Ese ruido es
//! de alta frecuencia y amplitud chica, así que casi no mueve el área de la
//! elipse... pero sí infla mucho la **longitud del trazo** y, con ella, la
//! velocidad media: cada muestra agrega un zigzag que se suma al recorrido.
//! La velocidad media es la métrica más usada y la más reproducible del
//! examen, así que sin filtrar los números no son comparables entre equipos.
//!
//! Lo estándar en posturografía es un Butterworth pasabajos con corte entre
//! 5 y 10 Hz (el balanceo humano vive por debajo de ~3 Hz), aplicado de ida y
//! de vuelta para que no meta desfase: un desfase correría el trazo en el
//! tiempo y ensuciaría la relación entre las señales ML y AP.

/// Butterworth pasabajos de 2º orden en forma directa II transpuesta.
#[derive(Clone, Copy, Debug)]
pub struct Butterworth2 {
    b: [f64; 3],
    a: [f64; 3], // a[0] siempre 1.0 tras normalizar
}

impl Butterworth2 {
    /// Diseña el filtro por transformada bilineal. `corte_hz` se recorta a la
    /// frecuencia de Nyquist con margen: pedir un corte por encima no tiene
    /// sentido y produciría coeficientes inestables.
    pub fn pasabajos(corte_hz: f64, muestreo_hz: f64) -> Option<Self> {
        if !(corte_hz.is_finite() && muestreo_hz.is_finite()) || corte_hz <= 0.0 || muestreo_hz <= 0.0 {
            return None;
        }
        let nyquist = muestreo_hz / 2.0;
        let corte = corte_hz.min(nyquist * 0.95);

        // Prewarping: la bilineal comprime el eje de frecuencias, tan() lo
        // compensa para que el corte quede donde se pidió.
        let w = (std::f64::consts::PI * corte / muestreo_hz).tan();
        let w2 = w * w;
        let k = std::f64::consts::SQRT_2; // Q = 1/√2: respuesta maximalmente plana
        let norma = 1.0 / (1.0 + k * w + w2);

        let b0 = w2 * norma;
        Some(Self { b: [b0, 2.0 * b0, b0], a: [1.0, 2.0 * (w2 - 1.0) * norma, (1.0 - k * w + w2) * norma] })
    }

    /// Una pasada hacia adelante. El estado arranca en régimen permanente
    /// para el primer valor, así el filtro no "sube desde cero" al principio
    /// (ese transitorio se vería como un salto enorme del COP).
    fn pasada(&self, x: &[f64]) -> Vec<f64> {
        let Some(&primero) = x.first() else { return Vec::new() };
        let mut z1 = primero * (self.b[1] + self.b[2] - self.a[1] - self.a[2]);
        let mut z2 = primero * (self.b[2] - self.a[2]);

        x.iter()
            .map(|&muestra| {
                let salida = self.b[0] * muestra + z1;
                z1 = self.b[1] * muestra + z2 - self.a[1] * salida;
                z2 = self.b[2] * muestra - self.a[2] * salida;
                salida
            })
            .collect()
    }

    /// Filtrado de fase cero: ida y vuelta. Al filtrar dos veces la
    /// atenuación es el doble (efectivamente 4º orden), pero el desfase de la
    /// ida se cancela exactamente con el de la vuelta.
    pub fn filtrar_cero_fase(&self, x: &[f64]) -> Vec<f64> {
        if x.len() < 3 {
            return x.to_vec();
        }
        let ida: Vec<f64> = self.pasada(x);
        let mut invertida: Vec<f64> = ida.into_iter().rev().collect();
        invertida = self.pasada(&invertida);
        invertida.reverse();
        invertida
    }
}

/// Frecuencia de muestreo efectiva de un registro `[t_s, ml, ap]`.
/// `None` si no hay suficientes muestras o el tiempo no avanzó.
pub fn frecuencia_muestreo(registro: &[[f64; 3]]) -> Option<f64> {
    if registro.len() < 3 {
        return None;
    }
    let duracion = registro.last()?[0] - registro.first()?[0];
    if duracion <= 0.0 {
        return None;
    }
    Some((registro.len() - 1) as f64 / duracion)
}

/// Filtra las dos componentes del COP de un registro. Devuelve el registro
/// tal cual si no se puede diseñar el filtro (muy pocas muestras, tiempo
/// inconsistente, corte absurdo): nunca inventa ni descarta datos.
pub fn filtrar_registro(registro: &[[f64; 3]], corte_hz: f64) -> Vec<[f64; 3]> {
    let Some(muestreo_hz) = frecuencia_muestreo(registro) else {
        return registro.to_vec();
    };
    let Some(filtro) = Butterworth2::pasabajos(corte_hz, muestreo_hz) else {
        return registro.to_vec();
    };

    let ml: Vec<f64> = registro.iter().map(|m| m[1]).collect();
    let ap: Vec<f64> = registro.iter().map(|m| m[2]).collect();
    let ml = filtro.filtrar_cero_fase(&ml);
    let ap = filtro.filtrar_cero_fase(&ap);

    registro.iter().enumerate().map(|(i, m)| [m[0], ml[i], ap[i]]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;

    fn senal(frecuencia_hz: f64, muestreo_hz: f64, n: usize) -> Vec<f64> {
        (0..n).map(|i| (TAU * frecuencia_hz * i as f64 / muestreo_hz).sin()).collect()
    }

    fn amplitud(x: &[f64]) -> f64 {
        // Se mide lejos de los bordes para no contar el arranque del filtro.
        let centro = &x[x.len() / 4..3 * x.len() / 4];
        centro.iter().fold(0.0f64, |acc, v| acc.max(v.abs()))
    }

    #[test]
    fn una_senal_constante_pasa_intacta() {
        let filtro = Butterworth2::pasabajos(6.0, 80.0).unwrap();
        let entrada = vec![3.5; 200];
        let salida = filtro.filtrar_cero_fase(&entrada);
        for v in salida {
            assert!((v - 3.5).abs() < 1e-9, "el continuo no debería moverse, dio {v}");
        }
    }

    #[test]
    fn el_balanceo_lento_sobrevive_y_el_ruido_rapido_no() {
        let filtro = Butterworth2::pasabajos(6.0, 80.0).unwrap();
        // 0.5 Hz: balanceo humano típico. 30 Hz: ruido del ADC.
        let lenta = filtro.filtrar_cero_fase(&senal(0.5, 80.0, 1600));
        let rapida = filtro.filtrar_cero_fase(&senal(30.0, 80.0, 1600));
        assert!(amplitud(&lenta) > 0.95, "el balanceo real se está perdiendo: {}", amplitud(&lenta));
        assert!(amplitud(&rapida) < 0.05, "el ruido rápido sigue pasando: {}", amplitud(&rapida));
    }

    #[test]
    fn el_filtrado_es_de_fase_cero() {
        // Con desfase, el pico de la senoidal se correría en el tiempo.
        let filtro = Butterworth2::pasabajos(5.0, 100.0).unwrap();
        let entrada = senal(1.0, 100.0, 1000);
        let salida = filtro.filtrar_cero_fase(&entrada);
        // Una sola cresta dentro de la ventana (1 Hz a 100 Hz = 100 muestras
        // por ciclo), para que el índice del máximo sea inequívoco.
        let pico_entrada = (200..300).max_by(|&a, &b| entrada[a].total_cmp(&entrada[b])).unwrap();
        let pico_salida = (200..300).max_by(|&a, &b| salida[a].total_cmp(&salida[b])).unwrap();
        assert_eq!(pico_entrada, pico_salida, "el pico se corrió: hay desfase");
    }

    #[test]
    fn filtrar_acorta_el_recorrido_inflado_por_el_ruido() {
        // Balanceo lento + ruido: es exactamente el caso que infla la
        // velocidad media del examen.
        let muestreo = 80.0;
        let n = 2400;
        let mut semilla = 12345u64;
        let mut ruidito = || {
            semilla = semilla.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((semilla >> 33) as f64 / (1u64 << 31) as f64 - 0.5) * 0.2
        };
        let registro: Vec<[f64; 3]> = (0..n)
            .map(|i| {
                let t = i as f64 / muestreo;
                [t, (TAU * 0.4 * t).sin() + ruidito(), (TAU * 0.3 * t).cos() + ruidito()]
            })
            .collect();

        let largo = |r: &[[f64; 3]]| {
            r.windows(2).map(|p| ((p[1][1] - p[0][1]).powi(2) + (p[1][2] - p[0][2]).powi(2)).sqrt()).sum::<f64>()
        };
        let filtrado = filtrar_registro(&registro, 6.0);
        assert!(
            largo(&filtrado) < largo(&registro) * 0.6,
            "el filtro debería sacar el zigzag del ruido: {} vs {}",
            largo(&filtrado),
            largo(&registro)
        );
        assert_eq!(filtrado.len(), registro.len(), "no se descartan muestras");
    }

    #[test]
    fn la_frecuencia_de_muestreo_sale_del_registro() {
        let registro: Vec<[f64; 3]> = (0..81).map(|i| [i as f64 / 80.0, 0.0, 0.0]).collect();
        let fs = frecuencia_muestreo(&registro).unwrap();
        assert!((fs - 80.0).abs() < 1e-9, "esperaba 80 Hz, dio {fs}");
    }

    #[test]
    fn un_registro_inservible_vuelve_tal_cual() {
        let corto = [[0.0, 1.0, 2.0], [0.1, 1.0, 2.0]];
        assert_eq!(filtrar_registro(&corto, 6.0), corto.to_vec());
        assert!(Butterworth2::pasabajos(0.0, 80.0).is_none());
        assert!(Butterworth2::pasabajos(6.0, 0.0).is_none());
    }
}
