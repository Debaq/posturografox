//! Fechas para mostrar: de lo que guarda la base a lo que lee una persona.
//!
//! La base guarda ISO 8601 **en UTC** (ver [`crate::pacientes::ahora_iso`]), que
//! es lo correcto para almacenar y ordenar y es ilegible en pantalla: nadie
//! quiere ver `2026-09-19T13:14:29Z` y restarle tres horas de cabeza para saber
//! si fue esta mañana.
//!
//! El formato de salida es `AAAA-MM-DD HH:MM` en hora local. No es una elección
//! estética: `19/09/2026` y `09/19/2026` son la misma cadena leída de dos
//! maneras. El orden ISO no se puede malinterpretar y además ordena
//! alfabéticamente.
//!
//! Se usa jiff y no una cuenta a mano porque la conversión a hora local necesita
//! la base de zonas horarias del sistema, y jiff ya entra por el selector de
//! fecha de `egui_extras`.

use jiff::civil::Date;
use jiff::{Timestamp, Zoned};

/// Marca de tiempo completa en hora local: `2026-09-19 10:14`.
///
/// Una cadena que no se puede interpretar se devuelve **tal cual**: es una base
/// que puede tener registros de otras versiones, y mostrar el texto crudo es
/// mejor que mostrar una fecha que solo parece una fecha.
pub fn local(iso: &str) -> String {
    match zonificar(iso) {
        Some(z) => z.strftime("%Y-%m-%d %H:%M").to_string(),
        None => iso.to_string(),
    }
}

/// Solo el día, en hora local: `2026-09-19`.
pub fn local_dia(iso: &str) -> String {
    match zonificar(iso) {
        Some(z) => z.strftime("%Y-%m-%d").to_string(),
        None => iso.to_string(),
    }
}

fn zonificar(iso: &str) -> Option<Zoned> {
    let t: Timestamp = iso.parse().ok()?;
    Some(t.in_tz("UTC").ok()?.with_time_zone(jiff::tz::TimeZone::system()))
}

/// El año más viejo que se acepta como fecha de nacimiento.
///
/// No es una restricción arbitraria: `%Y` acepta un año de dos dígitos, así que
/// `05/02/80` se parsea **sin error** como el año 80 d.C. Un año así entra por
/// un dedazo, no por un paciente, y sin este piso se guardaría en silencio y la
/// ficha mostraría una edad de casi dos mil años.
const ANIO_MINIMO: i16 = 1900;

/// Una fecha de nacimiento escrita a mano, si se entiende.
///
/// Acepta el `AAAA-MM-DD` que escribe el selector y también el `DD/MM/AAAA` que
/// escribe la gente cuando escribe. Lo demás se deja como está: no se adivina.
pub fn parse_dia(s: &str) -> Option<Date> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let d = Date::strptime("%Y-%m-%d", s).or_else(|_| Date::strptime("%d/%m/%Y", s)).ok()?;
    (d.year() >= ANIO_MINIMO).then_some(d)
}

/// Cómo se guarda una fecha elegida en el selector.
pub fn fmt_dia(d: Date) -> String {
    d.strftime("%Y-%m-%d").to_string()
}

pub fn hoy() -> Date {
    Zoned::now().date()
}

/// Una fecha plausible para empezar a elegir una de nacimiento.
///
/// No es hoy: nadie que se estudie nació hoy, y desde hoy hay que retroceder
/// cuarenta años a mano, mes por mes.
pub fn hoy_menos_40() -> Date {
    let hoy = hoy();
    Date::new(hoy.year() - 40, hoy.month(), hoy.day()).unwrap_or(hoy)
}

/// Edad en años cumplidos a una fecha dada.
///
/// `None` si la fecha de nacimiento está vacía, no se entiende, o **es
/// posterior** a la de referencia: una edad negativa en una ficha clínica es un
/// error de carga, y esconderlo detrás de un "0" lo hace más difícil de notar.
pub fn edad(nacimiento: &str, al_dia: Date) -> Option<i32> {
    let n = parse_dia(nacimiento)?;
    if n > al_dia {
        return None;
    }
    let mut anios = i32::from(al_dia.year() - n.year());
    // Todavía no cumplió este año.
    if (al_dia.month(), al_dia.day()) < (n.month(), n.day()) {
        anios -= 1;
    }
    Some(anios)
}

/// Días desde el epoch, para usar una fecha como eje X de un gráfico.
///
/// Fraccionario a propósito: dos exámenes del mismo día tienen que caer en
/// puntos distintos, o la curva de evolución los superpone.
pub fn dias_epoch(iso: &str) -> Option<f64> {
    let t: Timestamp = iso.parse().ok()?;
    Some(t.as_second() as f64 / 86_400.0)
}

/// La inversa de [`dias_epoch`], para etiquetar el eje.
pub fn dia_desde_epoch(dias: f64) -> String {
    let segundos = (dias * 86_400.0).round() as i64;
    match Timestamp::from_second(segundos) {
        Ok(t) => t.to_zoned(jiff::tz::TimeZone::system()).strftime("%Y-%m-%d").to_string(),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dia(a: i16, m: i8, d: i8) -> Date {
        Date::new(a, m, d).unwrap()
    }

    #[test]
    fn la_edad_cuenta_anios_cumplidos() {
        let hoy = dia(2026, 9, 19);
        assert_eq!(edad("1980-05-02", hoy), Some(46));
        // Cumple mañana: todavía tiene la de antes.
        assert_eq!(edad("1980-09-20", hoy), Some(45));
        // Cumple hoy.
        assert_eq!(edad("1980-09-19", hoy), Some(46));
    }

    #[test]
    fn una_fecha_que_no_se_entiende_no_da_edad() {
        let hoy = dia(2026, 9, 19);
        assert_eq!(edad("", hoy), None);
        assert_eq!(edad("cuando era chico", hoy), None);
        assert_eq!(edad("1980-13-45", hoy), None);
        // Y una posterior a hoy es un error de carga, no una edad negativa.
        assert_eq!(edad("2030-01-01", hoy), None);
    }

    #[test]
    fn se_acepta_la_fecha_como_la_escribe_la_gente() {
        assert_eq!(parse_dia("1980-05-02"), Some(dia(1980, 5, 2)));
        assert_eq!(parse_dia("02/05/1980"), Some(dia(1980, 5, 2)));
        assert_eq!(parse_dia("  1980-05-02  "), Some(dia(1980, 5, 2)));
    }

    #[test]
    fn un_anio_de_dos_digitos_no_pasa_por_una_fecha() {
        // Sin el piso de plausibilidad, un dedazo se guardaba como fecha válida
        // y la ficha mostraba una edad de casi dos mil años.
        assert_eq!(parse_dia("05/02/80"), None);
        assert_eq!(parse_dia("80-05-02"), None);
        // Y el límite se respeta por los dos lados.
        assert_eq!(parse_dia("1899-12-31"), None);
        assert_eq!(parse_dia("1900-01-01"), Some(dia(1900, 1, 1)));
    }

    #[test]
    fn una_marca_ilegible_se_muestra_cruda() {
        assert_eq!(local("vaya a saber"), "vaya a saber");
        assert_eq!(local_dia(""), "");
    }

    #[test]
    fn la_marca_utc_se_convierte_y_vuelve() {
        // El día del eje tiene que ser el mismo que muestra la tabla, o dos
        // vistas del mismo examen dirían fechas distintas.
        let iso = "2026-09-19T13:14:29Z";
        let dias = dias_epoch(iso).expect("es una fecha válida");
        assert_eq!(dia_desde_epoch(dias), local_dia(iso));
    }

    #[test]
    fn dos_examenes_del_mismo_dia_caen_en_puntos_distintos() {
        let a = dias_epoch("2026-09-19T09:00:00Z").unwrap();
        let b = dias_epoch("2026-09-19T15:00:00Z").unwrap();
        assert!(b > a, "{a} vs {b}");
    }

    #[test]
    fn el_selector_arranca_en_una_fecha_de_adulto() {
        assert_eq!(hoy_menos_40().year(), hoy().year() - 40);
    }
}
