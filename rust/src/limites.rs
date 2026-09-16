//! Ejercicio de límites de estabilidad: el paciente alcanza 8 objetivos
//! repartidos en círculo inclinándose sobre la plataforma real y sosteniendo
//! la posición un momento. Es un ejercicio de rehabilitación vestibular
//! estándar (entrenamiento de límites de estabilidad / weight-shifting) que
//! no necesita hardware adicional: usa el mismo COP que el resto de la app.

use std::f64::consts::TAU;

/// Objetivos repartidos en círculo (N, NE, E, SE, S, SO, O, NO).
pub const DIRECCIONES: usize = 8;

const RADIO_FRACCION: f64 = 0.7; // fracción del semieje de la plataforma
const TOLERANCIA_FRACCION: f64 = 0.2; // fracción del radio menor, "zona de acierto"
const TIEMPO_HOLD_S: f32 = 0.5; // cuánto hay que sostener el COP en el objetivo

#[derive(Clone, Copy)]
pub struct Objetivo {
    pub x: f64,
    pub y: f64,
}

/// Tiempo que tardó el paciente en alcanzar y sostener cada objetivo.
pub struct Intento {
    pub tiempo_s: f32,
}

/// Estado del ejercicio en curso. Vive en `PosturografoxApp` y se actualiza
/// cada frame con la posición COP real (`actualizar`); `app.rs` solo dibuja
/// lo que este módulo calcula, no tiene lógica propia del ejercicio.
pub struct EjercicioLimites {
    activo: bool,
    indice: usize,
    tiempo_en_objetivo: f32,
    tiempo_desde_aparicion: f32,
    pub intentos: Vec<Intento>,
}

impl Default for EjercicioLimites {
    fn default() -> Self {
        Self { activo: false, indice: 0, tiempo_en_objetivo: 0.0, tiempo_desde_aparicion: 0.0, intentos: Vec::new() }
    }
}

impl EjercicioLimites {
    pub fn iniciar(&mut self) {
        *self = Self::default();
        self.activo = true;
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

    pub fn objetivo_actual(&self, ancho_cm: f64, prof_cm: f64) -> Option<Objetivo> {
        if !self.activo || self.completo() {
            return None;
        }
        Some(objetivo_en(self.indice, ancho_cm, prof_cm))
    }

    /// Fracción 0.0..1.0 de cuánto lleva sostenido el objetivo actual (para
    /// animar el círculo de "llenado" mientras el paciente se mantiene ahí).
    pub fn progreso_hold(&self) -> f32 {
        (self.tiempo_en_objetivo / TIEMPO_HOLD_S).clamp(0.0, 1.0)
    }

    /// Avanza el ejercicio un frame. `cop_ml`/`cop_ap` en cm, `dt` en segundos.
    pub fn actualizar(&mut self, cop_ml: f64, cop_ap: f64, ancho_cm: f64, prof_cm: f64, dt: f32) {
        if !self.activo || self.completo() {
            return;
        }
        self.tiempo_desde_aparicion += dt;

        let obj = objetivo_en(self.indice, ancho_cm, prof_cm);
        let dx = cop_ml - obj.x;
        let dy = cop_ap - obj.y;
        let distancia = (dx * dx + dy * dy).sqrt();

        if distancia <= tolerancia_cm(ancho_cm, prof_cm) {
            self.tiempo_en_objetivo += dt;
            if self.tiempo_en_objetivo >= TIEMPO_HOLD_S {
                self.intentos.push(Intento { tiempo_s: self.tiempo_desde_aparicion });
                self.indice += 1;
                self.tiempo_en_objetivo = 0.0;
                self.tiempo_desde_aparicion = 0.0;
            }
        } else {
            self.tiempo_en_objetivo = 0.0; // hay que sostenerlo sin soltar, no solo "tocarlo"
        }
    }

    pub fn resumen(&self) -> Option<String> {
        if !self.completo() || self.intentos.is_empty() {
            return None;
        }
        let n = self.intentos.len() as f32;
        let promedio = self.intentos.iter().map(|i| i.tiempo_s).sum::<f32>() / n;
        Some(format!("Completo: {} objetivos · tiempo medio {:.1}s por objetivo", self.intentos.len(), promedio))
    }
}

fn objetivo_en(indice: usize, ancho_cm: f64, prof_cm: f64) -> Objetivo {
    let radio_x = ancho_cm / 2.0 * RADIO_FRACCION;
    let radio_y = prof_cm / 2.0 * RADIO_FRACCION;
    let angulo = indice as f64 / DIRECCIONES as f64 * TAU;
    Objetivo { x: angulo.sin() * radio_x, y: angulo.cos() * radio_y }
}

fn tolerancia_cm(ancho_cm: f64, prof_cm: f64) -> f64 {
    (ancho_cm.min(prof_cm) / 2.0 * RADIO_FRACCION) * TOLERANCIA_FRACCION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn objetivo_inicial_esta_en_el_frente_del_paciente() {
        // índice 0 -> ángulo 0 -> (sin 0, cos 0) = (0, 1): AP máximo, ML centrado
        let obj = objetivo_en(0, 40.0, 40.0);
        assert!(obj.x.abs() < 1e-9);
        assert!(obj.y > 0.0);
    }

    #[test]
    fn sostener_el_objetivo_el_tiempo_suficiente_avanza_al_siguiente() {
        let mut ej = EjercicioLimites::default();
        ej.iniciar();
        let obj = objetivo_en(0, 40.0, 40.0);

        // Varios frames de 0.2s parado justo en el objetivo: total 0.6s > TIEMPO_HOLD_S
        for _ in 0..3 {
            ej.actualizar(obj.x, obj.y, 40.0, 40.0, 0.2);
        }

        assert_eq!(ej.indice_actual(), 1);
        assert_eq!(ej.intentos.len(), 1);
    }

    #[test]
    fn soltar_el_objetivo_antes_de_tiempo_reinicia_el_conteo() {
        let mut ej = EjercicioLimites::default();
        ej.iniciar();
        let obj = objetivo_en(0, 40.0, 40.0);

        ej.actualizar(obj.x, obj.y, 40.0, 40.0, 0.4); // casi lo logra
        ej.actualizar(0.0, -100.0, 40.0, 40.0, 0.1); // se aleja: reinicia
        ej.actualizar(obj.x, obj.y, 40.0, 40.0, 0.4); // 0.4s no alcanza solo

        assert_eq!(ej.indice_actual(), 0, "no debería avanzar: se soltó antes de completar el hold");
    }

    #[test]
    fn completar_las_ocho_direcciones_marca_el_ejercicio_como_completo() {
        let mut ej = EjercicioLimites::default();
        ej.iniciar();
        for i in 0..DIRECCIONES {
            let obj = objetivo_en(i, 40.0, 40.0);
            for _ in 0..3 {
                ej.actualizar(obj.x, obj.y, 40.0, 40.0, 0.2);
            }
        }
        assert!(ej.completo());
        assert!(ej.objetivo_actual(40.0, 40.0).is_none());
        assert!(ej.resumen().is_some());
    }
}
