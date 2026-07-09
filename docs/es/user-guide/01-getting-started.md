# Parte 1: Primeros Pasos

## Instalacion

### Compilar desde el codigo fuente

```bash
git clone https://github.com/topcheer/ggterm.git
cd ggterm

# Debug build
cargo run --features "desktop ai plugin plugin-lua config-watch" --bin ggterm

# Release build (inicio mas rapido, optimizado)
cargo build --release --features "desktop ai plugin plugin-lua config-watch" --bin ggterm
# Binario: target/release/ggterm
```

### Versiones precompiladas

Descarga desde [GitHub Releases](https://github.com/topcheer/ggterm/releases):
- **macOS**: .dmg universal (Apple Silicon + Intel)
- **Linux**: paquete .deb o archivo tar
- **Windows**: .zip

## Interfaz de linea de comandos

```bash
ggterm [OPTIONS]

Options:
  -c, --cols <N>              Columnas iniciales (default: 80)
  -r, --rows <N>              Filas iniciales (default: 24)
  -s, --shell <PATH>          Ruta del shell (default: $SHELL)
  -t, --title <TITLE>         Titulo de ventana (default: "GGTerm")
      --theme <NAME>          Tema de colores (default: "dark")
      --font-size <PX>        Tamano de fuente en pixeles (default: 16)
      --cell-width <PX>       Ancho de celda (default: 8)
  -w, --working-directory <DIR>  Iniciar shell en este directorio
  -C, --config <PATH>         Ruta de archivo de configuracion personalizado
  -e, --execute <CMD...>      Ejecutar comando en lugar de shell interactivo
      --hold                  Mantener terminal abierta despues de que el comando termine
      --fullscreen            Iniciar en modo pantalla completa
      --maximize              Iniciar maximizado
  -v                          Registro detallado (-v info, -vv debug, -vvv trace)
```

### Ejemplos de CLI

```bash
# Terminal por defecto
ggterm

# Terminal grande con zsh
ggterm --cols 120 --rows 40 --shell /bin/zsh

# Tema Dracula, tamano de fuente 18
ggterm --theme dracula --font-size 18

# Ejecutar vim y mantener abierto despues de salir
ggterm -e vim --hold

# Iniciar pantalla completa en un directorio especifico
ggterm --fullscreen --working-directory ~/projects

# Archivo de configuracion personalizado
ggterm --config ~/.config/ggterm/custom.toml
```

## Archivo de Configuracion

Ubicacion: `~/.ggterm/config.toml`

```toml
[appearance]
theme = "dark"                # 9 temas + auto
font_family = "monospace"
font_size = 14
cursor_style = "block"         # block | underline | bar
cursor_blink = true
background_opacity = 1.0       # 0.0 transparente a 1.0 opaco
# padding = 8                 # Padding de contenido en pixeles
# cursor_line_highlight = false
# word_chars = ""             # Caracteres de palabra adicionales para seleccion

[terminal]
scrollback_lines = 10000
shell = ""                     # Vacio = $SHELL o /bin/sh
restore_session = false        # Restaurar pestanas/divisiones al iniciar

[ai]
enabled = false
api_endpoint = ""
model = ""

[plugins]
enabled = false
directory = "~/.ggterm/plugins"

[keybindings]
# Ver Parte 8: Configuracion

[profiles.develop]
# Sobrescrituras opcionales por perfil
theme = "nord"
font_size = 12
```

## Primera Ejecucion

1. GGTerm se inicia con tu shell predeterminado en una sola pestana
2. La integracion de shell (OSC 133) se inyecta automaticamente para bash/zsh/fish
3. El archivo de configuracion se crea en `~/.ggterm/config.toml` en el primer uso
4. Presiona `Ctrl+Shift+/` en cualquier momento para ver todos los atajos de teclado

## Integracion de Shell

GGTerm inyecta automaticamente marcas OSC 133 para:
- Deteccion de comandos (limites de prompt/comando/salida)
- Seguimiento de codigo de salida
- Barra lateral de historial de comandos
- Funcion "Copiar salida del ultimo comando"

Configuracion manual (si la inyeccion automatica falla):

```bash
# bash (~/.bashrc)
source /path/to/ggterm/shell/bash.sh

# zsh (~/.zshrc)
source /path/to/ggterm/shell/zsh.zsh

# fish (~/.config/fish/config.fish)
source /path/to/ggterm/shell/fish.fish
```
