/*
  Posturógrafo - ESP32-C3 Super Mini + 4 celdas de carga + 4 HX711 (SCK compartido)
  ----------------------------------------------------------------------
  Salida por consola (CSV):  fd,fi,bd,bi
    f = frontal, b = posterior (back), d = derecha, i = izquierda

  Lectura en paralelo con reloj común: se espera a que los 4 HX711 estén
  listos y se leen con el mismo tren de pulsos.

  Sincronización: manteniendo SCK en alto más de 60 µs los 4 HX711 se
  apagan; al bajarlo arrancan juntos y sus conversiones quedan alineadas.
  Esto se hace al iniciar, por comando, tras un timeout y, si se desea,
  de forma periódica.

  Notas ESP32-C3:
    - En el IDE activar "USB CDC On Boot: Enabled" para ver la consola.
    - GPIO 2 es pin de arranque. Si la placa no inicia bien al energizarla
      con los HX711 conectados, mover DOUT bi a GPIO 3 o GPIO 10.
    - Alimentar los HX711 con 3.3 V (los GPIO no toleran 5 V).

  Comandos por consola:
    t  -> tara (pone en cero los 4 sensores)
    c  -> alterna entre cuentas crudas y valores calibrados
    s  -> resincroniza los 4 HX711
    f  -> muestra la frecuencia de muestreo medida
    i  -> imprime el saludo de identificación (para autodetección del puerto)
*/

#include <Arduino.h>

// =================== PINES (ESP32-C3 Super Mini) ===================
// Pines libres y seguros en esta placa: 0, 1, 3, 4, 5, 6, 7, 10, 20, 21
// Evitar: 8 (LED de la placa, arranque), 9 (botón BOOT, arranque),
//         18 y 19 (USB). El GPIO 2 también es pin de arranque (ver notas).
#define PIN_HX_SCK    4    // Reloj común a los 4 HX711
#define PIN_DOUT_FD   5    // Frontal derecho
#define PIN_DOUT_FI   6    // Frontal izquierdo
#define PIN_DOUT_BD   7    // Posterior derecho
#define PIN_DOUT_BI   2    // Posterior izquierdo (pin de arranque, ver notas)

// Pin RATE común (opcional). Si los módulos exponen RATE y lo cableas
// junto a un GPIO, pon aquí el número. Déjalo en -1 si RATE va fijo
// por hardware (a VCC = 80 SPS, a GND = 10 SPS).
// Sugerido si se controla por software: GPIO 10.
#define PIN_HX_RATE   -1

// =================== ETIQUETAS ===================
#define ETQ_FD  "fd"
#define ETQ_FI  "fi"
#define ETQ_BD  "bd"
#define ETQ_BI  "bi"

// =================== CALIBRACIÓN ===================
// Cuentas por unidad (por ejemplo, cuentas por gramo). 1.0 = sin calibrar.
#define CAL_FD  1.0f
#define CAL_FI  1.0f
#define CAL_BD  1.0f
#define CAL_BI  1.0f

// =================== VELOCIDAD ===================
// 1 = 80 muestras/s, 0 = 10 muestras/s.
// Debe coincidir con cómo está conectado RATE (o se controla con PIN_HX_RATE).
#define VELOCIDAD_80SPS  1

// =================== SINCRONIZACIÓN ===================
#define RESYNC_PERIODO_MS   0     // 0 = desactivado. Ej.: 60000 = cada 60 s.
                                  // Cada resincronización deja un hueco en los datos
                                  // (~100 ms a 80 SPS, ~450 ms a 10 SPS).
#define RESYNC_TRAS_TIMEOUT 1     // 1 = resincroniza solo si algún HX711 no responde

// =================== CONFIGURACIÓN ===================
#define BAUDIOS          115200
#define ID_FIRMWARE      "POSTUROGRAFOX,1"  // saludo de identificación (comando 'i' o al arrancar)
#define N_TARA           20     // lecturas promediadas para la tara
#define PULSOS_EXTRA     1      // 1 = canal A ganancia 128 | 3 = canal A ganancia 64 | 2 = canal B ganancia 32
#define IMPRIMIR_CRUDO   0      // 1 = cuentas crudas al inicio, 0 = valores calibrados
#define DECIMALES        2

// ====================================================================
#define N_SENS 4

#if VELOCIDAD_80SPS
  #define PERIODO_MS        13    // 1/80 s (redondeado)
  #define ESTABILIZACION_MS 100   // hoja de datos: 50 ms + margen
#else
  #define PERIODO_MS        100   // 1/10 s
  #define ESTABILIZACION_MS 450   // hoja de datos: 400 ms + margen
#endif
#define TIMEOUT_MS (PERIODO_MS * 5 + 50)

const uint8_t PIN_DOUT[N_SENS] = { PIN_DOUT_FD, PIN_DOUT_FI, PIN_DOUT_BD, PIN_DOUT_BI };
const char*   ETQ[N_SENS]      = { ETQ_FD, ETQ_FI, ETQ_BD, ETQ_BI };
const float   CAL[N_SENS]      = { CAL_FD, CAL_FI, CAL_BD, CAL_BI };

long offsetTara[N_SENS] = { 0, 0, 0, 0 };
bool modoCrudo = IMPRIMIR_CRUDO;

unsigned long ultimoResync   = 0;
unsigned long inicioConteo   = 0;
unsigned long muestrasConteo = 0;

portMUX_TYPE muxHX = portMUX_INITIALIZER_UNLOCKED;

// ¿Los 4 HX711 tienen un dato listo? (DOUT en bajo)
bool todosListos() {
  for (int i = 0; i < N_SENS; i++) {
    if (digitalRead(PIN_DOUT[i]) == HIGH) return false;
  }
  return true;
}

inline void pulsoSCK() {
  digitalWrite(PIN_HX_SCK, HIGH);
  delayMicroseconds(1);            // debe ser < 60 µs para no apagar el HX711
  digitalWrite(PIN_HX_SCK, LOW);
  delayMicroseconds(1);
}

// Lee los 4 HX711 simultáneamente. Devuelve false si hubo timeout.
bool leerTodos(long crudo[N_SENS]) {
  unsigned long t0 = millis();
  while (!todosListos()) {
    if (millis() - t0 > TIMEOUT_MS) return false;
    delay(1);
  }

  uint32_t val[N_SENS] = { 0, 0, 0, 0 };

  portENTER_CRITICAL(&muxHX);      // evita que una interrupción alargue el pulso
  for (int b = 0; b < 24; b++) {
    digitalWrite(PIN_HX_SCK, HIGH);
    delayMicroseconds(1);
    for (int i = 0; i < N_SENS; i++) {
      val[i] = (val[i] << 1) | (digitalRead(PIN_DOUT[i]) & 1);
    }
    digitalWrite(PIN_HX_SCK, LOW);
    delayMicroseconds(1);
  }
  for (int p = 0; p < PULSOS_EXTRA; p++) pulsoSCK();   // fija canal/ganancia
  portEXIT_CRITICAL(&muxHX);

  for (int i = 0; i < N_SENS; i++) {
    if (val[i] & 0x800000UL) val[i] |= 0xFF000000UL;   // extensión de signo (24 -> 32 bits)
    crudo[i] = (int32_t)val[i];
  }
  return true;
}

// Apaga y enciende los 4 HX711 a la vez para alinear sus conversiones.
void resincronizar() {
  digitalWrite(PIN_HX_SCK, HIGH);
  delay(1);                        // > 60 µs: los 4 entran en reposo
  digitalWrite(PIN_HX_SCK, LOW);   // los 4 arrancan en el mismo instante

  delay(ESTABILIZACION_MS);        // salida estable según hoja de datos

  // Tras el reinicio vuelven a canal A ganancia 128; una lectura
  // descartada fija la ganancia elegida y limpia el primer dato.
  long basura[N_SENS];
  leerTodos(basura);
  leerTodos(basura);

  ultimoResync = millis();
}

void tarar() {
  Serial.println("# Tara en curso, no pisar la plataforma...");
  long long suma[N_SENS] = { 0, 0, 0, 0 };
  long crudo[N_SENS];
  int validas = 0;

  while (validas < N_TARA) {
    if (leerTodos(crudo)) {
      for (int i = 0; i < N_SENS; i++) suma[i] += crudo[i];
      validas++;
    } else {
      Serial.println("# Timeout durante la tara, revisar conexiones");
      return;
    }
  }
  for (int i = 0; i < N_SENS; i++) offsetTara[i] = (long)(suma[i] / N_TARA);
  Serial.printf("# Tara lista: %ld,%ld,%ld,%ld\n",
                offsetTara[0], offsetTara[1], offsetTara[2], offsetTara[3]);
}

void imprimirEncabezado() {
  Serial.printf("%s,%s,%s,%s\n", ETQ[0], ETQ[1], ETQ[2], ETQ[3]);
}

// Saludo de identificación: permite que el host distinga este dispositivo
// de cualquier otro puerto serie al buscar a qué puerto conectarse solo.
void imprimirIdentificacion() {
  Serial.print("# ");
  Serial.println(ID_FIRMWARE);
}

void reiniciarConteo() {
  inicioConteo = millis();
  muestrasConteo = 0;
}

void revisarComandos() {
  while (Serial.available()) {
    char c = Serial.read();
    switch (c) {
      case 't': case 'T':
        tarar();
        imprimirEncabezado();
        reiniciarConteo();
        break;
      case 'c': case 'C':
        modoCrudo = !modoCrudo;
        Serial.printf("# Modo %s\n", modoCrudo ? "crudo" : "calibrado");
        imprimirEncabezado();
        break;
      case 's': case 'S':
        Serial.println("# Resincronizando...");
        resincronizar();
        Serial.println("# Resincronizado");
        imprimirEncabezado();
        reiniciarConteo();
        break;
      case 'f': case 'F': {
        unsigned long dt = millis() - inicioConteo;
        float hz = dt > 0 ? muestrasConteo * 1000.0f / dt : 0;
        Serial.printf("# Frecuencia medida: %.2f muestras/s (esperada %d)\n",
                      hz, VELOCIDAD_80SPS ? 80 : 10);
        break;
      }
      case 'i': case 'I':
        imprimirIdentificacion();
        break;
      default:
        break;
    }
  }
}

void setup() {
  Serial.begin(BAUDIOS);
  delay(1500);                     // tiempo para abrir el monitor serie (USB CDC nativo del C3)
  imprimirIdentificacion();        // primero que nada: para que el host la vea aunque escuche poco

  #if PIN_HX_RATE >= 0
    pinMode(PIN_HX_RATE, OUTPUT);
    digitalWrite(PIN_HX_RATE, VELOCIDAD_80SPS ? HIGH : LOW);
  #endif

  pinMode(PIN_HX_SCK, OUTPUT);
  digitalWrite(PIN_HX_SCK, LOW);
  for (int i = 0; i < N_SENS; i++) pinMode(PIN_DOUT[i], INPUT);

  Serial.println("# Sincronizando HX711...");
  resincronizar();

  tarar();
  imprimirEncabezado();
  reiniciarConteo();
}

void loop() {
  revisarComandos();

  #if RESYNC_PERIODO_MS > 0
    if (millis() - ultimoResync >= RESYNC_PERIODO_MS) {
      Serial.println("# Resincronizacion periodica");
      resincronizar();
      imprimirEncabezado();
    }
  #endif

  long crudo[N_SENS];
  if (!leerTodos(crudo)) {
    Serial.println("# Timeout: algun HX711 no responde");
    #if RESYNC_TRAS_TIMEOUT
      resincronizar();
      imprimirEncabezado();
    #endif
    return;
  }
  muestrasConteo++;

  if (modoCrudo) {
    Serial.printf("%ld,%ld,%ld,%ld\n", crudo[0], crudo[1], crudo[2], crudo[3]);
  } else {
    float v[N_SENS];
    for (int i = 0; i < N_SENS; i++) v[i] = (crudo[i] - offsetTara[i]) / CAL[i];
    Serial.printf("%.*f,%.*f,%.*f,%.*f\n",
                  DECIMALES, v[0], DECIMALES, v[1], DECIMALES, v[2], DECIMALES, v[3]);
  }
}
