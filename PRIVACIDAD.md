# Manejo de datos de pacientes

Posturografox registra datos de salud: identificación de la persona
evaluada y su registro de centro de presión. Este documento describe qué
guarda el programa, dónde, y qué cuidados corresponden al uso clínico.

## Qué guarda y dónde

- **Base de pacientes de la suite.** `vhit.sqlite`, por defecto en
  `~/.local/share/vhit` (`%APPDATA%\vhit` en Windows). Es la **misma base que
  usa vHIT**: una ficha por persona (nombre, número de ficha, fecha de
  nacimiento, notas) y, colgados de ella, los exámenes de cada equipo. Los de
  posturografía incluyen las métricas, la configuración con la que se midieron
  y el registro de COP completo. Está **cifrada con SQLCipher (AES-256)** salvo
  que alguien elija lo contrario al configurarla; la frase de paso no se guarda
  en ningún lado, así que si se pierde los datos no se recuperan.
  - Dejarla sin cifrar es una decisión del responsable de los datos: queda
    fechada en `almacenamiento.ron` y el programa lo dice en rojo en la ventana
    de pacientes mientras siga así.
  - Borrar un paciente borra **todos** sus exámenes, también los de vHIT.
  - Cambiar la frase de paso la cambia para las dos aplicaciones: es la clave
    de la base, no del programa.
- **Historial local.** `historial.ronl`, una línea por sesión con el
  identificador tipeado y las métricas, sin cifrar. Es el archivo que funciona
  sin base abierta y sin frase de paso.
- **Sesiones exportadas.** Un CSV por ensayo con metadatos (identificación
  ingresada, condición, superficie, geometría de la plataforma), las
  métricas calculadas y la serie temporal completa del COP.
- **Configuración de la app.** Geometría, calibración, umbrales y
  preferencias. No contiene datos de pacientes.
- **Mejor puntaje del modo juego.** Un número, sin identificación asociada.

Nada se envía a ningún servidor: todo queda en el equipo donde corre el
programa. El proyecto no incluye telemetría ni analítica.

Las reglas de convivencia de las dos aplicaciones sobre el mismo archivo están
en `vhit-wout-google/SUITE.md`.

## Recomendaciones de uso clínico

1. **Dejar la base cifrada.** Es lo que pide la normativa para datos de salud
   en reposo, y es lo que el programa hace por defecto. Sin cifrado, el archivo
   lo abre cualquiera que llegue al disco con cualquier visor de SQLite.
2. **Cerrar la base al terminar con el paciente.** El botón está a la vista en
   la ventana de pacientes: deja de guardar y olvida la frase, y el programa
   sigue midiendo.
3. **Los archivos que salen del equipo van en claro.** Los CSV, los informes y
   el historial local son texto plano, aunque la base esté cifrada. Si el
   equipo es compartido, guardarlos en un perfil con contraseña o en un volumen
   cifrado, y para nombrar archivos preferir el número de ficha antes que el
   nombre completo.
4. **Respaldo y retención.** Definir cada cuánto se respaldan y cuándo se
   eliminan, según la normativa de la institución.
5. **Consentimiento.** La evaluación y el registro de sus datos requieren
   consentimiento informado de la persona evaluada, igual que cualquier
   otro examen.

## Marco legal (Chile)

Los datos de salud son datos personales sensibles bajo la Ley 19.628 sobre
protección de la vida privada; su tratamiento exige consentimiento expreso
y finalidad determinada. Si el uso es en una institución de salud, aplican
además sus propios protocolos de custodia de ficha clínica (Ley 20.584).

Este documento es una guía de uso del programa, no asesoría legal.
