# Política de Seguridad — eMuleRust

## Versiones compatibles

| Versión | Soporte |
| :--- | :--- |
| `0.2.x` (último lanzamiento) | ✅ Correcciones de seguridad |
| `< 0.2.0` / `-lab` / `-RC` | ❌ Sin soporte |

## Superficie de seguridad

eMuleRust es un cliente P2P expuesto a la red. Los siguientes mecanismos defensivos son **no negociables** y cualquier regresión en ellos se trata como un incidente de seguridad:

- Canalización anti-SSRF en 7 etapas (`mule-network-policy`)
- Aislamiento loopback por defecto e interruptor de corte (*kill switch*) atómico
- Filtrado IP (`ipfilter.dat`, `mule-filter`)
- Seudonimización con sal (*salted*) de los pares
- Aislamiento de las funcionalidades de laboratorio tras la característica `lab-tools` — nunca presentes en las compilaciones públicas `standard`

## Notificar una vulnerabilidad

**No abrir una incidencia pública** para una vulnerabilidad no divulgada.

Utiliza preferentemente **GitHub Security Advisories** (pestaña *Security* → *Report a vulnerability*) en el repositorio, o contacta con el mantenedor por correo electrónico: **Laurent Geynet "Loulach"** <Dev.iptvmediacenter@gmail.com>.

Incluye, si es posible:

- Versión afectada (`version.json` o etiqueta)
- Componente (`mule-daemon`, `mule-protocol`, WebUI…)
- Descripción del impacto y escenario de explotación
- Reproducción o prueba de concepto (preferiblemente en entorno sin conexión/loopback)

### Plazos orientativos

- Acuse de recibo: en menos de 7 días
- Evaluación y plan de corrección: en menos de 30 días
- Divulgación coordinada tras la publicación del parche

## Cadena de suministro y seguridad previa

- `deny.toml` (cargo-deny): CVE/RustSec en `deny`, licencias restringidas
- SBOM CycloneDX generada en cada lanzamiento (`sbom/` del paquete)
- `SHA256SUMS.txt` firmado por compilación para verificación de integridad
