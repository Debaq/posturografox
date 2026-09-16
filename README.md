# Posturografox

Interfaz PySide6 + pyqtgraph para un posturógrafo de 4 celdas de carga
(ESP32-C3 + HX711, ver `firmware/posturografox`).

Muestra el centro de presión (COP) en un gráfico cartesiano con trazo, y
el desplazamiento medio-lateral / antero-posterior en función del tiempo.
Incluye tara por software, calibración por canal y detección automática
de cuándo alguien sube o baja de la plataforma.

## Uso

```bash
pip install -r requirements.txt
python app/main.py
```

## Firmware

El ESP32 imprime por serie, a 115200 baudios, líneas CSV `fd,fi,bd,bi`
(frontal/posterior, derecha/izquierda). Ver `firmware/posturografox/posturografox.ino`.

## Releases

Cada push a `main` genera automáticamente un build para Windows y Linux
(ver Actions) y publica un release `<version>_<commit>` (ej. `0.1.0_a1b2c3d`).
Un release "limpio" `vX.Y.Z` se genera al pushear un tag con ese nombre.
