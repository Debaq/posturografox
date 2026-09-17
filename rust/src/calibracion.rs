//! Calibración a kilogramos con una masa conocida.
//!
//! Procedimiento pensado para el patrón que se usa aquí: un **cubo de 1 kg de
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
//!
//! Cada captura se toma en una ventana de tiempo controlada: primero se
//! descartan unos segundos de estabilización (la mano todavía está sobre la
//! plataforma, la masa se está asentando) y recién después se promedia. Sin
//! esa ventana, el promedio mezclaba las lecturas tomadas mientras se movía
//! la masa, y con esos datos el sistema no cerraba nunca.

/// Celdas, en el orden de siempre: fd, fi, bd, bi.
pub const N_CELDAS: usize = 4;

/// Segundos que se descartan tras apretar "Capturar", para que la mano salga
/// de la plataforma y la lectura se asiente.
pub const ESTABILIZACION_S: f64 = 1.5;
/// Segundos que se promedian para una captura.
pub const MEDICION_S: f64 = 2.0;

/// Ganancias por celda en kg por cuenta del ADC.
pub type Ganancias = [f64; N_CELDAS];

/// Por qué no se pudo resolver una calibración. Cada caso tiene una causa
/// distinta y una salida distinta, así que no alcanza con un "falló".
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ErrorCalibracion {
    /// La masa configurada no sirve (cero, negativa, NaN).
    MasaInvalida,
    /// Las cuatro capturas son demasiado parecidas entre sí: pasa cuando la
    /// masa no se movió de lugar entre una y otra.
    SistemaDegenerado,
    /// Una ganancia dio negativa o cero: celda cableada al revés, o una
    /// captura tomada mientras la masa no estaba donde correspondía.
    GananciaInvalida { celda: usize, valor: f64 },
}

impl ErrorCalibracion {
    pub fn explicacion(&self) -> String {
        match self {
            ErrorCalibracion::MasaInvalida => {
                "La masa del patrón configurada no es válida. Corríjala en Configuración → Calibración.".to_string()
            }
            ErrorCalibracion::SistemaDegenerado => {
                "Las cuatro capturas salieron casi iguales. Suele pasar si la masa no se apoyó sobre una celda \
                 distinta en cada paso, o si se capturó antes de que la lectura se asentara."
                    .to_string()
            }
            ErrorCalibracion::GananciaInvalida { celda, valor } => format!(
                "La celda {} dio una ganancia imposible ({valor:.6} kg/cuenta). Suele ser una celda conectada al \
                 revés, o una captura tomada con la masa en otro lugar.",
                ETIQUETAS[(*celda).min(N_CELDAS - 1)]
            ),
        }
    }
}

/// Resuelve las ganancias a partir de las 4 capturas (una por posición del
/// patrón). `deltas[i][j]` = cuentas que subió la celda `j` con la masa en la
/// posición `i`, ya descontada la línea de base con la plataforma vacía.
// Los índices de fila y columna son el lenguaje natural de una eliminación
// gaussiana; reescribirla con iteradores la haría menos legible, no más.
#[allow(clippy::needless_range_loop)]
pub fn resolver_ganancias(deltas: &[[f64; N_CELDAS]; N_CELDAS], masa_kg: f64) -> Result<Ganancias, ErrorCalibracion> {
    if !masa_kg.is_finite() || masa_kg <= 0.0 {
        return Err(ErrorCalibracion::MasaInvalida);
    }
    let mut matriz = *deltas;
    let mut independiente = [masa_kg; N_CELDAS];

    // Gauss con pivoteo parcial.
    for columna in 0..N_CELDAS {
        let pivote = (columna..N_CELDAS)
            .max_by(|&a, &b| matriz[a][columna].abs().total_cmp(&matriz[b][columna].abs()))
            .ok_or(ErrorCalibracion::SistemaDegenerado)?;
        if matriz[pivote][columna].abs() < 1e-6 {
            return Err(ErrorCalibracion::SistemaDegenerado);
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
    for (celda, valor) in ganancias.iter().enumerate() {
        if !valor.is_finite() || *valor <= 0.0 {
            return Err(ErrorCalibracion::GananciaInvalida { celda, valor: *valor });
        }
    }
    Ok(ganancias)
}

/// Calibración de respaldo: una sola escala para las 4 celdas, sacada del
/// total de cuentas que agrega la masa.
///
/// Es lo que queda cuando el sistema 4×4 no cierra (capturas ruidosas, celdas
/// muy parecidas entre sí). Deja el **peso** bien medido, pero no corrige las
/// diferencias entre celdas, así que el COP puede quedar algo corrido: sirve
/// para trabajar, no reemplaza a una calibración completa.
pub fn escala_global(deltas: &[[f64; N_CELDAS]; N_CELDAS], masa_kg: f64) -> Result<Ganancias, ErrorCalibracion> {
    if !masa_kg.is_finite() || masa_kg <= 0.0 {
        return Err(ErrorCalibracion::MasaInvalida);
    }
    let totales: Vec<f64> = deltas.iter().map(|d| d.iter().sum::<f64>()).filter(|t| *t > 0.0).collect();
    if totales.is_empty() {
        return Err(ErrorCalibracion::SistemaDegenerado);
    }
    let promedio = totales.iter().sum::<f64>() / totales.len() as f64;
    Ok([masa_kg / promedio; N_CELDAS])
}

/// Peso total en kg que miden las celdas con estas ganancias.
pub fn peso_kg(valores: &[f64; N_CELDAS], ganancias: &Ganancias) -> f64 {
    valores.iter().zip(ganancias.iter()).map(|(v, g)| v * g).sum()
}

/// Peso que da cada captura con las ganancias resueltas. Sirve de
/// verificación: las cuatro tienen que dar la masa del patrón.
pub fn verificacion(deltas: &[[f64; N_CELDAS]; N_CELDAS], ganancias: &Ganancias) -> [f64; N_CELDAS] {
    std::array::from_fn(|i| peso_kg(&deltas[i], ganancias))
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
            Paso::Vacia => "Deje la plataforma vacía y capture el cero.".to_string(),
            Paso::Celda(i) => format!(
                "Apoye la masa de {masa_kg:.2} kg ({lado_cm:.0}×{lado_cm:.0} cm) centrada sobre la celda {} y capture.",
                ETIQUETAS[i.min(N_CELDAS - 1)]
            ),
            Paso::Resultado => "Listo: revise el peso medido y guarde.".to_string(),
        }
    }

    /// Número de paso (1 a 6) para mostrar el avance.
    pub fn numero(self) -> usize {
        match self {
            Paso::Vacia => 1,
            Paso::Celda(i) => 2 + i,
            Paso::Resultado => N_CELDAS + 2,
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
pub const ETIQUETAS_CORTAS: [&str; N_CELDAS] = ["FD", "FI", "BD", "BI"];

/// En qué está la captura del paso actual.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Fase {
    /// Esperando que se apriete "Capturar".
    Esperando,
    /// Descartando muestras mientras la lectura se asienta.
    Estabilizando { desde_s: f64 },
    /// Promediando la ventana de medición.
    Midiendo { desde_s: f64 },
}

/// Estado del asistente mientras está abierto.
pub struct Asistente {
    pub paso: Paso,
    fase: Fase,
    /// Última lectura cruda recibida, para mostrarla en vivo.
    ultima: [f64; N_CELDAS],
    t_actual_s: f64,
    /// Suma de la ventana de medición en curso.
    acumulado: [f64; N_CELDAS],
    muestras: usize,
    vacia: [f64; N_CELDAS],
    tiene_vacia: bool,
    deltas: [[f64; N_CELDAS]; N_CELDAS],
    capturadas: [bool; N_CELDAS],
    pub resultado: Option<Ganancias>,
    /// `true` si el resultado salió de la escala global de respaldo.
    pub es_respaldo: bool,
    pub error: Option<ErrorCalibracion>,
    pub aviso: String,
}

impl Default for Asistente {
    fn default() -> Self {
        Self {
            paso: Paso::Vacia,
            fase: Fase::Esperando,
            ultima: [0.0; N_CELDAS],
            t_actual_s: 0.0,
            acumulado: [0.0; N_CELDAS],
            muestras: 0,
            vacia: [0.0; N_CELDAS],
            tiene_vacia: false,
            deltas: [[0.0; N_CELDAS]; N_CELDAS],
            capturadas: [false; N_CELDAS],
            resultado: None,
            es_respaldo: false,
            error: None,
            aviso: String::new(),
        }
    }
}

impl Asistente {
    /// Recibe una muestra cruda con su marca de tiempo. Devuelve `true` si con
    /// esta muestra se completó una captura.
    pub fn alimentar(&mut self, t_s: f64, crudos: [f64; N_CELDAS], masa_kg: f64) -> bool {
        self.ultima = crudos;
        self.t_actual_s = t_s;

        match self.fase {
            Fase::Esperando => false,
            Fase::Estabilizando { desde_s } => {
                if t_s - desde_s >= ESTABILIZACION_S {
                    self.acumulado = [0.0; N_CELDAS];
                    self.muestras = 0;
                    self.fase = Fase::Midiendo { desde_s: t_s };
                }
                false
            }
            Fase::Midiendo { desde_s } => {
                for (acumulado, valor) in self.acumulado.iter_mut().zip(crudos.iter()) {
                    *acumulado += valor;
                }
                self.muestras += 1;
                if t_s - desde_s >= MEDICION_S && self.muestras >= 2 {
                    self.cerrar_captura(masa_kg);
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Arranca la captura del paso actual.
    pub fn iniciar_captura(&mut self) {
        self.aviso.clear();
        self.acumulado = [0.0; N_CELDAS];
        self.muestras = 0;
        self.fase = Fase::Estabilizando { desde_s: self.t_actual_s };
    }

    pub fn cancelar_captura(&mut self) {
        self.fase = Fase::Esperando;
        self.muestras = 0;
    }

    /// Texto y avance (0.0..1.0) de lo que está pasando ahora.
    pub fn progreso(&self) -> Option<(String, f32)> {
        match self.fase {
            Fase::Esperando => None,
            Fase::Estabilizando { desde_s } => {
                let transcurrido = self.t_actual_s - desde_s;
                Some((
                    format!("Estabilizando... retire la mano ({:.1} s)", (ESTABILIZACION_S - transcurrido).max(0.0)),
                    (transcurrido / ESTABILIZACION_S).clamp(0.0, 1.0) as f32,
                ))
            }
            Fase::Midiendo { desde_s } => {
                let transcurrido = self.t_actual_s - desde_s;
                Some((
                    format!("Midiendo... {} muestras promediadas", self.muestras),
                    (transcurrido / MEDICION_S).clamp(0.0, 1.0) as f32,
                ))
            }
        }
    }

    /// Última lectura cruda de cada celda.
    pub fn lectura(&self) -> [f64; N_CELDAS] {
        self.ultima
    }

    /// Cuánto subió cada celda respecto del cero, si ya se capturó el cero.
    pub fn delta_en_vivo(&self) -> Option<[f64; N_CELDAS]> {
        self.tiene_vacia.then(|| std::array::from_fn(|i| self.ultima[i] - self.vacia[i]))
    }

    /// Qué celdas ya tienen su captura hecha.
    pub fn capturadas(&self) -> [bool; N_CELDAS] {
        self.capturadas
    }

    /// Deltas registrados hasta ahora.
    pub fn deltas(&self) -> &[[f64; N_CELDAS]; N_CELDAS] {
        &self.deltas
    }

    /// Vuelve a tomar el paso actual (o el último, si ya se llegó al final).
    pub fn repetir_paso(&mut self) {
        self.cancelar_captura();
        self.resultado = None;
        self.error = None;
        self.es_respaldo = false;
        if self.paso == Paso::Resultado {
            self.paso = Paso::Celda(N_CELDAS - 1);
        }
        if let Paso::Celda(i) = self.paso {
            self.capturadas[i] = false;
        }
        self.aviso = "Repita la captura de este paso.".to_string();
    }

    /// Acepta la escala global de respaldo cuando el sistema 4×4 no cerró.
    pub fn usar_escala_global(&mut self, masa_kg: f64) {
        match escala_global(&self.deltas, masa_kg) {
            Ok(ganancias) => {
                self.resultado = Some(ganancias);
                self.es_respaldo = true;
                self.error = None;
                self.aviso = "Escala global aplicada: el peso queda bien medido, pero el COP puede tener un \
                              pequeño sesgo si las celdas difieren entre sí."
                    .to_string();
            }
            Err(e) => {
                self.error = Some(e);
                self.aviso = e.explicacion();
            }
        }
    }

    fn promedio(&self) -> Option<[f64; N_CELDAS]> {
        if self.muestras == 0 {
            return None;
        }
        let n = self.muestras as f64;
        Some(std::array::from_fn(|i| self.acumulado[i] / n))
    }

    fn cerrar_captura(&mut self, masa_kg: f64) {
        let Some(promedio) = self.promedio() else {
            self.aviso = "No llegaron muestras durante la medición: revise la conexión.".to_string();
            self.fase = Fase::Esperando;
            return;
        };
        self.fase = Fase::Esperando;

        match self.paso {
            Paso::Vacia => {
                self.vacia = promedio;
                self.tiene_vacia = true;
                self.paso = self.paso.siguiente();
                self.aviso = "Cero registrado.".to_string();
            }
            Paso::Celda(i) => {
                let delta: [f64; N_CELDAS] = std::array::from_fn(|j| promedio[j] - self.vacia[j]);
                let total: f64 = delta.iter().sum();
                if total <= 0.0 {
                    self.aviso = format!(
                        "La carga no aumentó respecto del cero ({total:.0} cuentas). Verifique que la masa esté \
                         apoyada sobre la plataforma y repita la captura."
                    );
                    return;
                }
                self.deltas[i] = delta;
                self.capturadas[i] = true;
                self.aviso = format!("Celda {} capturada: +{total:.0} cuentas.", ETIQUETAS_CORTAS[i]);
                self.paso = self.paso.siguiente();
                if self.paso == Paso::Resultado {
                    match resolver_ganancias(&self.deltas, masa_kg) {
                        Ok(ganancias) => {
                            self.resultado = Some(ganancias);
                            self.es_respaldo = false;
                            self.error = None;
                            self.aviso.clear();
                        }
                        Err(e) => {
                            self.error = Some(e);
                            self.aviso = e.explicacion();
                        }
                    }
                }
            }
            Paso::Resultado => {}
        }
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

    /// Alimenta al asistente como lo haría el posturógrafo a 80 muestras/s.
    fn correr(asistente: &mut Asistente, t0: f64, segundos: f64, lectura: [f64; N_CELDAS], masa: f64) -> f64 {
        let paso = 1.0 / 80.0;
        let mut t = t0;
        while t < t0 + segundos {
            asistente.alimentar(t, lectura, masa);
            t += paso;
        }
        t
    }

    /// Una captura completa con la lectura dada.
    fn capturar(asistente: &mut Asistente, t0: f64, lectura: [f64; N_CELDAS], masa: f64) -> f64 {
        asistente.iniciar_captura();
        correr(asistente, t0, ESTABILIZACION_S + MEDICION_S + 0.2, lectura, masa)
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
        for peso in verificacion(&deltas, &ganancias) {
            assert!((peso - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn capturas_repetidas_dan_sistema_degenerado() {
        // Si el cubo no se movió, las 4 filas son iguales.
        let fila = [7000.0, 1000.0, 1000.0, 1000.0];
        assert_eq!(resolver_ganancias(&[fila; N_CELDAS], 1.0), Err(ErrorCalibracion::SistemaDegenerado));
    }

    #[test]
    fn una_masa_invalida_no_calibra() {
        let deltas = deltas_simulados([1.0e-4; N_CELDAS], 1.0);
        assert_eq!(resolver_ganancias(&deltas, 0.0), Err(ErrorCalibracion::MasaInvalida));
        assert_eq!(resolver_ganancias(&deltas, -1.0), Err(ErrorCalibracion::MasaInvalida));
        assert_eq!(resolver_ganancias(&deltas, f64::NAN), Err(ErrorCalibracion::MasaInvalida));
    }

    #[test]
    fn una_celda_al_reves_se_rechaza_diciendo_cual() {
        // Celda cableada invertida: su delta sale negativo y la ganancia daría
        // negativa, lo que invertiría el COP.
        let mut deltas = deltas_simulados([1.0e-4; N_CELDAS], 1.0);
        for fila in deltas.iter_mut() {
            fila[2] = -fila[2];
        }
        match resolver_ganancias(&deltas, 1.0) {
            Err(ErrorCalibracion::GananciaInvalida { celda, .. }) => assert_eq!(celda, 2),
            otro => panic!("esperaba GananciaInvalida en la celda 2, dio {otro:?}"),
        }
    }

    #[test]
    fn la_escala_global_deja_bien_el_peso_aunque_no_cierre_el_sistema() {
        let deltas = deltas_simulados([1.0e-4; N_CELDAS], 1.0);
        let ganancias = escala_global(&deltas, 1.0).unwrap();
        for peso in verificacion(&deltas, &ganancias) {
            assert!((peso - 1.0).abs() < 1e-9, "el peso total tiene que dar la masa del patrón, dio {peso}");
        }
    }

    #[test]
    fn la_captura_ignora_las_lecturas_de_mientras_se_mueve_la_masa() {
        // Este es el caso que rompía antes: entre paso y paso la mano está
        // sobre la plataforma, y esas lecturas se colaban en el promedio.
        let mut asistente = Asistente::default();
        let vacia = [1000.0; N_CELDAS];
        let mut t = 0.0;

        t = correr(&mut asistente, t, 1.0, vacia, 1.0);
        assert_eq!(asistente.paso, Paso::Vacia, "sin apretar Capturar no avanza");

        asistente.iniciar_captura();
        t = correr(&mut asistente, t, 0.5, [99_999.0; N_CELDAS], 1.0); // mano encima
        t = correr(&mut asistente, t, ESTABILIZACION_S + MEDICION_S + 0.5, vacia, 1.0);
        assert_eq!(asistente.paso, Paso::Celda(0), "la captura del cero tiene que haberse completado");

        let con_masa: [f64; N_CELDAS] = [8_000.0, 2_000.0, 2_000.0, 2_000.0];
        capturar(&mut asistente, t, con_masa, 1.0);

        let delta = asistente.deltas()[0];
        assert!((delta[0] - 7_000.0).abs() < 1.0, "delta contaminado por el manotazo: {delta:?}");
    }

    #[test]
    #[allow(clippy::needless_range_loop)]
    fn el_asistente_completo_resuelve_las_ganancias() {
        let reales = [1.0e-4, 1.1e-4, 0.9e-4, 1.05e-4];
        let deltas = deltas_simulados(reales, 1.0);
        let vacia = [5_000.0, -2_000.0, 1_500.0, 800.0]; // cero arbitrario de cada celda

        let mut asistente = Asistente::default();
        let mut t = capturar(&mut asistente, 0.0, vacia, 1.0);

        for posicion in 0..N_CELDAS {
            let lectura: [f64; N_CELDAS] = std::array::from_fn(|j| vacia[j] + deltas[posicion][j]);
            t = capturar(&mut asistente, t, lectura, 1.0);
            assert!(asistente.capturadas()[posicion], "la captura {posicion} debería estar hecha");
        }

        assert_eq!(asistente.paso, Paso::Resultado);
        assert!(asistente.error.is_none(), "no debería fallar: {:?}", asistente.error);
        let ganancias = asistente.resultado.expect("debería haber resultado");
        for (resuelta, real) in ganancias.iter().zip(reales.iter()) {
            assert!((resuelta - real).abs() / real < 1e-6);
        }
    }

    #[test]
    fn capturar_sin_masa_encima_avisa_y_no_avanza() {
        let mut asistente = Asistente::default();
        let vacia = [1000.0; N_CELDAS];

        let t = capturar(&mut asistente, 0.0, vacia, 1.0);
        assert_eq!(asistente.paso, Paso::Celda(0));

        capturar(&mut asistente, t, vacia, 1.0); // nada cambió

        assert_eq!(asistente.paso, Paso::Celda(0), "sin carga no puede darse por buena la captura");
        assert!(asistente.aviso.contains("no aumentó"), "aviso poco claro: {}", asistente.aviso);
    }

    #[test]
    fn repetir_paso_deshace_la_ultima_captura() {
        let mut asistente =
            Asistente { paso: Paso::Celda(2), capturadas: [true, true, true, false], ..Asistente::default() };
        asistente.repetir_paso();
        assert_eq!(asistente.paso, Paso::Celda(2));
        assert_eq!(asistente.capturadas(), [true, true, false, false]);

        let mut asistente = Asistente { paso: Paso::Resultado, capturadas: [true; N_CELDAS], ..Asistente::default() };
        asistente.repetir_paso();
        assert_eq!(asistente.paso, Paso::Celda(N_CELDAS - 1), "desde el resultado se vuelve al último paso");
        assert!(!asistente.capturadas()[N_CELDAS - 1]);
    }

    #[test]
    fn el_progreso_avisa_en_que_esta_la_captura() {
        let mut asistente = Asistente::default();
        assert!(asistente.progreso().is_none(), "sin captura en curso no hay progreso que mostrar");

        asistente.iniciar_captura();
        let t = correr(&mut asistente, 0.0, ESTABILIZACION_S * 0.5, [0.0; N_CELDAS], 1.0);
        let (texto, avance) = asistente.progreso().expect("hay captura en curso");
        assert!(texto.contains("Estabilizando"), "texto inesperado: {texto}");
        assert!((0.3..0.7).contains(&avance), "avance fuera de rango: {avance}");

        correr(&mut asistente, t, ESTABILIZACION_S, [0.0; N_CELDAS], 1.0);
        let (texto, _) = asistente.progreso().expect("sigue midiendo");
        assert!(texto.contains("Midiendo"), "texto inesperado: {texto}");
    }
}
