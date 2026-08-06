// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import * as api from "../lib/api";
import { toMessage } from "../lib/api";
import { formatCount, shortenPath } from "../lib/format";
import type { Progress, RepairReport, WriteMode } from "../types";
import { ProgressBar } from "./ProgressBar";
import { RevealButton } from "./RevealButton";

interface PhotoFixerProps {
  /** Cartella dei media da riparare. */
  path: string;
  /** Quanti file hanno la data solo nel sidecar. */
  repairable: number;
  /**
   * Cartella di lavoro condivisa con gli altri pannelli.
   *
   * Vive in `App` e non qui: sceglierla due volte, una per riparare e una per
   * pulire, è attrito inutile visto che nella pratica è sempre la stessa.
   */
  outputRoot: string | null;
  onOutputRoot: (path: string) => void;
  onDone: () => void;
  onError: (message: string) => void;
}

const MODE_LABELS: Record<WriteMode, string> = {
  dryRun: "Simulazione",
  copyToOutput: "Copia riparata",
  inPlace: "Modifica originali",
};

/**
 * Riparazione dei metadati delle foto.
 *
 * La modalità predefinita produce copie in una cartella separata: riscrivere
 * migliaia di file originali è irreversibile, e non deve essere il gesto che
 * capita per primo sotto al dito.
 */
export function PhotoFixer({
  path,
  repairable,
  outputRoot,
  onOutputRoot,
  onDone,
  onError,
}: PhotoFixerProps) {
  const [mode, setMode] = useState<WriteMode>("copyToOutput");
  const [confirmedInPlace, setConfirmedInPlace] = useState(false);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [report, setReport] = useState<RepairReport | null>(null);
  const [running, setRunning] = useState(false);

  // Il listener resta attivo per tutta la vita del componente: registrarlo a
  // ogni avvio esporrebbe alla finestra in cui l'evento arriva prima che la
  // registrazione sia completata.
  const runningRef = useRef(false);
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    api
      .onProgress((event) => {
        if (runningRef.current) setProgress(event);
      })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch((error) => onError(toMessage(error)));

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [onError]);

  const pickOutput = useCallback(async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Dove salvare le foto riparate",
      });
      if (typeof selected === "string") onOutputRoot(selected);
    } catch (error) {
      onError(toMessage(error));
    }
  }, [onOutputRoot, onError]);

  const run = useCallback(async () => {
    if (mode === "copyToOutput" && !outputRoot) {
      onError("Scegli prima la cartella di destinazione.");
      return;
    }

    setRunning(true);
    runningRef.current = true;
    setReport(null);
    setProgress(null);

    try {
      const result = await api.repairPhotos(path, {
        mode,
        outputRoot: mode === "copyToOutput" ? outputRoot : null,
        writeExif: true,
        writeFileTimes: true,
      });
      setReport(result);
      if (mode !== "dryRun") onDone();
    } catch (error) {
      onError(toMessage(error));
    } finally {
      runningRef.current = false;
      setRunning(false);
    }
  }, [mode, outputRoot, path, onDone, onError]);

  const inPlaceBlocked = mode === "inPlace" && !confirmedInPlace;
  const missingOutput = mode === "copyToOutput" && !outputRoot;

  return (
    <div className="space-y-4 rounded-xl border border-zinc-200 bg-white p-4 dark:border-zinc-800 dark:bg-zinc-900">
      <div>
        <h4 className="text-sm font-semibold text-zinc-900 dark:text-zinc-100">
          Ripara data e posizione
        </h4>
        <p className="mt-1 text-sm text-zinc-500 dark:text-zinc-400">
          {repairable > 0
            ? `${formatCount(repairable)} file hanno la data solo nel sidecar JSON. La riparazione la scrive nei tag EXIF del file, senza ricomprimere l'immagine.`
            : "Scrive nei tag EXIF la data e le coordinate risolte, senza ricomprimere l'immagine."}
        </p>
      </div>

      <fieldset disabled={running} className="space-y-2">
        <legend className="sr-only">Modalità di scrittura</legend>
        {(Object.keys(MODE_LABELS) as WriteMode[]).map((value) => (
          <label
            key={value}
            className="flex cursor-pointer items-start gap-2.5 text-sm"
          >
            <input
              type="radio"
              name="write-mode"
              value={value}
              checked={mode === value}
              onChange={() => {
                setMode(value);
                setConfirmedInPlace(false);
              }}
              className="mt-0.5"
            />
            <span>
              <span className="font-medium text-zinc-900 dark:text-zinc-100">
                {MODE_LABELS[value]}
              </span>
              <span className="block text-xs text-zinc-500 dark:text-zinc-400">
                {value === "dryRun"
                  ? "Conta soltanto quanti file cambierebbero. Non scrive nulla."
                  : value === "copyToOutput"
                    ? "Scrive le copie riparate altrove. Gli originali restano intatti."
                    : "Riscrive gli originali. L'operazione non è reversibile."}
              </span>
            </span>
          </label>
        ))}
      </fieldset>

      {mode === "copyToOutput" ? (
        <div className="flex flex-wrap items-center gap-3">
          <button
            type="button"
            onClick={pickOutput}
            disabled={running}
            className="rounded-lg border border-zinc-300 px-3 py-1.5 text-sm font-medium text-zinc-700 transition-colors hover:bg-zinc-100 disabled:opacity-50 dark:border-zinc-700 dark:text-zinc-200 dark:hover:bg-zinc-800"
          >
            {outputRoot ? "Cambia cartella" : "Scegli cartella"}
          </button>
          <span
            className="selectable min-w-0 truncate font-mono text-xs text-zinc-500 dark:text-zinc-400"
            title={outputRoot ?? undefined}
          >
            {outputRoot ? shortenPath(outputRoot, 56) : "nessuna cartella scelta"}
          </span>
        </div>
      ) : null}

      {mode === "inPlace" ? (
        <label className="flex items-start gap-2.5 rounded-lg border border-amber-300 bg-amber-50 p-3 text-sm text-amber-900 dark:border-amber-800 dark:bg-amber-950/40 dark:text-amber-200">
          <input
            type="checkbox"
            checked={confirmedInPlace}
            onChange={(event) => setConfirmedInPlace(event.target.checked)}
            disabled={running}
            className="mt-0.5"
          />
          <span>
            Ho una copia di sicurezza dei miei file e accetto che vengano
            riscritti.
          </span>
        </label>
      ) : null}

      <button
        type="button"
        onClick={run}
        disabled={running || inPlaceBlocked || missingOutput}
        className="rounded-lg bg-zinc-900 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-zinc-700 disabled:cursor-not-allowed disabled:opacity-50 dark:bg-zinc-100 dark:text-zinc-900 dark:hover:bg-white"
      >
        {running ? "Elaborazione..." : `Avvia: ${MODE_LABELS[mode]}`}
      </button>

      {running || progress ? (
        <ProgressBar progress={progress} label="Foto elaborate" />
      ) : null}

      {report ? <RepairSummary report={report} onError={onError} /> : null}
    </div>
  );
}

function RepairSummary({
  report,
  onError,
}: {
  report: RepairReport;
  onError: (message: string) => void;
}) {
  const isDryRun = report.mode === "dryRun";

  return (
    <div className="space-y-2 rounded-lg bg-zinc-50 p-3 text-sm dark:bg-zinc-800/50">
      <p className="text-zinc-900 dark:text-zinc-100">
        {isDryRun
          ? `Simulazione: ${formatCount(report.candidates)} file verrebbero aggiornati.`
          : `Scritti i tag EXIF di ${formatCount(report.exifWritten)} file su ${formatCount(report.candidates)}.`}
      </p>

      {!isDryRun && report.outputRoot ? (
        <div className="space-y-2">
          <p
            className="selectable truncate font-mono text-xs text-zinc-500 dark:text-zinc-400"
            title={report.outputRoot}
          >
            {shortenPath(report.outputRoot, 72)}
          </p>
          <RevealButton path={report.outputRoot} onError={onError} />
        </div>
      ) : null}

      <ul className="space-y-0.5 text-xs text-zinc-500 dark:text-zinc-400">
        {report.fileTimesWritten > 0 ? (
          <li>
            Date di modifica allineate: {formatCount(report.fileTimesWritten)}
          </li>
        ) : null}
        {report.sidecarsCopied > 0 ? (
          <li>
            Sidecar JSON conservati accanto alle copie:{" "}
            {formatCount(report.sidecarsCopied)}
          </li>
        ) : null}
        {report.skippedUnsupported > 0 ? (
          <li>
            {formatCount(report.skippedUnsupported)} file in un formato in cui
            non scriviamo l'EXIF: PNG, GIF e i video tengono i metadati altrove.
            Per questi vale la data del file, e il sidecar JSON viene copiato
            accanto alla foto perché la data non vada persa.
          </li>
        ) : null}
        {report.skippedTooLarge > 0 ? (
          <li>File troppo grandi: {formatCount(report.skippedTooLarge)}</li>
        ) : null}
      </ul>

      {report.failures.length > 0 ? (
        <details className="text-xs">
          <summary className="cursor-pointer text-red-700 dark:text-red-400">
            {formatCount(report.failures.length)} errori
          </summary>
          <ul className="selectable mt-1 max-h-40 space-y-0.5 overflow-y-auto font-mono text-zinc-500 dark:text-zinc-400">
            {report.failures.slice(0, 50).map((failure) => (
              <li key={failure}>{failure}</li>
            ))}
          </ul>
        </details>
      ) : null}
    </div>
  );
}
