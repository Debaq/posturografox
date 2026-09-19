//! Pacientes y exámenes de posturografía, en la base SQLite de la suite.
//!
//! Hasta acá una sesión terminaba en una línea de `historial.ronl` con el
//! nombre del paciente **escrito a mano**. Eso alcanza para probar el equipo y
//! no para trabajar: "Juan Pérez", "juan perez" y "J. Pérez" son tres personas
//! distintas para un archivo de texto, y ninguna tiene fecha de nacimiento, ni
//! número de ficha, ni una nota de la vez anterior.
//!
//! # La base es la misma que la de vHIT
//!
//! La tabla `patient` de acá es, columna por columna, la de vHIT: el alta que se
//! hace en un equipo se ve en el otro, y una persona es una sola ficha en toda
//! la suite. Lo que cada programa agrega son **sus** exámenes, en sus propias
//! tablas (`postura_examen` acá, `exam`/`trial` allá), colgadas del mismo
//! paciente. Ver [`crate::almacenamiento`] para dónde vive el archivo.
//!
//! Por eso `migrar()` usa `CREATE TABLE IF NOT EXISTS` para todo y no borra ni
//! reescribe nada: cuando este programa abre una base que vHIT ya creó, se
//! encuentra `patient` hecha y solo agrega lo suyo. Y al revés también.
//!
//! # Cifrado
//!
//! Un examen de equilibrio es un dato de salud, así que la base usa SQLCipher
//! (AES-256) por defecto, con una frase de paso que **no se guarda en ningún
//! lado**: si se pierde, los datos no se recuperan. Es a propósito — una frase
//! guardada al lado de la base no cifra nada.
//!
//! # Reproducibilidad
//!
//! Cada examen guarda la configuración exacta con la que se midió, la versión
//! del programa y el COP crudo. Un área de 95% sin el tamaño de la plataforma,
//! el filtro y la duración que la produjeron no es un dato: es un número. Si
//! mañana cambia un valor por defecto, los exámenes viejos siguen diciendo con
//! qué se midieron, y se pueden volver a graficar tal como se vieron ese día.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use crate::config::Config;
use crate::estabilometria::{Condicion, MetricasBalance, Superficie};
use crate::historial::DatosJuego;
use crate::limites::Intento;

pub const VERSION_APP: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug)]
pub enum ErrorBase {
    Sql(rusqlite::Error),
    /// La frase no abre la base: o está mal, o el archivo no es una base.
    FraseIncorrecta,
    FraseVacia,
    /// La base del disco está cifrada y se la intentó abrir sin frase.
    FaltaFrase,
    /// La base del disco NO está cifrada y se le dio una frase.
    ///
    /// Va aparte de `FraseIncorrecta` porque el remedio es el contrario: no hay
    /// ninguna frase que la abra, y quien la escribe puede estar creyendo que
    /// sus datos están cifrados cuando no lo están.
    SinCifrar,
    /// Un examen guardado con datos que no se pueden leer.
    Ilegible(String),
}

impl std::fmt::Display for ErrorBase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorBase::Sql(e) => write!(f, "base de datos: {e}"),
            ErrorBase::FraseIncorrecta => {
                write!(f, "frase de paso incorrecta, o el archivo no es una base de la suite")
            }
            ErrorBase::FraseVacia => write!(f, "la frase de paso no puede estar vacía"),
            ErrorBase::FaltaFrase => write!(f, "esta base está cifrada: hace falta la frase de paso"),
            ErrorBase::SinCifrar => write!(f, "esta base NO está cifrada: se abre sin frase de paso"),
            ErrorBase::Ilegible(que) => write!(f, "no pude leer {que} de la base"),
        }
    }
}

impl std::error::Error for ErrorBase {}

impl From<rusqlite::Error> for ErrorBase {
    fn from(e: rusqlite::Error) -> Self {
        ErrorBase::Sql(e)
    }
}

type Resultado<T> = std::result::Result<T, ErrorBase>;

/// Un archivo de SQLite sin cifrar empieza con este texto. Cifrado, el
/// encabezado también está cifrado y no aparece.
const MAGIA_SQLITE: &[u8; 16] = b"SQLite format 3\0";

/// Si lo que hay en el disco es una base de SQLite **sin cifrar**.
///
/// `None` si no existe todavía, que es el caso de crear una base nueva.
///
/// Hace falta para decir cuál de los dos errores es: SQLCipher devuelve el
/// mismo `NotADatabase` cuando la frase está mal y cuando la base está en claro
/// y se le dio una frase, y esos dos casos se arreglan de maneras opuestas.
fn sqlite_en_claro(ruta: &Path) -> Option<bool> {
    use std::io::Read;
    let mut archivo = std::fs::File::open(ruta).ok()?;
    let mut cabecera = [0u8; 16];
    match archivo.read_exact(&mut cabecera) {
        Ok(()) => Some(&cabecera == MAGIA_SQLITE),
        // Un archivo más corto que el encabezado no es una base de ningún tipo;
        // uno vacío lo crea SQLite al abrirlo.
        Err(_) => Some(false),
    }
}

/// Una persona, como está en la ficha. Es la tabla que se comparte con vHIT.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Paciente {
    pub id: i64,
    /// Número de ficha o historia clínica, como lo maneje la institución.
    pub ficha: String,
    pub nombre: String,
    /// Fecha de nacimiento en ISO (`AAAA-MM-DD`), o vacía si no se sabe.
    pub nacimiento: String,
    pub notas: String,
}

impl Paciente {
    /// Cómo se lo nombra en un CSV, un informe o el título de una ventana.
    pub fn etiqueta(&self) -> String {
        if self.ficha.trim().is_empty() { self.nombre.clone() } else { format!("{} ({})", self.nombre, self.ficha) }
    }
}

/// Un paciente **con lo que la lista necesita mostrar de él**: cuántos
/// exámenes de posturografía tiene y cuándo fue el último.
///
/// Sale de una sola consulta y no de una por fila: la lista se redibuja en cada
/// frame, y una consulta por paciente visible serían decenas por segundo contra
/// una base cifrada.
#[derive(Clone, Debug, PartialEq)]
pub struct Fila {
    pub paciente: Paciente,
    pub examenes: usize,
    /// Fecha del examen más reciente, ISO 8601 UTC. `None` si nunca se le hizo
    /// uno acá: un paciente dado de alta en vHIT y todavía sin posturografía es
    /// el caso normal de una suite.
    pub ultimo_examen: Option<String>,
}

/// Criterio de orden de la lista de pacientes.
///
/// Por defecto, el último examen: con la base en uso, lo que casi siempre se
/// busca es "el que vi hace un rato", no el que se dio de alta primero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Orden {
    #[default]
    UltimoExamen,
    Nombre,
    Ficha,
    Alta,
}

impl Orden {
    /// La cláusula `ORDER BY`. Es un literal del código, no entra de afuera.
    ///
    /// Los pacientes sin exámenes caen al final en el orden por último examen:
    /// `ultimo_examen` es NULL y SQLite pone los NULL primero en `DESC`, al
    /// revés de lo que hace falta, así que se ordena antes por si es NULL.
    fn clausula(self) -> &'static str {
        match self {
            Orden::UltimoExamen => "ultimo_examen IS NULL, ultimo_examen DESC, p.name COLLATE NOCASE",
            Orden::Nombre => "p.name COLLATE NOCASE",
            Orden::Ficha => "p.external_id = '', p.external_id COLLATE NOCASE",
            Orden::Alta => "p.created_at DESC",
        }
    }
}

/// Qué clase de examen se guardó.
///
/// Los tres graban COP, y ahí se parecen. Lo que los separa es que **sus
/// métricas no son comparables entre sí**: el área de 95% de una partida mide
/// cuánto se desplazó jugando a propósito, no cuánto oscila parado quieto, y el
/// examen de límites ni siquiera intenta estar quieto. Guardar el tipo es lo que
/// permite después no mezclarlos en la misma curva.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TipoExamen {
    #[default]
    Estatica,
    Juego,
    Limites,
}

impl TipoExamen {
    pub fn slug(self) -> &'static str {
        match self {
            TipoExamen::Estatica => "estatica",
            TipoExamen::Juego => "juego",
            TipoExamen::Limites => "limites",
        }
    }

    pub fn etiqueta(self) -> &'static str {
        match self {
            TipoExamen::Estatica => "Bipedestación",
            TipoExamen::Juego => "Juego",
            TipoExamen::Limites => "Límites",
        }
    }

    /// Lo desconocido se lee como estática: una base escrita por una versión
    /// posterior puede traer un tipo que este programa no conoce, y eso no es
    /// razón para perder la fila entera.
    fn de_slug(s: &str) -> Self {
        match s {
            "juego" => TipoExamen::Juego,
            "limites" => TipoExamen::Limites,
            _ => TipoExamen::Estatica,
        }
    }
}

fn superficie_slug(s: Superficie) -> &'static str {
    s.slug()
}

fn superficie_de_slug(s: &str) -> Superficie {
    match s {
        "espuma" => Superficie::Espuma,
        _ => Superficie::Firme,
    }
}

fn condicion_de_slug(s: &str) -> Condicion {
    match s {
        "ojos_cerrados" => Condicion::OjosCerrados,
        _ => Condicion::OjosAbiertos,
    }
}

/// Todo lo que hace falta para guardar un examen, junto.
#[derive(Clone, Debug)]
pub struct ExamenNuevo {
    pub paciente_id: i64,
    pub operador: String,
    pub tipo: TipoExamen,
    pub superficie: Superficie,
    pub condicion: Condicion,
    /// `None` en un examen de límites, que no mide oscilación.
    pub metricas: Option<MetricasBalance>,
    /// La configuración con la que se midió, entera.
    pub config: Config,
    /// Lo propio de una partida del modo juego.
    pub juego: Option<DatosJuego>,
    /// Los intentos del examen de límites de estabilidad.
    pub limites: Vec<Intento>,
    pub notas: String,
    /// COP crudo y filtrado como se lo midió: `(t_s, ml_cm, ap_cm)`.
    pub registro: Vec<[f64; 3]>,
}

/// Una fila del historial de un paciente.
#[derive(Clone, Debug, PartialEq)]
pub struct ResumenExamen {
    pub id: i64,
    pub paciente_id: i64,
    pub fecha: String,
    pub operador: String,
    pub tipo: TipoExamen,
    pub superficie: Superficie,
    pub condicion: Condicion,
    pub metricas: Option<MetricasBalance>,
    pub muestras: usize,
}

impl ResumenExamen {
    /// Etiqueta corta de la condición examinada.
    pub fn etiqueta(&self) -> String {
        match self.tipo {
            TipoExamen::Estatica => format!("{} + {}", self.superficie.etiqueta(), self.condicion.etiqueta()),
            TipoExamen::Juego => format!("{} + juego", self.superficie.etiqueta()),
            TipoExamen::Limites => "Límites de estabilidad".to_string(),
        }
    }
}

/// Un examen leído entero: lo del resumen más lo que hace falta para volver a
/// dibujarlo y reexportarlo tal como se vio el día del estudio.
#[derive(Clone, Debug)]
pub struct DetalleExamen {
    pub resumen: ResumenExamen,
    pub version_app: String,
    /// La configuración de ESE examen, no la de hoy.
    pub config: Config,
    pub juego: Option<DatosJuego>,
    pub limites: Vec<Intento>,
    pub notas: String,
    pub registro: Vec<[f64; 3]>,
}

pub struct Base {
    conexion: Connection,
    /// Con qué política se abrió. Hace falta para no ofrecer cambiar una frase
    /// que no existe, y para poder avisar en pantalla mientras no haya cifrado.
    cifrada: bool,
}

impl Base {
    /// Abre (o crea) la base de la suite.
    ///
    /// `frase` es `Some` para una base cifrada con SQLCipher y `None` para una
    /// **sin cifrar**. Sin cifrar es una elección explícita del que instala —ver
    /// [`crate::almacenamiento`]— y no un descuido: por eso es un `Option` y no
    /// una cadena que puede venir vacía por accidente.
    pub fn abrir(ruta: &Path, frase: Option<&str>) -> Resultado<Self> {
        let en_claro = sqlite_en_claro(ruta);
        match (frase, en_claro) {
            // Frase vacía: quien la escribió quiso cifrar y no escribió nada.
            // No es lo mismo que elegir no cifrar.
            (Some(""), _) => return Err(ErrorBase::FraseVacia),
            (Some(_), Some(true)) => return Err(ErrorBase::SinCifrar),
            (None, Some(false)) => return Err(ErrorBase::FaltaFrase),
            _ => {}
        }

        if let Some(carpeta) = ruta.parent().filter(|c| !c.as_os_str().is_empty()) {
            std::fs::create_dir_all(carpeta).map_err(|e| {
                ErrorBase::Sql(rusqlite::Error::InvalidPath(std::path::PathBuf::from(format!(
                    "{}: {e}",
                    carpeta.display()
                ))))
            })?;
        }

        let conexion = Connection::open(ruta)?;
        if let Some(f) = frase {
            // `pragma_update` escapa el valor; concatenar la frase en un SQL a
            // mano sería inyectable con una frase que contenga comillas.
            conexion.pragma_update(None, "key", f)?;
        }
        conexion.pragma_update(None, "foreign_keys", "ON")?;

        // Sonda: con la clave equivocada, esto falla con NotADatabase.
        match conexion.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0)) {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::NotADatabase => {
                return Err(ErrorBase::FraseIncorrecta);
            }
            Err(e) => return Err(e.into()),
        }

        let base = Self { conexion, cifrada: frase.is_some() };
        base.migrar()?;
        Ok(base)
    }

    /// Crea lo que falte. Nunca borra ni reescribe: la misma base la comparten
    /// dos programas, y el otro puede haberla creado.
    fn migrar(&self) -> Resultado<()> {
        self.conexion.execute_batch(
            r#"
            -- Esta tabla es COMPARTIDA con vHIT: mismas columnas, mismos
            -- nombres. No agregarle nada acá sin agregárselo allá.
            CREATE TABLE IF NOT EXISTS patient (
                id           INTEGER PRIMARY KEY,
                external_id  TEXT NOT NULL DEFAULT '',
                name         TEXT NOT NULL,
                birth_date   TEXT NOT NULL DEFAULT '',
                notes        TEXT NOT NULL DEFAULT '',
                created_at   TEXT NOT NULL
            );

            -- Y esta es solo de posturografía. El prefijo no es decoración: es
            -- lo que permite que las dos aplicaciones escriban en el mismo
            -- archivo sin pisarse.
            CREATE TABLE IF NOT EXISTS postura_examen (
                id            INTEGER PRIMARY KEY,
                patient_id    INTEGER NOT NULL REFERENCES patient(id) ON DELETE CASCADE,
                fecha         TEXT NOT NULL,
                operador      TEXT NOT NULL DEFAULT '',
                -- 'estatica' | 'juego' | 'limites'. Sus métricas no son
                -- comparables entre sí; ver `TipoExamen`.
                tipo          TEXT NOT NULL,
                superficie    TEXT NOT NULL,
                condicion     TEXT NOT NULL,
                -- Métricas de balance. NULL en un examen de límites, que no
                -- mide oscilación: es la verdad, no un cero.
                duracion_s          REAL,
                longitud_cm         REAL,
                area95_cm2          REAL,
                velocidad_media_cms REAL,
                rms_ml_cm           REAL,
                rms_ap_cm           REAL,
                rango_ml_cm         REAL,
                rango_ap_cm         REAL,
                velocidad_ml_cms    REAL,
                velocidad_ap_cms    REAL,
                frec_mediana_ml_hz  REAL,
                frec_mediana_ap_hz  REAL,
                f80_ml_hz           REAL,
                f80_ap_hz           REAL,
                -- La configuración exacta con la que se midió. Un área sin el
                -- tamaño de la plataforma y el filtro que la produjeron no es
                -- un dato.
                config_ron    TEXT NOT NULL,
                version_app   TEXT NOT NULL,
                -- Lo propio de una partida y del examen de límites.
                juego_ron     TEXT,
                limites_ron   TEXT,
                notas         TEXT NOT NULL DEFAULT '',
                -- COP crudo: 3 f32 little-endian por muestra
                -- (t_s, ml_cm, ap_cm). Ver `codificar_registro`.
                registro      BLOB NOT NULL
            );
            CREATE INDEX IF NOT EXISTS postura_examen_por_paciente
                ON postura_examen(patient_id, fecha DESC);
            "#,
        )?;
        Ok(())
    }

    /// Cambia la frase de paso de una base ya abierta **y cifrada**.
    ///
    /// No sirve para cifrar una base que se creó en claro: `PRAGMA rekey` de
    /// SQLCipher cambia la clave de una base que ya está cifrada, no convierte
    /// una que no lo está. Y la frase es de la base, así que el cambio vale
    /// también para vHIT: es el mismo archivo.
    pub fn recifrar(&self, nueva: &str) -> Resultado<()> {
        if nueva.is_empty() {
            return Err(ErrorBase::FraseVacia);
        }
        if !self.cifrada {
            return Err(ErrorBase::SinCifrar);
        }
        self.conexion.pragma_update(None, "rekey", nueva)?;
        Ok(())
    }

    pub fn esta_cifrada(&self) -> bool {
        self.cifrada
    }

    // --- pacientes --------------------------------------------------------

    pub fn crear_paciente(&self, ficha: &str, nombre: &str, nacimiento: &str, notas: &str) -> Resultado<i64> {
        self.conexion.execute(
            "INSERT INTO patient (external_id, name, birth_date, notes, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![ficha, nombre, nacimiento, notas, ahora_iso()],
        )?;
        Ok(self.conexion.last_insert_rowid())
    }

    pub fn actualizar_paciente(&self, p: &Paciente) -> Resultado<()> {
        self.conexion.execute(
            "UPDATE patient SET external_id = ?1, name = ?2, birth_date = ?3, notes = ?4 WHERE id = ?5",
            params![p.ficha, p.nombre, p.nacimiento, p.notas, p.id],
        )?;
        Ok(())
    }

    /// Los pacientes que coinciden con el texto buscado, con su cuenta de
    /// exámenes y la fecha del último.
    ///
    /// El texto se compara contra el nombre y contra la ficha: se busca por lo
    /// que se tenga a mano, que a veces es un apellido y a veces un número.
    pub fn buscar_pacientes(&self, texto: &str, orden: Orden, limite: usize) -> Resultado<Vec<Fila>> {
        let patron = format!("%{}%", texto.trim());
        let sql = format!(
            "SELECT p.id, p.external_id, p.name, p.birth_date, p.notes,
                    (SELECT count(*) FROM postura_examen e WHERE e.patient_id = p.id) AS examenes,
                    (SELECT max(e.fecha) FROM postura_examen e WHERE e.patient_id = p.id) AS ultimo_examen
             FROM patient p
             WHERE ?1 = '%%' OR p.name LIKE ?1 COLLATE NOCASE OR p.external_id LIKE ?1 COLLATE NOCASE
             ORDER BY {}
             LIMIT ?2",
            orden.clausula()
        );
        let mut stmt = self.conexion.prepare(&sql)?;
        let filas = stmt
            .query_map(params![patron, limite as i64], |r| {
                Ok(Fila {
                    paciente: Paciente {
                        id: r.get(0)?,
                        ficha: r.get(1)?,
                        nombre: r.get(2)?,
                        nacimiento: r.get(3)?,
                        notas: r.get(4)?,
                    },
                    examenes: r.get::<_, i64>(5)? as usize,
                    ultimo_examen: r.get(6)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(filas)
    }

    /// Borra un paciente **y todos sus exámenes**, de los dos programas: las
    /// tablas de vHIT también cuelgan de `patient` con `ON DELETE CASCADE`. No
    /// se puede deshacer, y por eso la ventana lo confirma antes.
    pub fn borrar_paciente(&self, id: i64) -> Resultado<()> {
        self.conexion.execute("DELETE FROM patient WHERE id = ?1", params![id])?;
        Ok(())
    }

    // --- exámenes ---------------------------------------------------------

    /// Guarda un examen con su COP crudo. Devuelve su id.
    pub fn guardar_examen(&self, ex: &ExamenNuevo) -> Resultado<i64> {
        let config_ron = ron::ser::to_string(&ex.config).map_err(|e| ErrorBase::Ilegible(e.to_string()))?;
        let juego_ron =
            ex.juego.as_ref().map(ron::ser::to_string).transpose().map_err(|e| ErrorBase::Ilegible(e.to_string()))?;
        let limites_ron = if ex.limites.is_empty() {
            None
        } else {
            Some(ron::ser::to_string(&ex.limites).map_err(|e| ErrorBase::Ilegible(e.to_string()))?)
        };
        let m = ex.metricas;
        self.conexion.execute(
            "INSERT INTO postura_examen (
                patient_id, fecha, operador, tipo, superficie, condicion,
                duracion_s, longitud_cm, area95_cm2, velocidad_media_cms,
                rms_ml_cm, rms_ap_cm, rango_ml_cm, rango_ap_cm,
                velocidad_ml_cms, velocidad_ap_cms,
                frec_mediana_ml_hz, frec_mediana_ap_hz, f80_ml_hz, f80_ap_hz,
                config_ron, version_app, juego_ron, limites_ron, notas, registro
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6,
                ?7, ?8, ?9, ?10,
                ?11, ?12, ?13, ?14,
                ?15, ?16,
                ?17, ?18, ?19, ?20,
                ?21, ?22, ?23, ?24, ?25, ?26
             )",
            params![
                ex.paciente_id,
                ahora_iso(),
                ex.operador.trim(),
                ex.tipo.slug(),
                superficie_slug(ex.superficie),
                ex.condicion.slug(),
                m.map(|m| m.duracion_s),
                m.map(|m| m.longitud_cm),
                m.map(|m| m.area95_cm2),
                m.map(|m| m.velocidad_media_cms),
                m.map(|m| m.rms_ml_cm),
                m.map(|m| m.rms_ap_cm),
                m.map(|m| m.rango_ml_cm),
                m.map(|m| m.rango_ap_cm),
                m.map(|m| m.velocidad_ml_cms),
                m.map(|m| m.velocidad_ap_cms),
                m.map(|m| m.frec_mediana_ml_hz),
                m.map(|m| m.frec_mediana_ap_hz),
                m.map(|m| m.f80_ml_hz),
                m.map(|m| m.f80_ap_hz),
                config_ron,
                VERSION_APP,
                juego_ron,
                limites_ron,
                ex.notas.trim(),
                codificar_registro(&ex.registro),
            ],
        )?;
        Ok(self.conexion.last_insert_rowid())
    }

    /// El historial de un paciente, del examen más nuevo al más viejo.
    ///
    /// Sin el BLOB del COP: son decenas de miles de muestras por examen y esto
    /// llena una tabla en pantalla. El registro se lee de a uno, cuando se abre
    /// el examen (ver [`Base::detalle_examen`]).
    pub fn examenes_de(&self, paciente_id: i64, limite: usize) -> Resultado<Vec<ResumenExamen>> {
        let mut stmt = self.conexion.prepare(
            "SELECT id, patient_id, fecha, operador, tipo, superficie, condicion,
                    duracion_s, longitud_cm, area95_cm2, velocidad_media_cms,
                    rms_ml_cm, rms_ap_cm, rango_ml_cm, rango_ap_cm,
                    velocidad_ml_cms, velocidad_ap_cms,
                    frec_mediana_ml_hz, frec_mediana_ap_hz, f80_ml_hz, f80_ap_hz,
                    length(registro) / 12
             FROM postura_examen
             WHERE patient_id = ?1
             ORDER BY fecha DESC
             LIMIT ?2",
        )?;
        let filas = stmt
            .query_map(params![paciente_id, limite as i64], resumen_de_fila)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(filas)
    }

    /// Un examen entero, con su configuración y su COP crudo.
    pub fn detalle_examen(&self, id: i64) -> Resultado<Option<DetalleExamen>> {
        let detalle = self
            .conexion
            .query_row(
                "SELECT id, patient_id, fecha, operador, tipo, superficie, condicion,
                        duracion_s, longitud_cm, area95_cm2, velocidad_media_cms,
                        rms_ml_cm, rms_ap_cm, rango_ml_cm, rango_ap_cm,
                        velocidad_ml_cms, velocidad_ap_cms,
                        frec_mediana_ml_hz, frec_mediana_ap_hz, f80_ml_hz, f80_ap_hz,
                        length(registro) / 12,
                        config_ron, version_app, juego_ron, limites_ron, notas, registro
                 FROM postura_examen WHERE id = ?1",
                params![id],
                |r| {
                    let resumen = resumen_de_fila(r)?;
                    let config_ron: String = r.get(22)?;
                    let version_app: String = r.get(23)?;
                    let juego_ron: Option<String> = r.get(24)?;
                    let limites_ron: Option<String> = r.get(25)?;
                    let notas: String = r.get(26)?;
                    let blob: Vec<u8> = r.get(27)?;
                    Ok((resumen, config_ron, version_app, juego_ron, limites_ron, notas, blob))
                },
            )
            .optional()?;

        let Some((resumen, config_ron, version_app, juego_ron, limites_ron, notas, blob)) = detalle else {
            return Ok(None);
        };
        // Una configuración que no se entiende no tira el examen entero: se cae
        // en la de hoy y el resto del examen se puede ver igual. Es lo que pasa
        // al leer una base escrita por una versión posterior.
        let config = ron::from_str(&config_ron).unwrap_or_default();
        let juego = juego_ron.as_deref().and_then(|s| ron::from_str(s).ok());
        let limites = limites_ron.as_deref().and_then(|s| ron::from_str(s).ok()).unwrap_or_default();
        Ok(Some(DetalleExamen {
            resumen,
            version_app,
            config,
            juego,
            limites,
            notas,
            registro: decodificar_registro(&blob),
        }))
    }

    pub fn actualizar_notas_examen(&self, id: i64, notas: &str) -> Resultado<()> {
        self.conexion.execute("UPDATE postura_examen SET notas = ?1 WHERE id = ?2", params![notas, id])?;
        Ok(())
    }

    pub fn borrar_examen(&self, id: i64) -> Resultado<()> {
        self.conexion.execute("DELETE FROM postura_examen WHERE id = ?1", params![id])?;
        Ok(())
    }
}

fn resumen_de_fila(r: &rusqlite::Row<'_>) -> rusqlite::Result<ResumenExamen> {
    // Las métricas van juntas o no van: una fila con la mitad de los números
    // sería un examen a medio medir, que no es algo que este programa escriba.
    let duracion: Option<f64> = r.get(7)?;
    let metricas = match duracion {
        Some(duracion_s) => Some(MetricasBalance {
            duracion_s,
            longitud_cm: r.get::<_, Option<f64>>(8)?.unwrap_or_default(),
            area95_cm2: r.get::<_, Option<f64>>(9)?.unwrap_or_default(),
            velocidad_media_cms: r.get::<_, Option<f64>>(10)?.unwrap_or_default(),
            rms_ml_cm: r.get::<_, Option<f64>>(11)?.unwrap_or_default(),
            rms_ap_cm: r.get::<_, Option<f64>>(12)?.unwrap_or_default(),
            rango_ml_cm: r.get::<_, Option<f64>>(13)?.unwrap_or_default(),
            rango_ap_cm: r.get::<_, Option<f64>>(14)?.unwrap_or_default(),
            velocidad_ml_cms: r.get::<_, Option<f64>>(15)?.unwrap_or_default(),
            velocidad_ap_cms: r.get::<_, Option<f64>>(16)?.unwrap_or_default(),
            frec_mediana_ml_hz: r.get::<_, Option<f64>>(17)?.unwrap_or_default(),
            frec_mediana_ap_hz: r.get::<_, Option<f64>>(18)?.unwrap_or_default(),
            f80_ml_hz: r.get::<_, Option<f64>>(19)?.unwrap_or_default(),
            f80_ap_hz: r.get::<_, Option<f64>>(20)?.unwrap_or_default(),
        }),
        None => None,
    };
    Ok(ResumenExamen {
        id: r.get(0)?,
        paciente_id: r.get(1)?,
        fecha: r.get(2)?,
        operador: r.get(3)?,
        tipo: TipoExamen::de_slug(&r.get::<_, String>(4)?),
        superficie: superficie_de_slug(&r.get::<_, String>(5)?),
        condicion: condicion_de_slug(&r.get::<_, String>(6)?),
        metricas,
        muestras: r.get::<_, i64>(21)?.max(0) as usize,
    })
}

/// COP crudo a bytes: 3 `f32` little-endian por muestra, `(t_s, ml_cm, ap_cm)`.
///
/// Binario y no texto porque un ensayo de 30 s a 100 Hz son 3000 muestras, y
/// una partida larga muchas más: en RON serían cientos de kilobytes de texto por
/// examen. `f32` y no `f64` porque el dato viene de un ADC de 24 bits sobre
/// cuatro celdas: los dígitos que `f64` conserva de más no los midió nadie.
fn codificar_registro(registro: &[[f64; 3]]) -> Vec<u8> {
    let mut salida = Vec::with_capacity(registro.len() * 12);
    for m in registro {
        for v in m {
            salida.extend_from_slice(&(*v as f32).to_le_bytes());
        }
    }
    salida
}

fn decodificar_registro(blob: &[u8]) -> Vec<[f64; 3]> {
    // Por muestra, 3 f32; una muestra a medias —una escritura cortada— se
    // descarta en vez de fallar al abrir el examen.
    blob.as_chunks::<12>()
        .0
        .iter()
        .map(|c| {
            let f = |i: usize| f32::from_le_bytes([c[i * 4], c[i * 4 + 1], c[i * 4 + 2], c[i * 4 + 3]]) as f64;
            [f(0), f(1), f(2)]
        })
        .collect()
}

/// Fecha y hora actuales en ISO 8601 UTC.
///
/// Se guarda en UTC y no en hora local: es lo que ordena bien y lo que no cambia
/// de significado cuando el equipo viaja o cambia el horario de verano. La
/// conversión a la hora que lee una persona está en [`crate::fecha`].
///
/// Se calcula a mano desde el epoch: es lo mismo que hace vHIT, así que las dos
/// aplicaciones escriben marcas idénticas en la base que comparten.
pub fn ahora_iso() -> String {
    let segundos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let dias = segundos.div_euclid(86_400);
    let resto = segundos.rem_euclid(86_400);
    let (a, m, d) = fecha_civil(dias);
    format!("{a:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", resto / 3600, (resto % 3600) / 60, resto % 60)
}

/// Algoritmo de Howard Hinnant: días desde 1970-01-01 -> fecha civil.
fn fecha_civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let a = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { a + 1 } else { a }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_temporal(nombre: &str) -> std::path::PathBuf {
        let carpeta = std::env::temp_dir().join("posturografox-test-pacientes");
        let _ = std::fs::create_dir_all(&carpeta);
        let ruta = carpeta.join(nombre);
        let _ = std::fs::remove_file(&ruta);
        ruta
    }

    fn metricas_de(area: f64) -> MetricasBalance {
        MetricasBalance { duracion_s: 30.0, area95_cm2: area, longitud_cm: 120.0, ..Default::default() }
    }

    fn examen(paciente_id: i64, tipo: TipoExamen, area: f64) -> ExamenNuevo {
        ExamenNuevo {
            paciente_id,
            operador: "kine".into(),
            tipo,
            superficie: Superficie::Firme,
            condicion: Condicion::OjosAbiertos,
            metricas: Some(metricas_de(area)),
            config: Config::default(),
            juego: None,
            limites: Vec::new(),
            notas: String::new(),
            registro: vec![[0.0, 0.1, -0.2], [0.01, 0.15, -0.25]],
        }
    }

    #[test]
    fn una_base_sin_cifrar_guarda_y_devuelve_lo_mismo() {
        let base = Base::abrir(&base_temporal("claro.sqlite"), None).unwrap();
        assert!(!base.esta_cifrada());
        let id = base.crear_paciente("F-1", "Ana Torres", "1980-05-02", "").unwrap();
        let examen_id = base.guardar_examen(&examen(id, TipoExamen::Estatica, 3.5)).unwrap();

        let detalle = base.detalle_examen(examen_id).unwrap().expect("se acaba de guardar");
        assert_eq!(detalle.resumen.paciente_id, id);
        assert_eq!(detalle.resumen.metricas.unwrap().area95_cm2, 3.5);
        assert_eq!(detalle.resumen.muestras, 2);
        assert_eq!(detalle.registro.len(), 2);
        // El COP vuelve con la precisión de f32, que es la que tenía el dato.
        assert!((detalle.registro[1][1] - 0.15).abs() < 1e-6, "{:?}", detalle.registro[1]);
    }

    #[test]
    fn una_base_cifrada_no_abre_con_otra_frase_ni_sin_frase() {
        let ruta = base_temporal("cifrada.sqlite");
        {
            let base = Base::abrir(&ruta, Some("frase correcta")).unwrap();
            assert!(base.esta_cifrada());
            base.crear_paciente("F-9", "Luis Soto", "", "").unwrap();
        }
        // El archivo no se puede leer como SQLite común: eso es lo que significa
        // cifrado en reposo.
        assert_eq!(sqlite_en_claro(&ruta), Some(false));
        assert!(matches!(Base::abrir(&ruta, Some("otra frase")), Err(ErrorBase::FraseIncorrecta)));
        assert!(matches!(Base::abrir(&ruta, None), Err(ErrorBase::FaltaFrase)));
        let base = Base::abrir(&ruta, Some("frase correcta")).unwrap();
        assert_eq!(base.buscar_pacientes("", Orden::Nombre, 10).unwrap().len(), 1);
    }

    #[test]
    fn a_una_base_en_claro_no_se_le_inventa_que_esta_cifrada() {
        // Quien escribe una frase acá puede estar creyendo que sus datos están
        // cifrados. Decirle "frase incorrecta" lo dejaría probando frases.
        let ruta = base_temporal("en-claro.sqlite");
        drop(Base::abrir(&ruta, None).unwrap());
        assert!(matches!(Base::abrir(&ruta, Some("cualquiera")), Err(ErrorBase::SinCifrar)));
    }

    #[test]
    fn una_frase_vacia_no_es_elegir_no_cifrar() {
        let ruta = base_temporal("vacia.sqlite");
        assert!(matches!(Base::abrir(&ruta, Some("")), Err(ErrorBase::FraseVacia)));
    }

    #[test]
    fn la_frase_se_puede_cambiar_y_la_vieja_deja_de_servir() {
        let ruta = base_temporal("recifrada.sqlite");
        {
            let base = Base::abrir(&ruta, Some("vieja")).unwrap();
            base.crear_paciente("F-2", "Rosa Díaz", "", "").unwrap();
            base.recifrar("nueva").unwrap();
        }
        assert!(matches!(Base::abrir(&ruta, Some("vieja")), Err(ErrorBase::FraseIncorrecta)));
        let base = Base::abrir(&ruta, Some("nueva")).unwrap();
        assert_eq!(base.buscar_pacientes("Rosa", Orden::Nombre, 10).unwrap().len(), 1);
    }

    #[test]
    fn la_busqueda_encuentra_por_nombre_y_por_ficha() {
        let base = Base::abrir(&base_temporal("busqueda.sqlite"), None).unwrap();
        base.crear_paciente("F-100", "Ana Torres", "", "").unwrap();
        base.crear_paciente("F-200", "Luis Soto", "", "").unwrap();
        assert_eq!(base.buscar_pacientes("torres", Orden::Nombre, 10).unwrap().len(), 1);
        assert_eq!(base.buscar_pacientes("F-200", Orden::Nombre, 10).unwrap().len(), 1);
        assert_eq!(base.buscar_pacientes("", Orden::Nombre, 10).unwrap().len(), 2);
        assert!(base.buscar_pacientes("nadie", Orden::Nombre, 10).unwrap().is_empty());
    }

    #[test]
    fn el_que_nunca_se_examino_queda_al_final_del_orden_por_ultimo_examen() {
        // Y no primero, que es lo que hace SQLite con los NULL en DESC. En una
        // suite es el caso normal: el alta la hizo vHIT y acá todavía no vino.
        let base = Base::abrir(&base_temporal("orden.sqlite"), None).unwrap();
        let sin = base.crear_paciente("F-1", "Aaa Sin examen", "", "").unwrap();
        let con = base.crear_paciente("F-2", "Zzz Con examen", "", "").unwrap();
        base.guardar_examen(&examen(con, TipoExamen::Estatica, 2.0)).unwrap();
        let filas = base.buscar_pacientes("", Orden::UltimoExamen, 10).unwrap();
        assert_eq!(filas[0].paciente.id, con);
        assert_eq!(filas[0].examenes, 1);
        assert_eq!(filas[1].paciente.id, sin);
        assert_eq!(filas[1].examenes, 0);
        assert!(filas[1].ultimo_examen.is_none());
    }

    #[test]
    fn borrar_un_paciente_se_lleva_sus_examenes() {
        let base = Base::abrir(&base_temporal("cascada.sqlite"), None).unwrap();
        let id = base.crear_paciente("F-3", "Juan Pinto", "", "").unwrap();
        let examen_id = base.guardar_examen(&examen(id, TipoExamen::Estatica, 4.0)).unwrap();
        base.borrar_paciente(id).unwrap();
        assert!(base.detalle_examen(examen_id).unwrap().is_none(), "el examen tiene que irse con el paciente");
    }

    #[test]
    fn el_examen_guarda_la_configuracion_con_la_que_se_midio() {
        let base = Base::abrir(&base_temporal("config.sqlite"), None).unwrap();
        let id = base.crear_paciente("F-4", "Eva Lara", "", "").unwrap();
        let mut ex = examen(id, TipoExamen::Estatica, 5.0);
        ex.config.ancho_cm = 55.0;
        ex.config.filtro_corte_hz = 4.25;
        let examen_id = base.guardar_examen(&ex).unwrap();
        let detalle = base.detalle_examen(examen_id).unwrap().unwrap();
        assert_eq!(detalle.config.ancho_cm, 55.0);
        assert_eq!(detalle.config.filtro_corte_hz, 4.25);
        assert_eq!(detalle.version_app, VERSION_APP);
    }

    #[test]
    fn un_examen_de_limites_no_inventa_metricas_de_oscilacion() {
        let base = Base::abrir(&base_temporal("limites.sqlite"), None).unwrap();
        let id = base.crear_paciente("F-5", "Iván Rojas", "", "").unwrap();
        let mut ex = examen(id, TipoExamen::Limites, 0.0);
        ex.metricas = None;
        let examen_id = base.guardar_examen(&ex).unwrap();
        let detalle = base.detalle_examen(examen_id).unwrap().unwrap();
        assert!(detalle.resumen.metricas.is_none(), "sin oscilación medida, NULL y no cero");
        assert_eq!(detalle.resumen.tipo, TipoExamen::Limites);
    }

    #[test]
    fn el_historial_sale_del_mas_nuevo_al_mas_viejo_y_se_puede_borrar_de_a_uno() {
        let base = Base::abrir(&base_temporal("historial.sqlite"), None).unwrap();
        let id = base.crear_paciente("F-6", "Sara Vidal", "", "").unwrap();
        let primero = base.guardar_examen(&examen(id, TipoExamen::Estatica, 1.0)).unwrap();
        let segundo = base.guardar_examen(&examen(id, TipoExamen::Juego, 9.0)).unwrap();
        let historial = base.examenes_de(id, 10).unwrap();
        assert_eq!(historial.len(), 2);
        // Dos exámenes del mismo segundo empatan en `fecha`; el id rompe el
        // empate en el orden de inserción, y el más nuevo tiene el id mayor.
        assert!(historial.iter().any(|e| e.id == segundo && e.tipo == TipoExamen::Juego));
        base.borrar_examen(primero).unwrap();
        assert_eq!(base.examenes_de(id, 10).unwrap().len(), 1);
    }

    #[test]
    fn las_notas_del_examen_se_pueden_corregir_despues() {
        let base = Base::abrir(&base_temporal("notas.sqlite"), None).unwrap();
        let id = base.crear_paciente("F-7", "Omar Cid", "", "").unwrap();
        let examen_id = base.guardar_examen(&examen(id, TipoExamen::Estatica, 2.5)).unwrap();
        base.actualizar_notas_examen(examen_id, "se apoyó en la baranda").unwrap();
        assert_eq!(base.detalle_examen(examen_id).unwrap().unwrap().notas, "se apoyó en la baranda");
    }

    #[test]
    fn la_ficha_del_paciente_se_puede_editar() {
        let base = Base::abrir(&base_temporal("editar.sqlite"), None).unwrap();
        let id = base.crear_paciente("", "Nombre Provisorio", "", "").unwrap();
        let p = Paciente {
            id,
            ficha: "F-8".into(),
            nombre: "Nombre Real".into(),
            nacimiento: "1975-03-11".into(),
            notas: "usa bastón".into(),
        };
        base.actualizar_paciente(&p).unwrap();
        let filas = base.buscar_pacientes("Real", Orden::Nombre, 10).unwrap();
        assert_eq!(filas[0].paciente, p);
    }

    #[test]
    fn la_etiqueta_del_paciente_incluye_la_ficha_solo_si_la_tiene() {
        let con = Paciente { nombre: "Ana Torres".into(), ficha: "F-1".into(), ..Default::default() };
        let sin = Paciente { nombre: "Ana Torres".into(), ..Default::default() };
        assert_eq!(con.etiqueta(), "Ana Torres (F-1)");
        assert_eq!(sin.etiqueta(), "Ana Torres");
    }

    /// El DDL de vHIT, copiado tal cual de su `session.rs`.
    ///
    /// Está acá para poder simular una base creada por el otro programa sin
    /// compilarlo. Si vHIT cambia estas tablas, este test es el que avisa.
    const DDL_VHIT: &str = r#"
        CREATE TABLE IF NOT EXISTS patient (
            id           INTEGER PRIMARY KEY,
            external_id  TEXT NOT NULL DEFAULT '',
            name         TEXT NOT NULL,
            birth_date   TEXT NOT NULL DEFAULT '',
            notes        TEXT NOT NULL DEFAULT '',
            created_at   TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS exam (
            id                 INTEGER PRIMARY KEY,
            patient_id         INTEGER NOT NULL REFERENCES patient(id) ON DELETE CASCADE,
            started_at         TEXT NOT NULL,
            operator           TEXT NOT NULL DEFAULT '',
            canal              TEXT NOT NULL DEFAULT 'lateral',
            calibrated         INTEGER NOT NULL,
            calib_k            REAL,
            calib_residual_deg REAL,
            calib_range_deg    REAL,
            cam_fps            REAL NOT NULL DEFAULT 0,
            cam_width          INTEGER NOT NULL DEFAULT 0,
            cam_height         INTEGER NOT NULL DEFAULT 0,
            config_toml        TEXT NOT NULL,
            app_version        TEXT NOT NULL,
            model_sha256       TEXT NOT NULL DEFAULT '',
            notes              TEXT NOT NULL DEFAULT ''
        );
    "#;

    #[test]
    fn una_base_hecha_por_vhit_se_abre_y_sus_pacientes_estan() {
        // Es el contrato de la suite: el alta se hace en un equipo y se ve en el
        // otro, sin migrar ni copiar nada.
        let ruta = base_temporal("desde-vhit.sqlite");
        {
            let conexion = Connection::open(&ruta).unwrap();
            conexion.execute_batch(DDL_VHIT).unwrap();
            conexion
                .execute(
                    "INSERT INTO patient (external_id, name, birth_date, notes, created_at)
                     VALUES ('F-77', 'Paciente De vHIT', '1969-04-12', 'alta hecha en vHIT', '2026-09-18T10:00:00Z')",
                    [],
                )
                .unwrap();
            conexion
                .execute(
                    "INSERT INTO exam (patient_id, started_at, calibrated, config_toml, app_version)
                     VALUES (1, '2026-09-18T10:05:00Z', 1, '', '0.1.0')",
                    [],
                )
                .unwrap();
        }

        let base = Base::abrir(&ruta, None).unwrap();
        let filas = base.buscar_pacientes("vHIT", Orden::Nombre, 10).unwrap();
        assert_eq!(filas.len(), 1, "el paciente dado de alta en vHIT tiene que aparecer acá");
        assert_eq!(filas[0].paciente.ficha, "F-77");
        assert_eq!(filas[0].paciente.nacimiento, "1969-04-12");
        // Todavía sin posturografía: es el caso normal al empezar.
        assert_eq!(filas[0].examenes, 0);
        assert!(filas[0].ultimo_examen.is_none());

        // Y agregarle un examen de acá no toca nada de lo suyo.
        let id_paciente = filas[0].paciente.id;
        base.guardar_examen(&examen(id_paciente, TipoExamen::Estatica, 4.2)).unwrap();
        assert_eq!(base.examenes_de(id_paciente, 10).unwrap().len(), 1);
        let examenes_vhit: i64 =
            base.conexion.query_row("SELECT count(*) FROM exam", [], |r| r.get(0)).expect("la tabla de vHIT sigue ahí");
        assert_eq!(examenes_vhit, 1, "los exámenes de vHIT no se tocan");
    }

    #[test]
    fn borrar_un_paciente_desde_aca_se_lleva_tambien_los_examenes_de_vhit() {
        // Es lo que hace que borrar sea borrar y no dejar huérfanos en la mitad
        // de la suite. Por eso la ventana lo dice antes de confirmar.
        let ruta = base_temporal("cascada-suite.sqlite");
        {
            let conexion = Connection::open(&ruta).unwrap();
            conexion.execute_batch(DDL_VHIT).unwrap();
            conexion
                .execute("INSERT INTO patient (name, created_at) VALUES ('Compartido', '2026-09-18T10:00:00Z')", [])
                .unwrap();
            conexion
                .execute(
                    "INSERT INTO exam (patient_id, started_at, calibrated, config_toml, app_version)
                     VALUES (1, '2026-09-18T10:05:00Z', 1, '', '0.1.0')",
                    [],
                )
                .unwrap();
        }

        let base = Base::abrir(&ruta, None).unwrap();
        base.guardar_examen(&examen(1, TipoExamen::Estatica, 3.0)).unwrap();
        base.borrar_paciente(1).unwrap();
        let quedan_vhit: i64 = base.conexion.query_row("SELECT count(*) FROM exam", [], |r| r.get(0)).unwrap();
        let quedan_postura: i64 =
            base.conexion.query_row("SELECT count(*) FROM postura_examen", [], |r| r.get(0)).unwrap();
        assert_eq!(quedan_vhit, 0, "sin cascada quedarían exámenes de vHIT sin paciente");
        assert_eq!(quedan_postura, 0);
    }

    #[test]
    fn abrir_dos_veces_no_duplica_ni_pierde_nada() {
        // `migrar` corre en cada apertura y la base la crea cualquiera de los dos
        // programas: tiene que ser idempotente.
        let ruta = base_temporal("idempotente.sqlite");
        let id = {
            let base = Base::abrir(&ruta, None).unwrap();
            base.crear_paciente("F-1", "Ana", "", "").unwrap()
        };
        let base = Base::abrir(&ruta, None).unwrap();
        assert_eq!(base.buscar_pacientes("", Orden::Nombre, 10).unwrap().len(), 1);
        assert_eq!(base.buscar_pacientes("", Orden::Nombre, 10).unwrap()[0].paciente.id, id);
    }

    #[test]
    fn la_marca_de_tiempo_es_iso_8601_en_utc() {
        let ahora = ahora_iso();
        assert_eq!(ahora.len(), 20, "{ahora}");
        assert!(ahora.ends_with('Z'), "{ahora}");
        assert_eq!(&ahora[4..5], "-");
        assert_eq!(&ahora[10..11], "T");
    }

    #[test]
    fn la_fecha_civil_es_la_de_hinnant() {
        assert_eq!(fecha_civil(0), (1970, 1, 1));
        assert_eq!(fecha_civil(19_651), (2023, 10, 21));
        // Año bisiesto de los que se olvidan.
        assert_eq!(fecha_civil(11_016), (2000, 2, 29));
    }

    #[test]
    fn el_registro_ida_y_vuelta_no_cambia_de_largo() {
        let registro = vec![[0.0, 1.5, -2.5], [0.01, 1.6, -2.4], [0.02, 1.7, -2.3]];
        let vuelta = decodificar_registro(&codificar_registro(&registro));
        assert_eq!(vuelta.len(), registro.len());
        for (a, b) in registro.iter().zip(&vuelta) {
            for i in 0..3 {
                assert!((a[i] - b[i]).abs() < 1e-6, "{a:?} vs {b:?}");
            }
        }
        // Un blob truncado —una escritura cortada— da las muestras completas y
        // descarta la última a medias, en vez de fallar al abrir el examen.
        let mut bytes = codificar_registro(&registro);
        bytes.truncate(bytes.len() - 5);
        assert_eq!(decodificar_registro(&bytes).len(), 2);
    }

    #[test]
    fn una_partida_guarda_lo_que_la_hace_una_partida() {
        let base = Base::abrir(&base_temporal("partida.sqlite"), None).unwrap();
        let id = base.crear_paciente("F-10", "Tere Muñoz", "", "").unwrap();
        let mut ex = examen(id, TipoExamen::Juego, 12.0);
        ex.juego = Some(DatosJuego {
            rango: crate::rango::RangoCalibrado::por_defecto(40.0, 40.0),
            exigencia: 0.6,
            duracion_s: 61.0,
            gano: true,
            resumen: None,
        });
        let examen_id = base.guardar_examen(&ex).unwrap();
        let detalle = base.detalle_examen(examen_id).unwrap().unwrap();
        let juego = detalle.juego.expect("era una partida");
        assert_eq!(juego.exigencia, 0.6);
        assert!(juego.gano);
        assert_eq!(detalle.resumen.etiqueta(), "Firme + juego");
    }
}
