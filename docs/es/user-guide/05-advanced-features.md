# Parte 5: Asistente de IA

## Funciones de IA

GGTerm integra un asistente de IA que puede explicar la salida de terminal, sugerir comandos y convertir lenguaje natural a comandos de shell.

### Atajos de IA

| Atajo | Accion |
|----------|--------|
| `Ctrl+Shift+E` | Explicar la salida actual |
| `Ctrl+Shift+S` | Sugerir el siguiente comando |
| `Ctrl+Shift+H` | Ayuda — pregunta cualquier cosa sobre la terminal |
| `Ctrl+Shift+N` | Lenguaje natural a comando |
| `Esc` | Cerrar la superposicion de IA |
| `Tab` (en superposicion de IA) | Insertar comando sugerido en la terminal |
| `Ctrl+Enter` (en superposicion de IA) | Ejecutar comando sugerido inmediatamente |

### Configuracion de IA

```toml
[ai]
enabled = true
api_endpoint = "https://api.openai.com/v1"
model = "gpt-4"
```

El motor de IA usa un cliente de API compatible con OpenAI, por lo que cualquier endpoint compatible funciona.

### Contexto de IA

Al activar las funciones de IA, GGTerm construye el contexto a partir de:
- La salida actual de la terminal (pantalla visible)
- El directorio de trabajo actual (OSC 7)
- El comando en ejecucion (marcas OSC 133)
- El codigo de salida (marca D de OSC 133)

### Comandos de IA en la Paleta de Comandos

Mediante la Paleta de Comandos (`Ctrl+Shift+P`):
- `ai.explain` — Explicar la salida
- `ai.suggest` — Sugerir comando
- `ai.help` — Ayuda general

## Selector de Shell

| Atajo | Accion |
|----------|--------|
| `Ctrl+Shift+L` | Selector rapido de shell |

Abre un menu desplegable para cambiar entre los shells instalados (bash, zsh, fish, etc.) en el panel actual.

## Barra Lateral de Historial de Comandos

| Atajo | Accion |
|----------|--------|
| `Ctrl+Shift+Y` | Alternar barra lateral de historial de comandos |

Funciones:
- Historial completo de comandos ejecutados en la sesion actual
- Sincronizado desde las marcas OSC 133
- Muestra el codigo de salida (indicador verde/rojo)
- Haz clic en un comando para volver a ejecutarlo

## Navegacion de Comandos

| Atajo | Accion |
|----------|--------|
| `Ctrl+Shift+Up/Down` | Navegar entre bloques de comandos |

Usa las marcas OSC 133 para saltar entre bloques de salida de comandos anteriores.

## Fragmentos (Snippets)

El gestor de fragmentos almacena comandos de uso frecuente:
- CRUD mediante la Paleta de Comandos
- Persistencia en TOML
- Relleno de marcadores de posicion (p. ej., `$USER`, `$HOST`)
- Acceso mediante la Paleta de Comandos

## Entrada Broadcast

| Atajo | Accion |
|----------|--------|
| `Ctrl+Shift+Alt+B` | Recorrer modo broadcast |

Tres modos:
1. **None** — La entrada va solo al panel activo (predeterminado)
2. **AllPanes** — La entrada se envia a todos los paneles de la pestana activa
3. **AllTabs** — La entrada se envia a los paneles activos de todas las pestanas

La barra de estado muestra `BCAST:AllPanes` o `BCAST:AllTabs` cuando esta activo.

Comandos de broadcast adicionales mediante la Paleta de Comandos:
- Enviar `Ctrl+C` a todos los paneles
- Restablecer todas las terminales

## Grabacion de Sesion

Graba sesiones de terminal en formato asciinema v2:
- Iniciar/detener mediante la Paleta de Comandos
- La barra de estado muestra el indicador `REC`
- Salida: archivo `.cast`

## Espacios de Trabajo (Workspaces)

Los espacios de trabajo separan grupos de pestanas:

| Atajo | Accion |
|----------|--------|
| `Ctrl+Shift+Alt+W` | Recorrer espacio de trabajo |

Mediante la Paleta de Comandos:
- `workspace.next` — Cambiar al siguiente espacio de trabajo
- `workspace.prev` — Cambiar al espacio de trabajo anterior
- `workspace.add` — Crear nuevo espacio de trabajo

## Monitor de Rendimiento

| Atajo | Accion |
|----------|--------|
| `Ctrl+Shift+G` | Alternar monitor de rendimiento |

Muestra una superposicion con FPS, memoria, conteo de celdas e informacion de PID.

## Sonido

| Atajo | Accion |
|----------|--------|
| `Ctrl+Shift+M` | Alternar sonido |

Cuando esta habilitado, la campana de terminal (`\a`) reproduce un sonido audible. La barra de estado muestra el indicador `SND`.

## Vista Previa de Archivos

Al arrastrar archivos sobre la terminal, aparece una tarjeta de vista previa que muestra:
- Icono de archivo (por categoria: codigo, imagen, archivo comprimido, etc.)
- Nombre y tamano del archivo
- Color especifico de la categoria

## Selector de Color

Al pasar el cursor sobre secuencias de color ANSI, aparece una muestra de color que muestra el valor hexadecimal.

## Menu Contextual

Clic derecho en el area de contenido de la terminal:

| Accion | Descripcion |
|--------|-------------|
| Copiar | Copiar seleccion |
| Pegar | Pegar desde el portapapeles |
| Seleccionar todo | Seleccionar todo el texto |
| Buscar | Abrir barra de busqueda |
| Limpiar | Limpiar pantalla + historial |
| Restablecer | Restablecer terminal (RIS) |
| Division horizontal | Dividir izquierda/derecha |
| Division vertical | Dividir arriba/abajo |

## Notificaciones

- **Notificaciones de escritorio**: Protocolos OSC 9 (iTerm2) y OSC 777 (urxvt)
- **Informes de progreso**: OSC 9;4 muestra el porcentaje en la barra de estado
- **Campana**: Destello visual + sonido opcional
