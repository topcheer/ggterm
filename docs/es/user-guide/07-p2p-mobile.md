# Parte 7: Comparticion P2P y Movil

## Comparticion de Terminal P2P

Comparte la terminal de tu escritorio con un dispositivo movil mediante codigo QR — sin necesidad de servidor en la nube.

### Como Funciona

GGTerm usa iroh (QUIC + NAT traversal) para conexiones directas peer-to-peer:
- Mas del 90% de exito en conexiones P2P directas
- Fallback automatico a relay para NATs dificiles
- Costo operativo cero (el relay publico de iroh es gratuito)
- Ticket: cadena base32 de ~130 caracteres, cabe en un codigo QR

### Escritorio (Host)

1. Presiona `Ctrl+Shift+Alt+Q` para abrir la superposicion de compartir
2. Aparece un codigo QR con tu ticket de conexion
3. Escanea el codigo QR con la aplicacion movil (o copia la cadena del ticket)
4. Una vez conectado, el dispositivo movil refleja tu terminal
5. Presiona `Esc` o `Ctrl+Shift+Alt+Q` para cerrar la comparticion

La superposicion muestra:
- Codigo QR (modulos oscuros renderizados como rectangulos)
- Estado de la conexion (esperando / conectado)
- Cadena del ticket (para entrada manual)
- Instrucciones

### Flujo de Datos

- **Salida del PTY → Movil**: Toda la salida de terminal se duplica al dispositivo movil conectado
- **Entrada del movil → PTY**: La entrada del teclado movil se reenvia al PTY del escritorio
- **Redimensionar**: Los cambios de dimension de terminal se propagan
- **Echo local en movil**: El movil ve los caracteres escritos inmediatamente (sin esperar el echo del PTY)

### Movil (Cliente)

#### Opciones de Conexion

| Opcion | Descripcion |
|--------|-------------|
| SSH | Conectar a servidor remoto (host, puerto, usuario, contrasena) |
| Echo Test | Diagnostico — repite los caracteres escritos (sin necesidad de servidor) |
| Scan QR | Conexion P2P a la terminal del escritorio mediante codigo QR |
| Share Terminal | Modo host P2P (solo Android — requiere shell local) |

#### Flujo de Escaneo QR

1. Toca **Scan QR** en la pantalla de conexion
2. Apunta la camara al codigo QR del escritorio
3. La salida de la terminal aparece en el movil
4. Escribe en el teclado movil para enviar entrada

#### iOS vs Android

- **iOS**: Solo SSH + cliente P2P (Scan QR) — sin terminal local
- **Android**: Todas las funciones, incluyendo shell local + host P2P

### Seguridad

- La conexion P2P esta cifrada (QUIC/TLS)
- La huella digital de la clave del servidor SSH se registra (formato SHA256:base64)
- SSH soporta tanto autenticacion por contrasena como por clave publica

## Gestor de Conexiones SSH

Almacena y gestiona conexiones SSH:

Mediante la Paleta de Comandos:
- `ssh.manager` — Abrir gestor de conexiones SSH
- `terminal.import_ssh` — Importar hosts desde `~/.ssh/config`

Funciones:
- Entradas de host con nombre, host, puerto, usuario, metodo de autenticacion
- Persistencia en TOML
- Busqueda difusa
- Conexion rapida

## Shell Local (Solo Android)

Los dispositivos Android con Termux o similar pueden ejecutar un shell local directamente en GGTerm movil.
