"""Lectura del puerto serie del posturógrafo en un hilo aparte."""
from __future__ import annotations

import time

import serial
from PySide6.QtCore import QObject, QThread, Signal


class SerialWorker(QObject):
    muestra = Signal(float, float, float, float, float)  # t, fd, fi, bd, bi
    mensaje = Signal(str)
    error = Signal(str)
    conectado = Signal(bool)

    def __init__(self, puerto: str, baudios: int = 115200):
        super().__init__()
        self._puerto = puerto
        self._baudios = baudios
        self._ser: serial.Serial | None = None
        self._corriendo = False
        self._t0 = 0.0

    def enviar_comando(self, c: str) -> None:
        if self._ser and self._ser.is_open:
            self._ser.write(c.encode("ascii"))

    def detener(self) -> None:
        self._corriendo = False

    def ejecutar(self) -> None:
        try:
            self._ser = serial.Serial(self._puerto, self._baudios, timeout=0.5)
        except serial.SerialException as e:
            self.error.emit(str(e))
            self.conectado.emit(False)
            return

        self._corriendo = True
        self._t0 = time.monotonic()
        self.conectado.emit(True)

        while self._corriendo:
            try:
                linea = self._ser.readline().decode("ascii", errors="ignore").strip()
            except serial.SerialException as e:
                self.error.emit(str(e))
                break

            if not linea:
                continue
            if linea.startswith("#"):
                self.mensaje.emit(linea.lstrip("#").strip())
                continue

            partes = linea.split(",")
            if len(partes) != 4:
                continue
            try:
                fd, fi, bd, bi = (float(p) for p in partes)
            except ValueError:
                continue  # encabezado "fd,fi,bd,bi" u otra línea no numérica

            self.muestra.emit(time.monotonic() - self._t0, fd, fi, bd, bi)

        if self._ser and self._ser.is_open:
            self._ser.close()
        self.conectado.emit(False)


class ConexionSerie(QObject):
    """Envuelve SerialWorker + QThread para uso simple desde la ventana principal."""

    muestra = Signal(float, float, float, float, float)
    mensaje = Signal(str)
    error = Signal(str)
    conectado = Signal(bool)

    def __init__(self):
        super().__init__()
        self._hilo: QThread | None = None
        self._worker: SerialWorker | None = None

    @property
    def activa(self) -> bool:
        return self._hilo is not None

    def conectar(self, puerto: str, baudios: int = 115200) -> None:
        if self.activa:
            return
        self._hilo = QThread()
        self._worker = SerialWorker(puerto, baudios)
        self._worker.moveToThread(self._hilo)

        self._hilo.started.connect(self._worker.ejecutar)
        self._worker.muestra.connect(self.muestra)
        self._worker.mensaje.connect(self.mensaje)
        self._worker.error.connect(self.error)
        self._worker.conectado.connect(self.conectado)
        self._worker.conectado.connect(self._on_conectado)

        self._hilo.start()

    def _on_conectado(self, ok: bool) -> None:
        if not ok:
            self._limpiar()

    def desconectar(self) -> None:
        if self._worker:
            self._worker.detener()
        if self._hilo:
            self._hilo.quit()
            self._hilo.wait(1000)
        self._limpiar()

    def _limpiar(self) -> None:
        self._hilo = None
        self._worker = None

    def enviar_comando(self, c: str) -> None:
        if self._worker:
            self._worker.enviar_comando(c)
