//! Informe imprimible de una sesión.
//!
//! Lo que se archiva en una ficha clínica no es un CSV: es una hoja con los
//! datos del paciente, el trazo, la elipse y la tabla de métricas. El informe
//! se genera como un HTML autocontenido (sin imágenes externas ni scripts)
//! que se abre en el navegador y se imprime o se guarda como PDF desde ahí.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::datos::carpeta_datos;
use crate::estabilometria::{Condicion, MetricasBalance, Superficie, ajustar_elipse95};

/// Todo lo que entra en el informe.
pub struct DatosInforme<'a> {
    pub paciente: &'a str,
    pub fecha: &'a str,
    pub superficie: Superficie,
    pub condicion: Condicion,
    pub ancho_cm: f64,
    pub prof_cm: f64,
    pub metricas: &'a MetricasBalance,
    pub registro: &'a [[f64; 3]],
    /// Cocientes del CTSIB ya calculados, como (nombre, valor).
    pub cocientes: &'a [(String, f64)],
    pub version: &'a str,
}

/// Escapa el texto que se inserta en el HTML. El nombre del paciente lo
/// escribe una persona: sin escapar, un `<` en el campo rompería la página.
fn escapar(texto: &str) -> String {
    let mut salida = String::with_capacity(texto.len());
    for c in texto.chars() {
        match c {
            '&' => salida.push_str("&amp;"),
            '<' => salida.push_str("&lt;"),
            '>' => salida.push_str("&gt;"),
            '"' => salida.push_str("&quot;"),
            '\'' => salida.push_str("&#39;"),
            _ => salida.push(c),
        }
    }
    salida
}

/// Dibuja el trazo y la elipse como SVG, en las mismas coordenadas de la
/// plataforma que muestra la app.
fn svg_trazo(datos: &DatosInforme) -> String {
    const LADO: f64 = 360.0; // px del dibujo
    let x_lim = (datos.ancho_cm / 2.0).max(1.0);
    let y_lim = (datos.prof_cm / 2.0).max(1.0);
    // cm -> px, con el eje Y invertido (en SVG crece hacia abajo).
    let px = |x: f64, y: f64| (LADO / 2.0 + x / x_lim * LADO / 2.0, LADO / 2.0 - y / y_lim * LADO / 2.0);

    let puntos: String = datos
        .registro
        .iter()
        .map(|m| {
            let (x, y) = px(m[1], m[2]);
            format!("{x:.1},{y:.1}")
        })
        .collect::<Vec<_>>()
        .join(" ");

    let elipse = ajustar_elipse95(datos.registro)
        .map(|e| {
            let contorno: String = e
                .contorno(72)
                .iter()
                .map(|p| {
                    let (x, y) = px(p[0], p[1]);
                    format!("{x:.1},{y:.1}")
                })
                .collect::<Vec<_>>()
                .join(" ");
            format!(
                "<polygon points=\"{contorno}\" fill=\"rgba(168,146,214,0.18)\" stroke=\"#a892d6\" stroke-width=\"1.5\"/>"
            )
        })
        .unwrap_or_default();

    // Delimitador r##" porque el SVG contiene la secuencia `"#` de los colores.
    format!(
        r##"<svg viewBox="0 0 {LADO} {LADO}" width="{LADO}" height="{LADO}" role="img" aria-label="Trazo del centro de presión">
  <rect x="0" y="0" width="{LADO}" height="{LADO}" fill="#fcfdfe" stroke="#dcdfe4"/>
  <line x1="0" y1="{medio}" x2="{LADO}" y2="{medio}" stroke="#d5d8dd"/>
  <line x1="{medio}" y1="0" x2="{medio}" y2="{LADO}" stroke="#d5d8dd"/>
  {elipse}
  <polyline points="{puntos}" fill="none" stroke="#5a95d2" stroke-width="1.2"/>
</svg>"##,
        medio = LADO / 2.0
    )
}

fn fila(nombre: &str, valor: String) -> String {
    format!("<tr><th>{}</th><td>{}</td></tr>", escapar(nombre), escapar(&valor))
}

/// Arma el HTML completo del informe.
pub fn generar_html(datos: &DatosInforme) -> String {
    let m = datos.metricas;
    let metricas = [
        fila("Longitud del trazo", format!("{:.1} cm", m.longitud_cm)),
        fila("Área elipse 95%", format!("{:.1} cm²", m.area95_cm2)),
        fila("Velocidad media", format!("{:.2} cm/s", m.velocidad_media_cms)),
        fila("Velocidad ML / AP", format!("{:.2} / {:.2} cm/s", m.velocidad_ml_cms, m.velocidad_ap_cms)),
        fila("RMS ML / AP", format!("{:.2} / {:.2} cm", m.rms_ml_cm, m.rms_ap_cm)),
        fila("Rango ML / AP", format!("{:.1} / {:.1} cm", m.rango_ml_cm, m.rango_ap_cm)),
        fila("Frecuencia mediana ML / AP", format!("{:.2} / {:.2} Hz", m.frec_mediana_ml_hz, m.frec_mediana_ap_hz)),
        fila("F80 ML / AP", format!("{:.2} / {:.2} Hz", m.f80_ml_hz, m.f80_ap_hz)),
        fila("Duración registrada", format!("{:.1} s", m.duracion_s)),
    ]
    .join("\n      ");

    let cocientes = if datos.cocientes.is_empty() {
        String::new()
    } else {
        let filas: String = datos
            .cocientes
            .iter()
            .map(|(nombre, valor)| fila(nombre, format!("{valor:.2}x")))
            .collect::<Vec<_>>()
            .join("\n      ");
        format!("<h2>Cocientes CTSIB</h2>\n    <table>\n      {filas}\n    </table>")
    };

    format!(
        r##"<!DOCTYPE html>
<html lang="es">
<head>
<meta charset="utf-8">
<title>Informe posturográfico — {paciente}</title>
<style>
  :root {{ color-scheme: light; }}
  body {{ font-family: system-ui, -apple-system, "Segoe UI", sans-serif; color: #2c3038;
         margin: 0 auto; padding: 28px; max-width: 800px; }}
  h1 {{ font-size: 1.4rem; margin: 0 0 4px; }}
  h2 {{ font-size: 1rem; text-transform: uppercase; letter-spacing: .06em;
        color: #5a95d2; margin: 24px 0 8px; }}
  .sub {{ color: #6b7280; margin: 0 0 20px; }}
  table {{ border-collapse: collapse; width: 100%; }}
  th, td {{ text-align: left; padding: 6px 8px; border-bottom: 1px solid #e6e8ec; }}
  th {{ font-weight: 600; color: #4b5563; width: 55%; }}
  .grafico {{ margin: 12px 0; }}
  footer {{ margin-top: 28px; color: #9ca3af; font-size: .8rem; }}
  @media print {{ body {{ padding: 0; }} }}
</style>
</head>
<body>
  <h1>Informe posturográfico</h1>
  <p class="sub">{paciente} · {fecha}</p>

  <h2>Condiciones del examen</h2>
  <table>
      {condiciones}
  </table>

  <h2>Centro de presión</h2>
  <div class="grafico">{svg}</div>

  <h2>Métricas</h2>
  <table>
      {metricas}
  </table>

  {cocientes}

  <footer>Generado por Posturografox {version}. Los valores dependen de la calibración
  de la plataforma; interpretar junto al resto de la evaluación clínica.</footer>
</body>
</html>
"##,
        paciente = escapar(if datos.paciente.trim().is_empty() { "Sin identificar" } else { datos.paciente.trim() }),
        fecha = escapar(datos.fecha),
        condiciones = [
            fila("Superficie", datos.superficie.etiqueta().to_string()),
            fila("Condición visual", datos.condicion.etiqueta().to_string()),
            fila("Plataforma", format!("{:.0} × {:.0} cm", datos.ancho_cm, datos.prof_cm)),
            fila("Muestras registradas", datos.registro.len().to_string()),
        ]
        .join("\n      "),
        svg = svg_trazo(datos),
        version = escapar(datos.version),
    )
}

/// Escribe el informe en la carpeta de informes del usuario.
pub fn escribir(datos: &DatosInforme) -> io::Result<PathBuf> {
    escribir_en(&carpeta_datos().join("informes"), datos)
}

pub fn escribir_en(carpeta: &Path, datos: &DatosInforme) -> io::Result<PathBuf> {
    fs::create_dir_all(carpeta)?;
    let identificador: String =
        datos.paciente.trim().chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect::<String>();
    let identificador = if identificador.is_empty() { "anonimo".to_string() } else { identificador };
    let ruta = carpeta.join(format!(
        "informe_{identificador}_{}_{}.html",
        datos.superficie.slug(),
        datos.fecha.replace([':', ' ', '-'], "")
    ));
    fs::write(&ruta, generar_html(datos))?;
    Ok(ruta)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn datos_de_prueba<'a>(
        paciente: &'a str,
        metricas: &'a MetricasBalance,
        registro: &'a [[f64; 3]],
        cocientes: &'a [(String, f64)],
    ) -> DatosInforme<'a> {
        DatosInforme {
            paciente,
            fecha: "2026-09-17 10:30",
            superficie: Superficie::Espuma,
            condicion: Condicion::OjosCerrados,
            ancho_cm: 40.0,
            prof_cm: 40.0,
            metricas,
            registro,
            cocientes,
            version: "0.1.0",
        }
    }

    fn registro_de_prueba() -> Vec<[f64; 3]> {
        (0..100).map(|i| [i as f64 / 80.0, (i as f64 * 0.1).sin(), (i as f64 * 0.07).cos()]).collect()
    }

    #[test]
    fn el_informe_trae_los_datos_del_examen_y_las_metricas() {
        let metricas = MetricasBalance { area95_cm2: 4.25, velocidad_media_cms: 1.75, ..MetricasBalance::default() };
        let registro = registro_de_prueba();
        let html = generar_html(&datos_de_prueba("ID-42", &metricas, &registro, &[]));

        assert!(html.contains("ID-42"));
        assert!(html.contains("Ojos cerrados"));
        assert!(html.contains("Espuma"));
        assert!(html.contains("4.2 cm²") || html.contains("4.3 cm²"));
        assert!(html.contains("1.75 cm/s"));
        assert!(html.contains("<polyline"), "debería incluir el trazo dibujado");
        assert!(html.contains("<polygon"), "debería incluir la elipse");
    }

    #[test]
    fn el_nombre_del_paciente_no_puede_romper_el_html() {
        let metricas = MetricasBalance::default();
        let registro = registro_de_prueba();
        let html = generar_html(&datos_de_prueba("<script>alert(1)</script>", &metricas, &registro, &[]));
        assert!(!html.contains("<script>alert"), "el nombre se tiene que escapar");
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn sin_paciente_el_informe_igual_se_genera() {
        let metricas = MetricasBalance::default();
        let registro = registro_de_prueba();
        let html = generar_html(&datos_de_prueba("   ", &metricas, &registro, &[]));
        assert!(html.contains("Sin identificar"));
    }

    #[test]
    fn los_cocientes_ctsib_aparecen_cuando_los_hay() {
        let metricas = MetricasBalance::default();
        let registro = registro_de_prueba();
        let cocientes = vec![("Romberg firme".to_string(), 2.5)];
        let html = generar_html(&datos_de_prueba("ID-1", &metricas, &registro, &cocientes));
        assert!(html.contains("Cocientes CTSIB"));
        assert!(html.contains("2.50x"));

        let sin = generar_html(&datos_de_prueba("ID-1", &metricas, &registro, &[]));
        assert!(!sin.contains("Cocientes CTSIB"));
    }

    #[test]
    fn el_informe_se_escribe_en_la_carpeta_pedida() {
        let carpeta = tempfile::tempdir().unwrap();
        let metricas = MetricasBalance::default();
        let registro = registro_de_prueba();
        let ruta = escribir_en(carpeta.path(), &datos_de_prueba("Ana Pérez", &metricas, &registro, &[])).unwrap();

        assert!(ruta.exists());
        assert!(ruta.extension().unwrap() == "html");
        assert!(fs::read_to_string(&ruta).unwrap().contains("Ana Pérez"));
    }
}
