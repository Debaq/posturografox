//! Métricas de maniobra: qué hizo el paciente con su peso ante cada roca.
//!
//! El puntaje del juego es jugabilidad —sube solo con la velocidad, que sube
//! sola con el tiempo—, así que no sirve como dato clínico. Lo que sirve es
//! el COP que la app graba mientras el paciente juega, cruzado con los
//! eventos de la partida (`juego::EventoRoca`): cada roca es un ensayo de
//! desplazamiento de carga con dirección conocida y momento de estímulo
//! conocido.
//!
//! De ahí salen las cuatro dimensiones del test de límites de estabilidad
//! —latencia de reacción, velocidad del desplazamiento, amplitud alcanzada y
//! control direccional—, medidas decenas de veces por partida en vez de las
//! ocho del examen guiado.

use crate::juego::EventoRoca;
use crate::rango::{EXIGENCIA_MAX, EXIGENCIA_MIN, RangoCalibrado};

/// Velocidad ML a partir de la cual se considera que el paciente arrancó la
/// maniobra. Por debajo de esto es oscilación de estar parado, no respuesta.
const UMBRAL_VELOCIDAD_CMS: f64 = 2.0;

/// Latencias fuera de esta ventana no son reacciones: por debajo es
/// anticipación (venía moviéndose o adivinó), por encima es no haber
/// respondido al estímulo.
const LATENCIA_MIN_S: f64 = 0.1;
const LATENCIA_MAX_S: f64 = 1.5;

/// Cuántas muestras a cada lado se usan para derivar la velocidad. A 80 SPS
/// son ±25 ms: suficiente para no amplificar el ruido de una sola muestra y
/// poco para no arrastrar la latencia.
const SEMIVENTANA: usize = 2;

/// Por qué una roca no cuenta como maniobra medible.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Descarte {
    /// El paciente ya estaba fuera de la trayectoria: la roca no pedía nada.
    NoExigia,
    /// Cayó dentro del congelamiento por golpe.
    Congelada,
    /// Su ventana se solapa con la de otra roca: no se sabe a cuál responde.
    Solapada,
    /// Ya venía moviéndose cuando apareció el estímulo: es continuación, no
    /// reacción.
    YaEnMovimiento,
    /// Nunca cruzó el umbral de velocidad, o lo hizo fuera de la ventana
    /// plausible de reacción.
    SinRespuesta,
    /// No hay suficientes muestras de COP en ese tramo.
    SinDatos,
}

/// Una roca medida contra el COP grabado.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Maniobra {
    /// Hacia dónde había que ir: -1.0 izquierda, 1.0 derecha.
    pub lado: f32,
    /// Del estímulo al primer desplazamiento sobre el umbral de velocidad,
    /// haya salido hacia donde haya salido: arrancar para el lado equivocado
    /// es una reacción igual, y el error queda en `direccion_correcta`. Si la
    /// latencia solo contara los arranques correctos, las maniobras mal
    /// dirigidas desaparecerían del registro justo por estar mal.
    pub latencia_s: f64,
    /// Velocidad ML máxima durante la maniobra, proyectada sobre el lado
    /// pedido: negativa si se fue para el otro lado.
    pub velocidad_pico_cms: f64,
    /// Cuánto se desplazó hacia el lado pedido, desde donde estaba. Negativo
    /// si terminó más lejos del lado que le pedían.
    pub amplitud_cm: f64,
    /// Esa amplitud como fracción del alcance calibrado de ese lado. Es la
    /// forma comparable entre sesiones y entre pacientes.
    pub fraccion_alcance: f64,
    /// Si el primer movimiento fue hacia el lado correcto.
    pub direccion_correcta: bool,
    pub golpeo: bool,
}

/// Lo que se puede decir de una partida entera.
#[derive(Clone, Copy, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct Resumen {
    pub validas: usize,
    pub total: usize,
    pub validas_izquierda: usize,
    pub validas_derecha: usize,
    pub latencia_mediana_izq_s: f64,
    pub latencia_mediana_der_s: f64,
    pub velocidad_pico_mediana_izq_cms: f64,
    pub velocidad_pico_mediana_der_cms: f64,
    pub fraccion_alcance_mediana_izq: f64,
    pub fraccion_alcance_mediana_der: f64,
    /// Cuánto más lejos llega hacia un lado que hacia el otro, de -1.0
    /// (todo a la izquierda) a 1.0 (todo a la derecha). Cerca de 0 es
    /// simétrico.
    pub asimetria: f64,
    /// Fracción de maniobras que arrancaron hacia el lado correcto.
    pub control_direccional: f64,
}

/// Mide cada roca contra el COP grabado. `registro` son las muestras
/// `[t_s, cop_ml_cm, cop_ap_cm]` de la sesión, en orden.
pub fn analizar(
    eventos: &[EventoRoca],
    registro: &[[f64; 3]],
    rango: &RangoCalibrado,
) -> Vec<Result<Maniobra, Descarte>> {
    eventos.iter().map(|evento| medir(evento, eventos, registro, rango)).collect()
}

fn medir(
    evento: &EventoRoca,
    todos: &[EventoRoca],
    registro: &[[f64; 3]],
    rango: &RangoCalibrado,
) -> Result<Maniobra, Descarte> {
    if evento.lado_exigido == 0.0 {
        return Err(Descarte::NoExigia);
    }
    if evento.congelado {
        return Err(Descarte::Congelada);
    }
    if se_solapa(evento, todos) {
        return Err(Descarte::Solapada);
    }

    let desde = indice_en(registro, evento.t_zona_s);
    let hasta = indice_en(registro, evento.t_resultado_s);
    if hasta <= desde || hasta - desde < SEMIVENTANA * 2 + 1 {
        return Err(Descarte::SinDatos);
    }

    let signo = f64::from(evento.lado_exigido);
    // Todo se mide proyectado sobre la dirección pedida, así una maniobra
    // hacia la izquierda y otra hacia la derecha se comparan con la misma
    // cuenta: "más" siempre es ir hacia donde había que ir.
    let ml_inicial = registro[desde][1];
    let velocidad_en = |i: usize| -> Option<f64> {
        let (a, b) = (i.checked_sub(SEMIVENTANA)?, i + SEMIVENTANA);
        let (antes, despues) = (registro.get(a)?, registro.get(b)?);
        let dt = despues[0] - antes[0];
        (dt > 0.0).then(|| (despues[1] - antes[1]) / dt * signo)
    };

    // Si en el instante del estímulo ya venía moviéndose hacia el lado
    // pedido, lo que siga es continuación y no una reacción.
    if velocidad_en(desde).is_some_and(|v| v.abs() >= UMBRAL_VELOCIDAD_CMS) {
        return Err(Descarte::YaEnMovimiento);
    }

    let mut latencia_s = None;
    let mut primer_movimiento = 0.0;
    let mut velocidad_pico_cms = f64::NEG_INFINITY;
    let mut amplitud_cm = f64::NEG_INFINITY;
    for (i, muestra) in registro.iter().enumerate().take(hasta).skip(desde) {
        let Some(v) = velocidad_en(i) else { continue };
        if latencia_s.is_none() && v.abs() >= UMBRAL_VELOCIDAD_CMS {
            latencia_s = Some(muestra[0] - evento.t_zona_s);
            primer_movimiento = v;
        }
        velocidad_pico_cms = velocidad_pico_cms.max(v);
        amplitud_cm = amplitud_cm.max((muestra[1] - ml_inicial) * signo);
    }

    let latencia_s = latencia_s.ok_or(Descarte::SinRespuesta)?;
    if !(LATENCIA_MIN_S..=LATENCIA_MAX_S).contains(&latencia_s) {
        return Err(Descarte::SinRespuesta);
    }

    let alcance = if signo > 0.0 { rango.ml.alcance_positivo_cm() } else { rango.ml.alcance_negativo_cm() };
    Ok(Maniobra {
        lado: evento.lado_exigido,
        latencia_s,
        velocidad_pico_cms,
        amplitud_cm,
        fraccion_alcance: if alcance > 0.0 { amplitud_cm / alcance } else { 0.0 },
        direccion_correcta: primer_movimiento > 0.0,
        golpeo: evento.golpeo,
    })
}

/// Dos rocas cuyas ventanas se pisan dejan sin saber a cuál responde el
/// movimiento, así que no se puede medir ninguna de las dos.
fn se_solapa(evento: &EventoRoca, todos: &[EventoRoca]) -> bool {
    todos.iter().any(|otro| {
        otro.t_zona_s != evento.t_zona_s && otro.t_zona_s < evento.t_resultado_s && evento.t_zona_s < otro.t_resultado_s
    })
}

/// Primer índice del registro cuyo tiempo llega a `t`. El registro viene
/// ordenado, así que basta una búsqueda binaria.
fn indice_en(registro: &[[f64; 3]], t: f64) -> usize {
    registro.partition_point(|m| m[0] < t)
}

/// Junta las maniobras medidas de una partida. `None` si no quedó ninguna
/// válida: un resumen de cero maniobras diría cualquier cosa.
pub fn resumir(medidas: &[Result<Maniobra, Descarte>]) -> Option<Resumen> {
    let validas: Vec<Maniobra> = medidas.iter().filter_map(|m| m.as_ref().ok().copied()).collect();
    if validas.is_empty() {
        return None;
    }
    let de_lado =
        |signo: f32| -> Vec<Maniobra> { validas.iter().copied().filter(|m| m.lado.signum() == signo).collect() };
    let izq = de_lado(-1.0);
    let der = de_lado(1.0);
    let mediana_de = |v: &[Maniobra], f: fn(&Maniobra) -> f64| mediana(&v.iter().map(f).collect::<Vec<_>>());

    let amplitud_izq = mediana_de(&izq, |m| m.fraccion_alcance);
    let amplitud_der = mediana_de(&der, |m| m.fraccion_alcance);
    let suma = amplitud_izq + amplitud_der;
    Some(Resumen {
        validas: validas.len(),
        total: medidas.len(),
        validas_izquierda: izq.len(),
        validas_derecha: der.len(),
        latencia_mediana_izq_s: mediana_de(&izq, |m| m.latencia_s),
        latencia_mediana_der_s: mediana_de(&der, |m| m.latencia_s),
        velocidad_pico_mediana_izq_cms: mediana_de(&izq, |m| m.velocidad_pico_cms),
        velocidad_pico_mediana_der_cms: mediana_de(&der, |m| m.velocidad_pico_cms),
        fraccion_alcance_mediana_izq: amplitud_izq,
        fraccion_alcance_mediana_der: amplitud_der,
        asimetria: if suma > 0.0 { (amplitud_der - amplitud_izq) / suma } else { 0.0 },
        control_direccional: validas.iter().filter(|m| m.direccion_correcta).count() as f64 / validas.len() as f64,
    })
}

/// Cuántas maniobras válidas hacen falta de cada lado para decir algo. Con
/// menos, la comparación entre lados es ruido.
const MINIMO_POR_LADO: usize = 10;

/// Control direccional por debajo del cual el problema no es la dosis: si la
/// mitad de las veces arranca para el lado equivocado, subir la exigencia no
/// entrena, frustra.
const CONTROL_MINIMO: f64 = 0.5;

/// Qué hacer con la exigencia en la próxima sesión.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Sugerencia {
    /// Cubrió holgado lo que se le pedía: se puede pedir más.
    Subir(f64),
    Mantener,
    /// No llegó a lo que se le pedía: pedir menos.
    Bajar(f64),
}

/// Por qué no se puede sugerir nada. Se muestra en vez de esconder la
/// sugerencia: el evaluador tiene que saber si falta dato o si el dato dice
/// que está bien así.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SinSugerencia {
    /// Se jugó con el rango por defecto: las amplitudes no son de la persona.
    SinCalibrar,
    /// Faltan maniobras válidas de alguno de los dos lados.
    PocasManiobras { izquierda: usize, derecha: usize },
}

const PASO_EXIGENCIA: f64 = 0.05;

/// Propone la exigencia de la próxima partida a partir de lo que el paciente
/// hizo, no del puntaje: el puntaje sube solo con la velocidad del juego.
///
/// La referencia es la amplitud que efectivamente usó contra la que se le
/// pedía: a exigencia `e`, llegar al borde de la pista exige cubrir esa
/// fracción de su alcance. Si la cubre de sobra, se puede pedir más.
pub fn sugerir_exigencia(resumen: &Resumen, exigencia: f64, calibrado: bool) -> Result<Sugerencia, SinSugerencia> {
    if !calibrado {
        return Err(SinSugerencia::SinCalibrar);
    }
    if resumen.validas_izquierda < MINIMO_POR_LADO || resumen.validas_derecha < MINIMO_POR_LADO {
        return Err(SinSugerencia::PocasManiobras {
            izquierda: resumen.validas_izquierda,
            derecha: resumen.validas_derecha,
        });
    }

    let cubierto = resumen.fraccion_alcance_mediana_izq.min(resumen.fraccion_alcance_mediana_der);
    // Redondeado al 1%: la exigencia se muestra y se elige en porcentaje, y
    // un 0.6499999 arrastrado por la suma en coma flotante sería ruido.
    let al_uno_por_ciento = |v: f64| (v * 100.0).round() / 100.0;
    let subir = al_uno_por_ciento((exigencia + PASO_EXIGENCIA).min(EXIGENCIA_MAX));
    let bajar = al_uno_por_ciento((exigencia - PASO_EXIGENCIA).max(EXIGENCIA_MIN));
    // El lado peor es el que manda: subir la dosis por el lado bueno dejaría
    // el hemicuerpo afectado sin poder esquivar nada.
    if resumen.control_direccional < CONTROL_MINIMO || cubierto < exigencia * 0.6 {
        return Ok(if bajar < exigencia { Sugerencia::Bajar(bajar) } else { Sugerencia::Mantener });
    }
    if cubierto >= exigencia && resumen.control_direccional >= 0.8 {
        return Ok(if subir > exigencia { Sugerencia::Subir(subir) } else { Sugerencia::Mantener });
    }
    Ok(Sugerencia::Mantener)
}

/// Mediana y no promedio: una maniobra en la que el paciente se distrajo
/// arrastra la media y deja de describir a las demás.
fn mediana(valores: &[f64]) -> f64 {
    if valores.is_empty() {
        return 0.0;
    }
    let mut v = valores.to_vec();
    v.sort_by(f64::total_cmp);
    let medio = v.len() / 2;
    if v.len().is_multiple_of(2) { (v[medio - 1] + v[medio]) / 2.0 } else { v[medio] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rango::{Eje, Origen};

    const PERIODO: f64 = 1.0 / 80.0; // el firmware muestrea a 80 SPS

    fn rango() -> RangoCalibrado {
        RangoCalibrado::medido(Eje::nuevo(0.0, -5.0, 5.0, 20.0), Eje::nuevo(0.0, -4.0, 8.0, 20.0), Origen::Juego)
            .expect("rango utilizable")
    }

    fn evento(t_zona_s: f64, lado_exigido: f32) -> EventoRoca {
        EventoRoca {
            t_aparicion_s: t_zona_s - 1.0,
            t_zona_s,
            t_resultado_s: t_zona_s + 1.2,
            x_roca: 0.0,
            fox_en_zona: 0.0,
            lado_exigido,
            golpeo: false,
            congelado: false,
        }
    }

    /// Registro sintético: el COP se queda quieto `espera_s` desde el
    /// estímulo y después se desplaza `amplitud_cm` hacia `signo` a
    /// velocidad constante durante 0.4 s.
    fn registro(t_zona_s: f64, espera_s: f64, amplitud_cm: f64, signo: f64) -> Vec<[f64; 3]> {
        let inicio = t_zona_s - 1.0;
        let arranque = t_zona_s + espera_s;
        let rampa_s = 0.4;
        let mut muestras = Vec::new();
        let mut t = inicio;
        while t < t_zona_s + 1.3 {
            let avance = ((t - arranque) / rampa_s).clamp(0.0, 1.0);
            muestras.push([t, signo * amplitud_cm * avance, 0.0]);
            t += PERIODO;
        }
        muestras
    }

    #[test]
    fn una_maniobra_limpia_da_latencia_velocidad_y_amplitud() {
        let e = evento(10.0, 1.0);
        let reg = registro(10.0, 0.3, 4.0, 1.0);

        let m = medir(&e, &[e], &reg, &rango()).expect("maniobra válida");

        assert!((m.latencia_s - 0.3).abs() < 0.05, "latencia {}", m.latencia_s);
        assert!((m.amplitud_cm - 4.0).abs() < 0.1, "amplitud {}", m.amplitud_cm);
        // 4 cm en 0.4 s son 10 cm/s.
        assert!((m.velocidad_pico_cms - 10.0).abs() < 0.5, "velocidad {}", m.velocidad_pico_cms);
        // El alcance derecho calibrado es 5 cm.
        assert!((m.fraccion_alcance - 0.8).abs() < 0.03);
        assert!(m.direccion_correcta);
    }

    #[test]
    fn la_amplitud_se_informa_contra_el_alcance_del_lado_que_toca() {
        // Mismos 4 cm hacia la izquierda, donde el alcance calibrado es otro.
        let e = evento(10.0, -1.0);
        let reg = registro(10.0, 0.3, 4.0, -1.0);
        let rango =
            RangoCalibrado::medido(Eje::nuevo(0.0, -8.0, 5.0, 20.0), Eje::nuevo(0.0, -4.0, 8.0, 20.0), Origen::Juego)
                .expect("rango utilizable");

        let m = medir(&e, &[e], &reg, &rango).expect("maniobra válida");

        assert!((m.amplitud_cm - 4.0).abs() < 0.1);
        assert!((m.fraccion_alcance - 0.5).abs() < 0.03, "4 de 8 cm es la mitad de su alcance izquierdo");
    }

    #[test]
    fn arrancar_para_el_lado_equivocado_queda_anotado() {
        let e = evento(10.0, 1.0);
        // Le pedían derecha y arrancó a la izquierda.
        let reg = registro(10.0, 0.3, 4.0, -1.0);

        let m = medir(&e, &[e], &reg, &rango()).expect("maniobra válida");
        assert!(!m.direccion_correcta);
    }

    #[test]
    fn la_roca_que_no_exigia_nada_no_es_maniobra() {
        let e = evento(10.0, 0.0);
        assert_eq!(medir(&e, &[e], &registro(10.0, 0.3, 4.0, 1.0), &rango()), Err(Descarte::NoExigia));
    }

    #[test]
    fn la_roca_congelada_por_un_golpe_no_es_maniobra() {
        let mut e = evento(10.0, 1.0);
        e.congelado = true;
        assert_eq!(medir(&e, &[e], &registro(10.0, 0.3, 4.0, 1.0), &rango()), Err(Descarte::Congelada));
    }

    #[test]
    fn dos_rocas_solapadas_no_se_pueden_atribuir() {
        let a = evento(10.0, 1.0);
        let b = evento(10.6, -1.0); // entra en zona antes de que se resuelva la otra
        let reg = registro(10.0, 0.3, 4.0, 1.0);

        assert_eq!(medir(&a, &[a, b], &reg, &rango()), Err(Descarte::Solapada));
        assert_eq!(medir(&b, &[a, b], &reg, &rango()), Err(Descarte::Solapada));
    }

    #[test]
    fn venir_moviendose_no_es_reaccionar() {
        let e = evento(10.0, 1.0);
        // Ya iba a 10 cm/s cuando apareció el estímulo.
        let reg = registro(10.0, -0.4, 4.0, 1.0);

        assert_eq!(medir(&e, &[e], &reg, &rango()), Err(Descarte::YaEnMovimiento));
    }

    #[test]
    fn quedarse_quieto_no_deja_latencia() {
        let e = evento(10.0, 1.0);
        let reg = registro(10.0, 0.3, 0.05, 1.0); // se mueve medio milímetro

        assert_eq!(medir(&e, &[e], &reg, &rango()), Err(Descarte::SinRespuesta));
    }

    #[test]
    fn una_reaccion_demasiado_tardia_no_cuenta() {
        let mut e = evento(10.0, 1.0);
        e.t_resultado_s = 12.0;
        let mut reg = registro(10.0, 1.7, 4.0, 1.0);
        reg.retain(|m| m[0] <= 12.0);

        assert_eq!(medir(&e, &[e], &reg, &rango()), Err(Descarte::SinRespuesta));
    }

    #[test]
    fn sin_muestras_del_tramo_no_se_inventa_una_medida() {
        let e = evento(10.0, 1.0);
        assert_eq!(medir(&e, &[e], &[], &rango()), Err(Descarte::SinDatos));
    }

    #[test]
    fn el_resumen_separa_los_dos_lados_y_mide_la_asimetria() {
        let rango = rango();
        let mut medidas = Vec::new();
        // Tres maniobras a la derecha, largas; tres a la izquierda, cortas.
        for i in 0..3 {
            let t = 10.0 + f64::from(i) * 5.0;
            medidas.push(medir(&evento(t, 1.0), &[], &registro(t, 0.3, 4.0, 1.0), &rango));
        }
        for i in 0..3 {
            let t = 30.0 + f64::from(i) * 5.0;
            medidas.push(medir(&evento(t, -1.0), &[], &registro(t, 0.3, 1.5, -1.0), &rango));
        }

        let r = resumir(&medidas).expect("hay maniobras válidas");

        assert_eq!(r.validas, 6);
        assert_eq!(r.validas_izquierda, 3);
        assert_eq!(r.validas_derecha, 3);
        assert!(r.asimetria > 0.3, "llega mucho más lejos a la derecha: {}", r.asimetria);
        assert!((r.control_direccional - 1.0).abs() < 1e-9);
    }

    #[test]
    fn sin_maniobras_validas_no_hay_resumen() {
        assert!(resumir(&[]).is_none());
        assert!(resumir(&[Err(Descarte::Congelada), Err(Descarte::NoExigia)]).is_none());
    }

    fn resumen_de(cubierto: f64, control: f64, por_lado: usize) -> Resumen {
        Resumen {
            validas: por_lado * 2,
            total: por_lado * 2,
            validas_izquierda: por_lado,
            validas_derecha: por_lado,
            latencia_mediana_izq_s: 0.3,
            latencia_mediana_der_s: 0.3,
            velocidad_pico_mediana_izq_cms: 8.0,
            velocidad_pico_mediana_der_cms: 8.0,
            fraccion_alcance_mediana_izq: cubierto,
            fraccion_alcance_mediana_der: cubierto,
            asimetria: 0.0,
            control_direccional: control,
        }
    }

    #[test]
    fn cubrir_lo_que_se_pide_sugiere_subir_la_dosis() {
        let r = resumen_de(0.75, 0.95, 12);
        assert_eq!(sugerir_exigencia(&r, 0.7, true), Ok(Sugerencia::Subir(0.75)));
    }

    #[test]
    fn quedarse_corto_sugiere_bajar_la_dosis() {
        let r = resumen_de(0.3, 0.9, 12);
        assert_eq!(sugerir_exigencia(&r, 0.7, true), Ok(Sugerencia::Bajar(0.65)));
    }

    #[test]
    fn arrancar_para_cualquier_lado_no_es_falta_de_dosis() {
        // Cubre la amplitud pero se equivoca de lado la mitad de las veces:
        // subir la exigencia no entrenaría nada.
        let r = resumen_de(0.9, 0.4, 12);
        assert_eq!(sugerir_exigencia(&r, 0.7, true), Ok(Sugerencia::Bajar(0.65)));
    }

    #[test]
    fn el_lado_peor_es_el_que_manda() {
        let mut r = resumen_de(0.9, 0.95, 12);
        r.fraccion_alcance_mediana_izq = 0.35; // el hemicuerpo afectado no llega
        assert_eq!(sugerir_exigencia(&r, 0.7, true), Ok(Sugerencia::Bajar(0.65)));
    }

    #[test]
    fn la_sugerencia_respeta_los_topes() {
        let r = resumen_de(0.95, 0.95, 12);
        assert_eq!(sugerir_exigencia(&r, EXIGENCIA_MAX, true), Ok(Sugerencia::Mantener));
        let flojo = resumen_de(0.1, 0.9, 12);
        assert_eq!(sugerir_exigencia(&flojo, EXIGENCIA_MIN, true), Ok(Sugerencia::Mantener));
    }

    #[test]
    fn sin_calibrar_no_se_sugiere_nada() {
        let r = resumen_de(0.8, 0.95, 12);
        assert_eq!(sugerir_exigencia(&r, 0.7, false), Err(SinSugerencia::SinCalibrar));
    }

    #[test]
    fn con_pocas_maniobras_de_un_lado_no_se_sugiere_nada() {
        let mut r = resumen_de(0.8, 0.95, 12);
        r.validas_izquierda = 3;
        assert_eq!(sugerir_exigencia(&r, 0.7, true), Err(SinSugerencia::PocasManiobras { izquierda: 3, derecha: 12 }));
    }

    #[test]
    fn la_mediana_aguanta_una_maniobra_perdida() {
        assert_eq!(mediana(&[1.0, 2.0, 100.0]), 2.0);
        assert_eq!(mediana(&[1.0, 2.0, 3.0, 4.0]), 2.5);
        assert_eq!(mediana(&[]), 0.0);
    }
}
