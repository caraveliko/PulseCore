# Especificación de funcionalidad: [NOMBRE DE LA FUNCIONALIDAD]

**Rama de funcionalidad**: `[###-nombre-funcionalidad]`

**Creada**: [FECHA]

**Estado**: Borrador

**Entrada**: Descripción del usuario: "$ARGUMENTS"

## Escenarios de usuario y pruebas *(obligatorio)*

<!--
  IMPORTANTE: Las historias de usuario deben PRIORIZARSE como recorridos de usuario ordenados por importancia.
  Cada historia de usuario/recorrido debe ser COMPROBABLE DE FORMA INDEPENDIENTE: es decir, si implementas solo UNA de ellas,
  deberías seguir teniendo un MVP (Producto Mínimo Viable) funcional que aporte valor.

  Asigna prioridades (P1, P2, P3, etc.) a cada historia, donde P1 es la más crítica.
  Piensa en cada historia como una porción autónoma de funcionalidad que puede:
  - Desarrollarse de forma independiente
  - Probarse de forma independiente
  - Desplegarse de forma independiente
  - Demostrarse a los usuarios de forma independiente
-->

### Historia de usuario 1 - [Título breve] (Prioridad: P1)

[Describe este recorrido de usuario en lenguaje claro]

**Por qué esta prioridad**: [Explica el valor y por qué tiene este nivel de prioridad]

**Prueba independiente**: [Describe cómo puede probarse de forma independiente; p. ej., «Se puede probar por completo mediante [acción específica] y aporta [valor específico]»]

**Escenarios de aceptación**:

1. **Dado** [estado inicial], **Cuando** [acción], **Entonces** [resultado esperado]
2. **Dado** [estado inicial], **Cuando** [acción], **Entonces** [resultado esperado]

---

### Historia de usuario 2 - [Título breve] (Prioridad: P2)

[Describe este recorrido de usuario en lenguaje claro]

**Por qué esta prioridad**: [Explica el valor y por qué tiene este nivel de prioridad]

**Prueba independiente**: [Describe cómo puede probarse de forma independiente]

**Escenarios de aceptación**:

1. **Dado** [estado inicial], **Cuando** [acción], **Entonces** [resultado esperado]

---

### Historia de usuario 3 - [Título breve] (Prioridad: P3)

[Describe este recorrido de usuario en lenguaje claro]

**Por qué esta prioridad**: [Explica el valor y por qué tiene este nivel de prioridad]

**Prueba independiente**: [Describe cómo puede probarse de forma independiente]

**Escenarios de aceptación**:

1. **Dado** [estado inicial], **Cuando** [acción], **Entonces** [resultado esperado]

---

[Añade más historias de usuario según sea necesario, cada una con una prioridad asignada]

### Casos límite

<!--
  ACCIÓN REQUERIDA: El contenido de esta sección representa marcadores de posición.
  Complétalos con los casos límite adecuados.
-->

- ¿Qué ocurre cuando [condición límite]?
- ¿Cómo gestiona el sistema [escenario de error]?

## Requisitos *(obligatorio)*

<!--
  ACCIÓN REQUERIDA: El contenido de esta sección representa marcadores de posición.
  Complétalos con los requisitos funcionales adecuados.
-->

### Requisitos funcionales

- **FR-001**: El sistema DEBE [capacidad específica, p. ej., «permitir a los usuarios crear cuentas»]
- **FR-002**: El sistema DEBE [capacidad específica, p. ej., «validar direcciones de correo electrónico»]
- **FR-003**: Los usuarios DEBEN poder [interacción clave, p. ej., «restablecer su contraseña»]
- **FR-004**: El sistema DEBE [requisito de datos, p. ej., «persistir las preferencias del usuario»]
- **FR-005**: El sistema DEBE [comportamiento, p. ej., «registrar todos los eventos de seguridad»]

*Ejemplo de marcado de requisitos poco claros:*

- **FR-006**: El sistema DEBE autenticar a los usuarios mediante [REQUIERE ACLARACIÓN: método de autenticación no especificado — ¿correo/contraseña, SSO, OAuth?]
- **FR-007**: El sistema DEBE conservar los datos de usuario durante [REQUIERE ACLARACIÓN: periodo de retención no especificado]

### Entidades clave *(incluir si la funcionalidad implica datos)*

- **[Entidad 1]**: [Qué representa, atributos clave sin detalles de implementación]
- **[Entidad 2]**: [Qué representa, relaciones con otras entidades]

## Criterios de éxito *(obligatorio)*

<!--
  ACCIÓN REQUERIDA: Define criterios de éxito medibles.
  Deben ser agnósticos respecto a la tecnología y medibles.
-->

### Resultados medibles

- **SC-001**: [Métrica medible, p. ej., «Los usuarios pueden completar la creación de la cuenta en menos de 2 minutos»]
- **SC-002**: [Métrica medible, p. ej., «El sistema soporta 1000 usuarios concurrentes sin degradación»]
- **SC-003**: [Métrica de satisfacción del usuario, p. ej., «El 90 % de los usuarios completan con éxito la tarea principal en el primer intento»]
- **SC-004**: [Métrica de negocio, p. ej., «Reducir los tiques de soporte relacionados con [X] en un 50 %»]

## Supuestos

<!--
  ACCIÓN REQUERIDA: El contenido de esta sección representa marcadores de posición.
  Complétalos con los supuestos adecuados basados en valores por defecto razonables
  elegidos cuando la descripción de la funcionalidad no especificó ciertos detalles.
-->

- [Supuesto sobre los usuarios objetivo, p. ej., «Los usuarios disponen de una conexión a internet estable»]
- [Supuesto sobre límites de alcance, p. ej., «El soporte móvil queda fuera del alcance para la v1»]
- [Supuesto sobre datos/entorno, p. ej., «Se reutilizará el sistema de autenticación existente»]
- [Dependencia de un sistema/servicio existente, p. ej., «Requiere acceso a la API de perfiles de usuario existente»]
