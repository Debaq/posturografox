# Manejo de datos de pacientes

Posturografox registra datos de salud: identificación de la persona
evaluada y su registro de centro de presión. Este documento describe qué
guarda el programa, dónde, y qué cuidados corresponden al uso clínico.

## Qué guarda y dónde

- **Sesiones exportadas.** Un CSV por ensayo con metadatos (identificación
  ingresada, condición, superficie, geometría de la plataforma), las
  métricas calculadas y la serie temporal completa del COP.
- **Configuración de la app.** Geometría, calibración, umbrales y
  preferencias. No contiene datos de pacientes.
- **Mejor puntaje del modo juego.** Un número, sin identificación asociada.

Nada se envía a ningún servidor: todo queda en el equipo donde corre el
programa. El proyecto no incluye telemetría ni analítica.

## Recomendaciones de uso clínico

1. **Usar un identificador, no el nombre.** El campo "Paciente / ID" se usa
   para el nombre del archivo exportado. Preferir un código (ficha, RUT
   truncado, número de orden) y mantener la correspondencia código→persona
   en el sistema de fichas de la institución, no acá.
2. **Resguardar la carpeta de sesiones.** Los CSV son texto plano. Si el
   equipo es compartido, guardarlos en un perfil de usuario con contraseña
   o en un volumen cifrado.
3. **Respaldo y retención.** Definir cada cuánto se respaldan y cuándo se
   eliminan, según la normativa de la institución.
4. **Consentimiento.** La evaluación y el registro de sus datos requieren
   consentimiento informado de la persona evaluada, igual que cualquier
   otro examen.

## Marco legal (Chile)

Los datos de salud son datos personales sensibles bajo la Ley 19.628 sobre
protección de la vida privada; su tratamiento exige consentimiento expreso
y finalidad determinada. Si el uso es en una institución de salud, aplican
además sus propios protocolos de custodia de ficha clínica (Ley 20.584).

Este documento es una guía de uso del programa, no asesoría legal.
