# Changelog / Cambios

Each release lists its changes in English and in Spanish. The release workflow
copies the section of the version being published into the GitHub release.

Cada versión lista sus cambios en inglés y en español. El flujo de publicación
copia la sección de la versión que se publica a la release de GitHub.

## [1.0.0]

### English

- Changed: the project is now called **Docky** — the continuation of Muelle
  under a new name, maintained by latiefahmad. The installer is now
  `Docky-Setup.exe`, the dock is `docky.exe`, and its settings live in
  `%APPDATA%\Docky`. Users of the old name migrate automatically: their
  settings, autostart choice and update schedule move over on first run.
- Changed: new logo — a geometric D monogram, white and orange on a purple
  tile.
- Fixed: some antivirus programs flagged Docky 0.1.6–0.1.8 as a trojan and
  deleted it, often right after updating. Two things looked like malware: to
  show GitHub Copilot and Cursor usage, Docky decrypted VS Code's stored
  secrets and read Cursor's database (what password stealers do), and since
  0.1.7 it set itself up to start with Windows by running `schtasks.exe`
  hidden. Docky no longer does either: it starts from the usual `Run`
  registry entry (an existing startup task is moved there once), Copilot
  usage comes only from the Copilot CLI sign-in or `GH_TOKEN`/`GITHUB_TOKEN`,
  and Cursor usage was removed. The dock is also 28% smaller.
- Fixed: after choosing an item in the dock's menu (Docky Settings, for
  one), the dock could stay magnified and stop reacting to the pointer.
- Alt+Tab: the hidden dock shows only for Alt+Tab, no longer for Task View,
  whose virtual desktops it covered at the bottom of the screen; and it no
  longer flashes for a moment after cancelling Alt+Tab with Esc.
- Alt+Tab: moving the mouse or clicking an empty spot no longer stops the
  dock's magnifier from following the selection, and closing a window from
  the switcher keeps it on the right app. Only the arrow keys, which move the
  selection in ways Docky cannot see, make it step aside.
- Automatic updates: once a day, a couple of minutes after you turn on the
  computer (or every 24 hours if it stays on), Docky installs new versions by
  itself; the dock restarts in a moment. Never while an app is full screen. It can be turned off in Settings → System → "Install
  updates automatically". The dock itself still never uses the network.
- Windows 10: the menu bar no longer shows a Widgets button, which Windows
  10 does not have; it opened Windows Ink instead.
- Glass follows energy saver: turning it on or off (by hand or at low battery)
  switches the dock, the menu bar and the dock menu between glass and solid
  at once.

### Español

- Cambio: el proyecto ahora se llama **Docky** — la continuación de Muelle con
  otro nombre, mantenida por latiefahmad. El instalador es `Docky-Setup.exe`,
  el dock es `docky.exe` y sus ajustes viven en `%APPDATA%\Docky`. Los usuarios
  del nombre antiguo migran solos: sus ajustes, su elección de inicio con
  Windows y su horario de actualizaciones se trasladan al primer arranque.
- Cambio: logo nuevo — un monograma geométrico de D, en blanco y naranja sobre
  un azulejo púrpura.
- Corregido: algunos antivirus marcaban Docky 0.1.6–0.1.8 como troyano y lo
  borraban, a menudo justo al actualizar. Dos cosas parecían malware: para
  mostrar el uso de GitHub Copilot y Cursor, Docky descifraba los secretos
  guardados de VS Code y leía la base de datos de Cursor (lo que hacen los
  ladrones de contraseñas), y desde la 0.1.7 se configuraba para iniciar con
  Windows ejecutando `schtasks.exe` oculto. Docky ya no hace ninguna de las
  dos: arranca desde la entrada habitual `Run` del registro (una tarea de
  inicio existente se traslada ahí una vez), el uso de Copilot sale solo de la
  sesión de Copilot CLI o de `GH_TOKEN`/`GITHUB_TOKEN`, y se quitó el uso de
  Cursor. Además, el dock ocupa un 28 % menos.
- Corregido: tras elegir una opción del menú del dock (por ejemplo, Ajustes
  de Docky), el dock podía quedarse ampliado y dejar de responder al puntero.
- Alt+Tab: el dock oculto aparece solo con Alt+Tab, ya no con la Vista de
  tareas, cuyos escritorios virtuales tapaba abajo; y ya no parpadea un
  momento después de cancelar Alt+Tab con Esc.
- Alt+Tab: mover el ratón o hacer clic en un hueco ya no impide que la lupa
  del dock siga la selección, y cerrar una ventana desde el selector la
  mantiene en la app correcta. Solo las flechas, que mueven la selección de
  formas que Docky no ve, hacen que se aparte.
- Actualizaciones automáticas: una vez al día, un par de minutos después de
  encender el ordenador (o cada 24 horas si sigue encendido), Docky instala
  las versiones nuevas por sí solo; el dock se reinicia en un momento. Nunca
  con una app a pantalla completa. Se puede desactivar en Ajustes →
  Sistema → «Instalar actualizaciones automáticamente». El dock en sí sigue
  sin usar la red.
- Windows 10: la barra superior ya no muestra un botón de Widgets, que Windows
  10 no tiene; abría Windows Ink en su lugar.
- El cristal sigue al ahorro de energía: al activarlo o desactivarlo (a mano o
  con batería baja), el dock, la barra superior y el menú del dock cambian
  entre cristal y sólido al momento.

## [0.1.8]

### English

- Opening apps and shell file operations run in temporary helper processes so
  slow shell handlers and native progress dialogs keep the dock responsive.
  Transfers run in order; ordinary launch helpers time out after 30 seconds,
  while file operations and administrator launches keep their native user controls.
- Recycle Bin notifications share one drive scan at a time, preserving changes
  that arrive during a scan without starting overlapping disk queries.
- Shell icons and app names load in the background through a bounded queue,
  keeping the dock responsive while the shell is slow. Repeated requests share
  the same load, and unused pixel buffers are released.
- Minimize animations prepare restore images off the UI thread and reuse
  thumbnail dimensions between window events. Batch minimization starts one
  capture; a stalled capture no longer keeps an overlay and frame loop alive.
- Drag and drop into apps: hold a file (or text, or an image) over an open
  app in the dock and its window comes to the front, so you can drop it there,
  as with the Windows taskbar. Passing over icons does not switch windows.
- Fixed: with "Automatically hide and show the menu bar" on, the bar covered the
  minimize, maximize and close buttons of maximized windows and took clicks
  meant for browser tabs at the top edge. The hidden bar no longer keeps any
  window on screen; it appears after the pointer rests briefly on the top
  edge, and never over a window's title-bar buttons.
- The top-right corner of the menu bar no longer shows the desktop: throwing
  the pointer there to close a window minimized everything. "Show Desktop"
  remains in the dock menu.
- Fixed: clicking the menu bar's "^" again left the hidden-icons popup open
  and moved the pointer to its first icon. The second click now closes it.
- The dock's menu looks like a macOS menu: frosted glass in the light or dark
  theme, compact rows and the selection in your Windows accent color. It
  works fully from the keyboard; with a screen reader or high contrast on,
  the native Windows menu is used instead.
- An auto-hidden dock shows while Alt+Tab (or Task View) is open, so the
  open apps are in view, and hides again when you let go. As you press Tab,
  the dock magnifies the app of the selected window. Windows does not report
  the selection, so it follows Tab and Shift+Tab; after arrow keys or the
  mouse in the switcher, the magnifier steps aside rather than guess.
- The menu bar no longer shows a location icon. Widgets keeps location in use
  for the weather all the time, and Windows shows no icon for that either.
- Windows 10: apps pinned to the taskbar from the Microsoft Store (Store, Mail
  and others) now come to the dock. Versions before 0.1.7 could not import
  them, and updating did not add them afterwards.

### Español

- Abrir apps y ejecutar operaciones de archivos del shell se hace en procesos
  auxiliares temporales para mantener la respuesta del dock mientras el shell
  tarda o muestra el progreso. Las transferencias se ejecutan en orden; las
  aperturas normales tienen un límite de 30 segundos, mientras las operaciones
  de archivos y las aperturas como administrador conservan sus controles nativos.
- Los avisos de Papelera comparten una sola lectura de discos a la vez y
  conservan los cambios que lleguen durante esa lectura, sin consultas solapadas.
- Los iconos y nombres del shell se cargan en segundo plano con una cola
  limitada para que el dock responda mientras el shell tarda. Las solicitudes
  repetidas comparten la misma carga y se liberan los píxeles que ya no se usan.
- Las animaciones de minimizar preparan la imagen de restauración fuera del
  hilo de interfaz y reutilizan las dimensiones de las miniaturas entre avisos
  de ventanas. Al minimizar en lote se inicia una captura; una captura atascada
  ya no mantiene una vista previa y un ciclo de fotogramas indefinidos.
- Arrastrar y soltar en las apps: mantén un archivo (o texto, o una imagen)
  sobre una app abierta del dock y su ventana pasa al frente para que lo
  sueltes ahí, como con la barra de tareas de Windows. Pasar por encima de los
  iconos no cambia de ventana.
- Corregido: con «Ocultar y mostrar la barra superior automáticamente», la barra tapaba
  los botones de minimizar, maximizar y cerrar de las ventanas maximizadas y
  se quedaba con los clics en las pestañas del navegador del borde superior.
  La barra oculta ya no deja ninguna ventana en pantalla: aparece cuando el
  puntero se detiene un momento en el borde superior, y nunca encima de los
  botones de una ventana.
- La esquina superior derecha de la barra ya no muestra el escritorio: lanzar
  el puntero ahí para cerrar una ventana minimizaba todo. «Mostrar escritorio»
  sigue en el menú del dock.
- Corregido: volver a pulsar «^» en la barra superior dejaba abierto el
  panel de iconos ocultos y movía el puntero a su primer icono. Ahora el
  segundo clic lo cierra.
- El menú del dock se ve como un menú de macOS: cristal esmerilado en tema
  claro u oscuro, filas compactas y la selección en el color de énfasis de
  Windows. Funciona por completo con el teclado; con un lector de pantalla o
  alto contraste activos se usa el menú nativo de Windows.
- El dock oculto aparece mientras Alt+Tab (o la Vista de tareas) está abierto,
  para ver las apps abiertas, y se vuelve a ocultar al soltar. Al pulsar Tab,
  el dock amplía la app de la ventana seleccionada. Windows no informa de la
  selección, así que sigue Tab y Mayús+Tab; si se usan las flechas o el ratón
  en el selector, la lupa se aparta en vez de adivinar.
- La barra superior ya no muestra el icono de ubicación. Widgets mantiene la
  ubicación en uso todo el tiempo para el clima, y Windows tampoco muestra un
  icono por eso.
- Windows 10: las apps de Microsoft Store fijadas en la barra de tareas
  (Store, Correo y otras) ahora llegan al dock. Las versiones anteriores a la
  0.1.7 no podían importarlas, y al actualizar no se añadían después.

## [0.1.7]

### English

- Faster startup: Docky now uses a Windows sign-in task instead of the Run
  key, and keeps the taskbar hidden when Windows shuts down or restarts.
- Taskbar pins stay in sync with the dock, including changes made while Docky
  is closed. Imported pins keep their order and include Store apps; removed
  apps no longer remain as broken dock icons.
- The dock menu can import apps pinned to the Windows taskbar. Unchecking
  "Keep in Dock" also unpins the app from the taskbar.
- Middle-click or Shift-click opens another app window; Ctrl+Shift-click or
  the dock menu opens an app as administrator. The menu bar's right edge and
  the dock menu can show the desktop.
- The menu bar shows the active input language and privacy indicators for
  microphone, camera, and location use. Its clock follows time and time-zone
  changes, and switching virtual desktops updates the dock immediately.
- Genie: maximized and snapped windows now fly using the area they occupy on
  screen, without jumping to a different size.

### Español

- Inicio más rápido: Docky ahora usa una tarea de Windows al iniciar sesión
  en lugar de la clave Run y mantiene oculta la barra de tareas al apagar o
  reiniciar Windows.
- Las apps fijadas en la barra de tareas se sincronizan con el dock, incluso
  si el cambio ocurre mientras Docky está cerrado. Las apps importadas
  conservan su orden e incluyen las de la Store; las desinstaladas ya no quedan
  como iconos rotos.
- El menú del dock permite importar las apps fijadas en la barra de tareas.
  Desmarcar «Mantener en el Dock» también desancla la app de la barra.
- El clic central o Mayús+clic abre otra ventana; Ctrl+Mayús+clic o el menú del
  dock abre una app como administrador. El borde derecho de la barra superior
  y el menú del dock permiten mostrar el escritorio.
- La barra superior muestra el idioma de entrada activo e indicadores cuando
  se usan el micrófono, la cámara o la ubicación. El reloj sigue los cambios
  de hora y zona horaria, y al cambiar de escritorio virtual el dock se
  actualiza de inmediato.
- Genio: las ventanas maximizadas o acopladas vuelan usando el área que ocupan
  en pantalla, sin saltar a otro tamaño.

## [0.1.6]

### English

- Windows 10 support (version 1903 or later). The dock used to close right
  after starting there. On Windows 10 the dock and menu bar are solid instead
  of glass, since it has no backdrop blur for desktop apps.
- The menu bar's "^" button opens Windows' hidden notification-area icons.
- A curved menu bar island shows usage for Claude, Codex, GitHub Copilot,
  Cursor, OpenCode Go, and Gemini or Antigravity while they are running. It can
  be turned off in Settings → Menu bar.
- Settings has a clearer layout with separate Dock, Windows, Menu bar and
  System sections, and its window can be resized or maximized.
- Minimized windows now fly into their app's dock icon by default. Live
  thumbnails on the right side of the dock remain available as an option.
- The active-app click option was removed: clicking the app in front minimizes
  it, as on the Windows taskbar.
- The dock's Docky icon is sharper, and the Start button uses the Windows
  logo.
- Settings can turn off automatic update checks. The website adds privacy and
  code-signing policy pages.

### Español

- Soporte para Windows 10 (versión 1903 o posterior). Antes el dock se cerraba
  nada más arrancar ahí. En Windows 10 el dock y la barra superior son sólidos
  en lugar de cristal, porque no ofrece desenfoque de fondo a las apps de
  escritorio.
- El botón «^» de la barra superior abre los iconos ocultos del área de
  notificación de Windows.
- Una isla curva en la barra superior muestra el uso de Claude, Codex, GitHub
  Copilot, Cursor, OpenCode Go y Gemini o Antigravity mientras están abiertos.
  Se puede desactivar en Ajustes → Barra superior.
- Ajustes tiene secciones separadas para Dock, Ventanas, Barra superior y
  Sistema; su ventana se puede redimensionar o maximizar.
- Por defecto, las ventanas minimizadas vuelan hacia el icono de su app en el
  dock. Las miniaturas en vivo a la derecha siguen disponibles como opción.
- Se quitó la opción de clic en la app activa: al hacer clic en la app que está
  delante, se minimiza, como en la barra de Windows.
- El icono de Docky se ve más nítido en el dock y el botón Inicio usa el logo
  de Windows.
- Ajustes permite desactivar la búsqueda automática de actualizaciones. La web
  incorpora páginas de privacidad y firma de código.

## [0.1.5]

### English

- The dock appears sooner at startup (about 170 ms instead of 260-450 ms).
- Lower idle CPU: windows changing their title (terminals, browsers) no longer
  make the dock re-read every window.
- Genie starts sooner after minimizing: the screen copy no longer waits extra
  frames.
- Fixed: the dock and menu bar could stay black after energy saver; without
  transparency they are now solid gray, as macOS draws them with "Reduce
  transparency".
- Fixed: the menu bar could miss an update and showed no app name until the
  first window switch.
- Menu bar: the battery shows a charging bolt.
- Settings: options that depend on another fold away under it.

### Español

- El dock aparece antes al arrancar (unos 170 ms en lugar de 260-450 ms).
- Menos CPU en reposo: las ventanas que cambian de título (terminales,
  navegadores) ya no hacen que el dock relea todas las ventanas.
- El genio empieza antes al minimizar: la copia de pantalla ya no espera
  fotogramas de más.
- Corregido: el dock y la barra superior podían quedarse negros tras el ahorro
  de energía; sin transparencia ahora son grises sólidos, como macOS con
  «Reducir transparencia».
- Corregido: la barra superior podía perder una actualización y no mostraba el
  nombre de la app hasta el primer cambio de ventana.
- Barra superior: la batería muestra un rayo mientras carga.
- Ajustes: las opciones que dependen de otra se pliegan bajo ella.

## [0.1.4]

### English

- Maintenance release: align Genie default documentation and tests with the
  shipped setting.
- Website: add an Open Graph preview image and document the observed idle
  memory reading.

### Español

- Versión de mantenimiento: alinear la documentación y las pruebas del efecto
  Genio con el ajuste publicado.
- Web: añadir una imagen de vista previa Open Graph y documentar la lectura
  observada de memoria en reposo.

## [0.1.2]

### English

- Updates are announced in Settings: a banner above every section with
  "Update now", "Later" and "What's new", download progress, and a badge on
  System while an update waits.
- Smoother Genie effect: the system compositor now runs the whole animation
  at the display's refresh rate, and it no longer skips its first frames.
- Genie: no more flash when minimizing, and a window restored with Alt+Tab or
  from the app itself animates again the next time it is minimized.
- Fixed: the Dock section of Settings could not scroll to its last options.
- Fixed: opening Settings twice opened two windows; it now brings the open one
  to the front.
- Fixed: with an app running as administrator in front (Task Manager, an admin
  terminal, an installer), the dock's Start button and the menu bar buttons did
  nothing, because Windows drops simulated keys aimed at such apps.

### Español

- Las actualizaciones se anuncian en Ajustes: un aviso encima de cada sección
  con «Actualizar ahora», «Más tarde» y «Novedades», progreso de la descarga y
  una insignia en Sistema mientras haya una pendiente.
- Efecto genio más fluido: el compositor del sistema ejecuta toda la animación
  a la frecuencia del monitor y ya no se salta sus primeros fotogramas.
- Genio: ya no hay destello al minimizar, y una ventana restaurada con Alt+Tab o
  desde la propia app vuelve a animarse la siguiente vez que se minimiza.
- Corregido: la sección Dock de Ajustes no dejaba bajar hasta las últimas opciones.
- Corregido: abrir Ajustes dos veces abría dos ventanas; ahora trae al frente la abierta.
- Corregido: con una app de administrador delante (Administrador de tareas, una
  terminal de administrador, un instalador), el botón Inicio del dock y los botones
  de la barra superior no hacían nada, porque Windows descarta las teclas simuladas
  dirigidas a esas apps.

## [0.1.1]

### English

- Menu bar: network (Wi-Fi strength, Ethernet or offline) and volume, updated
  as they change. Search, Task View and Widgets buttons appear when they are on
  in your Windows taskbar settings.
- Clicking the app that is already in front minimizes it, as the Windows
  taskbar does; click again to bring it back. Settings can switch this to
  "do nothing", as on a Mac.
- Apps opened from the dock come to the front reliably instead of sometimes
  opening behind other windows.
- Minimized apps that run as administrator, such as Task Manager, can be brought
  back from the dock.
- Settings: restore defaults, and options to show open-app indicators, show
  recent apps and animate opening apps.

### Español

- Barra superior: red (intensidad del Wi-Fi, Ethernet o sin conexión) y volumen,
  actualizados al momento. Los botones de búsqueda, Vista de tareas y Widgets
  aparecen si los tienes activados en la barra de tareas de Windows.
- Clic en la app que ya está delante la minimiza, como la barra de Windows; otro
  clic la vuelve a traer. En Ajustes se puede cambiar a «no hacer nada», como en Mac.
- Las apps abiertas desde el dock aparecen delante siempre, en vez de abrirse a
  veces detrás de otras ventanas.
- Las apps minimizadas que se ejecutan como administrador, como el Administrador
  de tareas, se pueden recuperar desde el dock.
- Ajustes: restablecer valores y opciones para mostrar los indicadores de apps
  abiertas, mostrar las apps recientes y animar al abrir apps.

## [0.1.0]

### English

- First release: glass dock with magnification, live minimized-window
  thumbnails, optional Genie effect, menu bar, Settings app in English and
  Spanish, one-step installer and in-app updates.

### Español

- Primera versión: dock de cristal con ampliación, miniaturas en vivo de las
  ventanas minimizadas, efecto genio opcional, barra superior, app de Ajustes en
  español e inglés, instalador de un paso y actualizaciones desde la app.
