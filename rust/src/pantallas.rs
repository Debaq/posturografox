//! Qué monitores hay conectados, para poder abrir el juego en la pantalla
//! que mira el paciente mientras el evaluador sigue viendo las métricas.
//!
//! `egui` sabe mandar una ventana a un monitor, pero no sabe decir cuántos
//! hay; eso lo resuelve `display-info`, que en Windows, X11 y Wayland
//! devuelve la geometría de cada pantalla en píxeles del escritorio.

use std::time::{Duration, Instant};

/// Cada cuánto se vuelve a preguntar al sistema. Enchufar o desenchufar un
/// proyector en medio de una sesión es lo normal, pero preguntar en cada
/// frame es tirar llamadas al sistema de gráficos al pedo.
const REFRESCO: Duration = Duration::from_secs(2);

/// Un monitor conectado, en píxeles del escritorio.
#[derive(Clone, PartialEq)]
pub struct Pantalla {
    pub nombre: String,
    pub x: i32,
    pub y: i32,
    pub ancho: u32,
    pub alto: u32,
    pub primaria: bool,
    /// Escalado del sistema en esa pantalla (1.0 = 100 %).
    pub escala: f32,
}

impl Pantalla {
    /// Esquina superior izquierda en puntos lógicos, que es lo que entiende
    /// `egui` para colocar una ventana.
    pub fn posicion_logica(&self) -> [f32; 2] {
        let escala = if self.escala > 0.0 { self.escala } else { 1.0 };
        [self.x as f32 / escala, self.y as f32 / escala]
    }

    /// Tamaño en puntos lógicos.
    pub fn tamano_logico(&self) -> [f32; 2] {
        let escala = if self.escala > 0.0 { self.escala } else { 1.0 };
        [self.ancho as f32 / escala, self.alto as f32 / escala]
    }

    /// Nombre corto para mostrarle al operador ("HDMI-1 · 1920×1080").
    pub fn etiqueta(&self) -> String {
        format!("{} · {}×{}", self.nombre, self.ancho, self.alto)
    }
}

/// Lista de monitores, cacheada y refrescada cada `REFRESCO`.
pub struct Pantallas {
    lista: Vec<Pantalla>,
    ultima: Option<Instant>,
}

impl Default for Pantallas {
    fn default() -> Self {
        Self::new()
    }
}

impl Pantallas {
    pub fn new() -> Self {
        Self { lista: Vec::new(), ultima: None }
    }

    /// Lista fija, para pruebas que no dependan de los monitores reales.
    #[cfg(test)]
    pub fn con_lista(lista: Vec<Pantalla>) -> Self {
        Self { lista, ultima: Some(Instant::now()) }
    }

    /// Devuelve la lista, releyéndola del sistema si ya está vieja.
    pub fn actual(&mut self) -> &[Pantalla] {
        let vencida = self.ultima.is_none_or(|t| t.elapsed() >= REFRESCO);
        if vencida {
            self.refrescar();
        }
        &self.lista
    }

    /// Vuelve a preguntar al sistema ahora mismo.
    pub fn refrescar(&mut self) {
        self.lista = detectar();
        self.ultima = Some(Instant::now());
    }

    /// Índice de la pantalla que contiene el punto `punto` (en puntos
    /// lógicos del escritorio), si alguna lo contiene.
    pub fn indice_de_punto(&self, punto: [f32; 2]) -> Option<usize> {
        self.lista.iter().position(|p| {
            let [x, y] = p.posicion_logica();
            let [ancho, alto] = p.tamano_logico();
            punto[0] >= x && punto[0] < x + ancho && punto[1] >= y && punto[1] < y + alto
        })
    }

    /// Índice de la pantalla donde conviene mostrarle el juego al paciente:
    /// cualquiera menos aquella en la que el evaluador tiene la ventana.
    ///
    /// `ventana_evaluador` es la esquina de esa ventana en puntos lógicos.
    /// No alcanza con mirar cuál es la "principal": en X11 puede no haber
    /// ninguna marcada como tal (pasa incluso con un solo monitor).
    pub fn para_el_paciente(&mut self, ventana_evaluador: Option<[f32; 2]>) -> usize {
        self.actual();
        if self.lista.len() < 2 {
            return 0;
        }
        if let Some(punto) = ventana_evaluador
            && let Some(actual) = self.indice_de_punto(punto)
        {
            // La primera que no sea donde está trabajando el evaluador.
            return (0..self.lista.len()).find(|i| *i != actual).unwrap_or(0);
        }
        // Sin saber dónde está la ventana: la primera que no sea la principal,
        // y si el sistema no marca ninguna, la segunda de la fila.
        if self.lista.iter().any(|p| p.primaria) { self.lista.iter().position(|p| !p.primaria).unwrap_or(0) } else { 1 }
    }

    /// Si hay más de un monitor conectado.
    pub fn hay_varias(&mut self) -> bool {
        self.actual().len() > 1
    }
}

/// Variable de entorno para probar el modo de dos pantallas en una máquina
/// que tiene una sola: parte la pantalla real en tantas columnas como diga.
const PANTALLAS_SIMULADAS: &str = "POSTUROGRAFOX_PANTALLAS";

/// Divide la pantalla real en `cuantas` columnas, como si fueran monitores
/// distintos. Solo para desarrollo (ver `PANTALLAS_SIMULADAS`).
fn partir(reales: &[Pantalla], cuantas: u32) -> Vec<Pantalla> {
    let base = reales.first().cloned().unwrap_or(Pantalla {
        nombre: "simulada".to_string(),
        x: 0,
        y: 0,
        ancho: 1920,
        alto: 1080,
        primaria: true,
        escala: 1.0,
    });
    let ancho = base.ancho / cuantas.max(1);
    (0..cuantas)
        .map(|i| Pantalla {
            nombre: format!("{}-{}", base.nombre, i + 1),
            x: base.x + (ancho * i) as i32,
            ancho,
            primaria: i == 0,
            ..base.clone()
        })
        .collect()
}

/// Le pregunta al sistema qué monitores hay. Si falla (Wayland sin los
/// protocolos, sesión remota, etc.) devuelve la lista vacía y el juego se
/// abre como siempre, en la ventana principal.
fn detectar() -> Vec<Pantalla> {
    let Ok(monitores) = display_info::DisplayInfo::all() else {
        return Vec::new();
    };
    let mut lista: Vec<Pantalla> = monitores
        .into_iter()
        .map(|d| Pantalla {
            nombre: if d.friendly_name.is_empty() { d.name.clone() } else { d.friendly_name.clone() },
            x: d.x,
            y: d.y,
            ancho: d.width,
            alto: d.height,
            primaria: d.is_primary,
            escala: d.scale_factor,
        })
        .collect();
    // Orden estable de izquierda a derecha, que es como están en el escritorio.
    // No se ordena por "principal" porque X11 puede no marcar ninguna.
    lista.sort_by_key(|p| (p.x, p.y));
    if let Ok(cuantas) = std::env::var(PANTALLAS_SIMULADAS)
        && let Ok(cuantas) = cuantas.parse::<u32>()
        && cuantas > 1
    {
        return partir(&lista, cuantas);
    }
    lista
}

/// Pantalla de mentira para las pruebas de otros módulos.
#[cfg(test)]
pub fn pantalla_de_prueba(nombre: &str, x: i32, primaria: bool) -> Pantalla {
    Pantalla { nombre: nombre.to_string(), x, y: 0, ancho: 1920, alto: 1080, primaria, escala: 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::pantalla_de_prueba as pantalla;

    fn pantallas(lista: Vec<Pantalla>) -> Pantallas {
        Pantallas::con_lista(lista)
    }

    #[test]
    fn el_juego_va_a_la_pantalla_que_no_usa_el_evaluador() {
        let mut p = pantallas(vec![pantalla("eDP-1", 0, true), pantalla("HDMI-1", 1920, false)]);
        assert!(p.hay_varias());
        assert_eq!(p.para_el_paciente(Some([10.0, 10.0])), 1, "el evaluador está en la primera");
        assert_eq!(p.para_el_paciente(Some([1930.0, 10.0])), 0, "y si se mudó, el juego va a la otra");
    }

    #[test]
    fn sin_ninguna_marcada_como_principal_igual_elige_bien() {
        // Caso real en X11: ningún monitor viene marcado como primario.
        let mut p = pantallas(vec![pantalla("eDP-1", 0, false), pantalla("HDMI-1", 1920, false)]);
        assert_eq!(p.para_el_paciente(Some([10.0, 10.0])), 1);
        assert_eq!(p.para_el_paciente(None), 1, "sin saber dónde está la ventana, la segunda de la fila");
    }

    #[test]
    fn con_una_sola_pantalla_el_juego_se_queda_donde_esta() {
        let mut p = pantallas(vec![pantalla("eDP-1", 0, true)]);
        assert!(!p.hay_varias());
        assert_eq!(p.para_el_paciente(Some([10.0, 10.0])), 0);
    }

    #[test]
    fn sin_pantallas_detectadas_no_se_rompe_nada() {
        // Pasa si el sistema no deja enumerar monitores: el juego sigue
        // andando en la ventana principal.
        let mut p = pantallas(Vec::new());
        assert!(!p.hay_varias());
        assert_eq!(p.para_el_paciente(None), 0);
    }

    #[test]
    fn ubica_la_ventana_en_su_pantalla() {
        let p = pantallas(vec![pantalla("eDP-1", 0, false), pantalla("HDMI-1", 1920, false)]);
        assert_eq!(p.indice_de_punto([100.0, 100.0]), Some(0));
        assert_eq!(p.indice_de_punto([2000.0, 100.0]), Some(1));
        assert_eq!(p.indice_de_punto([-50.0, 100.0]), None, "fuera de todas");
    }

    #[test]
    fn la_geometria_se_pasa_a_puntos_logicos() {
        let mut p = pantalla("HDMI-1", 1920, false);
        p.escala = 2.0;
        assert_eq!(p.posicion_logica(), [960.0, 0.0]);
        assert_eq!(p.tamano_logico(), [960.0, 540.0]);

        p.escala = 0.0; // dato roto: no dividir por cero
        assert_eq!(p.tamano_logico(), [1920.0, 1080.0]);
    }

    #[test]
    fn detectar_no_explota_en_esta_maquina() {
        let lista = detectar();
        for pantalla in &lista {
            assert!(pantalla.ancho > 0 && pantalla.alto > 0, "una pantalla sin tamaño no sirve");
            assert!(!pantalla.etiqueta().is_empty());
        }
    }

    #[test]
    fn partir_la_pantalla_simula_varios_monitores() {
        let real = pantalla("eDP-1", 0, true);
        let partidas = partir(std::slice::from_ref(&real), 2);
        assert_eq!(partidas.len(), 2);
        assert_eq!(partidas[0].x, 0);
        assert_eq!(partidas[1].x, 960);
        assert!(partidas.iter().all(|p| p.ancho == 960 && p.alto == 1080));
        assert!(partidas[0].primaria && !partidas[1].primaria, "la primera hace de principal");
    }
}
