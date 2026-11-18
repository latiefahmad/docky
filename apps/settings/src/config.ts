import { invoke } from "@tauri-apps/api/core";

export type Language = "auto" | "es" | "en";

/** Mirrors `docky_config::Config`. */
export interface Config {
  version: number;
  pinned: string[];
  recent: string[];
  autoHide: boolean;
  iconSize: number;
  magnification: boolean;
  magnifiedSize: number;
  genie: boolean;
  showMinimized: boolean;
  menuBar: boolean;
  menuBarAutoHide: boolean;
  aiUsage: boolean;
  showIndicators: boolean;
  showRecents: boolean;
  bounceOnOpen: boolean;
  checkUpdates: boolean;
  autoUpdate: boolean;
  language: Language;
}

export const ICON_SIZE = { min: 32, max: 96 } as const;
export const MAX_MAGNIFIED_SIZE = 160;

export const loadConfig = () => invoke<Config>("get_config");
export const defaultConfig = () => invoke<Config>("default_config");
export const saveConfig = (config: Config) => invoke<Config>("set_config", { config });
export const getAutostart = () => invoke<boolean>("get_autostart");
export const setAutostart = (enabled: boolean) => invoke<void>("set_autostart", { enabled });
export interface UpdateInfo {
  version: string;
  /** Release page with the notes, if the manifest has one. */
  notes: string | null;
}

/** Resolves to the newer version available, or null when up to date. */
export const checkUpdate = () => invoke<UpdateInfo | null>("check_update");
export const openReleaseNotes = (url: string) => invoke<void>("open_release_notes", { url });
export const installUpdate = () => invoke<void>("install_update");
