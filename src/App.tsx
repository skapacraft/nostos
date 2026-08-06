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

/** Risultato dell'analisi della sezione selezionata. */
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
  // Il benvenuto compare una volta per sessione: ricordare la scelta
  // richiederebbe un file di preferenze, che questa app non scrive.
  // `null` finché non sappiamo cosa ha scelto l'utente: mostrare il modale e
  // poi farlo sparire sarebbe peggio che aspettare qualche millisecondo.
  const [showWelcome, setShowWelcome] = useState<boolean | null>(null);
  // Cartella di lavoro condivisa da riparazione e pulizia. Vive qui e non nei
  // pannelli, così non va scelta due volte per la stessa operazione.
  const [workingFolder, setWorkingFolder] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

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
      // Un salvataggio fallito non deve bloccare l'avvio: al massimo la
      // presentazione ricompare la volta successiva.
      api.writePreferences({ hideWelcome: true }).catch(() => undefined);
    }
  }, []);

  // La voce di menu "Guida" arriva come evento dal backend.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    api
      .onShowHelp(() => {
        setShowWelcome(false);
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
      setError(toMessage(err));
    } finally {
      setBusy(false);
    }
  }, []);

  const handleClose = useCallback(async () => {
    setBusy(true);
    try {
      await api.closeSource();
    } catch (err) {
      setError(toMessage(err));
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
          setError(`Nessun analizzatore per la sezione ${section.dirName}.`);
      }
    } catch (err) {
      setAnalysis(null);
      setError(toMessage(err));
    } finally {
      setBusy(false);
    }
  }, []);

  /// Dopo una scrittura reale il report precedente non è più attuale.
  const handleRepaired = useCallback(async () => {
    if (analysis?.kind !== "photos") return;
    try {
      setAnalysis({
        ...analysis,
        data: await api.scanPhotos(analysis.path),
      });
    } catch (err) {
      setError(toMessage(err));
    }
  }, [analysis]);

  /// Dopo una pulizia l'analisi precedente non è più attuale.
  const handleCleaned = useCallback(async () => {
    if (analysis?.kind !== "drive") return;
    try {
      setAnalysis({ ...analysis, data: await api.scanDrive(analysis.path) });
    } catch (err) {
      setError(toMessage(err));
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
              Open Takeout Hub
            </h1>
            <p className="text-xs text-zinc-500 dark:text-zinc-400">
              I tuoi dati Google, elaborati sul tuo computer.
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-3">
            <button
              type="button"
              onClick={() => setShowHelp((open) => !open)}
              aria-pressed={showHelp}
              className="rounded-lg border border-zinc-300 px-3 py-1 text-xs font-medium text-zinc-700 transition-colors hover:bg-zinc-100 dark:border-zinc-700 dark:text-zinc-200 dark:hover:bg-zinc-800"
            >
              {showHelp ? "Chiudi guida" : "Guida"}
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
              Chiudi
            </button>
          </div>
        ) : null}

        {showHelp ? (
          <Help
            info={info}
            privacy={privacy}
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
          Nessuna connessione di rete, nessuna telemetria, nessun aggiornamento
          automatico. Le analisi restano in memoria fino alla chiusura.
        </p>
      </footer>
    </div>
  );
}
