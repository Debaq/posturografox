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
use crate::historial::DatosJuego;

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
    /// Última partida del modo juego, si hubo una en esta sesión.
    pub juego: Option<&'a DatosJuego>,
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

    let juego = datos.juego.map(bloque_juego).unwrap_or_default();
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

  {juego}

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

/// Sección del modo juego. Imprime la definición de la latencia y cuántas
/// maniobras válidas hubo sobre el total: sin eso la cifra sería una caja
/// negra, y quien lea el informe no puede saber si hablan de tres maniobras
/// o de cuarenta.
fn bloque_juego(juego: &DatosJuego) -> String {
    let mut filas = vec![
        fila("Duración de la partida", format!("{:.0} s", juego.duracion_s)),
        fila("Resultado", if juego.gano { "completó el tiempo".into() } else { "terminó antes".to_string() }),
        fila("Exigencia", format!("{:.0}% del alcance", juego.exigencia * 100.0)),
        fila(
            "Alcance usado",
            format!(
                "ML {:.1}/{:.1} cm · AP {:.1}/{:.1} cm ({})",
                juego.rango.ml.alcance_negativo_cm(),
                juego.rango.ml.alcance_positivo_cm(),
                juego.rango.ap.alcance_negativo_cm(),
                juego.rango.ap.alcance_positivo_cm(),
                juego.rango.origen.etiqueta()
            ),
        ),
    ];
    let nota = match &juego.resumen {
        None => {
            filas.push(fila("Maniobras medibles", "ninguna".to_string()));
            "No quedaron maniobras comparables: se descartan las rocas que no exigían desplazamiento, \
             las que caen en un congelamiento por golpe, las que se solapan con otra y las respuestas \
             fuera de la ventana plausible de reacción."
                .to_string()
        }
        Some(r) => {
            filas.push(fila("Maniobras válidas", format!("{} de {}", r.validas, r.total)));
            filas.push(fila(
                "Latencia mediana izq / der",
                format!(
                    "{:.0} / {:.0} ms ({} / {} maniobras)",
                    r.latencia_mediana_izq_s * 1000.0,
                    r.latencia_mediana_der_s * 1000.0,
                    r.validas_izquierda,
                    r.validas_derecha
                ),
            ));
            filas.push(fila(
                "Velocidad pico mediana izq / der",
                format!("{:.1} / {:.1} cm/s", r.velocidad_pico_mediana_izq_cms, r.velocidad_pico_mediana_der_cms),
            ));
            filas.push(fila(
                "Amplitud mediana izq / der",
                format!(
                    "{:.0}% / {:.0}% del alcance propio",
                    r.fraccion_alcance_mediana_izq * 100.0,
                    r.fraccion_alcance_mediana_der * 100.0
                ),
            ));
            filas.push(fila("Asimetría", format!("{:+.2}", r.asimetria)));
            filas.push(fila("Control direccional", format!("{:.0}%", r.control_direccional * 100.0)));
            "Latencia: desde que el obstáculo entra a 1.2 s del contacto hasta el primer desplazamiento \
             medio-lateral sobre el umbral de velocidad, haya salido hacia donde haya salido; el acierto \
             de dirección se informa aparte. La amplitud se expresa como fracción del alcance calibrado \
             del paciente, que es lo que la vuelve comparable entre sesiones."
                .to_string()
        }
    };
    format!(
        "<h2>Modo juego</h2>\n    <table>\n      {}\n    </table>\n    <p class=\"sub\">{}</p>",
        filas.join("\n      "),
        escapar(&nota)
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
            juego: None,
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

    fn partida_de_prueba(resumen: Option<crate::maniobras::Resumen>) -> DatosJuego {
        DatosJuego {
            rango: crate::rango::RangoCalibrado::por_defecto(40.0, 40.0),
            exigencia: 0.7,
            duracion_s: 120.0,
            gano: true,
            resumen,
        }
    }

    #[test]
    fn el_informe_de_una_partida_dice_como_se_mide_la_latencia() {
        let metricas = MetricasBalance::default();
        let registro: Vec<[f64; 3]> = Vec::new();
        let cocientes: Vec<(String, f64)> = Vec::new();
        let partida = partida_de_prueba(Some(crate::maniobras::Resumen {
            validas: 34,
            total: 41,
            validas_izquierda: 17,
            validas_derecha: 17,
            latencia_mediana_izq_s: 0.31,
            latencia_mediana_der_s: 0.28,
            velocidad_pico_mediana_izq_cms: 8.0,
            velocidad_pico_mediana_der_cms: 9.0,
            fraccion_alcance_mediana_izq: 0.6,
            fraccion_alcance_mediana_der: 0.7,
            asimetria: 0.08,
            control_direccional: 0.9,
        }));
        let mut datos = datos_de_prueba("ID-1", &metricas, &registro, &cocientes);
        datos.juego = Some(&partida);

        let html = generar_html(&datos);

        assert!(html.contains("Modo juego"));
        assert!(html.contains("34 de 41"), "hay que decir cuántas maniobras válidas sobre el total");
        assert!(html.contains("1.2 s del contacto"), "la definición de la latencia tiene que estar escrita");
    }

    #[test]
    fn una_partida_sin_maniobras_medibles_lo_dice() {
        let metricas = MetricasBalance::default();
        let registro: Vec<[f64; 3]> = Vec::new();
        let cocientes: Vec<(String, f64)> = Vec::new();
        let partida = partida_de_prueba(None);
        let mut datos = datos_de_prueba("ID-1", &metricas, &registro, &cocientes);
        datos.juego = Some(&partida);

        let html = generar_html(&datos);

        assert!(html.contains("ninguna"));
        assert!(!html.contains("Latencia mediana"), "sin datos no se inventa una cifra");
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
