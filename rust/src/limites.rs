//! Ejercicio de límites de estabilidad: el paciente alcanza 8 objetivos
//! repartidos en círculo inclinándose sobre la plataforma real y sosteniendo
//! la posición un momento. Es un ejercicio de rehabilitación vestibular
//! estándar (entrenamiento de límites de estabilidad / weight-shifting) que
//! no necesita hardware adicional: usa el mismo COP que el resto de la app.
//!
//! Los objetivos se reparten sobre el alcance del paciente (`rango.rs`), no
//! sobre el tamaño de la plataforma. Escalados a la plataforma quedaban a 14
//! cm de COP hacia el costado con la geometría por defecto: más que apoyarse
//! por completo en un pie, o sea inalcanzables para cualquiera, y como un
//! objetivo solo se daba por hecho al tocarlo, el ejercicio se quedaba
//! clavado en esa dirección para siempre.

use std::f64::consts::TAU;

use crate::rango::{Eje, Origen, RangoCalibrado};

/// Objetivos repartidos en círculo (N, NE, E, SE, S, SO, O, NO).
pub const DIRECCIONES: usize = 8;

/// Qué parte del alcance del paciente hay que cubrir para tocar el objetivo.
/// No es el alcance entero a propósito: llegar al límite real de caída en las
/// ocho direcciones no es un ejercicio, es una caída.
const RADIO_FRACCION: f64 = 0.7;
/// "Zona de acierto", como fracción de la distancia al objetivo de esa
/// dirección. Proporcional y no fija porque las direcciones ya no están todas
/// a la misma distancia: hacia atrás se llega bastante menos que adelante.
const TOLERANCIA_FRACCION: f64 = 0.2;
/// Piso de la zona de acierto: por debajo de esto se estaría pidiendo una
/// precisión que el ruido del COP no deja sostener.
const TOLERANCIA_MINIMA_CM: f64 = 1.0;
const TIEMPO_HOLD_S: f32 = 0.5; // cuánto hay que sostener el COP en el objetivo
/// Cuánto se espera en una dirección antes de anotar lo que haya y seguir.
/// Sin esto una dirección que el paciente no puede alcanzar —justo lo que el
/// ejercicio viene a medir— deja el examen colgado ahí para siempre.
const TIEMPO_MAXIMO_S: f32 = 12.0;

#[derive(Clone, Copy)]
pub struct Objetivo {
    pub x: f64,
    pub y: f64,
}

/// Cómo le fue al paciente en una dirección: cuánto tardó y hasta dónde
/// llegó. El alcance es lo que interesa clínicamente (el límite de
/// estabilidad en esa dirección); el tiempo dice cuánto le costó.
pub struct Intento {
    /// Índice de la dirección (0 = adelante, en sentido horario).
    pub direccion: usize,
    pub tiempo_s: f32,
    /// Máxima proyección del COP sobre la dirección del objetivo.
    pub alcance_cm: f64,
    /// Alcance como fracción de la distancia al objetivo (1.0 = lo tocó).
    pub fraccion_objetivo: f64,
}

/// Nombre corto de cada dirección, en el orden en que se recorren.
pub const NOMBRES: [&str; DIRECCIONES] = ["N", "NE", "E", "SE", "S", "SO", "O", "NO"];

/// Estado del ejercicio en curso. Vive en `PosturografoxApp` y se actualiza
/// cada frame con la posición COP real (`actualizar`); `app.rs` solo dibuja
/// lo que este módulo calcula, no tiene lógica propia del ejercicio.
pub struct EjercicioLimites {
    activo: bool,
    /// Alcance con el que se armaron los objetivos. Se fija al empezar y no
    /// se vuelve a mirar: si una calibración entrara a mitad del examen, los
    /// objetivos se moverían debajo de los pies del paciente y los alcances
    /// ya medidos dejarían de ser comparables con los que faltan.
    rango: RangoCalibrado,
    indice: usize,
    tiempo_en_objetivo: f32,
    tiempo_desde_aparicion: f32,
    /// Máximo alcance logrado hacia el objetivo actual, mientras se intenta.
    alcance_actual_cm: f64,
    pub intentos: Vec<Intento>,
}

impl Default for EjercicioLimites {
    fn default() -> Self {
        Self {
            activo: false,
            rango: RangoCalibrado::por_defecto(40.0, 40.0),
            indice: 0,
            tiempo_en_objetivo: 0.0,
            tiempo_desde_aparicion: 0.0,
            alcance_actual_cm: 0.0,
            intentos: Vec::new(),
        }
    }
}

impl EjercicioLimites {
    /// Arranca el examen con el alcance de este paciente: el juego ya lo tiene
    /// medido, o llega el rango de referencia recortado a la plataforma.
    pub fn iniciar(&mut self, rango: RangoCalibrado) {
        *self = Self { rango, ..Self::default() };
        self.activo = true;
    }

    /// Alcance con el que se está corriendo el examen.
    pub fn rango(&self) -> &RangoCalibrado {
        &self.rango
    }

    pub fn detener(&mut self) {
        self.activo = false;
    }

    pub fn activo(&self) -> bool {
        self.activo
    }

    pub fn completo(&self) -> bool {
        self.indice >= DIRECCIONES
    }

    pub fn indice_actual(&self) -> usize {
        self.indice
    }

    pub fn objetivo_actual(&self) -> Option<Objetivo> {
        if !self.activo || self.completo() {
            return None;
        }
        Some(objetivo_en(self.indice, &self.rango))
    }

    /// Fracción 0.0..1.0 de cuánto lleva sostenido el objetivo actual (para
    /// animar el círculo de "llenado" mientras el paciente se mantiene ahí).
    pub fn progreso_hold(&self) -> f32 {
        (self.tiempo_en_objetivo / TIEMPO_HOLD_S).clamp(0.0, 1.0)
    }

    /// Avanza el ejercicio un frame. `cop_ml`/`cop_ap` en cm, `dt` en segundos.
    pub fn actualizar(&mut self, cop_ml: f64, cop_ap: f64, dt: f32) {
        if !self.activo || self.completo() {
            return;
        }
        self.tiempo_desde_aparicion += dt;

        let obj = objetivo_en(self.indice, &self.rango);
        // Todo se mide desde el reposo del paciente, no desde el cero de la
        // plataforma: quien carga asimétrico arranca corrido, y medir desde el
        // origen le regalaría alcance hacia un lado y se lo descontaría del
        // otro.
        let (centro_x, centro_y) = (self.rango.ml.centro_cm, self.rango.ap.centro_cm);
        let (dir_x, dir_y) = (obj.x - centro_x, obj.y - centro_y);

        // Cuánto se inclinó hacia el objetivo: proyección del COP sobre la
        // dirección del objetivo. Es la medida del límite de estabilidad en
        // esa dirección, y queda registrada aunque no llegue a tocarlo.
        let norma_obj = (dir_x * dir_x + dir_y * dir_y).sqrt();
        if norma_obj > 0.0 {
            let proyeccion = ((cop_ml - centro_x) * dir_x + (cop_ap - centro_y) * dir_y) / norma_obj;
            self.alcance_actual_cm = self.alcance_actual_cm.max(proyeccion);
        }

        let dx = cop_ml - obj.x;
        let dy = cop_ap - obj.y;
        let distancia = (dx * dx + dy * dy).sqrt();

        if distancia <= tolerancia_cm(norma_obj) {
            self.tiempo_en_objetivo += dt;
        } else {
            self.tiempo_en_objetivo = 0.0; // hay que sostenerlo sin soltar, no solo "tocarlo"
        }

        // Se cierra al sostenerlo, o al agotar el tiempo. Lo segundo es la
        // mitad que faltaba: una dirección que no puede alcanzar es un
        // resultado del examen, no un motivo para dejarlo colgado.
        if self.tiempo_en_objetivo >= TIEMPO_HOLD_S || self.tiempo_desde_aparicion >= TIEMPO_MAXIMO_S {
            self.intentos.push(Intento {
                direccion: self.indice,
                tiempo_s: self.tiempo_desde_aparicion,
                alcance_cm: self.alcance_actual_cm,
                fraccion_objetivo: if norma_obj > 0.0 { self.alcance_actual_cm / norma_obj } else { 0.0 },
            });
            self.indice += 1;
            self.tiempo_en_objetivo = 0.0;
            self.tiempo_desde_aparicion = 0.0;
            self.alcance_actual_cm = 0.0;
        }
    }

    /// Rango de desplazamiento del paciente sacado de este ejercicio, para
    /// que el modo juego no tenga que pedir una calibración aparte cuando el
    /// examen clínico ya la midió.
    ///
    /// Solo usa las cuatro direcciones cardinales: son las que caen sobre los
    /// ejes ML y AP. El centro queda en cero porque el ejercicio no mide
    /// reposo —proyecta desde el origen de la plataforma—, así que la
    /// calibración del propio juego sigue siendo la que mejor corrige a quien
    /// carga asimétrico.
    pub fn rango_calibrado(&self, ancho_cm: f64, prof_cm: f64) -> Option<RangoCalibrado> {
        let alcance =
            |direccion: usize| self.intentos.iter().find(|i| i.direccion == direccion).map(|i| i.alcance_cm.max(0.0));
        // Orden de `NOMBRES`: 0 = N (adelante), 2 = E (derecha), 4 = S
        // (atrás), 6 = O (izquierda).
        let (adelante, derecha, atras, izquierda) = (alcance(0)?, alcance(2)?, alcance(4)?, alcance(6)?);
        // Los alcances se midieron desde el reposo con el que arrancó el
        // examen, así que ese mismo reposo es el centro del rango que sale.
        let (centro_ml, centro_ap) = (self.rango.ml.centro_cm, self.rango.ap.centro_cm);
        RangoCalibrado::medido(
            Eje::nuevo(centro_ml, centro_ml - izquierda, centro_ml + derecha, ancho_cm / 2.0),
            Eje::nuevo(centro_ap, centro_ap - atras, centro_ap + adelante, prof_cm / 2.0),
            Origen::Limites,
        )
    }

    /// Dirección donde menos llegó, para señalar el déficit. `None` si
    /// todavía no hay intentos registrados.
    pub fn direccion_mas_debil(&self) -> Option<&Intento> {
        self.intentos.iter().min_by(|a, b| a.alcance_cm.total_cmp(&b.alcance_cm))
    }

    /// Alcance medio de todas las direcciones completadas.
    pub fn alcance_medio_cm(&self) -> f64 {
        if self.intentos.is_empty() {
            return 0.0;
        }
        self.intentos.iter().map(|i| i.alcance_cm).sum::<f64>() / self.intentos.len() as f64
    }

    pub fn resumen(&self) -> Option<String> {
        if !self.completo() || self.intentos.is_empty() {
            return None;
        }
        let n = self.intentos.len() as f32;
        let promedio = self.intentos.iter().map(|i| i.tiempo_s).sum::<f32>() / n;
        let debil = self
            .direccion_mas_debil()
            .map(|i| format!(" · menor alcance hacia {} ({:.1} cm)", NOMBRES[i.direccion], i.alcance_cm))
            .unwrap_or_default();
        Some(format!(
            "Completo: {} objetivos · tiempo medio {:.1}s · alcance medio {:.1} cm{debil}",
            self.intentos.len(),
            promedio,
            self.alcance_medio_cm()
        ))
    }
}

/// Dónde cae el objetivo `indice` para este paciente.
///
/// Cada cuadrante usa el alcance de su propio lado, así que la figura no es
/// un círculo sino la forma real de sus límites: más estirada hacia adelante
/// que hacia atrás, y corrida hacia el lado que carga.
fn objetivo_en(indice: usize, rango: &RangoCalibrado) -> Objetivo {
    let angulo = indice as f64 / DIRECCIONES as f64 * TAU;
    let (seno, coseno) = (angulo.sin(), angulo.cos());
    let radio_x =
        if seno >= 0.0 { rango.ml.alcance_positivo_cm() } else { rango.ml.alcance_negativo_cm() } * RADIO_FRACCION;
    let radio_y =
        if coseno >= 0.0 { rango.ap.alcance_positivo_cm() } else { rango.ap.alcance_negativo_cm() } * RADIO_FRACCION;
    Objetivo { x: rango.ml.centro_cm + seno * radio_x, y: rango.ap.centro_cm + coseno * radio_y }
}

/// Zona de acierto para un objetivo que está a `distancia_cm` del reposo.
fn tolerancia_cm(distancia_cm: f64) -> f64 {
    (distancia_cm * TOLERANCIA_FRACCION).max(TOLERANCIA_MINIMA_CM)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paciente de referencia: llega 5 cm a cada lado, 8 adelante y 4 atrás.
    fn rango() -> RangoCalibrado {
        RangoCalibrado::medido(Eje::nuevo(0.0, -5.0, 5.0, 20.0), Eje::nuevo(0.0, -4.0, 8.0, 20.0), Origen::Juego)
            .expect("rango utilizable")
    }

    fn intento(direccion: usize, alcance_cm: f64) -> Intento {
        Intento { direccion, tiempo_s: 1.0, alcance_cm, fraccion_objetivo: 1.0 }
    }

    #[test]
    fn el_ejercicio_completo_le_sirve_de_calibracion_al_juego() {
        let mut ej = EjercicioLimites::default();
        // N, E, S, O más una diagonal, que no se usa.
        for (dir, alcance) in [(0, 7.0), (2, 5.0), (4, 3.0), (6, 4.0), (1, 6.0)] {
            ej.intentos.push(intento(dir, alcance));
        }

        let rango = ej.rango_calibrado(40.0, 40.0).expect("las cuatro cardinales alcanzan");

        assert_eq!(rango.origen, crate::rango::Origen::Limites);
        assert_eq!(rango.ml.max_cm, 5.0, "E es la derecha");
        assert_eq!(rango.ml.min_cm, -4.0, "O es la izquierda");
        assert_eq!(rango.ap.max_cm, 7.0, "N es adelante");
        assert_eq!(rango.ap.min_cm, -3.0, "S es atrás");
    }

    #[test]
    fn sin_las_cuatro_cardinales_no_hay_calibracion() {
        let mut ej = EjercicioLimites::default();
        for dir in [0, 2, 4] {
            ej.intentos.push(intento(dir, 5.0));
        }
        assert!(ej.rango_calibrado(40.0, 40.0).is_none(), "falta el alcance hacia la izquierda");
    }

    #[test]
    fn los_objetivos_salen_del_alcance_del_paciente_y_no_de_la_plataforma() {
        // Con la plataforma de 40 cm, el objetivo lateral quedaba a 14 cm de
        // COP: más que apoyarse entero en un pie, o sea inalcanzable.
        let rango = rango();
        let este = objetivo_en(2, &rango); // E, la derecha
        let norte = objetivo_en(0, &rango);
        let sur = objetivo_en(4, &rango);

        assert!((este.x - 5.0 * RADIO_FRACCION).abs() < 1e-9, "quedó en {}", este.x);
        assert!((norte.y - 8.0 * RADIO_FRACCION).abs() < 1e-9);
        assert!((sur.y + 4.0 * RADIO_FRACCION).abs() < 1e-9, "atrás se llega menos que adelante");
    }

    #[test]
    fn el_paciente_descentrado_tiene_los_objetivos_alrededor_de_su_reposo() {
        // Apoya más en la derecha: reposo en +3 cm, y llega a +8 y a -1.
        let rango =
            RangoCalibrado::medido(Eje::nuevo(3.0, -1.0, 8.0, 20.0), Eje::nuevo(0.0, -4.0, 8.0, 20.0), Origen::Juego)
                .expect("rango utilizable");

        let este = objetivo_en(2, &rango);
        let oeste = objetivo_en(6, &rango);

        assert!((este.x - (3.0 + 5.0 * RADIO_FRACCION)).abs() < 1e-9, "quedó en {}", este.x);
        assert!((oeste.x - (3.0 - 4.0 * RADIO_FRACCION)).abs() < 1e-9, "quedó en {}", oeste.x);
    }

    #[test]
    fn el_alcance_se_mide_desde_el_reposo_y_no_desde_el_cero_de_la_plataforma() {
        // Reposo adelantado 2 cm: el primer objetivo (N) queda a 2 + 10*0.7,
        // pero lo que se inclinó son los 7 cm que van de su reposo al
        // objetivo, no los 9 que lo separan del cero de la plataforma.
        let rango =
            RangoCalibrado::medido(Eje::nuevo(0.0, -5.0, 5.0, 20.0), Eje::nuevo(2.0, -2.0, 12.0, 20.0), Origen::Juego)
                .expect("rango utilizable");
        let mut ej = EjercicioLimites::default();
        ej.iniciar(rango);
        let obj = objetivo_en(0, &rango);
        assert!((obj.y - 9.0).abs() < 1e-9, "el objetivo quedó en {}", obj.y);

        for _ in 0..3 {
            ej.actualizar(obj.x, obj.y, 0.2);
        }

        let intento = &ej.intentos[0];
        assert!((intento.alcance_cm - 7.0).abs() < 1e-6, "alcance {}", intento.alcance_cm);
        assert!((intento.fraccion_objetivo - 1.0).abs() < 1e-6);
    }

    #[test]
    fn una_direccion_que_no_puede_alcanzar_no_deja_el_examen_colgado() {
        // El paciente empuja hacia adelante pero se queda a mitad de camino:
        // eso es un resultado del examen, no un motivo para esperar para
        // siempre en esa dirección.
        let mut ej = EjercicioLimites::default();
        ej.iniciar(rango());
        let obj = objetivo_en(0, &rango());

        let mut t = 0.0;
        while t < TIEMPO_MAXIMO_S + 0.5 {
            ej.actualizar(0.0, obj.y / 2.0, 0.1);
            t += 0.1;
        }

        assert_eq!(ej.indice_actual(), 1, "tiene que haber pasado a la siguiente dirección");
        let intento = &ej.intentos[0];
        assert!(intento.fraccion_objetivo < 0.6, "queda anotado que no llegó: {}", intento.fraccion_objetivo);
        assert!(intento.alcance_cm > 0.0, "y hasta dónde sí llegó");
    }

    #[test]
    fn el_rango_que_sale_del_examen_conserva_el_reposo_del_paciente() {
        let rango =
            RangoCalibrado::medido(Eje::nuevo(3.0, -1.0, 8.0, 20.0), Eje::nuevo(0.0, -4.0, 8.0, 20.0), Origen::Juego)
                .expect("rango utilizable");
        let mut ej = EjercicioLimites::default();
        ej.iniciar(rango);
        for (dir, alcance) in [(0, 7.0), (2, 5.0), (4, 3.0), (6, 4.0)] {
            ej.intentos.push(intento(dir, alcance));
        }

        let medido = ej.rango_calibrado(40.0, 40.0).expect("las cuatro cardinales alcanzan");

        assert_eq!(medido.ml.centro_cm, 3.0, "el reposo no se pierde");
        assert_eq!(medido.ml.max_cm, 8.0, "5 cm a la derecha de su reposo");
        assert_eq!(medido.ml.min_cm, -1.0, "4 cm a la izquierda de su reposo");
    }

    #[test]
    fn objetivo_inicial_esta_en_el_frente_del_paciente() {
        // índice 0 -> ángulo 0 -> (sin 0, cos 0) = (0, 1): AP máximo, ML centrado
        let obj = objetivo_en(0, &rango());
        assert!(obj.x.abs() < 1e-9);
        assert!(obj.y > 0.0);
    }

    #[test]
    fn sostener_el_objetivo_el_tiempo_suficiente_avanza_al_siguiente() {
        let mut ej = EjercicioLimites::default();
        ej.iniciar(rango());
        let obj = objetivo_en(0, &rango());

        // Varios frames de 0.2s parado justo en el objetivo: total 0.6s > TIEMPO_HOLD_S
        for _ in 0..3 {
            ej.actualizar(obj.x, obj.y, 0.2);
        }

        assert_eq!(ej.indice_actual(), 1);
        assert_eq!(ej.intentos.len(), 1);
    }

    #[test]
    fn soltar_el_objetivo_antes_de_tiempo_reinicia_el_conteo() {
        let mut ej = EjercicioLimites::default();
        ej.iniciar(rango());
        let obj = objetivo_en(0, &rango());

        ej.actualizar(obj.x, obj.y, 0.4); // casi lo logra
        ej.actualizar(0.0, -100.0, 0.1); // se aleja: reinicia
        ej.actualizar(obj.x, obj.y, 0.4); // 0.4s no alcanza solo

        assert_eq!(ej.indice_actual(), 0, "no debería avanzar: se soltó antes de completar el hold");
    }

    #[test]
    fn cada_intento_registra_hasta_donde_llego_el_paciente() {
        let mut ej = EjercicioLimites::default();
        ej.iniciar(rango());
        let obj = objetivo_en(0, &rango());

        // Se inclina hasta la mitad del camino, vuelve, y recién después llega.
        ej.actualizar(0.0, obj.y / 2.0, 0.1);
        ej.actualizar(0.0, 0.0, 0.1);
        for _ in 0..3 {
            ej.actualizar(obj.x, obj.y, 0.2);
        }

        let intento = &ej.intentos[0];
        assert_eq!(intento.direccion, 0);
        assert!((intento.alcance_cm - obj.y).abs() < 1e-6, "alcance {} vs objetivo {}", intento.alcance_cm, obj.y);
        assert!((intento.fraccion_objetivo - 1.0).abs() < 1e-6);
    }

    #[test]
    fn la_direccion_mas_debil_es_la_de_menor_alcance() {
        let mut ej = EjercicioLimites::default();
        ej.iniciar(rango());
        ej.intentos.push(Intento { direccion: 0, tiempo_s: 1.0, alcance_cm: 10.0, fraccion_objetivo: 1.0 });
        ej.intentos.push(Intento { direccion: 3, tiempo_s: 2.0, alcance_cm: 4.0, fraccion_objetivo: 0.4 });
        ej.intentos.push(Intento { direccion: 5, tiempo_s: 1.5, alcance_cm: 8.0, fraccion_objetivo: 0.8 });

        assert_eq!(ej.direccion_mas_debil().unwrap().direccion, 3);
        assert!((ej.alcance_medio_cm() - 22.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn completar_las_ocho_direcciones_marca_el_ejercicio_como_completo() {
        let mut ej = EjercicioLimites::default();
        ej.iniciar(rango());
        for i in 0..DIRECCIONES {
            let obj = objetivo_en(i, &rango());
            for _ in 0..3 {
                ej.actualizar(obj.x, obj.y, 0.2);
            }
        }
        assert!(ej.completo());
        assert!(ej.objetivo_actual().is_none());
        assert!(ej.resumen().is_some());
    }
}
