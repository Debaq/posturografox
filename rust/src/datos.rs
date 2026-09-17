//! Dónde escribe el programa: una sola carpeta de datos del usuario, en vez
//! del directorio de trabajo.
//!
//! Escribir en el directorio de trabajo rompe en cuanto la app se lanza desde
//! otra carpeta, desde un acceso directo de Windows o desde una ubicación de
//! solo lectura (Archivos de programa, un pendrive montado sin escritura).

use std::path::PathBuf;

const NOMBRE_APP: &str = "posturografox";

/// Carpeta base de datos del usuario, siguiendo la convención del sistema:
/// `$XDG_DATA_HOME` o `~/.local/share` en Linux, `%APPDATA%` en Windows y
/// `~/Library/Application Support` en macOS. Si no hay ninguna variable de
/// entorno utilizable, cae al directorio actual para no perder los datos.
pub fn carpeta_datos() -> PathBuf {
    base_del_sistema().unwrap_or_else(|| PathBuf::from(".")).join(NOMBRE_APP)
}

#[cfg(target_os = "windows")]
fn base_del_sistema() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(PathBuf::from)
}

#[cfg(target_os = "macos")]
fn base_del_sistema() -> Option<PathBuf> {
    let hogar = std::env::var_os("HOME")?;
    Some(PathBuf::from(hogar).join("Library/Application Support"))
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn base_del_sistema() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(xdg));
    }
    std::env::var_os("HOME").map(|hogar| PathBuf::from(hogar).join(".local/share"))
}

/// Carpeta donde se guardan los CSV de las sesiones exportadas.
pub fn carpeta_sesiones() -> PathBuf {
    carpeta_datos().join("sesiones")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_carpeta_de_datos_termina_en_el_nombre_de_la_app() {
        assert_eq!(carpeta_datos().file_name().unwrap(), NOMBRE_APP);
    }

    #[test]
    fn las_sesiones_cuelgan_de_la_carpeta_de_datos() {
        assert_eq!(carpeta_sesiones().parent().unwrap(), carpeta_datos());
    }
}
