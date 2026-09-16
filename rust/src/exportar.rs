//! Exporta una sesión (metadatos + métricas + serie temporal cruda) a CSV,
//! para que quede en el registro clínico del paciente.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::estabilometria::{Condicion, MetricasBalance, Superficie};

const CARPETA_SESIONES: &str = "sesiones";

fn sanitizar(texto: &str) -> String {
    let limpio: String = texto
        .trim()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    if limpio.is_empty() {
        "anonimo".to_string()
    } else {
        limpio
    }
}

/// Escribe `sesiones/sesion_<paciente>_<superficie>_<condicion>_<epoch>.csv`
/// (relativo al directorio de trabajo) y devuelve la ruta final.
pub fn exportar_csv(
    paciente: &str,
    condicion: Condicion,
    superficie: Superficie,
    ancho_cm: f64,
    prof_cm: f64,
    metricas: &MetricasBalance,
    registro: &[[f64; 3]],
) -> io::Result<PathBuf> {
    fs::create_dir_all(CARPETA_SESIONES)?;

    let epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let nombre =
        format!("sesion_{}_{}_{}_{}.csv", sanitizar(paciente), superficie.slug(), condicion.slug(), epoch);
    let ruta = PathBuf::from(CARPETA_SESIONES).join(nombre);

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
    writeln!(archivo, "t_s,cop_ml_cm,cop_ap_cm")?;
    for m in registro {
        writeln!(archivo, "{:.4},{:.4},{:.4}", m[0], m[1], m[2])?;
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

    #[test]
    fn exportar_csv_escribe_metadata_y_filas_esperadas() {
        let metricas = MetricasBalance {
            longitud_cm: 12.5,
            area95_cm2: 3.2,
            velocidad_media_cms: 1.1,
            duracion_s: 10.0,
            rms_ml_cm: 0.5,
            rms_ap_cm: 0.4,
            rango_ml_cm: 2.0,
            rango_ap_cm: 1.5,
        };
        let registro = [[0.0, 0.0, 0.0], [0.5, 0.1, -0.1], [1.0, 0.2, -0.2]];

        let ruta =
            exportar_csv("Test Paciente", Condicion::OjosCerrados, Superficie::Espuma, 40.0, 40.0, &metricas, &registro)
                .expect("exportar_csv no debería fallar");

        let contenido = fs::read_to_string(&ruta).expect("el archivo debe existir y ser legible");
        assert!(contenido.contains("# paciente: Test Paciente"));
        assert!(contenido.contains("# condicion: Ojos cerrados"));
        assert!(contenido.contains("# superficie: Espuma"));
        assert!(contenido.contains("t_s,cop_ml_cm,cop_ap_cm"));
        assert!(contenido.contains("0.5000,0.1000,-0.1000"));
        assert_eq!(contenido.lines().count(), 19, "15 líneas de metadata + encabezado + 3 filas");

        fs::remove_file(&ruta).ok();
    }
}
