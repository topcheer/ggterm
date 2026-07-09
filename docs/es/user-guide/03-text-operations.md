# Parte 3: Seleccion de Texto, Copiar y Pegar

## Seleccion de Texto

### Modos de Seleccion

| Accion | Resultado |
|--------|--------|
| Clic + Arrastrar | Seleccion de texto normal |
| `Alt` + Clic + Arrastrar | Seleccion de bloque (rectangular) |
| Doble clic | Seleccionar palabra |
| Triple clic | Seleccionar linea completa |

### Seleccion mediante Teclado

| Atajo | Accion |
|----------|--------|
| `Ctrl+Shift+A` | Seleccionar todo el texto |
| `Shift+Flechas` | Extender seleccion por caracter |
| `Ctrl+Shift+Left/Right` | Extender seleccion por palabra |

### Resaltado de Seleccion

El texto seleccionado se resalta con una superposicion azul semitransparente. Para la seleccion de bloque, se renderizan rectangulos por fila.

### Conteo de Palabras en Seleccion

Cuando hay texto seleccionado, la barra de estado muestra el conteo de caracteres y palabras:
- `SEL:42c/7w` — 42 caracteres, 7 palabras
- El sufijo `w` se omite cuando hay 0 palabras (seleccion solo de espacios en blanco)

## Copiar

| Atajo | Accion |
|----------|--------|
| `Ctrl+Shift+C` | Copiar seleccion al portapapeles |
| `Ctrl+Insert` | Copiar seleccion (convencion de Linux/Windows) |
| `Ctrl+Shift+Alt+H` | Copiar seleccion como HTML (con colores) |
| `Ctrl+Shift+Alt+O` | Copiar salida del ultimo comando (usa marcas OSC 133) |
| `Ctrl+Shift+Alt+P` | Copiar ruta del directorio de trabajo actual |

Comandos de copiado adicionales mediante la Paleta de Comandos:
- **Copiar texto visible** — copia solo la pantalla visible (sin historial de desplazamiento)
- **Copiar como Markdown** — convierte la salida de terminal a formato Markdown
- **Copiar como HTML** — preserva colores y formato

### Recorte Inteligente al Copiar

Las lineas vacias al inicio y al final se eliminan automaticamente del texto copiado.

## Pegar

| Atajo | Accion |
|----------|--------|
| `Ctrl+Shift+V` | Pegar desde el portapapeles |
| `Shift+Insert` | Pegar (convencion de Linux/Windows) |
| Clic central | Pegar seleccion (estilo X11) |

### Pegado con Delimitadores (Bracketed Paste)

Cuando el shell soporta bracketed paste (la mayoria de los shells modernos lo hacen):
- El contenido pegado se envuelve con `ESC[200~ ... ESC[201~`
- El shell puede manejar el pegado multi-linea de forma segura

### Pegado Seguro

Cuando NO se soporta bracketed paste:
- Los saltos de linea finales se eliminan para evitar la ejecucion accidental de comandos
- Notificacion emergente: "Pasted first line (N lines stripped)"

## Buscar

| Atajo | Accion |
|----------|--------|
| `Ctrl+Shift+F` | Alternar barra de busqueda flotante |
| `Enter` | Coincidencia siguiente |
| `Shift+Enter` | Coincidencia anterior |
| `Tab` (en busqueda) | Alternar sensibilidad de mayusculas |
| `Shift+Tab` (en busqueda) | Alternar modo de busqueda con regex |
| `Up/Down` (en busqueda) | Navegar historial de busqueda |
| `Esc` | Cerrar barra de busqueda |

### Funciones de Busqueda

- **Barra de busqueda flotante** con contador de coincidencias (p. ej., "3/12 matches")
- **Historial de busqueda**: ultimas 20 consultas guardadas, navegables con Up/Down
- **Alternancia de mayusculas**: sensible o insensible a mayusculas
- **Modo regex**: soporte completo de expresiones regulares
- **Resaltado**: las coincidencias se resaltan en la cuadricula de terminal

Comandos de busqueda adicionales mediante la Paleta de Comandos:
- **Buscar seleccion** — buscar usando el texto actualmente seleccionado
- **Buscar en web** — buscar texto seleccionado en el navegador

## Exportar

| Atajo | Accion |
|----------|--------|
| `Ctrl+Shift+Alt+S` | Guardar historial de desplazamiento en archivo de texto (`~/ggterm-export-{timestamp}.txt`) |
| `Ctrl+Shift+Alt+E` | Exportar terminal como HTML (con colores) |

Comandos de exportacion mediante la Paleta de Comandos:
- **Exportar historial de desplazamiento** — archivo de texto plano
- **Exportar como HTML** — preserva colores, formato, hiperenlaces

## Bloqueo de Terminal

| Atajo | Accion |
|----------|--------|
| `Ctrl+Shift+Alt+L` | Alternar bloqueo de terminal (modo solo lectura) |

Cuando esta bloqueado, toda la entrada de teclado se bloquea. Util cuando quieres leer la salida sin escribir accidentalmente.

## Desplazamiento

### Desplazamiento con Teclado

| Atajo | Accion |
|----------|--------|
| `Ctrl+Shift+Space` | Alternar modo de exploracion del historial (estilo less: j/k/G/g/d/u/q) |
| `Shift+PageUp` | Desplazar arriba una pagina |
| `Shift+PageDown` | Desplazar abajo una pagina |
| `Shift+Home` | Desplazar al inicio del historial |
| `Shift+End` | Desplazar al final |
| `Ctrl+Shift+End` | Desplazar al final (alternativo) |
| `Ctrl+Shift+Alt+Up` | Desplazar a marca (OSC 1337 SetMark) |

### Desplazamiento con Mouse

- **Rueda de desplazamiento**: Desplazar por el historial
- **Shift+Desplazamiento**: Desplazamiento sincronizado de todos los paneles simultaneamente
- **Barra de desplazamiento**: Clic o arrastrar la barra delgada en el borde derecho

### Indicadores de Desplazamiento

- **Pildora de desplazamiento al final**: Aparece una pildora azul con flecha hacia abajo cuando estas desplazado hacia arriba; haz clic para saltar al final
- **Indicador de porcentaje**: Muestra la posicion de desplazamiento como porcentaje (p. ej., "45%")
- **Conteo de lineas**: Muestra el conteo de lineas cuando se desplaza mas de 99 lineas (p. ej., "127 lines")

### Desplazamiento Inercial Suave

El momentum del trackpad es soportado con interpolacion de decaimiento exponencial para un desplazamiento suave.

### Desplazamiento en Pantalla Alternativa

En aplicaciones de pantalla completa (vim, less, htop) sin seguimiento de mouse:
- La rueda del mouse se convierte automaticamente en pulsaciones de teclas de flecha (DECSET 7727)
- Shift+rueda omite esto y desplaza la vista
