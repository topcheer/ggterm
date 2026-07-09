# Parte 2: Pestanas, Paneles y Divisiones

## Pestanas

### Gestion de Pestanas

| Atajo | Accion |
|----------|--------|
| `Ctrl+T` | Abrir nueva pestana |
| `Ctrl+W` | Cerrar pestana actual |
| `Alt+1-9` | Cambiar a pestana N |
| `Ctrl+Tab` | Pestana siguiente |
| `Ctrl+Shift+Tab` | Pestana anterior |
| `Ctrl+Shift+\`` | Alternar ultima pestana (cambiar entre las dos mas recientes) |
| `Ctrl+Shift+T` | Reabrir ultima pestana cerrada |
| `Ctrl+Shift+N` | Abrir nueva ventana |
| `Ctrl+Shift+I` | Renombrar pestana actual |
| `Ctrl+Shift+PageUp` | Mover pestana a la izquierda |
| `Ctrl+Shift+PageDown` | Mover pestana a la derecha |
| `Ctrl+Shift+Alt+D` | Duplicar pestana actual (mismo shell + cwd) |
| `Ctrl+Shift+Alt+W` | Cerrar todas las demas pestanas |

### Interacciones de Pestanas

- **Clic en pestana**: Cambiar a esa pestana
- **Doble clic en pestana**: Renombrarla
- **Clic central en pestana**: Cerrarla (estilo navegador)
- **Arrastrar pestana**: Reordenar entre hermanas
- **Clic derecho en pestana**: Menu contextual (Cerrar, Cerrar otras, Cerrar a la derecha, Fijar/Desfijar, Dividir)
- **Clic en "+"**: Menu desplegable (Nueva pestana, Division horizontal, Division vertical)

### Fijar Pestana

Fija una pestana mediante la Paleta de Comandos para evitar el cierre accidental:
- Las pestanas fijadas muestran un indicador de fijacion
- `Ctrl+W` se ignora en pestanas fijadas
- Desfija mediante la Paleta de Comandos para cerrar

### Sincronizacion de Titulo de Pestana

Los titulos de pestana se sincronizan automaticamente con el programa en ejecucion:
- Muestra el nombre del programa desde OSC 0/2 (p. ej., "vim", "htop", "less")
- Recurre al nombre del shell (p. ej., "zsh", "bash")
- Muestra indicador de campana cuando una pestana en segundo plano recibe una campana
- Muestra `(alt)` cuando esta en modo de pantalla alternativa

## Paneles de Division

### Crear Divisiones

| Atajo | Accion |
|----------|--------|
| `Ctrl+Shift+D` | Division horizontal (izquierda | derecha) |
| `Ctrl+Shift+\` | Division vertical (arriba / abajo) |

Los nuevos paneles heredan el directorio de trabajo del panel activo (desde OSC 7).

### Navegacion de Paneles

| Atajo | Accion |
|----------|--------|
| `Ctrl+Shift+[` | Enfocar panel anterior |
| `Ctrl+Shift+]` | Enfocar panel siguiente |
| `Alt+H` | Enfocar panel izquierdo (estilo vim) |
| `Alt+J` | Enfocar panel inferior (estilo vim) |
| `Alt+K` | Enfocar panel superior (estilo vim) |
| `Alt+L` | Enfocar panel derecho (estilo vim) |

- **Clic en panel**: Cambiar el foco a ese panel
- **Rueda del mouse sobre panel**: Desplazar el contenido de ese panel

### Operaciones de Paneles

| Atajo | Accion |
|----------|--------|
| `Ctrl+Shift+X` | Intercambiar contenido del panel activo con el siguiente |
| `Ctrl+Shift+Z` | Alternar zoom del panel (maximizar/restaurar) |
| `Ctrl+Shift+Alt+Flechas` | Ajustar proporcion de division |
| `Ctrl+Shift+Alt+B` | Equilibrar paneles divididos (espaciado uniforme) |
| `Ctrl+Shift+Alt+N` | Restablecer disposicion a un solo panel |

### Zoom de Panel

`Ctrl+Shift+Z` alterna el modo zoom:
- Con zoom activado: el panel activo llena toda la ventana
- Los bordes del panel estan ocultos
- El foco del mouse esta bloqueado en el panel activo
- El arrastre del separador esta deshabilitado
- La barra de estado muestra el indicador `ZOOM`

### Arrastre de Separador

- Arrastra el separador entre paneles para redimensionarlos
- El arrastre del separador esta deshabilitado con zoom activado

### Renderizado Multi-Panel

- Cada panel renderiza su propia cuadricula de terminal de forma independiente
- El panel activo tiene un borde azul brillante
- Los paneles inactivos tienen bordes tenues
- Espacio entre paneles: 6px
- El rectangulo de recorte (scissor rect) asegura que el contenido no se desborde entre limites de paneles
