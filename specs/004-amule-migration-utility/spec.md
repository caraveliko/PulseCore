# Especificación de funcionalidad: Utilidad de migración desde aMule (`mule-cli import-amule`)

**ID de funcionalidad**: `004-amule-migration-utility`  
**Estado**: Implementado  
**Versión objetivo**: `0.4.0`  
**Versión objetivo**: `0.4.0`  

## 1. Resumen ejecutivo
Los usuarios existentes de eMule y aMule han acumulado créditos con pares remotos, listas de servidores conocidos, contactos Kad y descargas en curso. Cambiar a `eMule-Rust` no debe descartar este estado histórico.
Esta especificación define la utilidad de migración en `mule-cli` que lee un directorio de configuración de aMule existente (por defecto: `~/.aMule`), extrae credenciales y metadatos y los importa directamente al almacenamiento de `eMule-Rust`.

## 2. Recursos importados y ubicaciones de destino
1. `cryptkey.dat` y `preferences.dat`: Userhash y par de claves RSA -> `IdentityService`.
2. `clients.met`: Tabla de créditos de clientes remotos -> `DownloadService::import_clients_met`.
3. `nodes.dat`: Contactos Kad -> `KadService::import_nodes_dat`.
4. `server.met`: Entradas de servidores eD2k -> tabla `servers` en SQLite.
5. `emfriends.met`: Lista de amigos -> `DownloadService::import_emfriends_met`.
6. `cancelled.met`: Filtros de archivos cancelados -> `DownloadService::import_cancelled_met`.
7. `known.met`: Metadatos de archivos compartidos -> `DownloadService::import_known_met`.
8. `*.part.met` y `*.part`: Descargas activas -> copiadas o movidas al directorio temporal de `mule-daemon` para su recuperación inmediata.
