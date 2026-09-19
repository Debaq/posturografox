//! Historial local de sesiones.
//!
//! Hasta ahora cada ensayo terminaba en un CSV suelto y no quedaba forma de
//! ver la evolución de una persona sin abrirlos a mano uno por uno. Acá se
//! guarda una línea por sesión, con sus métricas, para poder listarlas y
//! graficar cómo cambia el balance entre controles.
//!
//! El formato es una línea RON por sesión (append): sobrevive a que el
//! programa se cierre de golpe, no exige reescribir el archivo entero y una
//! línea corrupta no se lleva puesto el resto del historial.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::datos::carpeta_datos;
use crate::estabilometria::{Condicion, MetricasBalance, Superficie};
use crate::maniobras::Resumen;
use crate::rango::RangoCalibrado;

const ARCHIVO: &str = "historial.ronl";

/// Lo que deja una partida del modo juego, además de las métricas comunes.
///
/// Va junto a la sesión y no en lugar de ella porque durante la partida se
/// graba el mismo COP que en cualquier ensayo; lo que cambia es que además
/// hubo estímulos con dirección y momento conocidos.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DatosJuego {
    /// Alcance con el que se jugó, y de dónde salió. Sin esto el resto de las
    /// cifras no se puede interpretar.
    pub rango: RangoCalibrado,
    /// Fracción del alcance que había que cubrir para llegar al borde de la
    /// pista: la dosis del ejercicio.
    pub exigencia: f64,
    pub duracion_s: f64,
    pub gano: bool,
    /// Medianas por lado, asimetría y control direccional. `None` si no quedó
    /// ninguna maniobra medible.
    pub resumen: Option<Resumen>,
}

/// Una sesión cerrada, tal como queda archivada.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Sesion {
    /// Momento en que se cerró, en segundos desde epoch.
    pub epoch_s: u64,
    pub paciente: String,
    pub superficie: Superficie,
    pub condicion: Condicion,
    pub metricas: MetricasBalance,
    /// Presente solo si la sesión fue una partida del modo juego. Las
    /// métricas de bipedestación quieta de una partida no son comparables con
    /// las de un ensayo estático —el área 95% de una partida mide cuánto se
    /// movió jugando, no cuánto oscila—, así que las sesiones de juego se
    /// listan aparte y no entran en la evolución.
    ///
    /// `serde(default)` para que un historial escrito antes de que esto
    /// existiera siga cargando.
    #[serde(default)]
    pub juego: Option<DatosJuego>,
}

impl Sesion {
    pub fn nueva(paciente: &str, superficie: Superficie, condicion: Condicion, metricas: MetricasBalance) -> Self {
        Self {
            epoch_s: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
            paciente: paciente.trim().to_string(),
            superficie,
            condicion,
            metricas,
            juego: None,
        }
    }

    /// Una partida del modo juego archivada.
    pub fn de_juego(paciente: &str, superficie: Superficie, metricas: MetricasBalance, juego: DatosJuego) -> Self {
        Self { juego: Some(juego), ..Self::nueva(paciente, superficie, Condicion::OjosAbiertos, metricas) }
    }

    pub fn es_juego(&self) -> bool {
        self.juego.is_some()
    }

    /// Etiqueta corta para listarla.
    pub fn etiqueta(&self) -> String {
        if self.es_juego() {
            return format!("{} + juego", self.superficie.etiqueta());
        }
        format!("{} + {}", self.superficie.etiqueta(), self.condicion.etiqueta())
    }
}

pub fn ruta_historial() -> PathBuf {
    carpeta_datos().join(ARCHIVO)
}

/// Agrega la sesión al historial del usuario.
pub fn agregar(sesion: &Sesion) -> io::Result<()> {
    agregar_en(&ruta_historial(), sesion)
}

pub fn agregar_en(ruta: &Path, sesion: &Sesion) -> io::Result<()> {
    if let Some(carpeta) = ruta.parent() {
        fs::create_dir_all(carpeta)?;
    }
    let linea = ron::ser::to_string(sesion).map_err(io::Error::other)?;
    // Si la última escritura quedó cortada sin salto de línea (corte de luz,
    // disco lleno), lo agregamos antes: de lo contrario esta sesión se
    // pegaría a esa línea rota y también se perdería al leer.
    let termina_en_salto = fs::read_to_string(ruta).map(|c| c.is_empty() || c.ends_with('\n')).unwrap_or(true);
    let mut archivo = OpenOptions::new().create(true).append(true).open(ruta)?;
    if !termina_en_salto {
        writeln!(archivo)?;
    }
    writeln!(archivo, "{linea}")
}

/// Lee el historial completo, de la más vieja a la más nueva. Las líneas que
/// no se puedan leer se saltean: una escritura cortada a la mitad no puede
/// dejar inaccesible todo lo anterior.
pub fn cargar() -> Vec<Sesion> {
    cargar_de(&ruta_historial())
}

pub fn cargar_de(ruta: &Path) -> Vec<Sesion> {
    let Ok(contenido) = fs::read_to_string(ruta) else { return Vec::new() };
    contenido.lines().filter_map(|linea| ron::from_str::<Sesion>(linea).ok()).collect()
}

/// Sesiones de una persona, de la más nueva a la más vieja. La comparación
/// ignora mayúsculas y espacios de más para que "ID-42" y "id-42 " no queden
/// como dos pacientes distintos.
pub fn de_paciente<'a>(historial: &'a [Sesion], paciente: &str) -> Vec<&'a Sesion> {
    let buscado = paciente.trim().to_lowercase();
    let mut encontradas: Vec<&Sesion> =
        historial.iter().filter(|s| s.paciente.trim().to_lowercase() == buscado).collect();
    encontradas.sort_by_key(|s| std::cmp::Reverse(s.epoch_s));
    encontradas
}

/// Lista de pacientes distintos que aparecen en el historial.
pub fn pacientes(historial: &[Sesion]) -> Vec<String> {
    let mut vistos: Vec<String> = Vec::new();
    for sesion in historial {
        if !vistos.iter().any(|p| p.trim().to_lowercase() == sesion.paciente.trim().to_lowercase()) {
            vistos.push(sesion.paciente.clone());
        }
    }
    vistos.sort();
    vistos
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sesion_de(paciente: &str, epoch_s: u64, area: f64) -> Sesion {
        Sesion {
            epoch_s,
            paciente: paciente.to_string(),
            superficie: Superficie::Firme,
            condicion: Condicion::OjosAbiertos,
            metricas: MetricasBalance { area95_cm2: area, ..MetricasBalance::default() },
            juego: None,
        }
    }

    #[test]
    fn un_historial_viejo_sigue_cargando_sin_el_campo_del_juego() {
        // Línea escrita antes de que existiera el modo juego en el archivo.
        let linea = "(epoch_s: 1, paciente: \"ID-1\", superficie: Firme, condicion: OjosAbiertos, \
                     metricas: (longitud_cm: 1.0, area95_cm2: 2.0, velocidad_media_cms: 0.1, duracion_s: 30.0, \
                     rms_ml_cm: 0.1, rms_ap_cm: 0.1, rango_ml_cm: 1.0, rango_ap_cm: 1.0, velocidad_ml_cms: 0.1, \
                     velocidad_ap_cms: 0.1, frec_mediana_ml_hz: 0.5, frec_mediana_ap_hz: 0.5, f80_ml_hz: 1.0, \
                     f80_ap_hz: 1.0))";
        let sesion: Sesion = ron::from_str(linea).expect("deserializar una sesión vieja");
        assert!(!sesion.es_juego());
    }

    #[test]
    fn la_partida_se_archiva_marcada_como_juego() {
        let juego = DatosJuego {
            rango: RangoCalibrado::por_defecto(40.0, 40.0),
            exigencia: 0.7,
            duracion_s: 90.0,
            gano: false,
            resumen: None,
        };
        let sesion = Sesion::de_juego("ID-1", Superficie::Firme, MetricasBalance::default(), juego);

        assert!(sesion.es_juego());
        assert!(sesion.etiqueta().contains("juego"));
        let texto = ron::ser::to_string(&sesion).expect("serializar");
        let vuelta: Sesion = ron::from_str(&texto).expect("deserializar");
        assert_eq!(sesion, vuelta);
    }

    #[test]
    fn lo_guardado_se_recupera_igual() {
        let carpeta = tempfile::tempdir().unwrap();
        let ruta = carpeta.path().join("historial.ronl");
        let sesion = sesion_de("ID-1", 1000, 3.5);

        agregar_en(&ruta, &sesion).unwrap();
        let recuperadas = cargar_de(&ruta);

        assert_eq!(recuperadas, vec![sesion]);
    }

    #[test]
    fn las_sesiones_se_van_agregando_sin_pisar_las_anteriores() {
        let carpeta = tempfile::tempdir().unwrap();
        let ruta = carpeta.path().join("historial.ronl");
        for i in 0..5 {
            agregar_en(&ruta, &sesion_de("ID-1", 1000 + i, i as f64)).unwrap();
        }
        assert_eq!(cargar_de(&ruta).len(), 5);
    }

    #[test]
    fn una_linea_corrupta_no_se_lleva_puesto_el_resto() {
        let carpeta = tempfile::tempdir().unwrap();
        let ruta = carpeta.path().join("historial.ronl");
        agregar_en(&ruta, &sesion_de("ID-1", 1, 1.0)).unwrap();
        // Escritura cortada a la mitad, como si se hubiera ido la luz.
        fs::write(&ruta, format!("{}\n{{esto no es RON", fs::read_to_string(&ruta).unwrap().trim())).unwrap();
        agregar_en(&ruta, &sesion_de("ID-1", 2, 2.0)).unwrap();

        let recuperadas = cargar_de(&ruta);
        assert_eq!(recuperadas.len(), 2, "las dos sesiones buenas tienen que seguir estando");
    }

    #[test]
    fn un_historial_que_no_existe_es_una_lista_vacia() {
        assert!(cargar_de(Path::new("/no/existe/historial.ronl")).is_empty());
    }

    #[test]
    fn las_sesiones_de_un_paciente_salen_de_la_mas_nueva_a_la_mas_vieja() {
        let historial = vec![sesion_de("ID-1", 100, 1.0), sesion_de("ID-2", 200, 2.0), sesion_de(" id-1 ", 300, 3.0)];
        let suyas = de_paciente(&historial, "ID-1");
        assert_eq!(suyas.len(), 2, "la comparación debe ignorar mayúsculas y espacios");
        assert_eq!(suyas[0].epoch_s, 300);
        assert_eq!(suyas[1].epoch_s, 100);
    }

    #[test]
    fn la_lista_de_pacientes_no_repite() {
        let historial = vec![sesion_de("ID-1", 1, 1.0), sesion_de("id-1", 2, 1.0), sesion_de("ID-2", 3, 1.0)];
        assert_eq!(pacientes(&historial), vec!["ID-1".to_string(), "ID-2".to_string()]);
    }
}
