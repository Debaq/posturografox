# Plan futuro: transmisión por Bluetooth

Estado actual: firmware corre en ESP32-C3 Super Mini (ver `posturografox/posturografox.ino`).
Transporte hoy: USB serie (115200 baud), leído en Rust por `serial_link.rs` con el crate `serialport`.

## Decisión: cambiar de placa

ESP32-C3 **no tiene Bluetooth Classic**, solo BLE. Para no reescribir la capa de
transporte en Rust, conviene migrar a un **ESP32 "clásico"** (dual-core Xtensa
LX6, ej. ESP32-WROOM-32 / ESP32-DevKitC / NodeMCU-32S). Ese sí trae Bluetooth
Classic con perfil SPP (puerto serie emulado), y ese perfil expone un COM/tty
virtual en el host → `serial_link.rs` sigue funcionando **sin tocar el código
Rust**.

No confundir con ESP32-S3 ni ESP32-C6: tampoco traen BT Classic (solo BLE).
Buscar placas que digan "ESP32" a secas o "ESP32-WROOM-32".

## Por qué no BLE en el C3 (descartado por ahora)

- Habría que reescribir `serial_link.rs` entero con `btleplug` (BLE no es
  puerto serie).
- Firmware necesita stack BLE (NimBLE-Arduino recomendado, más liviano).
- Riesgo de jitter: `leerTodos()` hace bit-bang con `portENTER_CRITICAL` a
  microsegundos para sincronizar los 4 HX711. El C3 es un solo núcleo — el
  stack BLE compite por CPU y puede romper esa sincronía.
- Throughput ajustado: a 80 SPS hace falta notify cada ~12.5 ms; el
  connection interval de BLE práctico anda en 7.5–15 ms, sin margen.
- Poca RAM en el C3 (400 KB) para compartir con el stack BLE.

## Pasos cuando se cambie de placa

1. Migrar `posturografox.ino` a un ESP32 dual-core (WROOM-32 o similar).
   Revisar mapeo de pines (los usados hoy: SCK=4, DOUT en 5,6,7,2 — el 2 es
   pin de arranque en el C3, en el ESP32 clásico también evitar pines de
   arranque: 0, 2, 5, 12, 15).
2. Agregar `BluetoothSerial.h` (viene con el core de ESP32 Arduino, no hace
   falta librería externa) y reemplazar/espejar el `Serial.printf` por el
   objeto `BluetoothSerial` (SPP).
3. Emparejar el ESP32 con el host (Bluetooth del SO crea el puerto
   COM/tty virtual). No requiere cambios en `serial_link.rs`: solo aparece
   como otro puerto USB-like en `puertos_usables()`.
   - OJO: `puertos_usables()` hoy filtra con
     `SerialPortType::UsbPort(_)`. Un puerto BT-SPP puede listarse como
     `PortType` distinto según el SO (en Linux suele ser `rfcomm`, no
     siempre matchea `UsbPort`). Revisar y ajustar el filtro si el puerto
     no aparece en el combo.
4. Probar jitter del bit-bang HX711 con la radio BT prendida. Si hay
   glitches, considerar:
   - Mover la lectura HX711 a una tarea fija en el núcleo 0 (BT/WiFi corre
     en el núcleo 0 por defecto en el core de Arduino-ESP32 — pinnear la
     tarea de lectura al núcleo 1 con `xTaskCreatePinnedToCore`).
   - Evaluar reemplazar el bit-bang por RMT o una implementación por
     interrupciones en vez de `delayMicroseconds` + polling.
5. Validar alcance real (obstáculos, distancia típica de uso clínico) y
   estabilidad de la frecuencia de muestreo (comando `f` del firmware ya
   mide esto).

## Alternativa (no priorizada): quedarse en C3 con BLE

Si en el futuro no se puede cambiar de placa, retomar la opción BLE:
NimBLE-Arduino en el firmware + reescribir `serial_link.rs` con `btleplug`
del lado Rust. Requiere más trabajo y tiene los riesgos de jitter/throughput
descritos arriba.
