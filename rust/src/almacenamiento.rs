//! Dónde vive la base de pacientes de la suite, y si está cifrada.
//!
//! # Una sola base para todos los equipos
//!
//! Posturografox no es el único programa que examina a la misma persona: vHIT
//! mide el reflejo vestíbulo-ocular sobre los mismos pacientes, y dar de alta
//! dos veces al mismo señor —una por equipo— es garantizar que la ficha de uno
//! diga una cosa y la del otro otra. Así que la base es **la misma**: un solo
//! archivo SQLite, con una sola tabla `patient`, y cada equipo agregando sus
//! propios exámenes al lado de los del otro (ver [`crate::pacientes`]).
//!
//! Por eso la carpeta por defecto es la de vHIT y el nombre del archivo es el
//! suyo: el primero de los dos programas que arranque crea la base y el otro la
//! encuentra hecha. La ruta se puede cambiar —una carpeta de red, un disco
//! cifrado del hospital— y es lo único que hay que igualar entre los dos.
//!
//! # Por qué no va en [`crate::config::Config`]
//!
//! `Config` son los parámetros de la medición, se guardan con la ventana y se
//! restauran con el botón de valores por defecto. La ruta de la base de
//! pacientes no es un parámetro de medición: restaurarla por defecto junto con
//! el ancho de la plataforma dejaría al programa escribiendo en otra base sin
//! que nadie lo haya pedido. Son dos cosas, y van en dos archivos.
//!
//! # Cifrado opcional
//!
//! La base puede quedar **sin cifrar**, y es una decisión del que instala.
//! Conviene ser claro sobre lo que significa: un examen de equilibrio es un
//! dato de salud, y la ley 19.628 (Chile), el GDPR y la HIPAA piden cifrado en
//! reposo. Sin frase de paso, el archivo lo abre cualquiera que llegue al
//! disco con cualquier visor de SQLite. Por eso la ventana de pacientes lo
//! sigue diciendo en pantalla mientras esté así: una decisión de este tamaño no
//! puede quedar tomada una vez y olvidada.

use std::path::{Path, PathBuf};

use crate::datos::carpeta_datos;

/// Nombre del archivo de configuración, en la carpeta de datos del programa.
const ARCHIVO: &str = "almacenamiento.ron";

/// Nombre del archivo de la base, adentro de la carpeta de datos elegida.
///
/// Es el de vHIT a propósito: la base es compartida (ver el encabezado del
/// módulo) y renombrarla acá sería crear una segunda.
pub const ARCHIVO_BASE: &str = "vhit.sqlite";

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Almacenamiento {
    /// Carpeta donde vive la base de pacientes de la suite.
    pub carpeta: String,
    /// Si la base está cifrada con SQLCipher. Sin cifrar no hay frase de paso
    /// y no se pide ninguna al abrir.
    pub cifrada: bool,
    /// Si alguien ya eligió las dos cosas de arriba. Mientras sea `false` la
    /// ventana de pacientes pregunta antes de tocar nada.
    pub configurado: bool,
    /// Cuándo se decidió la política de cifrado, en ISO 8601 UTC.
    ///
    /// No es telemetría: queda escrito **en el equipo** que alguien tomó esta
    /// decisión y cuándo. Apagar el cifrado de una base de datos de salud es
    /// una decisión del responsable de los datos, y lo que distingue una
    /// decisión de un descuido es que conste.
    pub cifrado_decidido_en: String,
}

impl Default for Almacenamiento {
    fn default() -> Self {
        Self {
            carpeta: carpeta_de_la_suite_por_defecto(),
            // Cifrada por defecto: si alguien no elige, lo que pasa es lo
            // seguro y no lo cómodo.
            cifrada: true,
            configurado: false,
            cifrado_decidido_en: String::new(),
        }
    }
}

impl Almacenamiento {
    pub fn ruta() -> PathBuf {
        carpeta_datos().join(ARCHIVO)
    }

    pub fn cargar() -> Self {
        Self::cargar_de(&Self::ruta())
    }

    pub fn cargar_de(ruta: &Path) -> Self {
        let Ok(texto) = std::fs::read_to_string(ruta) else {
            return Self::default();
        };
        // Un archivo ilegible NO puede pasar por configurado: `configurado`
        // volvería a `false` y el programa propondría crear una base nueva al
        // lado de una que ya tiene pacientes. Los valores por defecto hacen
        // exactamente eso: vuelven a preguntar.
        ron::from_str(&texto).unwrap_or_default()
    }

    pub fn guardar(&self) -> Result<(), String> {
        self.guardar_en(&Self::ruta())
    }

    pub fn guardar_en(&self, ruta: &Path) -> Result<(), String> {
        if let Some(carpeta) = ruta.parent() {
            std::fs::create_dir_all(carpeta).map_err(|e| format!("no pude crear {}: {e}", carpeta.display()))?;
        }
        let texto = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default()).map_err(|e| e.to_string())?;
        std::fs::write(ruta, texto).map_err(|e| format!("no pude escribir {}: {e}", ruta.display()))
    }

    /// Deja constancia de la política de cifrado elegida, con su fecha.
    pub fn fijar_cifrado(&mut self, cifrada: bool) {
        self.cifrada = cifrada;
        self.cifrado_decidido_en = crate::pacientes::ahora_iso();
    }

    /// La ruta completa del archivo de la base.
    pub fn ruta_base(&self) -> PathBuf {
        Path::new(&self.carpeta).join(ARCHIVO_BASE)
    }

    /// `true` si ya hay una base creada donde dice esta configuración.
    pub fn base_existe(&self) -> bool {
        self.ruta_base().exists()
    }

    /// Crea la carpeta de la base si no está. Se llama al confirmar la
    /// configuración: es el momento en que el operador todavía está mirando y
    /// puede corregir una ruta que no se puede crear.
    pub fn crear_carpeta(&self) -> Result<(), String> {
        if self.carpeta.trim().is_empty() {
            return Err("la carpeta de la base está vacía".to_string());
        }
        std::fs::create_dir_all(&self.carpeta)
            .map_err(|e| format!("no pude crear la carpeta de la base ({}): {e}", self.carpeta))
    }
}

/// Carpeta compartida de la suite: la de vHIT, `~/.local/share/vhit` en Linux
/// y macOS y `%APPDATA%\vhit` en Windows.
///
/// NO el directorio de trabajo: un acceso directo o un AppImage se lanzan desde
/// cualquier lado y la base terminaría en un lugar distinto cada vez.
pub fn carpeta_de_la_suite_por_defecto() -> String {
    // La misma convención que usa vhit::storage, para caer en el mismo archivo.
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Path::new(&dir).join("vhit").to_string_lossy().into_owned();
    }
    if let Some(dir) = std::env::var_os("APPDATA").filter(|v| !v.is_empty()) {
        return Path::new(&dir).join("vhit").to_string_lossy().into_owned();
    }
    match std::env::var_os("HOME") {
        Some(h) => Path::new(&h).join(".local/share/vhit").to_string_lossy().into_owned(),
        // Sin HOME —un entorno raro, un servicio— el directorio actual es lo
        // único que con seguridad se puede escribir.
        None => ".".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporal(nombre: &str) -> PathBuf {
        let carpeta = std::env::temp_dir().join("posturografox-test-almacenamiento");
        let _ = std::fs::create_dir_all(&carpeta);
        let ruta = carpeta.join(nombre);
        let _ = std::fs::remove_file(&ruta);
        ruta
    }

    #[test]
    fn sin_archivo_hay_que_preguntar_y_se_cifra() {
        let a = Almacenamiento::cargar_de(&temporal("no-existe.ron"));
        assert!(!a.configurado, "sin archivo hay que preguntar dónde está la base");
        assert!(a.cifrada, "lo seguro es lo que pasa por defecto");
    }

    #[test]
    fn ida_y_vuelta_por_el_archivo() {
        let ruta = temporal("ida-vuelta.ron");
        let original = Almacenamiento {
            carpeta: "/tmp/suite-datos".into(),
            cifrada: false,
            configurado: true,
            cifrado_decidido_en: "2026-09-19T12:00:00Z".into(),
        };
        original.guardar_en(&ruta).unwrap();
        assert_eq!(Almacenamiento::cargar_de(&ruta), original);
    }

    #[test]
    fn un_archivo_roto_no_se_hace_pasar_por_configurado() {
        // Si un archivo ilegible cayera en los valores por defecto CON
        // `configurado` en true, el programa abriría una base que no es la del
        // operador sin decir nada.
        let ruta = temporal("roto.ron");
        std::fs::write(&ruta, "esto no es ) ron").unwrap();
        assert!(!Almacenamiento::cargar_de(&ruta).configurado);
    }

    #[test]
    fn la_base_cuelga_de_la_carpeta_elegida_con_el_nombre_de_la_suite() {
        let a = Almacenamiento { carpeta: "/tmp/equis".into(), ..Default::default() };
        assert_eq!(a.ruta_base(), Path::new("/tmp/equis/vhit.sqlite"));
    }

    #[test]
    fn apagar_el_cifrado_deja_constancia_fechada() {
        let mut a = Almacenamiento::default();
        assert!(a.cifrado_decidido_en.is_empty());
        a.fijar_cifrado(false);
        assert!(!a.cifrada);
        assert_eq!(a.cifrado_decidido_en.len(), 20, "{}", a.cifrado_decidido_en);
        assert!(a.cifrado_decidido_en.ends_with('Z'));
    }

    #[test]
    fn una_carpeta_vacia_se_rechaza_al_crear() {
        let a = Almacenamiento { carpeta: "   ".into(), ..Default::default() };
        assert!(a.crear_carpeta().is_err());
    }

    #[test]
    fn la_carpeta_por_defecto_es_la_de_la_suite() {
        // El nombre importa: es lo que hace que los dos programas caigan en el
        // mismo archivo sin que nadie configure nada.
        let carpeta = carpeta_de_la_suite_por_defecto();
        assert!(carpeta.ends_with("vhit") || carpeta == ".", "{carpeta}");
    }
}
