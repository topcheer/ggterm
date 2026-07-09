# Parte 6: Paleta de Comandos

La Paleta de Comandos (`Ctrl+Shift+P`) proporciona acceso de busqueda difusa a todas las acciones de GGTerm.

## Uso de la Paleta de Comandos

1. Presiona `Ctrl+Shift+P` para abrir
2. Escribe para buscar comandos difusamente
3. `Up/Down` para navegar los resultados
4. `Enter` para ejecutar
5. `Esc` para cerrar

## Lista Completa de Comandos

### Gestion de Pestanas

| Comando | Accion |
|---------|--------|
| `tab.new` | Nueva pestana |
| `tab.close` | Cerrar pestana |
| `tab.next` | Pestana siguiente |
| `tab.prev` | Pestana anterior |
| `tab.toggle_last` | Alternar ultima pestana |
| `tab.rename` | Renombrar pestana |
| `tab.move_left` | Mover pestana a la izquierda |
| `tab.move_right` | Mover pestana a la derecha |
| `tab.duplicate` | Duplicar pestana |
| `tab.close_others` | Cerrar otras pestanas |
| `tab.toggle_pin` | Fijar/Desfijar pestana |
| `tab.reopen_closed` | Reabrir pestana cerrada |
| `window.new` | Nueva ventana |

### Paneles de Division

| Comando | Accion |
|---------|--------|
| `split.horizontal` | Division horizontal |
| `split.vertical` | Division vertical |
| `split.focus_next` | Enfocar panel siguiente |
| `split.focus_prev` | Enfocar panel anterior |
| `split.zoom` | Alternar zoom de panel |
| `split.balance` | Equilibrar paneles |
| `split.swap` | Intercambiar contenido de panel |
| `split.close` | Cerrar panel actual |

### Operaciones de Terminal

| Comando | Accion |
|---------|--------|
| `terminal.clear` | Limpiar pantalla |
| `terminal.clear_all` | Limpiar pantalla + historial |
| `terminal.reset` | Restablecer terminal (RIS) |
| `terminal.reset_all` | Restablecer todas las terminales |
| `terminal.select_all` | Seleccionar todo el texto |
| `terminal.copy` | Copiar seleccion |
| `terminal.copy_cwd` | Copiar directorio actual |
| `terminal.paste` | Pegar |
| `terminal.search` | Buscar en historial |
| `terminal.open_url` | Abrir URL en el cursor |
| `terminal.save_scrollback` | Guardar historial en archivo |
| `terminal.export_html` | Exportar como HTML |
| `terminal.copy_as_html` | Copiar como HTML |
| `terminal.copy_last_output` | Copiar salida del ultimo comando |
| `terminal.copy_visible` | Copiar texto visible |
| `terminal.copy_markdown` | Copiar como Markdown |
| `terminal.toggle_lock` | Alternar bloqueo de terminal |
| `terminal.scroll_mode` | Alternar modo de exploracion del historial |
| `terminal.open_in_finder` | Abrir cwd en Finder/Explorer |
| `terminal.open_shell_config` | Editar configuracion de shell (.bashrc/.zshrc) |
| `terminal.import_ssh` | Importar hosts SSH desde ~/.ssh/config |
| `terminal.edit_selection` | Editar texto seleccionado |
| `terminal.run_selection` | Ejecutar texto seleccionado como comando |
| `terminal.search_selection` | Buscar texto seleccionado en la web |
| `terminal.send_ctrl_c_all` | Enviar Ctrl+C a todos los paneles |
| `terminal.new_session` | Nueva sesion SSH |

### Apariencia

| Comando | Accion |
|---------|--------|
| `theme.cycle` | Recorrer tema |
| `font.zoom_in` | Acercar |
| `font.zoom_out` | Alejar |
| `font.zoom_reset` | Restablecer tamano de fuente |
| `opacity.increase` | Aumentar opacidad |
| `opacity.decrease` | Disminuir opacidad |
| `view.toggle_cursor_line` | Alternar resaltado de linea del cursor |

### Ventana

| Comando | Accion |
|---------|--------|
| `view.fullscreen` | Alternar pantalla completa |
| `view.maximize` | Alternar maximizado |
| `view.status_bar` | Alternar barra de estado |
| `window.always_on_top` | Alternar siempre encima |
| `settings.open` | Abrir panel de Configuracion |
| `config.open` | Abrir archivo de configuracion |
| `config.reload` | Recargar configuracion |

### IA

| Comando | Accion |
|---------|--------|
| `ai.explain` | Explicar salida |
| `ai.suggest` | Sugerir comando |
| `ai.help` | Ayuda de IA |

### Sesiones y Perfiles

| Comando | Accion |
|---------|--------|
| `session.save` | Guardar sesion |
| `session.profile` | Recorrer perfiles |
| `ssh.manager` | Abrir gestor de conexiones SSH |

### Efectos del Cursor

| Comando | Accion |
|---------|--------|
| `cursor.trail` | Habilitar estela de particulas del cursor |
| `cursor.glow` | Habilitar brillo del cursor |
| `cursor.none` | Deshabilitar efectos del cursor |

### Otros

| Comando | Accion |
|---------|--------|
| `perf.toggle` | Alternar monitor de rendimiento |
| `sound.toggle` | Alternar sonido |
| `shell.switch` | Abrir selector de shell |
| `workspace.next` | Espacio de trabajo siguiente |
| `workspace.prev` | Espacio de trabajo anterior |
| `workspace.add` | Anadir espacio de trabajo |
