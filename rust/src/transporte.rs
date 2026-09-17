//! De dónde salen las muestras. La app no habla con el puerto serie
//! directamente: habla con un `Transporte`, que hoy puede ser el posturógrafo
//! real (`serial_link::ConexionSerie`) o el simulador (`simulador::Simulador`).
//!
//! Además de permitir trabajar sin la plataforma física, deja lugar para el
//! transporte inalámbrico descrito en `firmware/BLUETOOTH.md` sin tocar la UI.

use std::sync::mpsc::Receiver;

use crate::serial_link::EventoSerie;

pub trait Transporte {
    /// Canal por donde llegan las muestras y los avisos del dispositivo.
    fn eventos(&self) -> &Receiver<EventoSerie>;

    /// Manda un comando de un byte (tara, resync, consulta de estado).
    fn enviar_comando(&mut self, comando: u8);

    /// Nombre corto para mostrar en la UI.
    fn descripcion(&self) -> String;

    /// `true` si no hay hardware detrás: la UI lo avisa para que nadie
    /// confunda datos simulados con una medición real.
    fn es_simulado(&self) -> bool {
        false
    }
}
