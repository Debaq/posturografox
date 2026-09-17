//! Exporta una sesión (metadatos + métricas + serie temporal cruda) a CSV,
//! para que quede en el registro clínico del paciente.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::datos::carpeta_sesiones;
use crate::estabilometria::{Condicion, MetricasBalance, Superficie};

fn sanitizar(texto: &str) -> String {
    let limpio: String = texto.trim().chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
    if limpio.is_empty() { "anonimo".to_string() } else { limpio }
}

/// Escribe la sesión en la carpeta de datos del usuario (ver `src/datos.rs`)
/// y devuelve la ruta final.
pub fn exportar_csv(
    paciente: &str,
    condicion: Condicion,
    superficie: Superficie,
    ancho_cm: f64,
    prof_cm: f64,
    metricas: &MetricasBalance,
    registro: &[[f64; 3]],
) -> io::Result<PathBuf> {
    exportar_csv_en(&carpeta_sesiones(), paciente, condicion, superficie, ancho_cm, prof_cm, metricas, registro)
}

/// Igual que `exportar_csv`, pero con la carpeta de destino explícita: así
/// los tests escriben en un directorio temporal en vez de ensuciar el repo,
/// y más adelante se puede ofrecer "Guardar como..." sin tocar esta lógica.
#[allow(clippy::too_many_arguments)]
pub fn exportar_csv_en(
    carpeta: &Path,
    paciente: &str,
    condicion: Condicion,
    superficie: Superficie,
    ancho_cm: f64,
    prof_cm: f64,
    metricas: &MetricasBalance,
    registro: &[[f64; 3]],
) -> io::Result<PathBuf> {
    fs::create_dir_all(carpeta)?;

    let epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let nombre = format!("sesion_{}_{}_{}_{}.csv", sanitizar(paciente), superficie.slug(), condicion.slug(), epoch);
    let ruta = carpeta.join(nombre);

    let mut archivo = fs::File::create(&ruta)?;
    writeln!(archivo, "# Posturografox - registro de sesión")?;
    writeln!(archivo, "# paciente: {}", paciente.trim())?;
    writeln!(archivo, "# condicion: {}", condicion.etiqueta())?;
    writeln!(archivo, "# superficie: {}", superficie.etiqueta())?;
    writeln!(archivo, "# epoch_unix_s: {epoch}")?;
    writeln!(archivo, "# ancho_cm: {ancho_cm:.2}")?;
    writeln!(archivo, "# profundidad_cm: {prof_cm:.2}")?;
    writeln!(archivo, "# longitud_cm: {:.3}", metricas.longitud_cm)?;
    writeln!(archivo, "# area95_cm2: {:.3}", metricas.area95_cm2)?;
    writeln!(archivo, "# velocidad_media_cms: {:.3}", metricas.velocidad_media_cms)?;
    writeln!(archivo, "# duracion_s: {:.3}", metricas.duracion_s)?;
    writeln!(archivo, "# rms_ml_cm: {:.3}", metricas.rms_ml_cm)?;
    writeln!(archivo, "# rms_ap_cm: {:.3}", metricas.rms_ap_cm)?;
    writeln!(archivo, "# rango_ml_cm: {:.3}", metricas.rango_ml_cm)?;
    writeln!(archivo, "# rango_ap_cm: {:.3}", metricas.rango_ap_cm)?;
    writeln!(archivo, "# velocidad_ml_cms: {:.3}", metricas.velocidad_ml_cms)?;
    writeln!(archivo, "# velocidad_ap_cms: {:.3}", metricas.velocidad_ap_cms)?;
    writeln!(archivo, "# frec_mediana_ml_hz: {:.3}", metricas.frec_mediana_ml_hz)?;
    writeln!(archivo, "# frec_mediana_ap_hz: {:.3}", metricas.frec_mediana_ap_hz)?;
    writeln!(archivo, "# f80_ml_hz: {:.3}", metricas.f80_ml_hz)?;
    writeln!(archivo, "# f80_ap_hz: {:.3}", metricas.f80_ap_hz)?;
    writeln!(archivo, "t_s,cop_ml_cm,cop_ap_cm")?;
    for m in registro {
        writeln!(archivo, "{:.4},{:.4},{:.4}", m[0], m[1], m[2])?;
    }

    Ok(ruta)
}

/// Exporta los resultados del ejercicio de límites de estabilidad: una fila
/// por dirección, con cuánto alcanzó y cuánto tardó. Sin esto, el resultado
/// del ejercicio se perdía al salir de la pantalla.
pub fn exportar_limites(paciente: &str, intentos: &[crate::limites::Intento]) -> io::Result<PathBuf> {
    exportar_limites_en(&carpeta_sesiones(), paciente, intentos)
}

pub fn exportar_limites_en(
    carpeta: &Path,
    paciente: &str,
    intentos: &[crate::limites::Intento],
) -> io::Result<PathBuf> {
    fs::create_dir_all(carpeta)?;
    let epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let ruta = carpeta.join(format!("limites_{}_{}.csv", sanitizar(paciente), epoch));

    let mut archivo = fs::File::create(&ruta)?;
    writeln!(archivo, "# Posturografox - límites de estabilidad")?;
    writeln!(archivo, "# paciente: {}", paciente.trim())?;
    writeln!(archivo, "# epoch_unix_s: {epoch}")?;
    writeln!(archivo, "direccion,alcance_cm,fraccion_objetivo,tiempo_s")?;
    for intento in intentos {
        writeln!(
            archivo,
            "{},{:.3},{:.3},{:.2}",
            crate::limites::NOMBRES[intento.direccion.min(crate::limites::DIRECCIONES - 1)],
            intento.alcance_cm,
            intento.fraccion_objetivo,
            intento.tiempo_s
        )?;
    }
    Ok(ruta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizar_reemplaza_caracteres_raros_y_admite_vacio() {
        assert_eq!(sanitizar("Ana Pérez #12"), "Ana_Pérez__12");
        assert_eq!(sanitizar("   "), "anonimo");
        assert_eq!(sanitizar(""), "anonimo");
    }

    fn metricas_de_prueba() -> MetricasBalance {
        MetricasBalance {
            longitud_cm: 12.5,
            area95_cm2: 3.2,
            velocidad_media_cms: 1.1,
            duracion_s: 10.0,
            rms_ml_cm: 0.5,
            rms_ap_cm: 0.4,
            rango_ml_cm: 2.0,
            rango_ap_cm: 1.5,
            ..MetricasBalance::default()
        }
    }

    #[test]
    fn exportar_csv_escribe_metadata_y_filas_esperadas() {
        // Directorio temporal propio: el test no escribe en el repo ni en la
        // carpeta de datos del usuario, y se borra solo al terminar.
        let carpeta = tempfile::tempdir().expect("crear directorio temporal");
        let registro = [[0.0, 0.0, 0.0], [0.5, 0.1, -0.1], [1.0, 0.2, -0.2]];

        let ruta = exportar_csv_en(
            carpeta.path(),
            "Test Paciente",
            Condicion::OjosCerrados,
            Superficie::Espuma,
            40.0,
            40.0,
            &metricas_de_prueba(),
            &registro,
        )
        .expect("exportar_csv no debería fallar");

        let contenido = fs::read_to_string(&ruta).expect("el archivo debe existir y ser legible");
        assert!(contenido.contains("# paciente: Test Paciente"));
        assert!(contenido.contains("# condicion: Ojos cerrados"));
        assert!(contenido.contains("# superficie: Espuma"));
        assert!(contenido.contains("t_s,cop_ml_cm,cop_ap_cm"));
        assert!(contenido.contains("0.5000,0.1000,-0.1000"));
        assert_eq!(contenido.lines().count(), 25, "21 líneas de metadata + encabezado + 3 filas");
    }

    #[test]
    fn los_limites_se_exportan_con_una_fila_por_direccion() {
        use crate::limites::Intento;
        let carpeta = tempfile::tempdir().expect("crear directorio temporal");
        let intentos = vec![
            Intento { direccion: 0, tiempo_s: 1.5, alcance_cm: 12.0, fraccion_objetivo: 0.86 },
            Intento { direccion: 4, tiempo_s: 2.25, alcance_cm: 7.5, fraccion_objetivo: 0.54 },
        ];

        let ruta = exportar_limites_en(carpeta.path(), "ID-7", &intentos).expect("exportar límites");
        let contenido = fs::read_to_string(&ruta).unwrap();

        assert!(contenido.contains("direccion,alcance_cm,fraccion_objetivo,tiempo_s"));
        assert!(contenido.contains("N,12.000,0.860,1.50"));
        assert!(contenido.contains("S,7.500,0.540,2.25"));
    }

    #[test]
    fn exportar_crea_la_carpeta_destino_si_no_existe() {
        let base = tempfile::tempdir().expect("crear directorio temporal");
        let carpeta = base.path().join("sesiones/anidada");
        assert!(!carpeta.exists());

        let ruta = exportar_csv_en(
            &carpeta,
            "ID-42",
            Condicion::OjosAbiertos,
            Superficie::Firme,
            40.0,
            40.0,
            &metricas_de_prueba(),
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
        )
        .expect("debería crear la carpeta y escribir igual");

        assert!(ruta.starts_with(&carpeta));
        assert!(ruta.exists());
    }

    #[test]
    fn el_nombre_del_archivo_identifica_paciente_condicion_y_superficie() {
        let carpeta = tempfile::tempdir().expect("crear directorio temporal");
        let ruta = exportar_csv_en(
            carpeta.path(),
            "Ana Pérez",
            Condicion::OjosCerrados,
            Superficie::Espuma,
            40.0,
            40.0,
            &metricas_de_prueba(),
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
        )
        .expect("exportar");

        let nombre = ruta.file_name().unwrap().to_string_lossy().into_owned();
        assert!(nombre.starts_with("sesion_Ana_Pérez_espuma_ojos_cerrados_"), "nombre inesperado: {nombre}");
        assert!(nombre.ends_with(".csv"));
    }
}
