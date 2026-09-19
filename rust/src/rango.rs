//! Rango de desplazamiento del paciente: dónde tiene su centro de reposo y
//! hasta dónde alcanza a llevar el COP hacia cada lado.
//!
//! El modo juego escalaba el COP al semieje físico de la plataforma (20 cm
//! con la geometría por defecto). Nadie desplaza su COP 20 cm: la excursión
//! voluntaria de una persona sana ronda los ±5–8 cm y la de un hemiparético
//! los ±2–3 cm, además descentrada hacia el lado que carga. Con esa escala el
//! zorro usaba una fracción de la pista y las rocas del borde eran
//! inesquivables por escala, no por déficit.
//!
//! Acá vive la conversión que reemplaza a esa división: centímetros de COP a
//! posición normalizada (-1.0 .. 1.0), relativa al alcance *de esa persona*.
//! Eso hace jugable el juego para cualquiera y, de paso, vuelve comparables
//! las amplitudes entre sesiones y entre pacientes, porque pasan a expresarse
//! como fracción del límite propio.

/// Fracción del alcance calibrado que hay que cubrir para llegar al borde de
/// la pista. Con 1.0 el paciente tendría que ir a su límite real de caída
/// para esquivar la última roca: agotador y peligroso. Con 0.7 recorre toda
/// la pista sin salir de su zona segura.
///
/// Es el parámetro de dosificación —lo que un fisio sube sesión a sesión—,
/// así que en R40 pasa a ser una opción de configuración; este valor queda
/// como el que se aplica mientras tanto.
pub const EXIGENCIA_DEFECTO: f64 = 0.7;

/// Límites en los que tiene sentido la exigencia: por debajo el juego no pide
/// desplazamiento y por encima empuja al paciente contra su límite de caída.
pub const EXIGENCIA_MIN: f64 = 0.4;
pub const EXIGENCIA_MAX: f64 = 0.9;

/// Alcance mínimo hacia un lado para que la calibración sirva. Con menos que
/// esto el ruido de la señal (milímetros) bastaría para mandar al zorro de
/// punta a punta de la pista, así que un rango más chico se declara inválido
/// y se cae al rango por defecto.
const MEDIO_ALCANCE_MINIMO_CM: f64 = 1.0;

// Rango poblacional de referencia, para cuando no hay calibración de la
// persona. No es el semieje de la plataforma: son valores de excursión
// voluntaria plausibles, y el anterior casi duplica al posterior porque el
// tobillo permite mucho más inclinación hacia adelante que hacia atrás.
const DEFECTO_ML_CM: f64 = 7.0;
const DEFECTO_ANTERIOR_CM: f64 = 8.0;
const DEFECTO_POSTERIOR_CM: f64 = 4.0;

/// De dónde salieron los números. Importa para leer el registro: una partida
/// jugada con el rango por defecto no es un dato de la persona.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum Origen {
    /// Medido en el ejercicio de límites de estabilidad (ver `limites.rs`).
    Limites,
    /// Medido con la calibración del propio modo juego.
    Juego,
    /// Sin calibrar: rango poblacional recortado a la plataforma.
    #[default]
    Defecto,
}

/// Alcance en un eje: el centro de reposo y los dos extremos, en cm de COP.
///
/// Los dos extremos se guardan por separado, en vez de un único radio, porque
/// la carga asimétrica es la regla en rehabilitación: quien apoya más en una
/// pierna tiene el centro corrido y llega mucho más lejos hacia un lado. Con
/// un rango simétrico el zorro quedaría permanentemente descentrado y el
/// paciente pelearía contra su propio apoyo todo el rato.
#[derive(Clone, Copy, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct Eje {
    pub centro_cm: f64,
    /// Extremo negativo: izquierda en ML, posterior en AP.
    pub min_cm: f64,
    /// Extremo positivo: derecha en ML, anterior en AP.
    pub max_cm: f64,
}

impl Eje {
    /// Arma un eje medido, recortado al semieje físico `limite_cm` de la
    /// plataforma. Devuelve `None` si alguno de los dos alcances quedó por
    /// debajo del mínimo utilizable: es preferible el rango por defecto a una
    /// calibración que amplifique el ruido.
    pub fn nuevo(centro_cm: f64, min_cm: f64, max_cm: f64, limite_cm: f64) -> Option<Self> {
        let limite = limite_cm.abs();
        let eje = Self {
            centro_cm: centro_cm.clamp(-limite, limite),
            min_cm: min_cm.clamp(-limite, limite),
            max_cm: max_cm.clamp(-limite, limite),
        };
        (eje.alcance_positivo_cm() >= MEDIO_ALCANCE_MINIMO_CM && eje.alcance_negativo_cm() >= MEDIO_ALCANCE_MINIMO_CM)
            .then_some(eje)
    }

    /// Eje simétrico alrededor de cero, recortado a la plataforma. El recorte
    /// nunca baja del mínimo utilizable: una plataforma declarada más chica
    /// que eso dejaría el juego sin recorrido y con el COP saltando.
    fn poblacional(alcance_cm: f64, limite_cm: f64) -> Self {
        Self::asimetrico(alcance_cm, alcance_cm, limite_cm)
    }

    /// Igual que `poblacional`, con alcances distintos hacia cada lado.
    fn asimetrico(negativo_cm: f64, positivo_cm: f64, limite_cm: f64) -> Self {
        let techo = limite_cm.abs().max(MEDIO_ALCANCE_MINIMO_CM);
        Self { centro_cm: 0.0, min_cm: -negativo_cm.min(techo), max_cm: positivo_cm.min(techo) }
    }

    pub fn alcance_positivo_cm(&self) -> f64 {
        self.max_cm - self.centro_cm
    }

    pub fn alcance_negativo_cm(&self) -> f64 {
        self.centro_cm - self.min_cm
    }

    /// Convierte un COP en cm a posición normalizada (-1.0 .. 1.0) relativa al
    /// alcance de la persona.
    ///
    /// Lineal por tramo: cada lado se escala con su propio alcance, así que un
    /// paciente con el centro corrido igual llega a los dos bordes de la pista
    /// y su posición de reposo cae en el medio.
    pub fn normalizar(&self, cop_cm: f64, exigencia: f64) -> f64 {
        let exigencia = exigencia.clamp(EXIGENCIA_MIN, EXIGENCIA_MAX);
        let desvio = cop_cm - self.centro_cm;
        let alcance = if desvio >= 0.0 { self.alcance_positivo_cm() } else { self.alcance_negativo_cm() };
        // `nuevo` y los constructores poblacionales garantizan alcance > 0.
        (desvio / (alcance * exigencia)).clamp(-1.0, 1.0)
    }
}

/// Los dos ejes juntos, con la procedencia de la medición.
#[derive(Clone, Copy, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct RangoCalibrado {
    /// Medio-lateral: negativo a la izquierda, positivo a la derecha.
    pub ml: Eje,
    /// Antero-posterior: negativo atrás, positivo adelante. Todavía no mueve
    /// nada en el juego (no hay salto ni agachada), pero se mide y se guarda
    /// para no tener que migrar el formato cuando exista la mecánica.
    pub ap: Eje,
    pub origen: Origen,
}

impl RangoCalibrado {
    /// Rango poblacional recortado a la plataforma, para cuando no hay
    /// calibración de la persona.
    pub fn por_defecto(ancho_cm: f64, prof_cm: f64) -> Self {
        Self {
            ml: Eje::poblacional(DEFECTO_ML_CM, ancho_cm / 2.0),
            ap: Eje::asimetrico(DEFECTO_POSTERIOR_CM, DEFECTO_ANTERIOR_CM, prof_cm / 2.0),
            origen: Origen::Defecto,
        }
    }

    /// Rango medido. Si alguno de los dos ejes no pasa la validación, la
    /// calibración entera se descarta: mezclar un eje medido con otro por
    /// defecto daría un registro imposible de interpretar después.
    pub fn medido(ml: Option<Eje>, ap: Option<Eje>, origen: Origen) -> Option<Self> {
        Some(Self { ml: ml?, ap: ap?, origen })
    }
}

/// Cuánto dura la toma de reposo, y cuánto se descarta al principio para que
/// no entre el movimiento de acomodarse recién subido.
const REPOSO_S: f32 = 4.0;
const REPOSO_DESCARTE_S: f32 = 1.0;

/// Cuánto hay que sostener el alcance cerca del máximo para darlo por bueno.
/// Un pico instantáneo puede ser un tropiezo; medio segundo sostenido es un
/// alcance que la persona controla.
const SOSTENER_S: f32 = 0.5;

/// Margen dentro del cual se considera que sigue sosteniendo el alcance.
const TOLERANCIA_CM: f64 = 0.5;

/// Si no logra sostener nada en este tiempo, se toma lo que haya y se sigue:
/// dejar a alguien empujando contra su límite no mejora la medición.
const PASO_MAXIMO_S: f32 = 12.0;

/// Los pasos de la calibración, en orden.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Paso {
    Reposo,
    Izquierda,
    Derecha,
    Atras,
    Adelante,
}

const PASOS: [Paso; 5] = [Paso::Reposo, Paso::Izquierda, Paso::Derecha, Paso::Atras, Paso::Adelante];

impl Paso {
    /// Qué se le pide al paciente. La segunda línea es el detalle chico.
    pub fn instruccion(self) -> (&'static str, &'static str) {
        match self {
            Paso::Reposo => ("Quédese quieto", "Parado como siempre, mirando al frente"),
            Paso::Izquierda => ("Cárguese a la izquierda", "Sin despegar los pies, y sosténgalo"),
            Paso::Derecha => ("Cárguese a la derecha", "Sin despegar los pies, y sosténgalo"),
            Paso::Atras => ("Inclínese hacia atrás", "Sin despegar los talones, y sosténgalo"),
            Paso::Adelante => ("Inclínese hacia adelante", "Sin despegar los dedos, y sosténgalo"),
        }
    }

    /// Eje que mide el paso: `true` si es medio-lateral.
    pub fn es_ml(self) -> bool {
        matches!(self, Paso::Izquierda | Paso::Derecha)
    }

    /// Hacia dónde tiene que ir el COP (+1 derecha/adelante, -1 izquierda/atrás).
    /// En el reposo no se pide dirección.
    pub fn signo(self) -> f64 {
        match self {
            Paso::Reposo => 0.0,
            Paso::Izquierda | Paso::Atras => -1.0,
            Paso::Derecha | Paso::Adelante => 1.0,
        }
    }
}

/// Toma de la medida: reposo y un alcance sostenido hacia cada lado.
///
/// Es pura —se le van pasando muestras y devuelve en qué anda—, así que el
/// juego solo dibuja lo que ella decide y la secuencia se puede testear sin
/// ventana ni plataforma.
pub struct Calibracion {
    indice: usize,
    /// Cuánto lleva el paso actual.
    tiempo_s: f32,
    /// Cuánto lleva sosteniendo cerca del mejor alcance del paso.
    sostenido_s: f32,
    /// Suma y cuenta del reposo, para el promedio de los dos ejes.
    suma_reposo: [f64; 2],
    muestras_reposo: u32,
    centro: [f64; 2],
    /// Mejor alcance del paso en curso, en cm de COP con signo.
    mejor_cm: f64,
    /// Extremos ya confirmados: [izq, der, atrás, adelante].
    extremos_cm: [f64; 4],
}

impl Default for Calibracion {
    fn default() -> Self {
        Self {
            indice: 0,
            tiempo_s: 0.0,
            sostenido_s: 0.0,
            suma_reposo: [0.0; 2],
            muestras_reposo: 0,
            centro: [0.0; 2],
            mejor_cm: 0.0,
            extremos_cm: [0.0; 4],
        }
    }
}

impl Calibracion {
    /// Paso en curso, o `None` si ya terminó.
    pub fn paso(&self) -> Option<Paso> {
        PASOS.get(self.indice).copied()
    }

    pub fn termino(&self) -> bool {
        self.indice >= PASOS.len()
    }

    /// Número del paso en curso y total, para el "2 de 5" de la pantalla.
    pub fn numero(&self) -> (usize, usize) {
        (self.indice.min(PASOS.len() - 1) + 1, PASOS.len())
    }

    /// Cuánto falta del paso actual, de 0.0 a 1.0. En el reposo es el tiempo
    /// corrido; en los alcances, cuánto lleva sosteniendo.
    pub fn progreso(&self) -> f32 {
        match self.paso() {
            Some(Paso::Reposo) => (self.tiempo_s / REPOSO_S).clamp(0.0, 1.0),
            Some(_) => (self.sostenido_s / SOSTENER_S).clamp(0.0, 1.0),
            None => 1.0,
        }
    }

    /// Alcance que lleva medido en el paso actual, en cm desde el centro de
    /// reposo. Sirve para mostrarle al paciente hasta dónde llegó.
    pub fn alcance_actual_cm(&self) -> f64 {
        let Some(paso) = self.paso() else { return 0.0 };
        (self.mejor_cm - self.centro_de(paso)) * paso.signo()
    }

    fn centro_de(&self, paso: Paso) -> f64 {
        if paso.es_ml() { self.centro[0] } else { self.centro[1] }
    }

    /// Le pasa una muestra más. `dt` en segundos, COP en cm.
    pub fn avanzar(&mut self, dt: f32, cop_ml: f64, cop_ap: f64) {
        let Some(paso) = self.paso() else { return };
        let dt = dt.clamp(0.0, 0.1); // un frame largo no vale por medio segundo
        self.tiempo_s += dt;

        if paso == Paso::Reposo {
            // El primer segundo se tira: recién subido todavía se está acomodando.
            if self.tiempo_s > REPOSO_DESCARTE_S {
                self.suma_reposo[0] += cop_ml;
                self.suma_reposo[1] += cop_ap;
                self.muestras_reposo += 1;
            }
            if self.tiempo_s >= REPOSO_S {
                if self.muestras_reposo > 0 {
                    let n = f64::from(self.muestras_reposo);
                    self.centro = [self.suma_reposo[0] / n, self.suma_reposo[1] / n];
                }
                self.siguiente();
            }
            return;
        }

        let valor = if paso.es_ml() { cop_ml } else { cop_ap };
        // Se trabaja con la proyección sobre la dirección pedida, así los
        // cuatro pasos comparten la misma cuenta aunque apunten a lados
        // opuestos: "más lejos" siempre es un número más grande.
        let proyeccion = (valor - self.centro_de(paso)) * paso.signo();
        let mejor = (self.mejor_cm - self.centro_de(paso)) * paso.signo();
        if proyeccion > mejor {
            self.mejor_cm = valor;
            self.sostenido_s = 0.0; // llegó más lejos: el sostén empieza de nuevo
        } else if proyeccion >= mejor - TOLERANCIA_CM && mejor >= MEDIO_ALCANCE_MINIMO_CM {
            // El sostén solo corre una vez que llegó a un alcance utilizable:
            // si no, quedarse quieto cerraría el paso en medio segundo sin
            // haber medido nada.
            self.sostenido_s += dt;
        }

        // Se cierra el paso al sostener el alcance, o al agotar el tiempo: si
        // no pudo sostener nada, insistir no va a mejorar la medida.
        if self.sostenido_s >= SOSTENER_S || self.tiempo_s >= PASO_MAXIMO_S {
            self.extremos_cm[self.indice - 1] = self.mejor_cm;
            self.siguiente();
        }
    }

    fn siguiente(&mut self) {
        self.indice += 1;
        self.tiempo_s = 0.0;
        self.sostenido_s = 0.0;
        // El nuevo paso arranca sin alcance: el centro del eje que le toca.
        self.mejor_cm = self.paso().map(|p| self.centro_de(p)).unwrap_or(0.0);
    }

    /// Rango medido, una vez terminada. `None` mientras siga en curso, y
    /// también si lo medido no llega al mínimo utilizable: en ese caso el
    /// juego se queda con el rango por defecto.
    pub fn resultado(&self, ancho_cm: f64, prof_cm: f64) -> Option<RangoCalibrado> {
        if !self.termino() {
            return None;
        }
        let [izq, der, atras, adelante] = self.extremos_cm;
        RangoCalibrado::medido(
            Eje::nuevo(self.centro[0], izq, der, ancho_cm / 2.0),
            Eje::nuevo(self.centro[1], atras, adelante, prof_cm / 2.0),
            Origen::Juego,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plataforma de referencia: 40x40 cm, o sea semiejes de 20 cm.
    const LIMITE: f64 = 20.0;

    /// Corre la calibración entera con un COP que obedece: se queda en
    /// `centro` y en cada paso de alcance se va al valor pedido.
    fn calibrar(centro: [f64; 2], alcances: [f64; 4]) -> Calibracion {
        let mut cal = Calibracion::default();
        let mut i = 0;
        while !cal.termino() {
            let (ml, ap) = match cal.paso() {
                Some(Paso::Reposo) | None => (centro[0], centro[1]),
                Some(Paso::Izquierda) => (alcances[0], centro[1]),
                Some(Paso::Derecha) => (alcances[1], centro[1]),
                Some(Paso::Atras) => (centro[0], alcances[2]),
                Some(Paso::Adelante) => (centro[0], alcances[3]),
            };
            cal.avanzar(0.05, ml, ap);
            i += 1;
            assert!(i < 10_000, "la calibración no termina");
        }
        cal
    }

    #[test]
    fn la_calibracion_recorre_los_cinco_pasos_y_mide_los_cuatro_alcances() {
        let cal = calibrar([0.0, 0.0], [-5.0, 6.0, -3.0, 7.0]);
        let rango = cal.resultado(40.0, 40.0).expect("calibración utilizable");

        assert_eq!(rango.origen, Origen::Juego);
        assert!((rango.ml.min_cm + 5.0).abs() < 1e-9);
        assert!((rango.ml.max_cm - 6.0).abs() < 1e-9);
        assert!((rango.ap.min_cm + 3.0).abs() < 1e-9);
        assert!((rango.ap.max_cm - 7.0).abs() < 1e-9);
        assert_eq!(cal.numero().1, 5);
    }

    #[test]
    fn el_reposo_queda_como_centro_aunque_este_corrido() {
        let cal = calibrar([3.0, -1.0], [-1.0, 8.0, -4.0, 5.0]);
        let rango = cal.resultado(40.0, 40.0).expect("calibración utilizable");

        assert!((rango.ml.centro_cm - 3.0).abs() < 1e-9, "el centro es su reposo, no el cero de la plataforma");
        assert!((rango.ap.centro_cm + 1.0).abs() < 1e-9);
        assert!((rango.ml.alcance_positivo_cm() - 5.0).abs() < 1e-9);
        assert!((rango.ml.alcance_negativo_cm() - 4.0).abs() < 1e-9);
    }

    #[test]
    fn quedarse_quieto_no_cierra_los_pasos_de_alcance() {
        // Alguien que no se mueve nada: los pasos tienen que agotar su tiempo,
        // no darse por buenos en cuanto "sostiene" el cero.
        let mut cal = Calibracion::default();
        let mut tiempo = 0.0_f32;
        while !cal.termino() {
            cal.avanzar(0.05, 0.0, 0.0);
            tiempo += 0.05;
        }
        assert!(tiempo > REPOSO_S + 4.0 * PASO_MAXIMO_S - 1.0, "cerró demasiado rápido: {tiempo} s");
        assert!(cal.resultado(40.0, 40.0).is_none(), "sin alcance no hay calibración válida");
    }

    #[test]
    fn un_pico_suelto_no_cuenta_como_alcance() {
        // Toca 6 cm un frame y se vuelve al centro: no lo sostuvo, así que el
        // paso no se cierra por sostén y hay que esperar el tiempo máximo.
        let mut cal = Calibracion::default();
        for _ in 0..(REPOSO_S / 0.05) as usize + 1 {
            cal.avanzar(0.05, 0.0, 0.0);
        }
        assert_eq!(cal.paso(), Some(Paso::Izquierda));

        cal.avanzar(0.05, -6.0, 0.0);
        for _ in 0..20 {
            cal.avanzar(0.05, 0.0, 0.0);
        }
        assert_eq!(cal.paso(), Some(Paso::Izquierda), "un pico no alcanza para cerrar el paso");
        assert!(cal.progreso() < 1.0);
    }

    #[test]
    fn el_paso_se_cierra_al_sostener_el_alcance() {
        let mut cal = Calibracion::default();
        for _ in 0..(REPOSO_S / 0.05) as usize + 1 {
            cal.avanzar(0.05, 0.0, 0.0);
        }
        for _ in 0..(SOSTENER_S / 0.05) as usize + 1 {
            cal.avanzar(0.05, -6.0, 0.0);
        }
        assert_eq!(cal.paso(), Some(Paso::Derecha), "sostenido medio segundo, pasa al otro lado");
    }

    #[test]
    fn el_rango_por_defecto_no_usa_el_semieje_de_la_plataforma() {
        let rango = RangoCalibrado::por_defecto(40.0, 40.0);
        assert_eq!(rango.ml.max_cm, DEFECTO_ML_CM, "el alcance no es el borde de la plataforma");
        assert_eq!(rango.ml.min_cm, -DEFECTO_ML_CM);
        assert_eq!(rango.ap.max_cm, DEFECTO_ANTERIOR_CM);
        assert_eq!(rango.ap.min_cm, -DEFECTO_POSTERIOR_CM, "atrás se llega mucho menos que adelante");
        assert_eq!(rango.origen, Origen::Defecto, "sin calibrar, no es un dato de la persona");
    }

    #[test]
    fn la_plataforma_chica_recorta_el_rango_por_defecto() {
        let rango = RangoCalibrado::por_defecto(10.0, 10.0);
        assert_eq!(rango.ml.max_cm, 5.0, "no puede prometer alcance fuera de la plataforma");
        assert_eq!(rango.ap.max_cm, 5.0);
        assert_eq!(rango.ap.min_cm, -4.0, "el posterior ya cabía, queda igual");
    }

    #[test]
    fn el_rango_por_defecto_nunca_queda_degenerado() {
        // Geometría absurda pero aceptada por la configuración (1 cm de lado).
        let rango = RangoCalibrado::por_defecto(1.0, 1.0);
        assert!(rango.ml.alcance_positivo_cm() >= MEDIO_ALCANCE_MINIMO_CM);
        assert!(rango.ap.alcance_negativo_cm() >= MEDIO_ALCANCE_MINIMO_CM);
    }

    #[test]
    fn el_centro_corrido_deja_al_paciente_en_el_medio_de_la_pista() {
        // Apoya más en la derecha: reposo en +3 cm, llega a +8 y solo a -1.
        let eje = Eje::nuevo(3.0, -1.0, 8.0, LIMITE).expect("rango utilizable");

        assert_eq!(eje.normalizar(3.0, 1.0), 0.0, "su reposo es el centro de la pista");
        assert_eq!(eje.normalizar(8.0, 1.0), 1.0, "su alcance derecho es el borde derecho");
        assert_eq!(eje.normalizar(-1.0, 1.0), -1.0, "su alcance izquierdo es el borde izquierdo");
    }

    #[test]
    fn la_exigencia_decide_cuanto_hay_que_desplazarse() {
        let eje = Eje::nuevo(0.0, -6.0, 6.0, LIMITE).expect("rango utilizable");

        // Con exigencia 0.7 el borde de la pista está a 4.2 cm, no a 6.
        assert!((eje.normalizar(4.2, 0.7) - 1.0).abs() < 1e-9);
        assert!(eje.normalizar(4.2, 0.9) < 1.0, "más exigencia, hay que ir más lejos");
        assert_eq!(eje.normalizar(4.2, 0.5), 1.0, "menos exigencia, se llega antes");
    }

    #[test]
    fn la_posicion_normalizada_no_se_sale_de_la_pista() {
        let eje = Eje::nuevo(0.0, -6.0, 6.0, LIMITE).expect("rango utilizable");
        assert_eq!(eje.normalizar(100.0, EXIGENCIA_DEFECTO), 1.0);
        assert_eq!(eje.normalizar(-100.0, EXIGENCIA_DEFECTO), -1.0);
    }

    #[test]
    fn la_exigencia_fuera_de_rango_se_acota() {
        let eje = Eje::nuevo(0.0, -6.0, 6.0, LIMITE).expect("rango utilizable");
        assert_eq!(eje.normalizar(2.4, 0.0), eje.normalizar(2.4, EXIGENCIA_MIN));
        assert_eq!(eje.normalizar(2.4, 5.0), eje.normalizar(2.4, EXIGENCIA_MAX));
    }

    #[test]
    fn un_alcance_demasiado_chico_invalida_la_calibracion() {
        // Medio centímetro hacia la izquierda: el ruido del HX711 bastaría
        // para cruzar la pista entera.
        assert!(Eje::nuevo(0.0, -0.5, 6.0, LIMITE).is_none());
        assert!(Eje::nuevo(0.0, -6.0, 0.5, LIMITE).is_none());
        assert!(Eje::nuevo(0.0, 0.0, 0.0, LIMITE).is_none(), "un rango nulo no puede dividir");
    }

    #[test]
    fn la_calibracion_no_puede_exceder_la_plataforma() {
        // Alcance imposible: más allá del borde físico.
        let eje = Eje::nuevo(0.0, -50.0, 50.0, LIMITE).expect("rango utilizable");
        assert_eq!(eje.max_cm, LIMITE);
        assert_eq!(eje.min_cm, -LIMITE);
    }

    #[test]
    fn un_eje_invalido_descarta_la_calibracion_entera() {
        let ml = Eje::nuevo(0.0, -6.0, 6.0, LIMITE);
        let ap = Eje::nuevo(0.0, -0.2, 0.2, LIMITE);
        assert!(ap.is_none());
        assert!(RangoCalibrado::medido(ml, ap, Origen::Juego).is_none());
        assert!(RangoCalibrado::medido(ml, ml, Origen::Juego).is_some());
    }

    #[test]
    fn los_dos_alcances_se_miden_desde_el_centro() {
        let eje = Eje::nuevo(2.0, -1.0, 7.0, LIMITE).expect("rango utilizable");
        assert_eq!(eje.alcance_positivo_cm(), 5.0);
        assert_eq!(eje.alcance_negativo_cm(), 3.0);
    }
}
