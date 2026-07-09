# Parte 4: Temas, Fuentes y Apariencia

## Temas

### 9 Temas Integrados + Auto

| Tema | Fondo | Estilo |
|-------|-----------|-------|
| `dark` | Gris oscuro | Tema oscuro por defecto |
| `light` | Blanco | Entornos brillantes |
| `dracula` | Purpura oscuro | Tema oscuro popular |
| `solarized-dark` | Azul profundo | Enfoque de desarrollo |
| `solarized-light` | Crema calido | Lectura |
| `gruvbox` | Oscuro terroso | Calido retro |
| `nord` | Azul artico | Minimalista limpio |
| `tokyo-night` | Azul marino profundo | Codificacion nocturna |
| `catppuccin-mocha` | Marron suave | Oscuro calido |
| `auto` | Sigue el SO | Cambio automatico |

### Controles de Tema

| Atajo | Accion |
|----------|--------|
| `Ctrl+Shift+Alt+T` | Recorrer temas |
| `Ctrl+Shift+T` | Recorrer temas (alternativo) |

### Tema Automatico

Cuando `theme = "auto"`, GGTerm detecta la apariencia del SO:
- **macOS**: AppleInterfaceStyle
- **Linux**: GTK_THEME / gsettings
- **Windows**: Comprobacion del registro

### Colores Dinamicos (OSC 10/11/12)

Los programas pueden sobrescribir los colores del tema en tiempo de ejecucion:
- `OSC 10` — Establecer/consultar color de primer plano
- `OSC 11` — Establecer/consultar color de fondo
- `OSC 12` — Establecer/consultar color del cursor
- `OSC 104/110/111/112` — Restablecer a valores por defecto

### Paleta Personalizada (OSC 4)

Programas como base16-shell, wal y pywal pueden establecer paletas personalizadas de 16 colores:
- `OSC 4 ; N ; rgb:RR/GG/BB` — Establecer color de paleta N
- `OSC 104 ; N` — Restablecer color de paleta N
- El renderizador aplica las sobrescrituras a los colores indexados

## Fuentes

### Controles de Fuente

| Atajo | Accion |
|----------|--------|
| `Ctrl+=` | Acercar (tamano de fuente +1.5px) |
| `Ctrl+-` | Alejar (tamano de fuente -1.5px) |
| `Ctrl+0` | Restablecer al tamano de fuente por defecto |
| `Ctrl+Shift+Wheel` | Zoom de fuente con rueda del mouse |

### Fuentes por Defecto segun Plataforma

| Plataforma | Fuente |
|----------|------|
| macOS | Menlo (solo Regular — la variante Bold carece de glifos de dibujo de lineas) |
| Linux | DejaVu Sans Mono |
| Windows | Cascadia Mono |

**Texto en negrita**: Se distingue por color brillante, no por peso de fuente (estandar xterm/Alacritty).

**Fallback CJK**: `Shaping::Advanced` habilita el fallback automatico de fuentes para caracteres CJK.

### Dimensiones de Celda

- Ancho de celda = avance de punto flotante exacto de 'M' (sin redondeo)
- Altura de celda = tamano de fuente en pixeles
- Las dimensiones reales en pixeles se reportan via CSI 14t/15t/16t

## Opacidad de Fondo

| Atajo | Accion |
|----------|--------|
| `Ctrl+Shift+Alt+]` | Aumentar opacidad (+5%) |
| `Ctrl+Shift+Alt+[` | Disminuir opacidad (-5%) |

Rango de opacidad: 0.0 (completamente transparente) a 1.0 (completamente opaco). La notificacion emergente muestra el porcentaje.

Configuracion: `[appearance] background_opacity = 0.85`

## Controles de Ventana

| Atajo | Accion |
|----------|--------|
| `F11` | Alternar pantalla completa |
| `Ctrl+Shift+Enter` | Alternar maximizado |
| `Ctrl+Shift+Alt+A` | Alternar siempre encima |
| `Ctrl+Shift+B` | Alternar barra de estado |

### Barra de titulo transparente (macOS)

En macOS, la barra de titulo se hace transparente para una apariencia integrada.

## Cursor

### Estilos de Cursor

Configuracion: `[appearance] cursor_style = "block"`

Opciones: `block`, `underline`, `bar`

Los programas pueden cambiar el estilo del cursor mediante DECSCUSR (CSI N q).

### Parpadeo del Cursor

Configuracion: `[appearance] cursor_blink = true`

- El parpadeo usa alfa de onda senoidal para un desvanecimiento suave
- El parpadeo se reinicia con la entrada del usuario
- La fase de parpadeo se comparte con el renderizado de texto parpadeante SGR 5

### Resaltado de Linea del Cursor

Configuracion: `[appearance] cursor_line_highlight = false`

Resalta toda la linea donde esta posicionado el cursor (como `cursorline` de Vim).

### Efectos del Cursor

Mediante la Paleta de Comandos:
- **cursor.trail** — El cursor deja una estela de particulas
- **cursor.glow** — El cursor tiene un efecto de brillo
- **cursor.none** — Deshabilitar efectos del cursor

## Barra de Estado

Alternar: `Ctrl+Shift+B`

La barra de estado muestra:
- Posicion del cursor (fila:col)
- Numero de pestanas
- Directorio actual (desde OSC 7)
- Host remoto (indicador SSH desde OSC 1337)
- Comando en ejecucion + temporizador
- Porcentaje de progreso (desde OSC 9;4)
- Indicador de modo broadcast
- Indicador de grabacion
- Indicador de zoom de panel
- Indicador de campana
- Indicador de alternancia de sonido
- Conteo de palabras en seleccion
- Indicador de error de configuracion

## Perfiles

Los perfiles permiten alternar entre configuraciones de apariencia:

```toml
[profiles.develop]
theme = "nord"
font_size = 12

[profiles.present]
theme = "light"
font_size = 18
```

| Atajo | Accion |
|----------|--------|
| `Ctrl+Shift+Alt+F` | Recorrer perfil de configuracion |
| `Ctrl+Shift+Alt+P` | Recorrer perfiles (alternativo) |

## Panel de Configuracion

| Atajo | Accion |
|----------|--------|
| `Ctrl+,` | Abrir panel de Configuracion |

Navega las configuraciones con las teclas de flecha, edita valores directamente.

## Superposiciones de Depuracion

| Atajo | Accion |
|----------|--------|
| `F1` | Alternar superposicion de depuracion (FPS, conteo de celdas, info de paneles) |
| `Ctrl+Shift+G` | Alternar monitor de rendimiento |

## Renderizado por Panel

Cada panel mantiene un estado de renderizador independiente:
- Modo video inverso (DECSCNM)
- Colores dinamicos de primer plano/fondo (OSC 10/11)
- Color de subrayado (SGR 58)
- Fase de texto parpadeante (SGR 5)
