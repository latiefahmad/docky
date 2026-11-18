<div align="center">

<img src="assets/brand/docky-banner.png" alt="Docky" width="620" />

  **Un dock ligero, inspirado en macOS, para Windows 10 y 11.**

  Ampliación suave, miniaturas de ventanas en vivo y un efecto genio opcional al minimizar.

  [English](README.md) · **Español**
</div>

Docky sustituye la barra de tareas de Windows por un dock personalizable y una
barra superior opcional al estilo de macOS. Está hecho para Windows 10 y 11 y sigue
en desarrollo activo.

<img width="1919" height="1008" alt="Docky en Windows 11" src="https://github.com/user-attachments/assets/b6c44203-c425-43fb-adf5-e2edd2f02257" />

## Funciones

- Dock de cristal con ampliación suave de los iconos bajo el cursor.
- Abrir, fijar, reordenar y gestionar apps, incluidas las recientes y las ventanas abiertas.
- Las ventanas minimizadas se guardan en el icono de su app con el efecto genio;
  activa Ajustes → Ventanas → *Mostrar ventanas minimizadas* para verlas como
  miniaturas en vivo a la derecha del dock.
- Clic en la app que ya está delante para minimizarla, como en la barra de Windows.
- Arrastrar y soltar archivos sobre apps, Descargas o la Papelera.
- Ocultación automática opcional.
- Barra superior con la app en primer plano, hora, red, volumen y batería, el idioma
  del teclado, el micrófono/cámara en uso y los iconos ocultos de la bandeja,
  además de los botones de búsqueda, Vista de tareas y Widgets si los tienes en la barra
  de Windows.
- Una isla curva en la barra superior con los límites de Claude Code, Codex,
  GitHub Copilot, OpenCode Go, Gemini CLI y Antigravity. Las cuotas de
  Gemini Pro y Flash aparecen por separado cuando están disponibles.
- App de Ajustes aparte, con cambios en vivo, en español e inglés, y efecto genio configurable.
- Un proceso guardián devuelve la barra de tareas de Windows si el dock se cierra de forma inesperada.

El efecto genio está **activado por defecto** y se puede desactivar en Ajustes →
Ventanas. Mientras está activo, Docky
desactiva las transiciones de Windows en las ventanas que sigue, para poder
dibujar su animación; las devuelve al cerrarse.

## Instalar

**[Descargar Docky-Setup.exe](https://github.com/latiefahmad/Docky/releases/latest/download/Docky-Setup.exe)**
(Windows 10 versión 1903 o posterior, o Windows 11; 64 bits) o elige una versión en
[Releases](https://github.com/latiefahmad/Docky/releases).

1. Ejecuta `Docky-Setup.exe`. Se instala solo para tu usuario: sin permisos de
   administrador, sin asistente, en unos dos segundos.
2. Docky es nuevo, así que SmartScreen puede no reconocerlo todavía y decir que
   protegió tu PC. Elige **Más información → Ejecutar de todas formas**. Ver la
   [política de firma de código](https://latief.id/docky/code-signing-policy).
3. Aparece el dock y se oculta la barra de tareas de Windows. Desde entonces Docky
   se abre al iniciar Windows; puedes desactivarlo en Ajustes → Sistema.

Abre **Docky** desde el menú Inicio (o clic derecho en el dock → *Ajustes de
Docky…*) para cambiar el dock. Ajustes también busca actualizaciones y las instala.

**Recuperar la barra de tareas:** clic derecho en el dock → *Salir de Docky*.
Para quitarlo del todo: Configuración de Windows → Aplicaciones → Aplicaciones
instaladas → Docky → Desinstalar; la barra vuelve y no queda nada en marcha.

## Probarlo desde el código

### Requisitos

- Windows 10 (versión 1903 o posterior) o Windows 11. En Windows 10 el dock y la
  barra superior son sólidos en lugar de cristal: Windows 10 no ofrece desenfoque
  de fondo a las apps de escritorio.
- Rust con la toolchain MSVC y las Build Tools de C++ de Visual Studio
- Node.js y pnpm

### Ejecutar el dock

Desde la raíz del repositorio:

```powershell
cargo run -p docky
```

El dock oculta la barra de tareas mientras está en marcha. Pulsa **Ctrl+C** en la
terminal para salir; Docky debería devolver la barra solo. Para devolverla a mano:

```powershell
cargo run -p docky -- --restore
```

### Ejecutar Ajustes

Con el dock en marcha, abre otra terminal y ejecuta:

```powershell
cd apps\settings
pnpm install
pnpm tauri dev
```

Se abre la ventana de Ajustes. Los cambios de la interfaz se recargan solos; tras
cambiar el backend en Rust, reinicia `tauri dev`. Los cambios de Ajustes se
aplican al dock en marcha.

## Compilar y probar

Desde la raíz del repositorio:

```powershell
cargo fmt --all -- --check
cargo test --workspace --locked
```

Para compilar la interfaz de Ajustes:

```powershell
cd apps\settings
pnpm build
```

## Estructura

| Ruta | Para qué |
|---|---|
| `apps/dock` | El dock nativo de Windows |
| `apps/settings` | La app de Ajustes (Tauri) |
| `crates/docky-core` | Lógica de disposición y animación, independiente de la plataforma |
| `crates/docky-shell` | Integración con el shell de Windows |
| `crates/docky-config` | Ajustes compartidos y su guardado |
| `docky/` | La web (Astro) |

## Privacidad

Docky no recoge datos, no tiene telemetría y nunca envía nada al proyecto. Hace
dos tipos de conexión de red, ambas opcionales:

- **Actualizaciones:** Ajustes pregunta a GitHub Releases si hay una versión
  nueva al abrirse y, una vez al día, un par de minutos después de encender el
  ordenador, Docky hace lo mismo en segundo plano e instala lo que encuentre
  (el dock en sí nunca usa la red). La instalación se desactiva en Ajustes →
  Sistema → *Instalar actualizaciones automáticamente*, y todo con *Buscar
  actualizaciones automáticamente*.
- **Uso de proveedores de IA:** mientras un cliente compatible está abierto,
  Docky reutiliza su sesión local para consultar los límites disponibles de
  su proveedor cada cinco minutos mientras está visible, o al pulsar Actualizar.
  Gemini CLI y Antigravity pueden mostrar por separado las cuotas de Gemini Pro
  y Flash; Antigravity se consulta mediante su servicio local. Las credenciales
  permanecen en memoria; Docky no las guarda. Se desactiva en Ajustes → Barra
  superior → *Mostrar uso de proveedores de IA*.

Detalles: [latief.id/docky/privacy](https://latief.id/docky/privacy).

## Licencia

[MIT](LICENSE). Docky continúa el trabajo del proyecto original
[Muelle](https://github.com/crafter-station/Muelle) de Crafter Station.
