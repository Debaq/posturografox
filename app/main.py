"""Posturógrafo: interfaz PySide6 + pyqtgraph.

Lee CSV "fd,fi,bd,bi" desde el ESP32 (ver firmware/posturografox) y calcula
el centro de presión (COP) de una plataforma rectangular de 4 celdas:
    fd = frontal derecha, fi = frontal izquierda
    bd = posterior derecha, bi = posterior izquierda

Antes de combinar los 4 canales se les resta un cero (offset) y se les
aplica una ganancia por canal, porque las celdas no vienen calibradas
igual entre sí (ver botones "Tara (software)" y los campos de ganancia):
    valor_i = (crudo_i - offset_i) * ganancia_i

    COP_ml (medio-lateral, + = derecha) = ((fd+bd)-(fi+bi))/suma * ancho/2
    COP_ap (antero-posterior, + = frente) = ((fd+fi)-(bd+bi))/suma * profundidad/2

El dibujo de los gráficos va desacoplado de la llegada de muestras: los
datos se acumulan en cada muestra (barato) y un QTimer los pinta a una
tasa fija, para no redibujar miles de puntos 80 veces por segundo.
"""
from __future__ import annotations

import sys
from collections import deque

import numpy as np
import pyqtgraph as pg
from PySide6.QtCore import Qt, QTimer, Slot
from PySide6.QtWidgets import (
    QApplication,
    QDoubleSpinBox,
    QComboBox,
    QHBoxLayout,
    QLabel,
    QMainWindow,
    QPushButton,
    QSpinBox,
    QVBoxLayout,
    QWidget,
)
from serial.tools import list_ports

from serial_reader import ConexionSerie

BAUDIOS = 115200
MAX_MUESTRAS_TIEMPO = 8000
MAX_PUNTOS_TRAZO = 20000
VENTANA_TIEMPO_S = 20.0          # ventana visible del gráfico tiempo-desplazamiento
INTERVALO_REDIBUJO_MS = 33       # ~30 fps, independiente de la tasa de muestreo
MUESTRAS_TARA_SW = 20            # muestras crudas promediadas al pedir tara por software
DEBOUNCE_DETECCION = 5           # muestras consecutivas para confirmar subida/bajada de la plataforma
ESPACIADO_PUNTOS_DEFAULT = 8     # 1 punto visible cada N muestras en el trazo COP


class VentanaPrincipal(QMainWindow):
    def __init__(self):
        super().__init__()
        self.setWindowTitle("Posturografox")
        self.resize(900, 900)

        self.conexion = ConexionSerie()
        self.conexion.muestra.connect(self.on_muestra)
        self.conexion.mensaje.connect(self.on_mensaje_firmware)
        self.conexion.error.connect(self.on_error)
        self.conexion.conectado.connect(self.on_conectado)

        self._t = deque(maxlen=MAX_MUESTRAS_TIEMPO)
        self._ml = deque(maxlen=MAX_MUESTRAS_TIEMPO)
        self._ap = deque(maxlen=MAX_MUESTRAS_TIEMPO)
        self._trazo_x = deque(maxlen=MAX_PUNTOS_TRAZO)
        self._trazo_y = deque(maxlen=MAX_PUNTOS_TRAZO)

        # Calibración por canal: offset (tara por software) y ganancia. Orden fd,fi,bd,bi.
        self._offset = [0.0, 0.0, 0.0, 0.0]
        self._buffer_crudo = deque(maxlen=MUESTRAS_TARA_SW)
        self._buffer_arriba = deque(maxlen=MUESTRAS_TARA_SW)  # solo muestras ya sobre el umbral

        self._pendiente_redibujo = False
        self._ultimo_ml = 0.0
        self._ultimo_ap = 0.0

        # Detección automática de subida/bajada de la plataforma (por suma cruda)
        self._ocupado = False
        self._contador_arriba = 0
        self._contador_abajo = 0

        self._armar_ui()

        self._timer_redibujo = QTimer(self)
        self._timer_redibujo.setInterval(INTERVALO_REDIBUJO_MS)
        self._timer_redibujo.timeout.connect(self._redibujar)
        self._timer_redibujo.start()

    # ------------------------------------------------------------------ UI
    def _armar_ui(self) -> None:
        central = QWidget()
        self.setCentralWidget(central)
        layout = QVBoxLayout(central)

        layout.addLayout(self._armar_barra_controles())
        layout.addLayout(self._armar_barra_calibracion())
        layout.addLayout(self._armar_barra_deteccion())

        # --- Gráfico COP (cartesiano, X = ML, Y = AP) ---
        self.plot_cop = pg.PlotWidget(title="Centro de presión (COP)")
        self.plot_cop.setLabel("bottom", "Medio-lateral", units="cm")
        self.plot_cop.setLabel("left", "Antero-posterior", units="cm")
        self.plot_cop.showGrid(x=True, y=True, alpha=0.3)
        self.plot_cop.setAspectLocked(True)
        self.plot_cop.setMouseEnabled(x=False, y=False)
        self.plot_cop.addLine(x=0, pen=pg.mkPen((120, 120, 120), style=Qt.DashLine))
        self.plot_cop.addLine(y=0, pen=pg.mkPen((120, 120, 120), style=Qt.DashLine))

        self.curva_trazo = self.plot_cop.plot(
            pen=pg.mkPen((0, 150, 255), width=1.5),
            antialias=False,
            autoDownsample=True,
            clipToView=True,
        )
        self.trazo_puntos = pg.ScatterPlotItem(
            size=5, brush=pg.mkBrush(0, 150, 255, 180), pen=None
        )
        self.plot_cop.addItem(self.trazo_puntos)
        self.punto_actual = pg.ScatterPlotItem(
            size=14, brush=pg.mkBrush(255, 60, 60), pen=pg.mkPen("k")
        )
        self.plot_cop.addItem(self.punto_actual)
        self._actualizar_rango_cop()

        # --- Gráfico movimiento-tiempo ---
        self.plot_tiempo = pg.PlotWidget(title="Movimiento en el tiempo")
        self.plot_tiempo.setLabel("bottom", "Tiempo", units="s")
        self.plot_tiempo.setLabel("left", "Desplazamiento", units="cm")
        self.plot_tiempo.showGrid(x=True, y=True, alpha=0.3)
        self.plot_tiempo.addLegend()
        self.plot_tiempo.enableAutoRange(x=False, y=False)
        self.plot_tiempo.setMouseEnabled(x=False, y=False)
        self.curva_ml = self.plot_tiempo.plot(
            pen=pg.mkPen((0, 150, 255), width=1.5),
            name="ML",
            antialias=False,
            autoDownsample=True,
            clipToView=True,
        )
        self.curva_ap = self.plot_tiempo.plot(
            pen=pg.mkPen((255, 140, 0), width=1.5),
            name="AP",
            antialias=False,
            autoDownsample=True,
            clipToView=True,
        )
        self._actualizar_rango_tiempo()

        layout.addWidget(self.plot_cop, stretch=3)
        layout.addWidget(self.plot_tiempo, stretch=2)

        self.statusBar().showMessage("Desconectado")

    def _armar_barra_controles(self) -> QHBoxLayout:
        fila = QHBoxLayout()

        self.combo_puerto = QComboBox()
        self._refrescar_puertos()
        fila.addWidget(QLabel("Puerto:"))
        fila.addWidget(self.combo_puerto)

        btn_refrescar = QPushButton("Actualizar")
        btn_refrescar.clicked.connect(self._refrescar_puertos)
        fila.addWidget(btn_refrescar)

        self.btn_conectar = QPushButton("Conectar")
        self.btn_conectar.clicked.connect(self._toggle_conexion)
        fila.addWidget(self.btn_conectar)

        self.btn_tara = QPushButton("Tara firmware")
        self.btn_tara.clicked.connect(lambda: self.conexion.enviar_comando("t"))
        self.btn_tara.setEnabled(False)
        fila.addWidget(self.btn_tara)

        self.btn_resync = QPushButton("Resincronizar")
        self.btn_resync.clicked.connect(lambda: self.conexion.enviar_comando("s"))
        self.btn_resync.setEnabled(False)
        fila.addWidget(self.btn_resync)

        btn_limpiar = QPushButton("Limpiar trazo")
        btn_limpiar.clicked.connect(self._limpiar_trazo)
        fila.addWidget(btn_limpiar)

        fila.addWidget(QLabel("Ancho (cm):"))
        self.spin_ancho = QDoubleSpinBox()
        self.spin_ancho.setRange(1.0, 500.0)
        self.spin_ancho.setValue(40.0)
        self.spin_ancho.valueChanged.connect(self._actualizar_rango_cop)
        fila.addWidget(self.spin_ancho)

        fila.addWidget(QLabel("Profundidad (cm):"))
        self.spin_prof = QDoubleSpinBox()
        self.spin_prof.setRange(1.0, 500.0)
        self.spin_prof.setValue(40.0)
        self.spin_prof.valueChanged.connect(self._actualizar_rango_cop)
        fila.addWidget(self.spin_prof)

        fila.addStretch(1)
        return fila

    def _armar_barra_calibracion(self) -> QHBoxLayout:
        fila = QHBoxLayout()

        btn_tara_sw = QPushButton("Tara (software)")
        btn_tara_sw.setToolTip(
            f"Promedia las últimas {MUESTRAS_TARA_SW} muestras crudas y las fija como cero"
        )
        btn_tara_sw.clicked.connect(self._tara_software)
        fila.addWidget(btn_tara_sw)

        self.spin_ganancia = {}
        for etq in ("fd", "fi", "bd", "bi"):
            fila.addWidget(QLabel(f"Gan. {etq}:"))
            spin = QDoubleSpinBox()
            spin.setRange(0.0001, 1000.0)
            spin.setDecimals(4)
            spin.setSingleStep(0.01)
            spin.setValue(1.0)
            fila.addWidget(spin)
            self.spin_ganancia[etq] = spin

        fila.addStretch(1)
        return fila

    def _armar_barra_deteccion(self) -> QHBoxLayout:
        fila = QHBoxLayout()

        fila.addWidget(QLabel("Umbral detección (cuentas crudas):"))
        self.spin_umbral = QDoubleSpinBox()
        self.spin_umbral.setRange(0.0, 10_000_000.0)
        self.spin_umbral.setDecimals(0)
        self.spin_umbral.setSingleStep(1000.0)
        self.spin_umbral.setValue(20000.0)
        self.spin_umbral.setToolTip(
            "Suma cruda (fd+fi+bd+bi) por encima de esto = alguien parado en la plataforma"
        )
        fila.addWidget(self.spin_umbral)

        fila.addWidget(QLabel("Espaciado puntos (muestras):"))
        self.spin_espaciado = QSpinBox()
        self.spin_espaciado.setRange(1, 200)
        self.spin_espaciado.setValue(ESPACIADO_PUNTOS_DEFAULT)
        fila.addWidget(self.spin_espaciado)

        fila.addStretch(1)
        return fila

    def _actualizar_rango_cop(self) -> None:
        ancho = self.spin_ancho.value() if hasattr(self, "spin_ancho") else 40.0
        prof = self.spin_prof.value() if hasattr(self, "spin_prof") else 40.0
        margen = 1.2
        self.plot_cop.setRange(
            xRange=(-ancho / 2 * margen, ancho / 2 * margen),
            yRange=(-prof / 2 * margen, prof / 2 * margen),
        )
        if hasattr(self, "plot_tiempo"):
            self._actualizar_rango_tiempo()

    def _actualizar_rango_tiempo(self) -> None:
        ancho = self.spin_ancho.value()
        prof = self.spin_prof.value()
        limite = max(ancho, prof) / 2 * 1.2
        t_ultimo = self._t[-1] if self._t else 0.0
        self.plot_tiempo.setYRange(-limite, limite, padding=0)
        self.plot_tiempo.setXRange(t_ultimo - VENTANA_TIEMPO_S, max(t_ultimo, VENTANA_TIEMPO_S), padding=0)

    def _refrescar_puertos(self) -> None:
        actual = self.combo_puerto.currentText()
        self.combo_puerto.clear()
        for p in list_ports.comports():
            self.combo_puerto.addItem(p.device)
        i = self.combo_puerto.findText(actual)
        if i >= 0:
            self.combo_puerto.setCurrentIndex(i)

    # ------------------------------------------------------------ conexión
    def _toggle_conexion(self) -> None:
        if self.conexion.activa:
            self.conexion.desconectar()
        else:
            puerto = self.combo_puerto.currentText()
            if not puerto:
                self.statusBar().showMessage("Elegí un puerto primero")
                return
            self.conexion.conectar(puerto, BAUDIOS)

    @Slot(bool)
    def on_conectado(self, ok: bool) -> None:
        self.btn_conectar.setText("Desconectar" if ok else "Conectar")
        self.btn_tara.setEnabled(ok)
        self.btn_resync.setEnabled(ok)
        self.statusBar().showMessage("Conectado" if ok else "Desconectado")

    @Slot(str)
    def on_mensaje_firmware(self, msg: str) -> None:
        self.statusBar().showMessage(msg, 4000)

    @Slot(str)
    def on_error(self, msg: str) -> None:
        self.statusBar().showMessage(f"Error: {msg}", 6000)

    def _limpiar_trazo(self) -> None:
        self._trazo_x.clear()
        self._trazo_y.clear()
        self.curva_trazo.setData([], [])
        self.trazo_puntos.setData([], [])

    def _reiniciar_sesion(self) -> None:
        self._limpiar_trazo()
        self._t.clear()
        self._ml.clear()
        self._ap.clear()
        self._pendiente_redibujo = True

    def _tara_software(self, buffer: deque | None = None) -> None:
        buffer = buffer if buffer is not None else self._buffer_crudo
        if not buffer:
            self.statusBar().showMessage("Todavía no llegaron muestras", 4000)
            return
        columnas = zip(*buffer)
        self._offset = [sum(c) / len(c) for c in columnas]
        self.statusBar().showMessage("Tara por software aplicada", 4000)

    def _procesar_deteccion(self, crudos: tuple[float, float, float, float]) -> None:
        """Detecta cuándo alguien sube o baja de la plataforma por la suma cruda.

        Al subir: tara automática (centra la calibración con el peso ya puesto),
        usando solo las muestras tomadas mientras ya estaba sobre el umbral
        (no se mezclan con las de plataforma vacía de antes de la transición).
        Al bajar: reinicia trazo y curvas para dejar la sesión lista para el siguiente.
        Con debounce de N muestras para no disparar con ruido puntual.
        """
        suma_cruda = sum(crudos)
        umbral = self.spin_umbral.value()
        if abs(suma_cruda) >= umbral:
            self._buffer_arriba.append(crudos)
            self._contador_arriba += 1
            self._contador_abajo = 0
            if not self._ocupado and self._contador_arriba >= DEBOUNCE_DETECCION:
                self._ocupado = True
                self._tara_software(self._buffer_arriba)
                self._reiniciar_sesion()
                self.statusBar().showMessage("Persona detectada: tara automática", 4000)
        else:
            self._buffer_arriba.clear()
            self._contador_abajo += 1
            self._contador_arriba = 0
            if self._ocupado and self._contador_abajo >= DEBOUNCE_DETECCION:
                self._ocupado = False
                self._reiniciar_sesion()
                self.statusBar().showMessage("Plataforma libre: sesión reiniciada", 4000)

    # -------------------------------------------------------------- datos
    @Slot(float, float, float, float, float)
    def on_muestra(self, t: float, fd: float, fi: float, bd: float, bi: float) -> None:
        self._buffer_crudo.append((fd, fi, bd, bi))
        self._procesar_deteccion((fd, fi, bd, bi))

        g = self.spin_ganancia
        crudos = (fd, fi, bd, bi)
        vals = [
            (crudos[i] - self._offset[i]) * g[etq].value()
            for i, etq in enumerate(("fd", "fi", "bd", "bi"))
        ]
        fd_c, fi_c, bd_c, bi_c = vals
        suma = fd_c + fi_c + bd_c + bi_c

        ancho = self.spin_ancho.value()
        prof = self.spin_prof.value()
        if suma == 0:
            cop_ml = 0.0
            cop_ap = 0.0
        else:
            cop_ml = ((fd_c + bd_c) - (fi_c + bi_c)) / suma * (ancho / 2)
            cop_ap = ((fd_c + fi_c) - (bd_c + bi_c)) / suma * (prof / 2)

        if self._ocupado:
            self._trazo_x.append(cop_ml)
            self._trazo_y.append(cop_ap)
            self._t.append(t)
            self._ml.append(cop_ml)
            self._ap.append(cop_ap)
        self._ultimo_ml = cop_ml
        self._ultimo_ap = cop_ap
        self._pendiente_redibujo = True

    def _redibujar(self) -> None:
        if not self._pendiente_redibujo:
            return
        self._pendiente_redibujo = False

        trazo_x = np.fromiter(self._trazo_x, dtype=float)
        trazo_y = np.fromiter(self._trazo_y, dtype=float)
        self.curva_trazo.setData(trazo_x, trazo_y)

        paso = self.spin_espaciado.value()
        self.trazo_puntos.setData(trazo_x[::paso], trazo_y[::paso])
        self.punto_actual.setData([self._ultimo_ml], [self._ultimo_ap])

        t = np.fromiter(self._t, dtype=float)
        self.curva_ml.setData(t, np.fromiter(self._ml, dtype=float))
        self.curva_ap.setData(t, np.fromiter(self._ap, dtype=float))
        self.plot_tiempo.setXRange(
            self._ultimo_t() - VENTANA_TIEMPO_S, max(self._ultimo_t(), VENTANA_TIEMPO_S), padding=0
        )

    def _ultimo_t(self) -> float:
        return self._t[-1] if self._t else 0.0

    def closeEvent(self, event) -> None:
        self.conexion.desconectar()
        super().closeEvent(event)


def main() -> None:
    app = QApplication(sys.argv)
    ventana = VentanaPrincipal()
    ventana.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
