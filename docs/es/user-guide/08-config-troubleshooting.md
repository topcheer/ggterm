# Parte 8: Configuracion, Plugins y Solucion de Problemas

## Archivo de Configuracion

Ubicacion: `~/.ggterm/config.toml`

### Todas las Opciones de Configuracion

```toml
[appearance]
theme = "dark"                  # Ver lista de temas a continuacion
font_family = "monospace"        # Nombre de familia de fuente
font_size = 14                   # Tamano de fuente en pixeles
cell_width = 8                   # Ancho de celda en pixeles
cell_height = 16                 # Altura de celda en pixeles
cursor_style = "block"           # block | underline | bar
cursor_blink = true              # Parpadeo del cursor on/off
background_opacity = 1.0         # 0.0 transparente a 1.0 opaco
padding = 8                      # Padding de contenido en pixeles
cursor_line_highlight = false    # Resaltar linea del cursor (estilo Vim)
word_chars = ""                  # Caracteres de palabra adicionales para seleccion

[terminal]
scrollback_lines = 10000         # Maximo de historial de desplazamiento
shell = ""                       # Vacio = $SHELL o /bin/sh
restore_session = false           # Restaurar pestanas/divisiones al iniciar

[ai]
enabled = false
api_endpoint = ""
model = ""

[plugins]
enabled = false
directory = "~/.ggterm/plugins"

[keybindings]
# Personalizar atajos de teclado (ver a continuacion)
new_tab = "Ctrl+T"
close_tab = "Ctrl+W"
new_split_horizontal = "Ctrl+Shift+D"
new_split_vertical = "Ctrl+Shift+\"
focus_next_pane = "Ctrl+Shift+]"
focus_prev_pane = "Ctrl+Shift+["
copy = "Ctrl+Shift+C"
paste = "Ctrl+Shift+V"
search = "Ctrl+Shift+F"
toggle_fullscreen = "F11"
zoom_in = "Ctrl+="
zoom_out = "Ctrl+-"
zoom_reset = "Ctrl+0"
reset_terminal = "Ctrl+Shift+R"
clear_screen = "Ctrl+Shift+K"
select_all = "Ctrl+Shift+A"
cycle_theme = "Ctrl+Shift+T"
open_url = "Ctrl+Shift+U"
command_palette = "Ctrl+Shift+P"
copy_cwd = "Ctrl+Shift+Alt+P"

[profiles.develop]
# Sobrescrituras opcionales por perfil
theme = "nord"
font_size = 12

[profiles.present]
theme = "light"
font_size = 18
```

### Atajos de Gestion de Configuracion

| Atajo | Accion |
|----------|--------|
| `Ctrl+Shift+,` (Cmd+,) | Abrir archivo de configuracion en el editor |
| `Ctrl+Shift+O` | Abrir archivo de configuracion (alternativo) |
| `Ctrl+Shift+J` | Editar configuracion de shell (.bashrc/.zshrc) |
| `Ctrl+Shift+Alt+E` | Exportar configuracion al portapapeles (TOML) |
| `Ctrl+Shift+Alt+I` | Importar configuracion desde el portapapeles |
| `Ctrl+Shift+Alt+R` | Restablecer configuracion a valores por defecto |
| `Ctrl+Shift+Alt+L` | Recargar configuracion desde archivo |
| `Ctrl+,` | Abrir panel de Configuracion |

### Recarga en Caliente (Hot-Reload)

Con la caracteristica `config-watch`, los cambios en `config.toml` se detectan automaticamente:
- Los cambios de tema se aplican instantaneamente
- Los cambios de tamano de fuente se aplican instantaneamente
- El limite de lineas de historial se actualiza
- Notificacion emergente: "Config reloaded"

## Personalizacion de Atajos de Teclado

Todos los atajos de teclado se pueden personalizar en la seccion `[keybindings]`. Formato de teclas:

- Teclas individuales: `F11`, `Escape`, `Tab`, `Enter`
- Teclas modificadas: `Ctrl+T`, `Ctrl+Shift+D`, `Alt+H`
- Especiales: `Ctrl+Shift+/`, `Ctrl+Shift+\`

## Plugins

### Plugins de Lua

```toml
[plugins]
enabled = true
directory = "~/.ggterm/plugins"
```

Ejemplo de plugin:
```lua
-- ~/.ggterm/plugins/hello.lua
function on_load()
    print("Hello from GGTerm plugin!")
end

function on_resize(cols, rows)
    -- Reaccionar al redimensionamiento de terminal
end
```

### Ciclo de Vida del Plugin

1. Los plugins se cargan al iniciar desde el directorio configurado
2. Se llama a `on_load()` cuando se carga el plugin
3. Runtime de Lua via `mlua`

## Persistencia de Sesion

```toml
[terminal]
restore_session = false  # predeterminado: inicio limpio
# restore_session = true  # restaurar pestanas/divisiones de la ultima sesion
```

- La sesion se guarda inmediatamente cuando se cierra un panel o pestana
- Al iniciar con `restore_session = true`, se restauran pestanas/divisiones/directorios de trabajo
- La posicion y el tamano de la ventana tambien se persisten

## Configuracion SSH

GGTerm lee la configuracion SSH desde:
- `~/.ssh/config` (importable mediante la Paleta de Comandos)
- El gestor de conexiones almacena las entradas en TOML
- Soporta autenticacion por contrasena y clave publica

## Soporte de Protocolos de Terminal

GGTerm implementa un conjunto completo de protocolos de terminal:

| Protocolo | Ejemplos | Estado |
|----------|---------|--------|
| SGR | Bold, italic, underline, blink, strikethrough, overline | Completo |
| Cursor | CSI A/B/C/D/E/F/G/H, SCP/RCP, DECSC/DECRC | Completo |
| Erase | ED, EL, DECSED (selectivo) | Completo |
| Scroll | SU, SD, DECSET 7727 (alt scroll) | Completo |
| Modes | DECSET 1/5/6/7/12/25/47/1000-1006/1015-1016/1047-1049/2004/2026/2027 | Completo |
| OSC | 0/2/4/7/8/9/10-12/52/104/110-112/133/1337/9;4 | Completo |
| DCS | XTGETTCAP, DECRQSS | Completo |
| DA | DA1/DA2/DA3 | Completo |
| DSR | Posicion del cursor, estado, estado de ventana | Completo |
| DECRQM | Todos los modos estandar + privados | Completo |
| Kitty keyboard | CSI > u push/pop, CSI = u | Completo |
| Character sets | G0/G1, US/UK/special graphics | Completo |
| DECSCUSR | Cambio de forma del cursor (6 estilos) | Completo |
| Alt screen | DECSET 47/1047/1049 con guardado/restauracion de cuadricula | Completo |

## Solucion de Problemas

### Problemas de Fuente

**Los caracteres de dibujo de lineas se muestran como cuadrados (tofu):**
- macOS: Se usa Menlo Regular (no Bold) porque Menlo Bold carece de glifos de dibujo de lineas
- La negrita se muestra mediante color brillante, no por peso

**Los caracteres CJK no se renderizan:**
- Asegurate de que `Shaping::Advanced` este habilitado (predeterminado)
- Instala fuentes CJK en tu sistema

### Terminal Atascada en Modo Incorrecto

Si el shell se comporta de forma extrana despues de que GGTerm se cuelga:
```bash
reset   # o: stty sane
```

GGTerm envia secuencias de restablecimiento al salir normalmente:
- Bracketed paste desactivado
- Seguimiento de mouse desactivado
- Teclas de cursor normales
- Cursor visible
- Teclado numerico
- Soft reset (DECSTR)

### Uso Elevado de CPU en Reposo

GGTerm duerme 50ms cuando no se necesita redibujar. Si el uso de CPU es alto:
- Comprueba si hay procesos en segundo plano que produzcan salida de terminal
- Desactiva el parpadeo del cursor: `cursor_blink = false`
- Verifica si `config-watch` esta provocando recargas excesivas

### Sesion No Se Restaura

Establece `restore_session = true` en config.toml. La sesion se guarda al cerrar pestanas/paneles y al salir de la aplicacion.

### Texto de la Barra de Pestanas Invisible

Este era un error conocido (corregido). Orden de renderizado de superposicion: primero fondos, luego texto.

### Posicion de Ventana No Se Persiste

La geometria de la ventana se guarda con los datos de sesion. Habilita `restore_session = true`.

### Problemas de Conexion SSH

- La huella digital de la clave del servidor se registra para verificacion
- Se soporta tanto autenticacion por contrasena como por clave publica
- La E/S sin bloqueo evita congelamientos de la interfaz durante la conexion

### Problemas de Conexion P2P

- Asegurate de que ambos dispositivos esten en linea
- Comprueba la configuracion del firewall (QUIC usa UDP)
- Prueba la entrada manual del ticket si falla el escaneo QR
- El fallback de relay de iroh maneja la mayoria de los escenarios NAT

### Obtener Ayuda

- Presiona `Ctrl+Shift+/` para la ayuda de atajos dentro de la aplicacion
- Presiona `Ctrl+Shift+H` para ayuda basada en IA
- Consulta los registros: `ggterm -vv` para salida de depuracion
- GitHub Issues: https://github.com/topcheer/ggterm/issues
