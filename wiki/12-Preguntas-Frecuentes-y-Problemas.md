# ❓ 12. Preguntas Frecuentes (FAQ) y Solución de Problemas

En esta guía encontrarás respuesta a las dudas y situaciones más habituales a la hora de utilizar **Pulse Core**.

---

## 1. Preguntas Frecuentes de Descarga

### «Mi descarga está al 99 % y no termina nunca (la última franja es roja)»
- **Causa**: Significa que tienes descargado casi todo el archivo, pero la última parte de 9,28 MB es extremadamente rara y ninguna de las fuentes conectadas en este momento la tiene disponible.
- **Solución**: No canceles la descarga. Deja el archivo en la lista; en las redes eD2K es habitual que usuarios veteranos con copias completas se conecten cada ciertos días o fines de semana. En cuanto una fuente con esa última parte aparezca en línea, Pulse Core la descargará y completará el archivo.

### «La descarga empieza muy lenta o tarda unos minutos en coger velocidad»
- **Causa**: Las redes eD2K y Kad utilizan un sistema de **colas remotas**. Cuando te conectas a una fuente, entras en su lista de espera (p. ej. en el puesto 150). Hasta que no llegues al puesto 1 de su cola, la fuente no comenzará a enviarte datos.
- **Solución**: La paciencia es la clave en eD2K. Una vez que las fuentes van alcanzando el turno 1, la velocidad de bajada se estabiliza. Además, cuanto más compartas tú en la red, más créditos ganarás y más rápido avanzarás en las colas.

---

## 2. Preguntas Frecuentes de Conectividad

### «¿Por qué tengo ID Baja (*LowID*) en los servidores eD2K?»
- **Causa**: El puerto TCP `39735` de tu daemon no puede recibir conexiones entrantes desde Internet porque tu router o el cortafuegos de tu sistema operativo lo están bloqueando.
- **Solución**:
  1. Activa UPnP en la pestaña **Ajustes** de Pulse Core para que intente abrir el puerto automáticamente en el router.
  2. Si tu router no soporta UPnP, entra en su configuración web y crea una regla de *Port Forwarding* redirigiendo el puerto TCP `39735` y el UDP `39737` hacia la IP de tu ordenador.
  3. Consulta la guía detallada en el [Manual 05: Red eD2K y Solución de ID Baja](05-Red-eD2K-Servidores-e-ID-Alta).

### «En la pestaña Kad dice: "Estado: Desconectado"»
- **Causa**: Si acabas de instalar Pulse Core, tu archivo de contactos `nodes.dat` está vacío y el cliente no sabe con quién iniciar la comunicación.
- **Solución**: Conéctate a cualquier servidor eD2K y pon a descargar cualquier archivo popular. En cuanto el cliente contacte con los primeros usuarios, se unirá automáticamente a la red Kad y el indicador pasará a verde en menos de dos minutos. También puedes pulsar en **«Cargar nodes.dat»** para iniciarla directamente.

---

## 3. Preguntas de Biblioteca y Almacenamiento

### «¿Los enlaces duros para Plex funcionan si mis descargas están en un disco y mis películas en otro?»
- **No mediante enlaces duros**: Los enlaces duros del sistema de archivos solo pueden crearse dentro de la misma partición o disco físico (mismo inodo).
- **Qué hace Pulse Core**: Si detecta que la carpeta de destino está en otro disco físico distinto (`error EXDEV`), el motor realizará automáticamente una copia completa convencional hacia el disco de destino, conservando el archivo original para seguir compartiendo.
- **Consejo**: Para aprovechar al máximo el ahorro de espacio (*Zero-Copy*), coloca la carpeta de descargas temporales y tu biblioteca de películas dentro del mismo disco duro o volumen NAS.

### «¿Dónde se guardan los archivos temporales y cómo recupero espacio?»
- Por defecto se guardan en:
  - En Linux: `~/.local/share/eMuleRust/temp/`
  - En Windows: `%LOCALAPPDATA%\eMuleRust\temp\`
- Si cancelas una descarga desde la WebUI con la opción **«Cancelar y Borrar»**, Pulse Core eliminará automáticamente tanto el archivo `.part` como los metadatos `.part.met`, liberando el espacio en disco de inmediato.

---

## 4. Tabla de Diagnóstico Rápido

| Síntoma | Diagnóstico | Acción recomendada |
| :--- | :--- | :--- |
| Punto rojo en la barra superior | Desconexión WebSocket local | Comprueba que `mule-daemon` no se haya cerrado. |
| Velocidad de subida = 0 | Límite de subida en 0 o sin fuentes en cola | Revisa en Ajustes que `maxUploadKbs` no esté restringido a un valor ínfimo. |
| Kad en estado *Firewalled* | Puerto UDP bloqueado | Comprueba el reenvío del puerto UDP `39737` en el router. |
| Error `403 Forbidden` al entrar en la WebUI | Ataque de DNS Rebinding o Host no permitido | Accede utilizando siempre `http://127.0.0.1:4660` o añade tu nombre de dominio local a la lista permitida. |
| Trazas de `ipfilter.dat` en los logs | Conexión bloqueada por IP maliciosa | Funcionamiento normal de seguridad. El daemon está protegiendo tu conexión descartando agentes sospechosos. |
