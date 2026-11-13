# Lista de comprobación de [TIPO DE LISTA]: [NOMBRE DE LA FUNCIONALIDAD]

**Propósito**: [Breve descripción de lo que cubre esta lista de comprobación]
**Creada**: [FECHA]
**Funcionalidad**: [Enlace a spec.md o documentación relevante]

**Nota**: Esta lista de comprobación personalizada se genera mediante el comando `/speckit.checklist` en función del contexto y los requisitos de la funcionalidad.
**Propiedad de la revisión**: Esta lista es un artefacto de revisión de calidad de requisitos perteneciente al revisor. Marca un elemento `[x]` únicamente cuando el revisor determine que se cumple el criterio de calidad de los requisitos.
**Semántica de la marca**: `[x]` significa que el criterio ha sido revisado y cumplido en cuanto a calidad de requisitos. No significa que el trabajo de implementación esté completado.

<!--
  ============================================================================
  IMPORTANTE: Los elementos de la lista a continuación son ELEMENTOS DE EJEMPLO solo a modo ilustrativo.

  El comando /speckit.checklist DEBE sustituirlos por elementos reales basados en:
  - La solicitud específica de lista de comprobación del usuario
  - Los requisitos de la funcionalidad en spec.md
  - El contexto técnico de plan.md
  - Los detalles de implementación de tasks.md

  NO conserves estos elementos de ejemplo en el archivo de lista generado.
  ============================================================================
-->

## [Categoría 1]

- [ ] CHK001 Primer elemento de la lista con acción clara
- [ ] CHK002 Segundo elemento de la lista
- [ ] CHK003 Tercer elemento de la lista

## [Categoría 2]

- [ ] CHK004 Otro elemento de categoría
- [ ] CHK005 Elemento con criterios específicos
- [ ] CHK006 Último elemento de esta categoría

## Notas

- Marca los elementos `[x]` solo después de que la revisión confirme que se cumple el criterio de calidad del requisito
- Deja sin marcar los elementos cuando aún requieran aclaración, corrección o evaluación del revisor
- `/speckit.implement` lee el estado de las casillas de la lista como puerta de control y no debe modificar las marcas
- `checklists/requirements.md` tiene un ciclo de vida integrado independiente mantenido por `/speckit.specify` y `/speckit.clarify`
- Añade comentarios o hallazgos en línea
- Enlaza los recursos o documentación relevantes
- Los elementos se numeran secuencialmente para facilitar su referencia
