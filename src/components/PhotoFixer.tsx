// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import * as api from "../lib/api";
import { toMessage } from "../lib/api";
import { formatBytes, formatCount, shortenPath } from "../lib/format";
import type {
  OutputLayout,
  Progress,
  RepairReport,
  SpaceEstimate,
  WriteMode,
} from "../types";
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

const LAYOUT_LABELS: Record<OutputLayout, string> = {
  preserve: "Come l'originale",
  byYear: "Una cartella per anno",
  byYearMonth: "Anno e mese",
  flat: "Tutto in una cartella",
};

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
  const [layout, setLayout] = useState<OutputLayout>("preserve");
  const [spazio, setSpazio] = useState<SpaceEstimate | null>(null);
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
      if (typeof selected === "string") {
        onOutputRoot(selected);
        // I conti si fanno subito: su una libreria grande la scelta della
        // modalità dipende da quanto spazio resta, e scoprirlo a metà lavoro
        // sarebbe la scoperta peggiore possibile.
        setSpazio(null);
        api
          .estimateSpace(path, selected)
          .then(setSpazio)
          .catch(() => setSpazio(null));
      }
    } catch (error) {
      onError(toMessage(error));
    }
  }, [path, onOutputRoot, onError]);

  const run = useCallback(
    async (soloQuesta?: string) => {
      if (mode === "copyToOutput" && !outputRoot) {
        onError("Scegli prima la cartella di destinazione.");
        return;
      }

      setRunning(true);
      runningRef.current = true;
      setReport(null);
      setProgress(null);

      try {
        const result = await api.repairPhotos(soloQuesta ?? path, {
          mode,
          layout,
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
    },
    [mode, layout, outputRoot, path, onDone, onError],
  );

  const inPlaceBlocked = mode === "inPlace" && !confirmedInPlace;
  const missingOutput = mode === "copyToOutput" && !outputRoot;
  // Inutile far partire un'operazione che il backend rifiuterebbe comunque.
  const noSpace = mode === "copyToOutput" && spazio !== null && !spazio.copyFits;

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

      {mode === "copyToOutput" && spazio ? (
        <div
          className={[
            "space-y-2 rounded-lg border p-3 text-sm",
            spazio.copyFits
              ? "border-zinc-200 bg-zinc-50 text-zinc-700 dark:border-zinc-800 dark:bg-zinc-800/50 dark:text-zinc-300"
              : "border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-800 dark:bg-amber-950/40 dark:text-amber-200",
          ].join(" ")}
        >
          <p>
            Libreria {formatBytes(spazio.sourceBytes)}, servono{" "}
            {formatBytes(spazio.neededForCopy)} per la copia. Sulla destinazione
            restano {formatBytes(spazio.availableBytes)}.
          </p>
          {!spazio.copyFits ? (
            <>
              <p className="font-medium">
                Non ci sta. La copia duplica l'intera libreria, e deduplicare
                prima recupera in genere una frazione: non basta quando manca
                l'ordine di grandezza.
              </p>
              <p>
                Due vie d'uscita. La prima è{" "}
                <strong>Modifica originali</strong>: lavora un file per volta e
                richiede circa {formatBytes(spazio.neededInPlace)} di spazio,
                qualunque sia la dimensione della libreria. Fai prima una copia
                di sicurezza.
              </p>
              {spazio.subfolders.some((cartella) => cartella.fits) ? (
                <div className="space-y-1.5">
                  <p>
                    La seconda è <strong>procedere a tranche</strong>: ripari una
                    cartella, sposti il risultato altrove, passi alla
                    successiva. Comincia dalle cartelle per anno, che
                    contengono quasi tutto; gli album sono in gran parte copie
                    delle stesse foto.
                  </p>
                  <ul className="space-y-1">
                    {spazio.subfolders.map((cartella) => (
                      <li
                        key={cartella.path}
                        className="flex flex-wrap items-center justify-between gap-2 rounded border border-amber-200 bg-white/60 px-2 py-1 dark:border-amber-900 dark:bg-zinc-900/40"
                      >
                        <span className="min-w-0">
                          <span className="truncate">{cartella.name}</span>
                          <span className="ml-2 text-xs opacity-70">
                            {formatBytes(cartella.bytes)},{" "}
                            {formatCount(cartella.fileCount)} file
                            {cartella.isYear ? " · annata" : ""}
                            {cartella.isAlbum && cartella.uniqueHere === 0
                              ? " · album, solo copie"
                              : ""}
                          </span>
                          {cartella.isAlbum && cartella.uniqueHere > 0 ? (
                            <span className="block text-xs font-medium">
                              {formatCount(cartella.uniqueHere)} foto stanno
                              solo qui: saltando questo album le perderesti.
                            </span>
                          ) : null}
                        </span>
                        {cartella.fits ? (
                          <button
                            type="button"
                            onClick={() => run(cartella.path)}
                            disabled={running}
                            className="shrink-0 rounded border border-amber-500 px-2 py-0.5 text-xs font-medium transition-colors hover:bg-amber-100 disabled:opacity-50 dark:hover:bg-amber-900/40"
                          >
                            Ripara solo questa
                          </button>
                        ) : (
                          <span className="shrink-0 text-xs opacity-60">
                            troppo grande
                          </span>
                        )}
                      </li>
                    ))}
                  </ul>
                </div>
              ) : null}
            </>
          ) : null}
        </div>
      ) : null}

      {mode === "copyToOutput" ? (
        <label className="flex flex-wrap items-center gap-2 text-sm">
          <span className="text-zinc-700 dark:text-zinc-300">Disposizione:</span>
          <select
            value={layout}
            onChange={(event) =>
              setLayout(event.target.value as OutputLayout)
            }
            disabled={running}
            className="rounded-lg border border-zinc-300 bg-white px-2 py-1 text-sm text-zinc-900 disabled:opacity-50 dark:border-zinc-700 dark:bg-zinc-800 dark:text-zinc-100"
          >
            {(Object.keys(LAYOUT_LABELS) as OutputLayout[]).map((value) => (
              <option key={value} value={value}>
                {LAYOUT_LABELS[value]}
              </option>
            ))}
          </select>
          {layout !== "preserve" ? (
            <span className="text-xs text-zinc-500 dark:text-zinc-400">
              I file senza data finiscono in <span className="font-mono">senza-data/</span>.
            </span>
          ) : null}
        </label>
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
        onClick={() => run()}
        disabled={running || inPlaceBlocked || missingOutput || noSpace}
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
