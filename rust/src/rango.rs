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
    // Lo consumen las dos fuentes de calibración: el botón del juego (R38) y
    // el ejercicio de límites (R39).
    #[allow(dead_code)]
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
    // Lo consumen las dos fuentes de calibración (R38 y R39).
    #[allow(dead_code)]
    pub fn medido(ml: Option<Eje>, ap: Option<Eje>, origen: Origen) -> Option<Self> {
        Some(Self { ml: ml?, ap: ap?, origen })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plataforma de referencia: 40x40 cm, o sea semiejes de 20 cm.
    const LIMITE: f64 = 20.0;

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
