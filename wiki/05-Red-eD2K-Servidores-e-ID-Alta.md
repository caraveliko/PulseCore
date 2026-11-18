# ⚡ 05. Red eD2K: Servidores, Conexión y Solución de ID Baja

La red **eD2K (eDonkey2000)** es una de las redes P2P más veteranas y ricas en contenido histórico, descargas raras y archivos de difícil localización. En esta guía desglosamos cómo funciona su arquitectura cliente-servidor y el concepto crucial de **ID Alta (*HighID*)** frente a **ID Baja (*LowID*)**.

---

## 1. El Rol de los Servidores eD2K

A diferencia de los servidores de descarga directa, los servidores eD2K **no almacenan ningún archivo**:
- Actúan como **directorios telefónicos e indexadores**: registran los archivos que tú tienes compartidos y los que otros usuarios están buscando.
- Cuando buscas un archivo, el servidor busca en su base de datos qué clientes conectados lo tienen y te devuelve sus direcciones IP para que la transferencia se realice directamente de usuario a usuario (*peer-to-peer*).

---

## 2. ¿Qué es la ID de Cliente eD2K?

Cada vez que te conectas a un servidor eD2K, este realiza una prueba de conexión hacia tu equipo para comprobar si tu puerto TCP está abierto. En función del resultado, te asigna una **ID numérica**:

### 🟢 ID Alta (*HighID*)
- Si tu puerto TCP es directamente accesible desde Internet, el servidor calcula tu ID a partir de tu dirección IP pública real ($A.B.C.D$) mediante la fórmula:
  $$\text{ID} = A + (B \times 256) + (C \times 256^2) + (D \times 256^3)$$
- Cualquier ID superior a **`16.777.216`** es una **ID Alta**.
- **Ventajas**: Puedes conectar con el 100 % de los usuarios de la red (tanto con los que tienen ID Alta como con los que tienen ID Baja). Máxima velocidad de descarga y menor tiempo de espera en colas.

### 🟡 ID Baja (*LowID*)
- Si tu puerto TCP está bloqueado por un cortafuegos o por tu router (NAT), el servidor no puede alcanzarte directamente y te asigna un número pequeño (inferior a `16.777.216`).
- **Consecuencia grave**: Dos usuarios con ID Baja **NO pueden comunicarse entre sí** porque ninguno de los dos puede recibir conexiones entrantes. Si un archivo solo lo tienen fuentes con LowID, no podrás descargarlo.

---

## 3. Guía Paso a Paso para Conseguir ID Alta

Si en la pestaña de Servidores ves que tu ID es Baja, sigue estos tres pasos para solucionarlo:

### Paso 1: Mapear los puertos en tu router (Port Forwarding)
Accede a la interfaz de administración de tu router (habitualmente entrando en `http://192.168.1.1` desde el navegador) y dirígete a la sección **Port Forwarding**, **Reenvío de Puertos** o **NAT**:

Crea dos reglas dirigidas hacia la IP local de tu ordenador o servidor (ej. `192.168.1.50`):
- **Puerto eD2K**: Puerto `39735` ➔ Protocolo **TCP**.
- **Puerto Kad**: Puerto `39737` ➔ Protocolo **UDP**.

*(Si has cambiado los puertos predeterminados en la pestaña Ajustes de Pulse Core, abre los puertos que hayas configurado).*

### Paso 2: Configurar el Cortafuegos de tu Sistema Operativo

#### En Linux (con `ufw`)
```bash
sudo ufw allow 39735/tcp comment "Pulse Core eD2K TCP"
sudo ufw allow 39737/udp comment "Pulse Core Kad UDP"
sudo ufw reload
```

#### En Windows
Al arrancar por primera vez, Windows Defender te mostrará un aviso preguntando si autorizas a `mule-daemon.exe` a comunicarse en redes privadas y públicas. Asegúrate de **marcar ambas casillas** y pulsar «Permitir acceso».

### Paso 3: ¿Qué pasa si sigues teniendo LowID? (El problema de CG-NAT)
Si has abierto los puertos en el router y en el cortafuegos pero continúas recibiendo ID Baja, es muy probable que tu proveedor de Internet (ISP) te tenga bajo **CG-NAT (Carrier-Grade NAT)**:
- Muchos operadores comparten una misma IP pública entre cientos de clientes.
- **Solución**: Llama al servicio de atención al cliente de tu operador y solicita que te saquen de CG-NAT o que te asignen una «IP pública dinámica estándar» (en la mayoría de operadores es gratuito).

---

## 4. Listas de Servidores Fiables (`server.met`)

En las redes eD2K existen servidores falsos o maliciosos (*fake servers*) creados para espiar o enviar resultados spam.

Para mantener una lista de servidores limpia y segura:
1. En la pestaña **Ajustes** de Pulse Core, mantén activada la opción **«Filtro de IPs maliciosas (`ipfilter.dat`)»**.
2. Utiliza siempre listas comunitarias de confianza mantenidas por grupos históricos de eMule (como *Gruk* o *Peerates*).
3. Pulse Core incluye protección anti-SSRF integrada que impide que servidores maliciosos intenten redirigir tu tráfico hacia dispositivos de tu red doméstica.
