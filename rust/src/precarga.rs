//! Precarga de los recursos del modo juego, mientras se muestra el splash.
//!
//! Los assets no son pesados en disco, pero descomprimir los PNG/JPEG y
//! decodificar los .ogg sí cuesta: hacerlo dentro del frame, la primera vez
//! que se abre el modo juego, se siente como un tirón largo justo al entrar.
//!
//! Acá se hace todo eso en un hilo aparte apenas arranca la app, que es
//! cuando el tiempo no le molesta a nadie. La parte que **no** se puede hacer
//! fuera del hilo de la UI —subir la textura a la GPU— queda para después y
//! es la barata.

use std::sync::mpsc::{self, Receiver};
use std::thread;

use egui::ColorImage;

/// Un recurso terminado de decodificar.
pub enum Recurso {
    Imagen(&'static str, ColorImage),
    /// Índice dentro de `juego::AUDIOS` y sus muestras ya decodificadas.
    Audio(usize, crate::juego::Pcm),
}

/// Precarga en curso.
pub struct Precarga {
    recursos: Receiver<Recurso>,
    total: usize,
    listos: usize,
    termino: bool,
}

impl Default for Precarga {
    fn default() -> Self {
        Self::iniciar()
    }
}

impl Precarga {
    /// Arranca la decodificación en segundo plano.
    pub fn iniciar() -> Self {
        let (tx, rx) = mpsc::channel();
        let total = crate::juego::HOJAS.len() + crate::juego::AUDIOS.len();

        thread::spawn(move || {
            for hoja in &crate::juego::HOJAS {
                if tx.send(Recurso::Imagen(hoja.nombre, crate::juego::decodificar(hoja.bytes))).is_err() {
                    return; // la app se cerró antes de terminar
                }
            }
            for (indice, bytes) in crate::juego::AUDIOS.iter().enumerate() {
                let Some(buffer) = decodificar_audio(bytes) else { continue };
                if tx.send(Recurso::Audio(indice, buffer)).is_err() {
                    return;
                }
            }
        });

        Self { recursos: rx, total, listos: 0, termino: false }
    }

    /// Recoge lo que haya terminado desde el frame anterior.
    pub fn recoger(&mut self) -> Vec<Recurso> {
        let recibidos: Vec<Recurso> = self.recursos.try_iter().collect();
        self.listos += recibidos.len();
        if self.listos >= self.total {
            self.termino = true;
        }
        recibidos
    }

    pub fn termino(&self) -> bool {
        self.termino
    }

    /// Avance 0.0..1.0 para la barra del splash.
    pub fn progreso(&self) -> f32 {
        if self.total == 0 { 1.0 } else { (self.listos as f32 / self.total as f32).clamp(0.0, 1.0) }
    }

    pub fn listos(&self) -> usize {
        self.listos
    }

    pub fn total(&self) -> usize {
        self.total
    }
}

/// Pasa un .ogg a muestras crudas. `None` si el archivo no se puede leer: el
/// juego funciona igual, mudo, así que no vale la pena abortar por esto.
pub fn decodificar_audio(bytes: &'static [u8]) -> Option<crate::juego::Pcm> {
    use rodio::Source;
    let decodificador = rodio::Decoder::new(std::io::Cursor::new(bytes)).ok()?;
    let canales = decodificador.channels();
    let tasa = decodificador.sample_rate();
    let muestras: Vec<i16> = decodificador.collect();
    Some(rodio::buffer::SamplesBuffer::new(canales, tasa, muestras))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_precarga_va_entregando_los_recursos_a_medida_que_los_termina() {
        // Se espera solo por las imágenes: el audio completo son 21 MB de ogg
        // y en compilación de depuración tarda demasiado para un test.
        // `todos_los_audios_se_decodifican` cubre esa parte.
        let mut precarga = Precarga::iniciar();
        assert_eq!(precarga.total(), crate::juego::HOJAS.len() + crate::juego::AUDIOS.len());
        assert_eq!(precarga.progreso(), 0.0);

        let mut imagenes = 0;
        for _ in 0..1_500 {
            for recurso in precarga.recoger() {
                if matches!(recurso, Recurso::Imagen(..)) {
                    imagenes += 1;
                }
            }
            if imagenes == crate::juego::HOJAS.len() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        assert_eq!(imagenes, crate::juego::HOJAS.len(), "faltaron imágenes");
        // Las imágenes van primero, así que con todas entregadas el avance ya
        // tiene que cubrirlas (puede ser más: el audio sigue llegando).
        let solo_imagenes = crate::juego::HOJAS.len() as f32 / precarga.total() as f32;
        assert!(precarga.progreso() >= solo_imagenes, "el avance debería reflejar lo ya entregado");
        assert!(!precarga.termino(), "todavía falta el audio");
    }

    #[test]
    fn las_imagenes_decodificadas_tienen_tamano_real() {
        for hoja in &crate::juego::HOJAS {
            let imagen = crate::juego::decodificar(hoja.bytes);
            let [ancho, alto] = imagen.size;
            assert!(ancho > 0 && alto > 0, "{} quedó vacía", hoja.nombre);
        }
    }

    #[test]
    fn todos_los_audios_se_decodifican() {
        for bytes in crate::juego::AUDIOS {
            assert!(decodificar_audio(bytes).is_some(), "un .ogg del juego no se pudo decodificar");
        }
    }
}
