use std::{
    env, fs,
    path::{Path, PathBuf},
};

use windows::{
    Win32::{
        Storage::FileSystem::GetDriveTypeW,
        System::{
            Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile, STGM_READ},
            Registry::{HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RegGetValueW},
        },
        UI::{
            Shell::{
                BHID_SFUIObject, CMF_NORMAL, CMINVOKECOMMANDINFO, GCS_VERBW, IContextMenu,
                IShellItem, IShellLinkW, SHCreateItemFromParsingName, ShellLink,
            },
            WindowsAndMessaging::{
                CreatePopupMenu, DestroyMenu, GetMenuItemCount, GetMenuItemID, SW_SHOWNORMAL,
            },
        },
    },
    core::{HSTRING, Interface, PCSTR, PCWSTR, PSTR, w},
};

const TASKBAND: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Taskband");

use crate::{tracker::normalize_id, watch::RegistryWatch};

/// The apps pinned to the Windows taskbar, in the taskbar's order: desktop
/// apps by the program their shortcut opens, Store apps (Media Player, Phone
/// Link, WhatsApp...) as `shell:AppsFolder\<AppUserModelID>`.
///
/// Desktop pins are the shortcuts in `User Pinned\TaskBar`. The order, and
/// the Store apps, which have no shortcut there, come from
/// `Taskband\Favorites`, an undocumented binary list. It is only searched
/// for those shortcut names and for AppUserModelIDs, so if its format
/// changes the desktop pins still come, in alphabetical order.
///
/// Requires COM to be initialized on the calling thread.
#[must_use]
pub fn taskbar_pins() -> Vec<String> {
    taskbar_pins_if_readable().unwrap_or_default()
}

/// Like [`taskbar_pins`], but `None` when Windows' pinned list cannot be
/// read at all, so a failed read is never mistaken for "everything was
/// unpinned".
#[must_use]
pub fn taskbar_pins_if_readable() -> Option<Vec<String>> {
    let appdata = env::var("APPDATA").ok()?;
    let folder = PathBuf::from(appdata)
        .join("Microsoft\\Internet Explorer\\Quick Launch\\User Pinned\\TaskBar");
    let links = fs::read_dir(folder).map(|entries| {
        entries
            .filter_map(|entry| Some(entry.ok()?.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
            })
            .collect::<Vec<PathBuf>>()
    });
    // Either source alone still gives pins: Windows 10 may have shortcuts
    // and no `Favorites` value yet, if the taskbar was never edited.
    let (favorites, mut links) = match (favorites(), links) {
        (None, Err(_)) => return None,
        (favorites, links) => (favorites.unwrap_or_default(), links.unwrap_or_default()),
    };
    links.sort();

    // (position in Favorites, id); shortcuts it does not mention go last.
    let mut found: Vec<(usize, String)> = Vec::new();
    for link in &links {
        let Some(target) = link_target(link) else {
            continue;
        };
        let id = normalize_id(&target);
        if !id.ends_with(".exe") || !Path::new(&id).exists() {
            continue;
        }
        let position = link
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| find_utf16(&favorites, name))
            .unwrap_or(usize::MAX);
        found.push((position, id));
    }
    found.extend(
        app_user_model_ids(&favorites)
            .into_iter()
            .map(|(position, aumid)| (position, format!("shell:AppsFolder\\{aumid}"))),
    );
    // Stable: shortcuts with the same (unknown) position keep their alphabetical order.
    found.sort_by_key(|(position, _)| *position);

    let mut pins: Vec<String> = Vec::new();
    for (_, id) in found {
        if !pins.contains(&id) {
            pins.push(id);
        }
    }
    Some(pins)
}

/// Tells a window, by posting it `message`, when apps are pinned to or
/// unpinned from the Windows taskbar (from Start, Explorer or an app's own
/// menu): Explorer rewrites `Taskband` for every pin and unpin.
#[must_use]
pub fn watch(hwnd: isize, message: u32) -> Option<RegistryWatch> {
    RegistryWatch::new(TASKBAND, false, hwnd, message)
}

/// The taskbar's pinned list as Explorer stores it.
fn favorites() -> Option<Vec<u8>> {
    let key = TASKBAND;
    let mut size = 0u32;
    // SAFETY: First call only asks for the size; the second fills a buffer of that size.
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            w!("Favorites"),
            RRF_RT_REG_BINARY,
            None,
            None,
            Some(&mut size),
        )
        .ok()
        .ok()?;
        let mut blob = vec![0u8; size as usize];
        RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            w!("Favorites"),
            RRF_RT_REG_BINARY,
            None,
            Some(blob.as_mut_ptr().cast()),
            Some(&mut size),
        )
        .ok()
        .ok()?;
        blob.truncate(size as usize);
        Some(blob)
    }
}

/// Byte offset of `text`, as UTF-16, inside `blob`, ignoring ASCII case.
fn find_utf16(blob: &[u8], text: &str) -> Option<usize> {
    let needle: Vec<u8> = text
        .to_ascii_lowercase()
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    if needle.is_empty() {
        return None;
    }
    let lowered: Vec<u8> = blob.iter().map(u8::to_ascii_lowercase).collect();
    lowered
        .windows(needle.len())
        .position(|window| window == needle.as_slice())
}

/// AppUserModelIDs of Store apps (`Name_publisherid!App`) found in `blob`,
/// with their byte offsets, in order. Strings may start at any byte, so
/// both alignments are read.
fn app_user_model_ids(blob: &[u8]) -> Vec<(usize, String)> {
    let mut found: Vec<(usize, String)> = Vec::new();
    for start in 0..2 {
        let units: Vec<u16> = blob[start.min(blob.len())..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| u16::from_le_bytes(pair))
            .collect();
        let allowed = |unit: u16| {
            char::from_u32(u32::from(unit))
                .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '!'))
        };
        let mut index = 0;
        while index < units.len() {
            if !allowed(units[index]) {
                index += 1;
                continue;
            }
            let begin = index;
            while index < units.len() && allowed(units[index]) {
                index += 1;
            }
            let word = String::from_utf16_lossy(&units[begin..index]);
            if is_app_user_model_id(&word) {
                found.push((start + begin * 2, word));
            }
        }
    }
    found.sort_by_key(|(position, _)| *position);
    found
}

/// `Package.Name_publisherid!AppId`, where the publisher id is 13 lowercase
/// letters and digits.
fn is_app_user_model_id(word: &str) -> bool {
    let Some((family, app)) = word.split_once('!') else {
        return false;
    };
    let Some((name, publisher)) = family.rsplit_once('_') else {
        return false;
    };
    !app.is_empty()
        && !app.contains('!')
        && !name.is_empty()
        && publisher.len() == 13
        && publisher
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// Unpins `id` from the Windows taskbar with the shell's own "Unpin from
/// taskbar" command, so unpinning in the dock unpins there too. Windows lets
/// programs unpin but not pin, so the reverse is not possible. `false` if it
/// is not pinned there or the command is missing.
///
/// Requires COM on the calling thread; it may load shell extensions, so call
/// it off the UI thread.
#[must_use]
pub fn unpin_from_taskbar(id: &str) -> bool {
    let Some(item) = pinned_item(id) else {
        return false;
    };
    invoke_verb(&item, "taskbarunpin")
}

/// What Explorer pinned for `id`: the Store app itself, or the shortcut in
/// `User Pinned\TaskBar` that opens the program.
fn pinned_item(id: &str) -> Option<String> {
    if id.starts_with("shell:AppsFolder\\") {
        return Some(id.to_owned());
    }
    let folder = PathBuf::from(env::var("APPDATA").ok()?)
        .join("Microsoft\\Internet Explorer\\Quick Launch\\User Pinned\\TaskBar");
    fs::read_dir(folder)
        .ok()?
        .filter_map(|entry| Some(entry.ok()?.path()))
        .find(|link| link_target(link).is_some_and(|target| normalize_id(&target) == id))
        .map(|link| link.to_string_lossy().into_owned())
}

/// Runs the context menu command `verb` (canonical name) on the shell item
/// at `path`, as clicking it in Explorer would.
fn invoke_verb(path: &str, verb: &str) -> bool {
    // SAFETY: COM objects and the popup menu live for the block; the menu is
    // destroyed before returning.
    unsafe {
        let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(path), None)
        else {
            return false;
        };
        let Ok(menu) = item.BindToHandler::<_, IContextMenu>(None, &BHID_SFUIObject) else {
            return false;
        };
        let Ok(popup) = CreatePopupMenu() else {
            return false;
        };
        let mut done = false;
        if menu
            .QueryContextMenu(popup, 0, 1, 0x7FFF, CMF_NORMAL)
            .is_ok()
        {
            for position in 0..GetMenuItemCount(Some(popup)).max(0) {
                let command = GetMenuItemID(popup, position);
                if command == u32::MAX || command == 0 {
                    continue;
                }
                let offset = (command - 1) as usize;
                let mut name = [0u16; 64];
                let named = menu
                    .GetCommandString(
                        offset,
                        GCS_VERBW,
                        None,
                        PSTR(name.as_mut_ptr().cast()),
                        name.len() as u32,
                    )
                    .is_ok();
                let len = name.iter().position(|&unit| unit == 0).unwrap_or(0);
                if !named || !String::from_utf16_lossy(&name[..len]).eq_ignore_ascii_case(verb) {
                    continue;
                }
                let info = CMINVOKECOMMANDINFO {
                    cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
                    // A command offset passed where a verb name would go.
                    lpVerb: PCSTR(offset as *const u8),
                    nShow: SW_SHOWNORMAL.0,
                    ..Default::default()
                };
                done = menu.InvokeCommand(&info).is_ok();
                break;
            }
        }
        let _ = DestroyMenu(popup);
        done
    }
}

/// `false` only when `id` certainly no longer exists: a program removed from
/// a fixed drive, or a Store app that was uninstalled. Programs on removable
/// or network drives count as present; they may just be unplugged.
///
/// Requires COM on the calling thread.
#[must_use]
pub fn app_exists(id: &str) -> bool {
    if id.starts_with("shell:AppsFolder\\") {
        // SAFETY: Only asks the shell whether it can resolve the item.
        return unsafe {
            SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(id), None)
        }
        .is_ok();
    }
    let path = Path::new(id);
    let Some(root) = id.get(..3).filter(|root| root.ends_with(":\\")) else {
        return true;
    };
    const DRIVE_FIXED: u32 = 3;
    // SAFETY: Read-only query on a drive root.
    let fixed = unsafe { GetDriveTypeW(&HSTRING::from(root)) } == DRIVE_FIXED;
    !fixed || path.exists()
}

pub fn link_target(link: &std::path::Path) -> Option<String> {
    // SAFETY: COM calls use buffers and strings that remain alive for the duration of each call.
    unsafe {
        let shell_link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        shell_link
            .cast::<IPersistFile>()
            .ok()?
            .Load(&HSTRING::from(link.as_os_str()), STGM_READ)
            .ok()?;
        let mut buffer = [0u16; 1024];
        shell_link
            .GetPath(&mut buffer, std::ptr::null_mut(), 0)
            .ok()?;
        let len = buffer.iter().position(|&unit| unit == 0)?;
        (len > 0).then(|| String::from_utf16_lossy(&buffer[..len]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn favorites_should_give_store_apps_and_shortcuts_in_taskbar_order() {
        // Shaped like the real blob: binary noise around UTF-16 names, one of
        // them at an odd offset.
        let mut blob = vec![0x14, 0x00, 0x1f, 0x80];
        blob.extend(utf16("File Explorer.lnk"));
        blob.extend([0x00, 0x00, 0x07]);
        blob.extend(utf16(
            "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic",
        ));
        blob.extend([0x00, 0x00, 0x49]);
        blob.extend(utf16("Brave.lnk"));

        let explorer = find_utf16(&blob, "file explorer.lnk");
        let brave = find_utf16(&blob, "Brave.lnk");
        let store = app_user_model_ids(&blob);
        assert_eq!(store.len(), 1);
        assert_eq!(
            store[0].1,
            "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"
        );
        assert!(explorer < Some(store[0].0) && Some(store[0].0) < brave);
    }

    #[test]
    fn app_user_model_id_should_need_a_publisher_id_and_an_app() {
        assert!(is_app_user_model_id(
            "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"
        ));
        assert!(!is_app_user_model_id("Microsoft.Windows.Explorer"));
        assert!(!is_app_user_model_id(
            "Microsoft.ZuneMusic_11.2312.8.0_x64__8wekyb3d8bbwe"
        ));
        assert!(!is_app_user_model_id("Name_8wekyb3d8bbwe!"));
    }
}
