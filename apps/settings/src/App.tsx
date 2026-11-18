import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  type Config,
  type Language,
  type UpdateInfo,
  ICON_SIZE,
  MAX_MAGNIFIED_SIZE,
  checkUpdate,
  defaultConfig,
  getAutostart,
  installUpdate,
  loadConfig,
  openReleaseNotes,
  saveConfig,
  setAutostart,
} from "./config";
import { Card, Row, Segmented, Slider, SubRow, Toggle } from "./controls";
import { type SectionId, SectionIcon } from "./icons";
import { UpdateBanner } from "./UpdateBanner";
import en from "./i18n/en.json";
import es from "./i18n/es.json";

type UpdateState =
  | { kind: "off" }
  | { kind: "checking" }
  | { kind: "current" }
  | { kind: "available"; info: UpdateInfo }
  | { kind: "installing"; info: UpdateInfo; progress: number | null }
  | { kind: "installFailed"; info: UpdateInfo }
  | { kind: "error" };

const toUpdateState = (found: UpdateInfo | null): UpdateState =>
  found ? { kind: "available", info: found } : { kind: "current" };

/** An update is waiting for the user: the banner and the badge show. */
const pendingUpdate = (state: UpdateState): UpdateInfo | null =>
  state.kind === "available" || state.kind === "installing" || state.kind === "installFailed"
    ? state.info
    : null;

interface ProgressEvent {
  downloaded: number;
  total: number | null;
}
type Texts = typeof es;

/** Dragging a slider saves at most this often; the dock redraws on each save. */
const SAVE_DELAY_MS = 60;
/** How long the reset button stays armed after the first click. */
const RESET_CONFIRM_MS = 4000;

function textsFor(language: Language): Texts {
  const spanish =
    language === "es" || (language === "auto" && navigator.language.toLowerCase().startsWith("es"));
  return spanish ? es : en;
}

export function App() {
  const [config, setConfig] = useState<Config | null>(null);
  const [autostart, setAutostartState] = useState(false);
  const [section, setSection] = useState<SectionId>("dock");
  const [failed, setFailed] = useState(false);
  const [version, setVersion] = useState("");
  const [updateState, setUpdateState] = useState<UpdateState>({ kind: "checking" });
  const [confirmingReset, setConfirmingReset] = useState(false);
  /** "Later" hides the banner until Settings is opened again. */
  const [bannerDismissed, setBannerDismissed] = useState(false);
  const pending = useRef<Config | null>(null);
  const timer = useRef<number | undefined>(undefined);
  const resetTimer = useRef<number | undefined>(undefined);

  useEffect(() => {
    let alive = true;
    Promise.all([loadConfig(), getAutostart()])
      .then(([loaded, starts]) => {
        if (!alive) return;
        setConfig(loaded);
        setAutostartState(starts);
        // The only network access in Docky, and the user can turn it off.
        if (!loaded.checkUpdates) {
          setUpdateState({ kind: "off" });
          return;
        }
        checkUpdate()
          .then((found) => alive && setUpdateState(toUpdateState(found)))
          .catch(() => alive && setUpdateState({ kind: "error" }));
      })
      .catch(() => alive && setFailed(true));
    getVersion()
      .then((current) => alive && setVersion(current))
      .catch(() => {});
    const progress = listen<ProgressEvent>("update-progress", ({ payload }) => {
      const share = payload.total ? Math.min(payload.downloaded / payload.total, 1) : null;
      setUpdateState((state) => (state.kind === "installing" ? { ...state, progress: share } : state));
    });
    return () => {
      alive = false;
      window.clearTimeout(timer.current);
      window.clearTimeout(resetTimer.current);
      progress.then((unlisten) => unlisten()).catch(() => {});
    };
  }, []);

  // Shows the change at once and saves the latest value shortly after, so a
  // slider drag does not write the file on every pixel.
  const update = useCallback((patch: Partial<Config>) => {
    setConfig((current) => {
      if (!current) return current;
      const next = { ...current, ...patch };
      if (next.magnifiedSize < next.iconSize) next.magnifiedSize = next.iconSize;
      pending.current = next;
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => {
        const latest = pending.current;
        pending.current = null;
        if (latest) {
          saveConfig(latest)
            .then(() => setFailed(false))
            .catch(() => setFailed(true));
        }
      }, SAVE_DELAY_MS);
      return next;
    });
  }, []);

  const changeAutostart = useCallback((enabled: boolean) => {
    setAutostartState(enabled);
    setAutostart(enabled).catch(() => {
      setAutostartState(!enabled);
      setFailed(true);
    });
  }, []);

  const recheck = useCallback(() => {
    setUpdateState({ kind: "checking" });
    checkUpdate()
      .then((found) => setUpdateState(toUpdateState(found)))
      .catch(() => setUpdateState({ kind: "error" }));
  }, []);

  const install = useCallback((info: UpdateInfo) => {
    setUpdateState({ kind: "installing", info, progress: null });
    // On success the installer replaces this app and this process exits.
    installUpdate().catch(() => setUpdateState({ kind: "installFailed", info }));
  }, []);

  const showNotes = useCallback((info: UpdateInfo) => {
    if (info.notes) openReleaseNotes(info.notes).catch(() => {});
  }, []);

  // Two clicks: the first arms the button, the second resets. Pinned and
  // recent apps stay; they belong to the user, not to the settings.
  const reset = useCallback(() => {
    if (!confirmingReset) {
      setConfirmingReset(true);
      resetTimer.current = window.setTimeout(() => setConfirmingReset(false), RESET_CONFIRM_MS);
      return;
    }
    setConfirmingReset(false);
    defaultConfig()
      .then(({ pinned: _pinned, recent: _recent, ...defaults }) => update(defaults))
      .catch(() => setFailed(true));
  }, [confirmingReset, update]);

  if (!config) return <div className="window" />;
  const t = textsFor(config.language);
  const appWindow = getCurrentWindow();
  const waiting = pendingUpdate(updateState);
  // An update the user asked for keeps its banner even after "Later".
  const showBanner =
    waiting !== null && (!bannerDismissed || updateState.kind !== "available");

  return (
    <div className="window">
      <aside className="sidebar" data-tauri-drag-region>
        <div className="traffic">
          <button
            type="button"
            className="light light-close"
            aria-label={t.close}
            onClick={() => appWindow.close()}
          />
          <button
            type="button"
            className="light light-minimize"
            aria-label={t.minimize}
            onClick={() => appWindow.minimize()}
          />
          <button
            type="button"
            className="light light-zoom"
            aria-label={t.zoom}
            onClick={() => appWindow.toggleMaximize()}
          />
        </div>
        <nav>
          {SECTIONS.map((id) => (
            <NavItem
              key={id}
              section={id}
              active={section === id}
              onSelect={() => setSection(id)}
              badge={id === "system" && waiting ? t.updateBadge : undefined}
            >
              {sectionTitle(t, id)}
            </NavItem>
          ))}
        </nav>
      </aside>

      <main className="content">
        <header className="content-title" data-tauri-drag-region>
          {sectionTitle(t, section)}
        </header>
        {showBanner && waiting ? (
          <UpdateBanner
            t={t}
            info={waiting}
            progress={updateState.kind === "installing" ? updateState.progress : null}
            installing={updateState.kind === "installing"}
            failed={updateState.kind === "installFailed"}
            onInstall={() => install(waiting)}
            onLater={() => setBannerDismissed(true)}
            onNotes={() => showNotes(waiting)}
          />
        ) : null}
        <div className="content-body">
          {section === "dock" ? (
            <>
              <Card>
                <Row label={t.size}>
                  <Slider
                    label={t.size}
                    value={config.iconSize}
                    min={ICON_SIZE.min}
                    max={ICON_SIZE.max}
                    before={t.small}
                    after={t.large}
                    onChange={(iconSize) => update({ iconSize })}
                  />
                </Row>
                <Row label={t.magnification} hint={t.magnificationHint}>
                  <Toggle
                    label={t.magnification}
                    checked={config.magnification}
                    onChange={(magnification) => update({ magnification })}
                  />
                </Row>
                <SubRow label={t.magnifiedSize} open={config.magnification}>
                  <Slider
                    label={t.magnifiedSize}
                    value={config.magnifiedSize}
                    min={config.iconSize}
                    max={MAX_MAGNIFIED_SIZE}
                    before={t.small}
                    after={t.large}
                    onChange={(magnifiedSize) => update({ magnifiedSize })}
                  />
                </SubRow>
              </Card>
              <Card title={t.groupBehavior}>
                <Row label={t.autoHide} hint={t.autoHideHint}>
                  <Toggle
                    label={t.autoHide}
                    checked={config.autoHide}
                    onChange={(autoHide) => update({ autoHide })}
                  />
                </Row>
                <Row label={t.bounceOnOpen}>
                  <Toggle
                    label={t.bounceOnOpen}
                    checked={config.bounceOnOpen}
                    onChange={(bounceOnOpen) => update({ bounceOnOpen })}
                  />
                </Row>
                <Row label={t.showIndicators}>
                  <Toggle
                    label={t.showIndicators}
                    checked={config.showIndicators}
                    onChange={(showIndicators) => update({ showIndicators })}
                  />
                </Row>
                <Row label={t.showRecents} hint={t.showRecentsHint}>
                  <Toggle
                    label={t.showRecents}
                    checked={config.showRecents}
                    onChange={(showRecents) => update({ showRecents })}
                  />
                </Row>
              </Card>
            </>
          ) : section === "windows" ? (
            <>
              <Card title={t.groupMinimize}>
                <Row label={t.genie} hint={t.genieHint}>
                  <Toggle
                    label={t.genie}
                    checked={config.genie}
                    onChange={(genie) => update({ genie })}
                  />
                </Row>
                <Row label={t.showMinimized} hint={t.showMinimizedHint}>
                  <Toggle
                    label={t.showMinimized}
                    checked={config.showMinimized}
                    onChange={(showMinimized) => update({ showMinimized })}
                  />
                </Row>
              </Card>
            </>
          ) : section === "menubar" ? (
            <Card>
              <Row label={t.menuBarShow} hint={t.menuBarHint}>
                <Toggle
                  label={t.menuBarShow}
                  checked={config.menuBar}
                  onChange={(menuBar) => update({ menuBar })}
                />
              </Row>
              <SubRow label={t.menuBarAutoHide} hint={t.menuBarAutoHideHint} open={config.menuBar}>
                <Toggle
                  label={t.menuBarAutoHide}
                  checked={config.menuBarAutoHide}
                  onChange={(menuBarAutoHide) => update({ menuBarAutoHide })}
                />
              </SubRow>
              <SubRow label={t.aiUsage} hint={t.aiUsageHint} open={config.menuBar}>
                <Toggle
                  label={t.aiUsage}
                  checked={config.aiUsage}
                  onChange={(aiUsage) => update({ aiUsage })}
                />
              </SubRow>
            </Card>
          ) : (
            <>
              <Card>
                <Row label={t.autostart}>
                  <Toggle label={t.autostart} checked={autostart} onChange={changeAutostart} />
                </Row>
                <Row label={t.language}>
                  <Segmented<Language>
                    label={t.language}
                    value={config.language}
                    options={[
                      { value: "auto", label: t.languageAuto },
                      { value: "es", label: "Español" },
                      { value: "en", label: "English" },
                    ]}
                    onChange={(language) => update({ language })}
                  />
                </Row>
              </Card>
              <Card title={t.updates}>
                <Row label={t.checkUpdates} hint={t.checkUpdatesHint}>
                  <Toggle
                    label={t.checkUpdates}
                    checked={config.checkUpdates}
                    onChange={(checkUpdates) => update({ checkUpdates })}
                  />
                </Row>
                <SubRow label={t.autoUpdate} hint={t.autoUpdateHint} open={config.checkUpdates}>
                  <Toggle
                    label={t.autoUpdate}
                    checked={config.autoUpdate}
                    onChange={(autoUpdate) => update({ autoUpdate })}
                  />
                </SubRow>
                <Row label={t.updatesNow} hint={updateHint(t, updateState, version)}>
                  {waiting && updateState.kind !== "installing" ? (
                    <button type="button" className="push push-primary" onClick={() => install(waiting)}>
                      {t.updatesInstall}
                    </button>
                  ) : (
                    <button
                      type="button"
                      className="push"
                      disabled={updateState.kind === "checking" || updateState.kind === "installing"}
                      onClick={recheck}
                    >
                      {t.updatesCheck}
                    </button>
                  )}
                </Row>
              </Card>
              <Card>
                <Row label={t.reset} hint={confirmingReset ? t.resetConfirmHint : t.resetHint}>
                  <button
                    type="button"
                    className={confirmingReset ? "push push-danger" : "push"}
                    onClick={reset}
                  >
                    {confirmingReset ? t.resetConfirm : t.resetButton}
                  </button>
                </Row>
              </Card>
            </>
          )}
          {failed ? (
            <p className="error" role="alert">
              {t.saveError}
            </p>
          ) : null}
        </div>
      </main>
    </div>
  );
}

function updateHint(t: Texts, state: UpdateState, version: string): string {
  switch (state.kind) {
    case "off":
      return t.updatesOff;
    case "checking":
      return t.updatesChecking;
    case "current":
      return t.updatesCurrent.replace("{version}", version);
    case "available":
      return t.updatesAvailable.replace("{version}", state.info.version);
    case "installing":
      return t.updatesInstalling;
    case "installFailed":
      return t.updateInstallError;
    case "error":
      return t.updatesError;
  }
}

const SECTIONS: readonly SectionId[] = ["dock", "windows", "menubar", "system"];

function sectionTitle(t: Texts, section: SectionId): string {
  switch (section) {
    case "dock":
      return t.dock;
    case "windows":
      return t.windows;
    case "menubar":
      return t.menuBar;
    case "system":
      return t.system;
  }
}

interface NavItemProps {
  section: SectionId;
  active: boolean;
  onSelect: () => void;
  /** Accessible text of a red badge, as macOS shows for a pending update. */
  badge?: string;
  children: string;
}

function NavItem({ section, active, onSelect, badge, children }: NavItemProps) {
  return (
    <button
      type="button"
      className="nav-item"
      aria-current={active ? "page" : undefined}
      onClick={onSelect}
    >
      <SectionIcon section={section} />
      {children}
      {badge ? (
        <span className="nav-badge" role="img" aria-label={badge}>
          1
        </span>
      ) : null}
    </button>
  );
}
