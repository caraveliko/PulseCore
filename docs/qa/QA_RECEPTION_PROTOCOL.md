# Protocolo de Recepción QA y Validación Interna — eMuleRust RC v0.1.0-lab

Este documento define las comprobaciones obligatorias que debe ejecutar cualquier tester interno que reciba el paquete autónomo **eMuleRust Release Candidate** (`v0.1.0-lab`) en una máquina Windows 10 o Windows 11 limpia.

---

> [!IMPORTANT]
> **Alcance y Confinamiento de Red Estricto**  
> *eMuleRust* es una aplicación de laboratorio local con compatibilidad eD2K experimental.  
> - **Cero red pública**: ningún intento de conexión hacia un servidor eD2K externo, ningún nodo Kad, ningún intercambio P2P en Internet.  
> - **Loopback exclusivo**: todas las comunicaciones entre procesos (IPC WebSocket JSON-RPC y pares de prueba locales) están confinadas a la dirección `127.0.0.1`.  
> - **Certificación formal**: *«Compatibilidad eD2K experimental validada sobre corpus estático y par de referencia emulado, en entorno loopback cerrado. El arnés Tier 3 para un binario eMule/aMule externo fijado está disponible, pero su prueba en vivo permanece ignorada por defecto mientras no se proporcione un binario verificado.»*

---

## 1. Verificación Criptográfica del Archivo

Antes de descomprimir nada, comprueba la huella SHA-256 del fichero ZIP descargado:

```powershell
Get-FileHash .\eMuleRust-v0.1.0-lab-windows-x64.zip -Algorithm SHA256
```

La huella obtenida debe coincidir rigurosamente con el valor publicado en el fichero compañero `.zip.sha256`:

```text
09023bb37bfb45d5a028ada95f5681a4e42df451badd04ca4b9d4ecc99836245
```
*(o la huella sellada de la Release Candidate correspondiente)*

---

## 2. Extracción y Control de Integridad de los Ficheros

Extrae el archivo en una carpeta de tu elección (p. ej.: `C:\eMuleRust-RC` o `%LOCALAPPDATA%\Programs\eMuleRust`):

```powershell
Expand-Archive -Path .\eMuleRust-v0.1.0-lab-windows-x64.zip -DestinationPath .\
```

Después ejecuta el bucle de verificación criptográfica sobre la totalidad de los ficheros del paquete:

```powershell
Get-Content .\eMuleRust-Windows\SHA256SUMS.txt |
ForEach-Object {
    $parts = $_ -split '\s{2,}', 2
    if ($parts.Count -eq 2) {
        $expected = $parts[0].Trim().ToLowerInvariant()
        $relPath = $parts[1].Trim()
        $path = Join-Path .\eMuleRust-Windows $relPath
        if (!(Test-Path $path)) {
            throw "Fichero faltante en el paquete: $relPath"
        }
        $actual = (Get-FileHash $path -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne $expected) {
            throw "Error de integridad (hash mismatch) en: $relPath (esperado: $expected, obtenido: $actual)"
        }
    }
}
Write-Host "✅ Integridad verificada con éxito: el 100 % de los ficheros son conformes." -ForegroundColor Green
```

---

## 3. Examen del Manifiesto y de las Licencias

1. **Manifiesto de versión**: abre `.\eMuleRust-Windows\version.json` y verifica:
   - `schemaVersion`: `1`
   - `product`: `"eMuleRust"`
   - `version`: `"0.1.0-lab"`
   - `gitDirty`: `false` (prueba de compilación desde un commit limpio)
   - `signed`: `false` (distribución de laboratorio sin firma Authenticode comercial)
   - `artifacts`: lista de todos los binarios y ficheros de datos con hashes SHA-256.

2. **Inventario SBOM y Licencias**:
   - `.\eMuleRust-Windows\sbom\sbom.cdx.json`: inventario CycloneDX 1.5 estandarizado de los 239 componentes.
   - `.\eMuleRust-Windows\sbom\license-audit.md`: informe de conformidad que confirma un 100 % de adecuación con la política `deny.toml` (licencias permisivas MIT, Apache-2.0, BSD, ISC y GPL-2.0-or-later para las fuentes derivadas).

---

## 4. Arranque y Validación de la Interfaz de Usuario

### 4.1 Ejecución
Inicia la aplicación principal:
```powershell
.\eMuleRust-Windows\eMuleRust.exe
```

### 4.2 Comportamiento Esperado
1. El ejecutable `eMuleRust.exe` arranca sin privilegios de administrador (cuenta de usuario Windows estándar).
2. La interfaz Flutter Desktop se muestra instantáneamente con el tema oscuro moderno de eMule.
3. El daemon en segundo plano `bin\mule-daemon.exe` se inicializa automáticamente mediante el supervisor en el directorio de usuario `%LOCALAPPDATA%\eMuleRust` (o `.mule_data`).
4. El indicador de estado arriba a la derecha pasa a **«Conectado al daemon local»** (verde).
5. No debe aparecer ningún mensaje de alerta del cortafuegos de Windows, ya que el socket escucha exclusivamente en loopback (`127.0.0.1`).

---

## 5. Matriz de Validación del Supervisor

| Escenario | Acción QA | Resultado Esperado |
| :--- | :--- | :--- |
| **Puerto 4660 ocupado** | Lanzar otra aplicación en el puerto TCP 4660 antes de abrir eMuleRust. | El daemon cambia automáticamente a un puerto efímero de loopback (`127.0.0.1:XXXX`). La interfaz descubre el puerto mediante `session.json` y se conecta sin errores. |
| **Dos instancias UI simultáneas** | Hacer doble clic una segunda vez en `eMuleRust.exe`. | La segunda instancia se conecta al daemon ya activo sin intentar iniciar un segundo (`mule.lock` respetado). |
| **Detención inesperada del daemon** | Matar `mule-daemon.exe` desde el Administrador de tareas. | La interfaz muestra el estado «Reconectando...» con retroceso exponencial, limpia el bloqueo obsoleto y reinicia un daemon limpio. |
| **Carpeta con espacios/acentos** | Mover la carpeta `eMuleRust-Windows` a `C:\Test ÉmuleRust 📁\`. | La aplicación y su daemon hijo arrancan y funcionan sin errores de ruta. |
| **Cierre de la UI** | Cerrar la ventana principal de eMuleRust. | El daemon hijo se detiene limpiamente y `session.json` se elimina automáticamente. |

---

## 6. Reglas de Notificación de Anomalías

Si se detecta cualquier desviación durante este protocolo:
1. Anota el código de error exacto o la huella no coincidente.
2. Comprueba si el antivirus (p. ej.: Windows Defender) ha bloqueado el acceso al puerto local o al fichero de token temporal.
3. No transmitas **ningún fichero de sesión**, ni registros que contengan rutas personales o nombres de usuario.
