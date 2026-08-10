// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

import { useCallback, useEffect, useState } from "react";

import { Dropzone } from "./components/Dropzone";
import { Help } from "./components/Help";
import { Welcome } from "./components/Welcome";
import {
  CalendarReportView,
  ContactsReportView,
  DriveReportView,
  PhotoReportView,
} from "./components/Reports";
import { SourcePanel } from "./components/SourcePanel";
import * as api from "./lib/api";
import { toMessage } from "./lib/api";
import type {
  AppInfo,
  Preferences,
  CalendarReport,
  ContactsReport,
  DriveReport,
  PhotoScanReport,
  PrivacyReport,
  SectionSummary,
  SourceSummary,
} from "./types";

/** Result of analysing the selected section. */
type Analysis =
  | { kind: "photos"; path: string; label: string; data: PhotoScanReport }
  | { kind: "contacts"; path: string; label: string; data: ContactsReport }
  | { kind: "drive"; path: string; label: string; data: DriveReport }
  | { kind: "calendar"; path: string; label: string; data: CalendarReport };

export default function App() {
  const [summary, setSummary] = useState<SourceSummary | null>(null);
  const [analysis, setAnalysis] = useState<Analysis | null>(null);
  const [privacy, setPrivacy] = useState<PrivacyReport | null>(null);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [showHelp, setShowHelp] = useState(false);
  const [openReport, setOpenReport] = useState(false);
  const [openVersion, setOpenVersion] = useState(false);
  // The welcome screen shows once per session: remembering the choice
  // would need a preferences file, which this app does not write.
  // `null` until we know what the user chose: showing the modal and then
  // making it disappear would be worse than waiting a few milliseconds.
  const [showWelcome, setShowWelcome] = useState<boolean | null>(null);
  // Working folder shared by repair and cleanup. It lives here and not in the
  // panels, so it does not have to be chosen twice for the same operation.
  const [workingFolder, setWorkingFolder] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /**
   * Errors seen this session, for the problem report.
   *
   * In memory and bounded: it disappears when the window closes, like every
   * other result, so it adds nothing to what section 6 of PRIVACY_AUDIT.md
   * says gets written to disk.
   */
  const [errorLog, setErrorLog] = useState<string[]>([]);

  const reportError = useCallback((message: string) => {
    setError(message);
    setErrorLog((previous) => [...previous, message].slice(-20));
  }, []);

  useEffect(() => {
    api
      .privacyReport()
      .then(setPrivacy)
      .catch(() => setPrivacy(null));
    api
      .appInfo()
      .then(setInfo)
      .catch(() => setInfo(null));
    api
      .readPreferences()
      .then((prefs: Preferences) => setShowWelcome(!prefs.hideWelcome))
      .catch(() => setShowWelcome(true));
  }, []);

  const dismissWelcome = useCallback((hideNextTime: boolean) => {
    setShowWelcome(false);
    if (hideNextTime) {
      // A failed save must not block startup: at worst the introduction
      // reappears next time.
      api.writePreferences({ hideWelcome: true }).catch(() => undefined);
    }
  }, []);

  // The "Report a problem" menu item opens the guide already on that section.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    api
      .onShowReport(() => {
        setShowWelcome(false);
        setOpenVersion(false);
        setOpenReport(true);
        setShowHelp(true);
      })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch(() => undefined);

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // The "Guide" menu item arrives as an event from the backend.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    api
      .onShowHelp(() => {
        setShowWelcome(false);
        setOpenReport(false);
        setOpenVersion(false);
        setShowHelp(true);
      })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch(() => undefined);

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // The "Version and updates" menu item opens the guide on that section.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    api
      .onShowVersion(() => {
        setShowWelcome(false);
        setOpenReport(false);
        setOpenVersion(true);
        setShowHelp(true);
      })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch(() => undefined);

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const handleSelect = useCallback(async (path: string) => {
    setBusy(true);
    setError(null);
    setAnalysis(null);
    try {
      setSummary(await api.loadSource(path));
    } catch (err) {
      setSummary(null);
      reportError(toMessage(err));
    } finally {
      setBusy(false);
    }
  }, []);

  const handleClose = useCallback(async () => {
    setBusy(true);
    try {
      await api.closeSource();
    } catch (err) {
      reportError(toMessage(err));
    } finally {
      setSummary(null);
      setAnalysis(null);
      setBusy(false);
    }
  }, []);

  const handleAnalyze = useCallback(async (section: SectionSummary) => {
    setBusy(true);
    setError(null);
    try {
      switch (section.section) {
        case "googlePhotos":
          setAnalysis({
            kind: "photos",
            path: section.path,
            label: section.dirName,
            data: await api.scanPhotos(section.path),
          });
          break;
        case "contacts":
          setAnalysis({
            kind: "contacts",
            path: section.path,
            label: section.dirName,
            data: await api.scanContacts(section.path),
          });
          break;
        case "drive":
          setAnalysis({
            kind: "drive",
            path: section.path,
            label: section.dirName,
            data: await api.scanDrive(section.path),
          });
          break;
        case "calendar":
          setAnalysis({
            kind: "calendar",
            path: section.path,
            label: section.dirName,
            data: await api.scanCalendar(section.path),
          });
          break;
        default:
          reportError(`No analyser for the ${section.dirName} section.`);
      }
    } catch (err) {
      setAnalysis(null);
      reportError(toMessage(err));
    } finally {
      setBusy(false);
    }
  }, []);

  /// After a real write the previous report is no longer current.
  const handleRepaired = useCallback(async () => {
    if (analysis?.kind !== "photos") return;
    try {
      setAnalysis({
        ...analysis,
        data: await api.scanPhotos(analysis.path),
      });
    } catch (err) {
      reportError(toMessage(err));
    }
  }, [analysis]);

  /// After a cleanup the previous analysis is no longer current.
  const handleCleaned = useCallback(async () => {
    if (analysis?.kind !== "drive") return;
    try {
      setAnalysis({ ...analysis, data: await api.scanDrive(analysis.path) });
    } catch (err) {
      reportError(toMessage(err));
    }
  }, [analysis]);

  return (
    <div className="min-h-full">
      {showWelcome ? (
        <Welcome
          info={info}
          onStart={dismissWelcome}
          onOpenHelp={() => {
            setShowWelcome(false);
            setShowHelp(true);
          }}
        />
      ) : null}
      <header className="border-b border-zinc-200 bg-white/80 backdrop-blur dark:border-zinc-800 dark:bg-zinc-950/80">
        <div className="mx-auto flex max-w-4xl items-center justify-between gap-4 px-6 py-4">
          <div>
            <h1 className="text-base font-semibold text-zinc-900 dark:text-zinc-100">
              Nostos
            </h1>
            <p className="text-xs text-zinc-500 dark:text-zinc-400">
              Your Google data, processed on your own computer.
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-3">
            <button
              type="button"
              onClick={() => setShowHelp((open) => !open)}
              aria-pressed={showHelp}
              className="rounded-lg border border-zinc-300 px-3 py-1 text-xs font-medium text-zinc-700 transition-colors hover:bg-zinc-100 dark:border-zinc-700 dark:text-zinc-200 dark:hover:bg-zinc-800"
            >
              {showHelp ? "Close guide" : "Guide"}
            </button>
          {privacy && !privacy.networkCalls ? (
            <span
              className="inline-flex shrink-0 items-center gap-1.5 rounded-full border border-emerald-300 bg-emerald-50 px-3 py-1 text-xs font-medium text-emerald-800 dark:border-emerald-800 dark:bg-emerald-950/40 dark:text-emerald-300"
              title={privacy.notes.join("\n")}
            >
              <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
              Offline
            </span>
          ) : null}
          </div>
        </div>
      </header>

      <main className="mx-auto max-w-4xl space-y-8 px-6 py-8">
        {error ? (
          <div
            role="alert"
            className="flex items-start justify-between gap-4 rounded-xl border border-red-300 bg-red-50 px-4 py-3 text-sm text-red-800 dark:border-red-900 dark:bg-red-950/40 dark:text-red-300"
          >
            <p className="selectable">{error}</p>
            <button
              type="button"
              onClick={() => setError(null)}
              className="shrink-0 font-medium underline underline-offset-2"
            >
              Close
            </button>
          </div>
        ) : null}

        {showHelp ? (
          <Help
            info={info}
            privacy={privacy}
            errors={errorLog}
            openReport={openReport}
            openVersion={openVersion}
            onError={reportError}
            onClose={() => setShowHelp(false)}
          />
        ) : summary ? (
          <>
            <SourcePanel
              summary={summary}
              activeSection={analysis?.path ?? null}
              busy={busy}
              onAnalyze={handleAnalyze}
              onClose={handleClose}
            />

            {analysis ? (
              <section className="space-y-4">
                <h3 className="text-lg font-semibold text-zinc-900 dark:text-zinc-100">
                  {analysis.label}
                </h3>
                {analysis.kind === "photos" ? (
                  <PhotoReportView
                    report={analysis.data}
                    path={analysis.path}
                    workingFolder={workingFolder}
                    onWorkingFolder={setWorkingFolder}
                    onRepaired={handleRepaired}
                    onError={setError}
                  />
                ) : analysis.kind === "contacts" ? (
                  <ContactsReportView
                    report={analysis.data}
                    path={analysis.path}
                    onError={setError}
                  />
                ) : analysis.kind === "calendar" ? (
                  <CalendarReportView
                    report={analysis.data}
                    path={analysis.path}
                    onError={setError}
                  />
                ) : (
                  <DriveReportView
                    report={analysis.data}
                    path={analysis.path}
                    workingFolder={workingFolder}
                    onWorkingFolder={setWorkingFolder}
                    onCleaned={handleCleaned}
                    onError={setError}
                  />
                )}
              </section>
            ) : null}
          </>
        ) : (
          <Dropzone onSelect={handleSelect} onError={setError} busy={busy} />
        )}
      </main>

      <footer className="mx-auto max-w-4xl px-6 pb-8">
        <p className="text-xs text-zinc-400 dark:text-zinc-600">
          No network connections, no telemetry, no automatic updates. What is
          examined stays in memory until you close the window.
        </p>
        <p className="mt-2 text-xs text-zinc-400 dark:text-zinc-600">
          Not affiliated with, endorsed by or sponsored by Google LLC.
          Google, Google Photos, Google Drive and Google Takeout are
          trademarks of Google LLC, named here only to identify the
          export this software reads.
        </p>
      </footer>
    </div>
  );
}
