//! User-facing dock text in Spanish and English. The language follows the
//! Windows display language; Settings will be able to override it (phase 7).
//! `&` marks the menu access key.

use std::sync::atomic::{AtomicU8, Ordering};

use docky_config::Language;
use docky_shell::system::ui_language_is_spanish;

pub struct Strings {
    pub open: &'static str,
    pub new_window: &'static str,
    pub keep_in_dock: &'static str,
    pub show_in_explorer: &'static str,
    pub quit_app: &'static str,
    pub empty_bin: &'static str,
    pub turn_hiding_on: &'static str,
    pub turn_hiding_off: &'static str,
    pub quit_dock: &'static str,
    pub start: &'static str,
    pub remove: &'static str,
    pub close_window: &'static str,
    pub settings: &'static str,
    pub import_pins: &'static str,
    pub run_as_admin: &'static str,
    pub show_desktop: &'static str,
    pub usage_title: &'static str,
    pub usage_refresh: &'static str,
    pub usage_refreshing: &'static str,
    pub usage_sign_in: &'static str,
    pub usage_unavailable: &'static str,
    pub usage_loading: &'static str,
    pub usage_stale: &'static str,
    pub usage_resets: &'static str,
    pub usage_updated: &'static str,
    pub usage_used: &'static str,
    pub shell_operation_failed: &'static str,
}

const ES: Strings = Strings {
    shell_operation_failed: "No se pudo iniciar la operación. Inténtalo de nuevo en un momento.",
    open: "&Abrir",
    new_window: "&Nueva ventana",
    keep_in_dock: "&Mantener en el Dock",
    show_in_explorer: "Mostrar en el &Explorador",
    quit_app: "&Salir",
    empty_bin: "&Vaciar papelera",
    turn_hiding_on: "Activar &ocultación automática",
    turn_hiding_off: "Desactivar &ocultación automática",
    quit_dock: "Salir de &Docky",
    start: "Inicio",
    remove: "Quitar",
    close_window: "&Cerrar",
    settings: "&Ajustes de Docky…",
    import_pins: "Añadir apps &fijadas en la barra de Windows",
    run_as_admin: "Ejecutar como a&dministrador",
    show_desktop: "Mostrar &escritorio",
    usage_title: "Uso de IA",
    usage_refresh: "Actualizar",
    usage_refreshing: "Actualizando…",
    usage_sign_in: "Inicia sesión",
    usage_unavailable: "No disponible",
    usage_loading: "Cargando…",
    usage_stale: "Sin actualizar",
    usage_resets: "Reinicia",
    usage_updated: "Actualizado hace",
    usage_used: "usado",
};

const EN: Strings = Strings {
    shell_operation_failed: "The operation could not start. Please try again in a moment.",
    open: "&Open",
    new_window: "&New Window",
    keep_in_dock: "&Keep in Dock",
    show_in_explorer: "Show in &Explorer",
    quit_app: "&Quit",
    empty_bin: "&Empty Recycle Bin",
    turn_hiding_on: "Turn Hiding &On",
    turn_hiding_off: "Turn Hiding &Off",
    quit_dock: "Quit &Docky",
    start: "Start",
    remove: "Remove",
    close_window: "&Close",
    settings: "Docky &Settings…",
    import_pins: "Add Apps &Pinned to the Windows Taskbar",
    run_as_admin: "Run as a&dministrator",
    show_desktop: "Show &Desktop",
    usage_title: "AI usage",
    usage_refresh: "Refresh",
    usage_refreshing: "Refreshing…",
    usage_sign_in: "Sign in",
    usage_unavailable: "Unavailable",
    usage_loading: "Loading…",
    usage_stale: "Not updated",
    usage_resets: "Resets",
    usage_updated: "Updated",
    usage_used: "used",
};

/// 0 = not chosen yet (follow Windows), 1 = Spanish, 2 = English.
static LANGUAGE: AtomicU8 = AtomicU8::new(0);

pub fn set_language(language: Language) {
    let spanish = match language {
        Language::Auto => ui_language_is_spanish(),
        Language::Es => true,
        Language::En => false,
    };
    LANGUAGE.store(if spanish { 1 } else { 2 }, Ordering::Relaxed);
}

pub fn text() -> &'static Strings {
    match LANGUAGE.load(Ordering::Relaxed) {
        0 => {
            set_language(Language::Auto);
            text()
        }
        1 => &ES,
        _ => &EN,
    }
}
