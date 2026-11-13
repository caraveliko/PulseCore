---

description: "Plantilla de lista de tareas para la implementación de funcionalidades"
---

# Tareas: [NOMBRE DE LA FUNCIONALIDAD]

**Entrada**: Documentos de diseño desde `/specs/[###-nombre-funcionalidad]/`

**Prerrequisitos**: plan.md (obligatorio), spec.md (obligatorio para historias de usuario), research.md, data-model.md, contracts/

**Pruebas**: Los ejemplos siguientes incluyen tareas de prueba. Las pruebas son OPCIONALES — inclúyelas únicamente si se solicitan de forma explícita en la especificación de la funcionalidad.

**Organización**: Las tareas se agrupan por historia de usuario para permitir la implementación y comprobación independientes de cada historia.

## Formato: `[ID] [P?] [Historia] Descripción`

- **[P]**: Puede ejecutarse en paralelo (archivos distintos, sin dependencias)
- **[Historia]**: A qué historia de usuario pertenece esta tarea (p. ej., US1, US2, US3)
- Incluir rutas de archivo exactas en las descripciones

## Convenciones de rutas

- **Proyecto único**: `src/`, `tests/` en la raíz del repositorio
- **Aplicación web**: `backend/src/`, `frontend/src/`
- **Móvil**: `api/src/`, `ios/src/` o `android/src/`
- Las rutas mostradas a continuación asumen un proyecto único — ajústalas según la estructura de plan.md

<!--
  ============================================================================
  IMPORTANTE: Las tareas siguientes son TAREAS DE EJEMPLO solo a modo ilustrativo.

  El comando /speckit.tasks DEBE sustituirlas por tareas reales basadas en:
  - Historias de usuario de spec.md (con sus prioridades P1, P2, P3...)
  - Requisitos de la funcionalidad de plan.md
  - Entidades de data-model.md
  - Puntos finales (endpoints) de contracts/

  Las tareas DEBEN organizarse por historia de usuario para que cada historia pueda:
  - Implementarse de forma independiente
  - Probarse de forma independiente
  - Entregarse como un incremento MVP

  NO conserves estas tareas de ejemplo en el archivo tasks.md generado.
  ============================================================================
-->

## Fase 1: Configuración (Infraestructura compartida)

**Propósito**: Inicialización del proyecto y estructura básica

- [ ] T001 Crear la estructura del proyecto según el plan de implementación
- [ ] T002 Inicializar el proyecto de [lenguaje] con las dependencias de [framework]
- [ ] T003 [P] Configurar herramientas de *linting* y formateo

---

## Fase 2: Fundacional (Prerrequisitos bloqueantes)

**Propósito**: Infraestructura central que DEBE completarse antes de poder implementar CUALQUIER historia de usuario

**⚠️ CRÍTICO**: No puede comenzar ningún trabajo de historia de usuario hasta que esta fase esté completa

Ejemplos de tareas fundacionales (ajustar según el proyecto):

- [ ] T004 Configurar el esquema de base de datos y el marco de migraciones
- [ ] T005 [P] Implementar el marco de autenticación/autorización
- [ ] T006 [P] Configurar el enrutamiento de la API y la estructura de *middleware*
- [ ] T007 Crear los modelos/entidades base de los que dependen todas las historias
- [ ] T008 Configurar el manejo de errores y la infraestructura de registro (*logging*)
- [ ] T009 Configurar la gestión de configuración por entorno

**Punto de control**: Base lista — la implementación de historias de usuario puede comenzar ahora en paralelo

---

## Fase 3: Historia de usuario 1 - [Título] (Prioridad: P1) 🎯 MVP

**Objetivo**: [Breve descripción de lo que aporta esta historia]

**Prueba independiente**: [Cómo verificar que esta historia funciona por sí sola]

### Pruebas para la Historia de usuario 1 (OPCIONAL — solo si se solicitan pruebas) ⚠️

> **NOTA: Escribir estas pruebas PRIMERO; asegurarse de que FALLEN antes de la implementación**

- [ ] T010 [P] [US1] Prueba de contrato para [endpoint] en tests/contract/test_[name].py
- [ ] T011 [P] [US1] Prueba de integración para [recorrido de usuario] en tests/integration/test_[name].py

### Implementación para la Historia de usuario 1

- [ ] T012 [P] [US1] Crear el modelo [Entidad1] en src/models/[entity1].py
- [ ] T013 [P] [US1] Crear el modelo [Entidad2] en src/models/[entity2].py
- [ ] T014 [US1] Implementar [Servicio] en src/services/[service].py (depende de T012, T013)
- [ ] T015 [US1] Implementar [endpoint/funcionalidad] en src/[location]/[file].py
- [ ] T016 [US1] Añadir validación y manejo de errores
- [ ] T017 [US1] Añadir registro (*logging*) para las operaciones de la historia de usuario 1

**Punto de control**: En este punto, la Historia de usuario 1 debería ser completamente funcional y comprobable de forma independiente

---

## Fase 4: Historia de usuario 2 - [Título] (Prioridad: P2)

**Objetivo**: [Breve descripción de lo que aporta esta historia]

**Prueba independiente**: [Cómo verificar que esta historia funciona por sí sola]

### Pruebas para la Historia de usuario 2 (OPCIONAL — solo si se solicitan pruebas) ⚠️

- [ ] T018 [P] [US2] Prueba de contrato para [endpoint] en tests/contract/test_[name].py
- [ ] T019 [P] [US2] Prueba de integración para [recorrido de usuario] en tests/integration/test_[name].py

### Implementación para la Historia de usuario 2

- [ ] T020 [P] [US2] Crear el modelo [Entidad] en src/models/[entity].py
- [ ] T021 [US2] Implementar [Servicio] en src/services/[service].py
- [ ] T022 [US2] Implementar [endpoint/funcionalidad] en src/[location]/[file].py
- [ ] T023 [US2] Integrar con los componentes de la Historia de usuario 1 (si es necesario)

**Punto de control**: En este punto, las Historias de usuario 1 Y 2 deberían funcionar de forma independiente

---

## Fase 5: Historia de usuario 3 - [Título] (Prioridad: P3)

**Objetivo**: [Breve descripción de lo que aporta esta historia]

**Prueba independiente**: [Cómo verificar que esta historia funciona por sí sola]

### Pruebas para la Historia de usuario 3 (OPCIONAL — solo si se solicitan pruebas) ⚠️

- [ ] T024 [P] [US3] Prueba de contrato para [endpoint] en tests/contract/test_[name].py
- [ ] T025 [P] [US3] Prueba de integración para [recorrido de usuario] en tests/integration/test_[name].py

### Implementación para la Historia de usuario 3

- [ ] T026 [P] [US3] Crear el modelo [Entidad] en src/models/[entity].py
- [ ] T027 [US3] Implementar [Servicio] en src/services/[service].py
- [ ] T028 [US3] Implementar [endpoint/funcionalidad] en src/[location]/[file].py

**Punto de control**: Todas las historias de usuario deberían ser ahora funcionalmente independientes

---

[Añadir más fases de historias de usuario según sea necesario, siguiendo el mismo patrón]

---

## Fase N: Pulido y aspectos transversales

**Propósito**: Mejoras que afectan a múltiples historias de usuario

- [ ] TXXX [P] Actualizaciones de documentación en docs/
- [ ] TXXX Limpieza y refactorización de código
- [ ] TXXX Optimización de rendimiento en todas las historias
- [ ] TXXX [P] Pruebas unitarias adicionales (si se solicitan) en tests/unit/
- [ ] TXXX Endurecimiento de seguridad
- [ ] TXXX Ejecutar validación de quickstart.md

---

## Dependencias y orden de ejecución

### Dependencias entre fases

- **Configuración (Fase 1)**: Sin dependencias — puede comenzar de inmediato
- **Fundacional (Fase 2)**: Depende de completar la Configuración — BLOQUEA todas las historias de usuario
- **Historias de usuario (Fase 3+)**: Todas dependen de completar la fase Fundacional
  - Las historias de usuario pueden avanzar en paralelo (si hay personal suficiente)
  - O secuencialmente en orden de prioridad (P1 → P2 → P3)
- **Pulido (Fase final)**: Depende de completar todas las historias de usuario deseadas

### Dependencias entre historias de usuario

- **Historia de usuario 1 (P1)**: Puede comenzar tras la fase Fundacional (Fase 2) — Sin dependencias de otras historias
- **Historia de usuario 2 (P2)**: Puede comenzar tras la fase Fundacional (Fase 2) — Puede integrarse con US1 pero debe ser comprobable de forma independiente
- **Historia de usuario 3 (P3)**: Puede comenzar tras la fase Fundacional (Fase 2) — Puede integrarse con US1/US2 pero debe ser comprobable de forma independiente

### Dentro de cada historia de usuario

- Las pruebas (si se incluyen) DEBEN escribirse y FALLAR antes de la implementación
- Modelos antes que servicios
- Servicios antes que puntos finales (*endpoints*)
- Implementación central antes que integración
- Historia completada antes de pasar a la siguiente prioridad

### Oportunidades de paralelismo

- Todas las tareas de Configuración marcadas con [P] pueden ejecutarse en paralelo
- Todas las tareas Fundacionales marcadas con [P] pueden ejecutarse en paralelo (dentro de la Fase 2)
- Una vez completada la fase Fundacional, todas las historias de usuario pueden comenzar en paralelo (si la capacidad del equipo lo permite)
- Todas las pruebas de una historia de usuario marcadas con [P] pueden ejecutarse en paralelo
- Los modelos dentro de una historia marcados con [P] pueden ejecutarse en paralelo
- Distintos miembros del equipo pueden trabajar en paralelo en diferentes historias de usuario

---

## Ejemplo en paralelo: Historia de usuario 1

```bash
# Lanzar juntas todas las pruebas de la Historia de usuario 1 (si se solicitan pruebas):
Task: "Prueba de contrato para [endpoint] en tests/contract/test_[name].py"
Task: "Prueba de integración para [recorrido de usuario] en tests/integration/test_[name].py"

# Lanzar juntos todos los modelos de la Historia de usuario 1:
Task: "Crear el modelo [Entidad1] en src/models/[entity1].py"
Task: "Crear el modelo [Entidad2] en src/models/[entity2].py"
```

---

## Estrategia de implementación

### MVP primero (Solo Historia de usuario 1)

1. Completar Fase 1: Configuración
2. Completar Fase 2: Fundacional (CRÍTICO — bloquea todas las historias)
3. Completar Fase 3: Historia de usuario 1
4. **DETENERSE y VALIDAR**: Probar la Historia de usuario 1 de forma independiente
5. Desplegar/demostrar si está lista

### Entrega incremental

1. Completar Configuración + Fundacional → Base lista
2. Añadir Historia de usuario 1 → Probar de forma independiente → Desplegar/Demostrar (¡MVP!)
3. Añadir Historia de usuario 2 → Probar de forma independiente → Desplegar/Demostrar
4. Añadir Historia de usuario 3 → Probar de forma independiente → Desplegar/Demostrar
5. Cada historia aporta valor sin romper las historias anteriores

### Estrategia de equipo en paralelo

Con varios desarrolladores:

1. El equipo completa Configuración + Fundacional conjuntamente
2. Una vez completada la fase Fundacional:
   - Desarrollador A: Historia de usuario 1
   - Desarrollador B: Historia de usuario 2
   - Desarrollador C: Historia de usuario 3
3. Las historias se completan e integran de forma independiente

---

## Notas

- Tareas [P] = archivos distintos, sin dependencias
- La etiqueta [Historia] asocia la tarea con una historia de usuario específica para trazabilidad
- Cada historia de usuario debe poder completarse y probarse de forma independiente
- Verificar que las pruebas fallen antes de implementar
- Hacer *commit* tras cada tarea o grupo lógico
- Detenerse en cualquier punto de control para validar la historia de forma independiente
- Evitar: tareas vagas, conflictos en el mismo archivo, dependencias entre historias que rompan la independencia
