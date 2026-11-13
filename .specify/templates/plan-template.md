# Plan de implementación: [FUNCIONALIDAD]

**Rama**: `[###-nombre-funcionalidad]` | **Fecha**: [FECHA] | **Especificación**: [enlace]

**Entrada**: Especificación de funcionalidad desde `/specs/[###-nombre-funcionalidad]/spec.md`

**Nota**: Esta plantilla la rellena el comando `/speckit.plan`; su definición describe el flujo de ejecución.

## Resumen

[Extraer de la especificación de funcionalidad: requisito principal + enfoque técnico a partir de la investigación]

## Contexto técnico

<!--
  ACCIÓN REQUERIDA: Sustituye el contenido de esta sección por los detalles técnicos
  del proyecto. La estructura aquí presentada tiene carácter orientativo para guiar
  el proceso de iteración.
-->

**Lenguaje/Versión**: [p. ej., Python 3.11, Swift 5.9, Rust 1.75 o REQUIERE ACLARACIÓN]

**Dependencias principales**: [p. ej., FastAPI, UIKit, LLVM o REQUIERE ACLARACIÓN]

**Almacenamiento**: [si aplica, p. ej., PostgreSQL, CoreData, archivos o N/A]

**Pruebas**: [p. ej., pytest, XCTest, cargo test o REQUIERE ACLARACIÓN]

**Plataforma objetivo**: [p. ej., servidor Linux, iOS 15+, WASM o REQUIERE ACLARACIÓN]

**Tipo de proyecto**: [p. ej., biblioteca/cli/servicio-web/app-móvil/compilador/app-escritorio o REQUIERE ACLARACIÓN]

**Objetivos de rendimiento**: [específicos del dominio, p. ej., 1000 req/s, 10k líneas/s, 60 fps o REQUIERE ACLARACIÓN]

**Restricciones**: [específicas del dominio, p. ej., <200ms p95, <100MB memoria, funcionamiento sin conexión o REQUIERE ACLARACIÓN]

**Escala/Alcance**: [específicos del dominio, p. ej., 10k usuarios, 1M LOC, 50 pantallas o REQUIERE ACLARACIÓN]

## Comprobación de la Constitución

*PUERTA: Debe superarse antes de la investigación de la Fase 0. Volver a comprobar tras el diseño de la Fase 1.*

[Puertas determinadas a partir del archivo de constitución]

## Estructura del proyecto

### Documentación (esta funcionalidad)

```text
specs/[###-funcionalidad]/
├── plan.md              # Este archivo (salida del comando /speckit.plan)
├── research.md          # Salida de la Fase 0 (/speckit.plan)
├── data-model.md        # Salida de la Fase 1 (/speckit.plan)
├── quickstart.md        # Salida de la Fase 1 (/speckit.plan)
├── contracts/           # Salida de la Fase 1 (/speckit.plan)
└── tasks.md             # Salida de la Fase 2 (/speckit.tasks - NO creado por /speckit.plan)
```

### Código fuente (raíz del repositorio)
<!--
  ACCIÓN REQUERIDA: Sustituye el árbol de marcadores de posición siguiente por la disposición concreta
  para esta funcionalidad. Elimina las opciones no utilizadas y amplía la estructura elegida con
  rutas reales (p. ej., apps/admin, packages/algo). El plan entregado no debe
  incluir etiquetas de Opción.
-->

```text
# [ELIMINAR SI NO SE USA] Opción 1: Proyecto único (POR DEFECTO)
src/
├── models/
├── services/
├── cli/
└── lib/

tests/
├── contract/
├── integration/
└── unit/

# [ELIMINAR SI NO SE USA] Opción 2: Aplicación web (cuando se detectan "frontend" + "backend")
backend/
├── src/
│   ├── models/
│   ├── services/
│   └── api/
└── tests/

frontend/
├── src/
│   ├── components/
│   ├── pages/
│   └── services/
└── tests/

# [ELIMINAR SI NO SE USA] Opción 3: Móvil + API (cuando se detecta "iOS/Android")
api/
└── [igual que backend arriba]

ios/ o android/
└── [estructura específica de plataforma: módulos de funcionalidad, flujos UI, pruebas de plataforma]
```

**Decisión de estructura**: [Documentar la estructura seleccionada y referenciar los directorios reales capturados arriba]

## Seguimiento de complejidad

> **Rellenar ÚNICAMENTE si la Comprobación de la Constitución tiene incumplimientos que deban justificarse**

| Incumplimiento | Por qué es necesario | Alternativa más simple descartada porque |
|----------------|----------------------|------------------------------------------|
| [p. ej., 4.º proyecto] | [necesidad actual] | [por qué 3 proyectos son insuficientes] |
| [p. ej., patrón Repositorio] | [problema específico] | [por qué el acceso directo a BD es insuficiente] |
