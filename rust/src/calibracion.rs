//! Calibración a kilogramos con una masa conocida.
//!
//! Procedimiento pensado para el patrón que se usa acá: un **cubo de 1 kg de
//! 10×10 cm** que se va apoyando sobre cada celda.
//!
//! La idea clave: no importa dónde se apoye el cubo, el peso total sobre la
//! plataforma es siempre el mismo (1 kg). Así que si `d_ij` son las cuentas
//! que subió la celda `j` con el cubo en la posición `i`, y `g_j` es la
//! ganancia (kg por cuenta) de esa celda, para cada posición vale:
//!
//! ```text
//!     g_0·d_i0 + g_1·d_i1 + g_2·d_i2 + g_3·d_i3 = masa
//! ```
//!
//! Con las 4 posiciones queda un sistema de 4×4 que se resuelve exacto. Esto
//! es mejor que el atajo de `g_j = masa / d_jj`: una celda de esquina nunca
//! se lleva el 100% de una carga apoyada encima (parte se reparte por la
//! rigidez de la tapa), y ese atajo se come esa diafonía como si fuera
//! ganancia, dejando el COP corrido.

/// Celdas, en el orden de siempre: fd, fi, bd, bi.
pub const N_CELDAS: usize = 4;

/// Ganancias por celda en kg por cuenta del ADC.
pub type Ganancias = [f64; N_CELDAS];

/// Resuelve las ganancias a partir de las 4 capturas (una por posición del
/// patrón). `deltas[i][j]` = cuentas que subió la celda `j` con la masa en la
/// posición `i`, ya descontada la línea de base con la plataforma vacía.
///
/// `None` si el sistema no tiene solución utilizable: pasa cuando dos
/// posiciones quedaron casi iguales (el cubo no se movió lo suficiente) o si
/// alguna ganancia sale negativa o absurda, que siempre es un error de
/// procedimiento y no algo para dejar guardado.
// Los índices de fila y columna son el lenguaje natural de una eliminación
// gaussiana; reescribirla con iteradores la haría menos legible, no más.
#[allow(clippy::needless_range_loop)]
pub fn resolver_ganancias(deltas: &[[f64; N_CELDAS]; N_CELDAS], masa_kg: f64) -> Option<Ganancias> {
    if !masa_kg.is_finite() || masa_kg <= 0.0 {
        return None;
    }
    let mut matriz = *deltas;
    let mut independiente = [masa_kg; N_CELDAS];

    // Gauss con pivoteo parcial.
    for columna in 0..N_CELDAS {
        let pivote =
            (columna..N_CELDAS).max_by(|&a, &b| matriz[a][columna].abs().total_cmp(&matriz[b][columna].abs()))?;
        if matriz[pivote][columna].abs() < 1e-6 {
            return None; // sistema degenerado: capturas demasiado parecidas
        }
        matriz.swap(columna, pivote);
        independiente.swap(columna, pivote);

        for fila in (columna + 1)..N_CELDAS {
            let factor = matriz[fila][columna] / matriz[columna][columna];
            if factor == 0.0 {
                continue;
            }
            for k in columna..N_CELDAS {
                matriz[fila][k] -= factor * matriz[columna][k];
            }
            independiente[fila] -= factor * independiente[columna];
        }
    }

    // Sustitución hacia atrás.
    let mut ganancias = [0.0f64; N_CELDAS];
    for fila in (0..N_CELDAS).rev() {
        let mut acumulado = independiente[fila];
        for k in (fila + 1)..N_CELDAS {
            acumulado -= matriz[fila][k] * ganancias[k];
        }
        ganancias[fila] = acumulado / matriz[fila][fila];
    }

    // Una ganancia negativa o nula significa celda al revés o captura mal
    // tomada: mejor rechazar que guardar una calibración que invierte el COP.
    if ganancias.iter().all(|g| g.is_finite() && *g > 0.0) { Some(ganancias) } else { None }
}

/// Peso total en kg que miden las celdas con estas ganancias.
pub fn peso_kg(valores: &[f64; N_CELDAS], ganancias: &Ganancias) -> f64 {
    valores.iter().zip(ganancias.iter()).map(|(v, g)| v * g).sum()
}

/// Etapas del asistente de calibración.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Paso {
    /// Explicación y chequeo de que la plataforma esté vacía.
    Vacia,
    /// Masa apoyada sobre la celda `0..N_CELDAS`.
    Celda(usize),
    /// Ganancias resueltas, esperando confirmación.
    Resultado,
}

impl Paso {
    pub fn instruccion(self, lado_cm: f64, masa_kg: f64) -> String {
        match self {
            Paso::Vacia => "Dejá la plataforma vacía y capturá el cero.".to_string(),
            Paso::Celda(i) => format!(
                "Apoyá la masa de {masa_kg:.2} kg ({lado_cm:.0}×{lado_cm:.0} cm) centrada sobre la celda {} y capturá.",
                ETIQUETAS[i.min(N_CELDAS - 1)]
            ),
            Paso::Resultado => "Listo: revisá el peso medido y guardá.".to_string(),
        }
    }

    pub fn siguiente(self) -> Paso {
        match self {
            Paso::Vacia => Paso::Celda(0),
            Paso::Celda(i) if i + 1 < N_CELDAS => Paso::Celda(i + 1),
            Paso::Celda(_) | Paso::Resultado => Paso::Resultado,
        }
    }
}

pub const ETIQUETAS: [&str; N_CELDAS] =
    ["FD (frontal derecha)", "FI (frontal izquierda)", "BD (posterior derecha)", "BI (posterior izquierda)"];

/// Estado del asistente mientras está abierto.
pub struct Asistente {
    pub paso: Paso,
    /// Promedio acumulado de la captura en curso.
    acumulado: [f64; N_CELDAS],
    muestras: usize,
    vacia: [f64; N_CELDAS],
    deltas: [[f64; N_CELDAS]; N_CELDAS],
    pub resultado: Option<Ganancias>,
    pub aviso: String,
}

impl Default for Asistente {
    fn default() -> Self {
        Self {
            paso: Paso::Vacia,
            acumulado: [0.0; N_CELDAS],
            muestras: 0,
            vacia: [0.0; N_CELDAS],
            deltas: [[0.0; N_CELDAS]; N_CELDAS],
            resultado: None,
            aviso: String::new(),
        }
    }
}

impl Asistente {
    /// Acumula una muestra cruda mientras el asistente está abierto.
    pub fn alimentar(&mut self, crudos: [f64; N_CELDAS]) {
        for (acumulado, valor) in self.acumulado.iter_mut().zip(crudos.iter()) {
            *acumulado += valor;
        }
        self.muestras += 1;
    }

    pub fn muestras_acumuladas(&self) -> usize {
        self.muestras
    }

    fn promedio(&self) -> Option<[f64; N_CELDAS]> {
        if self.muestras == 0 {
            return None;
        }
        let n = self.muestras as f64;
        Some(std::array::from_fn(|i| self.acumulado[i] / n))
    }

    fn limpiar_captura(&mut self) {
        self.acumulado = [0.0; N_CELDAS];
        self.muestras = 0;
    }

    /// Toma el promedio acumulado como la captura del paso actual y avanza.
    /// Devuelve `true` si el paso se completó.
    pub fn capturar(&mut self, masa_kg: f64) -> bool {
        let Some(promedio) = self.promedio() else {
            self.aviso = "Todavía no llegaron muestras".to_string();
            return false;
        };

        match self.paso {
            Paso::Vacia => {
                self.vacia = promedio;
                self.paso = self.paso.siguiente();
                self.aviso.clear();
            }
            Paso::Celda(i) => {
                let delta: [f64; N_CELDAS] = std::array::from_fn(|j| promedio[j] - self.vacia[j]);
                let total: f64 = delta.iter().sum();
                if total.abs() < 1e-6 {
                    self.aviso = "No se nota la masa sobre la plataforma: revisá que esté apoyada".to_string();
                    self.limpiar_captura();
                    return false;
                }
                self.deltas[i] = delta;
                self.paso = self.paso.siguiente();
                self.aviso.clear();
                if self.paso == Paso::Resultado {
                    self.resultado = resolver_ganancias(&self.deltas, masa_kg);
                    if self.resultado.is_none() {
                        self.aviso =
                            "No se pudo resolver la calibración: repetí las capturas apoyando la masa bien centrada \
                             sobre cada celda"
                                .to_string();
                    }
                }
            }
            Paso::Resultado => return false,
        }
        self.limpiar_captura();
        true
    }

    /// Peso que daría la última captura con las ganancias resueltas: sirve de
    /// verificación inmediata (tiene que dar la masa del patrón).
    pub fn peso_verificacion(&self, masa_kg: f64) -> Option<f64> {
        let ganancias = self.resultado?;
        let ultima = self.deltas.last()?;
        let _ = masa_kg;
        Some(peso_kg(ultima, &ganancias))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simula la plataforma: ganancias reales conocidas y una fracción de
    /// diafonía hacia las celdas vecinas.
    #[allow(clippy::needless_range_loop)]
    fn deltas_simulados(ganancias_reales: Ganancias, masa_kg: f64) -> [[f64; N_CELDAS]; N_CELDAS] {
        // Con la masa sobre la celda i, esa celda toma el 70% y el resto se
        // reparte 10% en cada una de las otras tres.
        let mut deltas = [[0.0; N_CELDAS]; N_CELDAS];
        for posicion in 0..N_CELDAS {
            for celda in 0..N_CELDAS {
                let fraccion = if celda == posicion { 0.7 } else { 0.1 };
                // cuentas = kg / (kg por cuenta)
                deltas[posicion][celda] = masa_kg * fraccion / ganancias_reales[celda];
            }
        }
        deltas
    }

    #[test]
    fn recupera_las_ganancias_aunque_haya_diafonia_entre_celdas() {
        let reales = [1.0e-4, 1.1e-4, 0.9e-4, 1.05e-4];
        let deltas = deltas_simulados(reales, 1.0);
        let resueltas = resolver_ganancias(&deltas, 1.0).expect("debería resolver");
        for (resuelta, real) in resueltas.iter().zip(reales.iter()) {
            assert!((resuelta - real).abs() / real < 1e-6, "esperaba {real}, dio {resuelta}");
        }
    }

    #[test]
    fn el_peso_medido_da_la_masa_del_patron_en_cualquier_posicion() {
        let reales = [1.0e-4, 1.2e-4, 0.8e-4, 1.0e-4];
        let deltas = deltas_simulados(reales, 1.0);
        let ganancias = resolver_ganancias(&deltas, 1.0).unwrap();
        for posicion in deltas.iter() {
            assert!((peso_kg(posicion, &ganancias) - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn capturas_repetidas_no_dan_calibracion() {
        // Si el cubo no se movió, las 4 filas son iguales: sistema degenerado.
        let fila = [7000.0, 1000.0, 1000.0, 1000.0];
        assert!(resolver_ganancias(&[fila; N_CELDAS], 1.0).is_none());
    }

    #[test]
    fn una_masa_invalida_no_calibra() {
        let deltas = deltas_simulados([1.0e-4; N_CELDAS], 1.0);
        assert!(resolver_ganancias(&deltas, 0.0).is_none());
        assert!(resolver_ganancias(&deltas, -1.0).is_none());
        assert!(resolver_ganancias(&deltas, f64::NAN).is_none());
    }

    #[test]
    fn una_celda_al_reves_se_rechaza() {
        // Celda cableada invertida: su delta sale negativo y la ganancia daría
        // negativa, lo que invertiría el COP. Mejor no guardar nada.
        let mut deltas = deltas_simulados([1.0e-4; N_CELDAS], 1.0);
        for fila in deltas.iter_mut() {
            fila[2] = -fila[2];
        }
        assert!(resolver_ganancias(&deltas, 1.0).is_none());
    }

    #[test]
    #[allow(clippy::needless_range_loop)]
    fn el_asistente_recorre_vacia_las_cuatro_celdas_y_resultado() {
        let reales = [1.0e-4, 1.1e-4, 0.9e-4, 1.05e-4];
        let deltas = deltas_simulados(reales, 1.0);
        let vacia = [5_000.0, -2_000.0, 1_500.0, 800.0]; // cero arbitrario de cada celda

        let mut asistente = Asistente::default();
        asistente.alimentar(vacia);
        assert!(asistente.capturar(1.0));
        assert_eq!(asistente.paso, Paso::Celda(0));

        for posicion in 0..N_CELDAS {
            let lectura: [f64; N_CELDAS] = std::array::from_fn(|j| vacia[j] + deltas[posicion][j]);
            // Dos muestras iguales: el promedio tiene que dar lo mismo.
            asistente.alimentar(lectura);
            asistente.alimentar(lectura);
            assert!(asistente.capturar(1.0), "la captura {posicion} debería completarse");
        }

        assert_eq!(asistente.paso, Paso::Resultado);
        let ganancias = asistente.resultado.expect("debería haber resultado");
        for (resuelta, real) in ganancias.iter().zip(reales.iter()) {
            assert!((resuelta - real).abs() / real < 1e-6);
        }
        let peso = asistente.peso_verificacion(1.0).unwrap();
        assert!((peso - 1.0).abs() < 1e-9, "la verificación debería dar la masa del patrón, dio {peso}");
    }

    #[test]
    fn capturar_sin_masa_encima_avisa_y_no_avanza() {
        let mut asistente = Asistente::default();
        asistente.alimentar([1000.0; N_CELDAS]);
        assert!(asistente.capturar(1.0)); // cero
        asistente.alimentar([1000.0; N_CELDAS]); // nada cambió
        assert!(!asistente.capturar(1.0));
        assert_eq!(asistente.paso, Paso::Celda(0));
        assert!(!asistente.aviso.is_empty());
    }
}
