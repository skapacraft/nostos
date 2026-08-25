// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import * as api from "../lib/api";
import { toMessage } from "../lib/api";
import { formatBytes, formatCount, shortenPath } from "../lib/format";
import { locale } from "../lib/locale";
import type {
  OutputLayout,
  Progress,
  RepairReport,
  SpaceEstimate,
  WriteMode,
} from "../types";
import { ProgressBar } from "./ProgressBar";
import { RevealButton } from "./RevealButton";
import { SidecarSweep } from "./SidecarSweep";

interface PhotoFixerProps {
  /** Folder of the media to repair. */
  path: string;
  /** How many files have their date only in the sidecar. */
  repairable: number;
  /**
   * Working folder shared with the other panels.
   *
   * It lives in `App` and not here: choosing it twice, once to repair and once
   * to clean, is pointless friction given it is always the same in practice.
   */
  outputRoot: string | null;
  onOutputRoot: (path: string) => void;
  onDone: () => void;
  onError: (message: string) => void;
}

const LAYOUT_LABELS: Record<"en" | "it", Record<OutputLayout, string>> = {
  en: {
    preserve: "Same as the original",
    byYear: "One folder per year",
    byYearMonth: "Year and month",
    flat: "Everything in one folder",
  },
  it: {
    preserve: "Come l'originale",
    byYear: "Una cartella per anno",
    byYearMonth: "Anno e mese",
    flat: "Tutto in una cartella",
  },
};

const MODE_LABELS: Record<"en" | "it", Record<WriteMode, string>> = {
  en: {
    dryRun: "Dry run",
    copyToOutput: "Repaired copy",
    inPlace: "Rewrite originals",
  },
  it: {
    dryRun: "Simulazione",
    copyToOutput: "Copia riparata",
    inPlace: "Riscrivi originali",
  },
};

/**
 * Repair of photo metadata.
 *
 * The default mode produces copies in a separate folder: rewriting thousands
 * of original files is irreversible, and must not be the gesture that falls
 * under the finger first.
 */
export function PhotoFixer({
  path,
  repairable,
  outputRoot,
  onOutputRoot,
  onDone,
  onError,
}: PhotoFixerProps) {
  const it = locale() === "it";
  const layoutLabels = LAYOUT_LABELS[it ? "it" : "en"];
  const modeLabels = MODE_LABELS[it ? "it" : "en"];
  const [mode, setMode] = useState<WriteMode>("copyToOutput");
  const [layout, setLayout] = useState<OutputLayout>("preserve");
  const [space, setSpace] = useState<SpaceEstimate | null>(null);
  const [confirmedInPlace, setConfirmedInPlace] = useState(false);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [report, setReport] = useState<RepairReport | null>(null);
  const [running, setRunning] = useState(false);

  // The listener stays active for the whole life of the component:
  // registering it on every start would expose the window in which the event
  // arrives before the registration is complete.
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
        title: it
          ? "Dove salvare le foto riparate"
          : "Where to save the repaired photos",
      });
      if (typeof selected === "string") {
        onOutputRoot(selected);
        // The arithmetic is done straight away: on a large library the choice of
        // mode depends on how much room is left, and finding out halfway through
        // would be the worst possible discovery.
        setSpace(null);
        api
          .estimateSpace(path, selected)
          .then(setSpace)
          .catch(() => setSpace(null));
      }
    } catch (error) {
      onError(toMessage(error));
    }
  }, [path, onOutputRoot, onError]);

  const run = useCallback(
    async (onlyThis?: string) => {
      if (mode === "copyToOutput" && !outputRoot) {
        onError(
          it
            ? "Scegli prima la cartella di destinazione."
            : "Choose the destination folder first.",
        );
        return;
      }

      setRunning(true);
      runningRef.current = true;
      setReport(null);
      setProgress(null);

      try {
        const result = await api.repairPhotos(onlyThis ?? path, {
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
  // No point starting an operation the backend would refuse anyway.
  const noSpace = mode === "copyToOutput" && space !== null && !space.copyFits;

  return (
    <div className="space-y-4 rounded-xl border border-zinc-200 bg-white p-4 dark:border-zinc-800 dark:bg-zinc-900">
      <div>
        <h4 className="text-sm font-semibold text-zinc-900 dark:text-zinc-100">
          {it ? "Ripara data e posizione" : "Repair date and location"}
        </h4>
        <p className="mt-1 text-sm text-zinc-500 dark:text-zinc-400">
          {it
            ? repairable > 0
              ? `${formatCount(repairable)} file hanno la data solo nel sidecar JSON. La riparazione la scrive nei tag EXIF del file, senza ricomprimere l'immagine.`
              : "Scrive la data e le coordinate risolte nei tag EXIF, senza ricomprimere l'immagine."
            : repairable > 0
              ? `${formatCount(repairable)} files have their date only in the JSON sidecar. The repair writes it into the EXIF tags of the file, without recompressing the image.`
              : "Writes the resolved date and coordinates into the EXIF tags, without recompressing the image."}
        </p>
      </div>

      <fieldset disabled={running} className="space-y-2">
        <legend className="sr-only">{it ? "Modalità di scrittura" : "Write mode"}</legend>
        {(Object.keys(modeLabels) as WriteMode[]).map((value) => (
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
                {modeLabels[value]}
              </span>
              <span className="block text-xs text-zinc-500 dark:text-zinc-400">
                {it
                  ? value === "dryRun"
                    ? "Conta soltanto quanti file cambierebbero. Non scrive nulla."
                    : value === "copyToOutput"
                      ? "Scrive copie riparate altrove. Gli originali restano intatti."
                      : "Riscrive gli originali. Non può essere annullato."
                  : value === "dryRun"
                    ? "Only counts how many files would change. Writes nothing."
                    : value === "copyToOutput"
                      ? "Writes repaired copies elsewhere. Your originals stay untouched."
                      : "Rewrites the originals. This cannot be undone."}
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
            {outputRoot
              ? it
                ? "Cambia cartella"
                : "Change folder"
              : it
                ? "Scegli cartella"
                : "Choose folder"}
          </button>
          <span
            className="selectable min-w-0 truncate font-mono text-xs text-zinc-500 dark:text-zinc-400"
            title={outputRoot ?? undefined}
          >
            {outputRoot
              ? shortenPath(outputRoot, 56)
              : it
                ? "nessuna cartella scelta"
                : "no folder chosen"}
          </span>
        </div>
      ) : null}

      {mode === "copyToOutput" && space ? (
        <div
          className={[
            "space-y-2 rounded-lg border p-3 text-sm",
            space.copyFits
              ? "border-zinc-200 bg-zinc-50 text-zinc-700 dark:border-zinc-800 dark:bg-zinc-800/50 dark:text-zinc-300"
              : "border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-800 dark:bg-amber-950/40 dark:text-amber-200",
          ].join(" ")}
        >
          <p>
            {it ? (
              <>
                Libreria {formatBytes(space.sourceBytes)}, la copia richiede{" "}
                {formatBytes(space.neededForCopy)}. Sulla destinazione
                restano{" "}
                {formatBytes(space.availableBytes)}.
              </>
            ) : (
              <>
                Library {formatBytes(space.sourceBytes)}, the copy needs{" "}
                {formatBytes(space.neededForCopy)}. The destination has{" "}
                {formatBytes(space.availableBytes)} left.
              </>
            )}
          </p>
          {!space.copyFits ? (
            <>
              <p className="font-medium">
                {it
                  ? "Non ci sta. La copia duplica l'intera libreria, e deduplicare prima di solito recupera solo una frazione: non basta quando manca un ordine di grandezza."
                  : "It does not fit. The copy duplicates the whole library, and deduplicating first usually recovers a fraction: not enough when what is missing is an order of magnitude."}
              </p>
              <p>
                {it ? (
                  <>
                    Due vie d'uscita. La prima è{" "}
                    <strong>Riscrivi originali</strong>: lavora un file alla
                    volta e serve circa {formatBytes(space.neededInPlace)},
                    qualunque sia la dimensione della libreria. Fai prima un
                    backup.
                  </>
                ) : (
                  <>
                    Two ways out. The first is{" "}
                    <strong>Rewrite originals</strong>: it works one file at a
                    time and needs about {formatBytes(space.neededInPlace)},
                    whatever the size of the library. Make a backup first.
                  </>
                )}
              </p>
              {space.subfolders.some((folder) => folder.fits) ? (
                <div className="space-y-1.5">
                  <p>
                    {it ? (
                      <>
                        La seconda è{" "}
                        <strong>procedere per lotti</strong>: ripara una
                        cartella, sposta il risultato altrove, passa alla
                        successiva. Inizia dalle cartelle per anno, che
                        contengono quasi tutto; gli album sono per lo più
                        copie delle stesse foto.
                      </>
                    ) : (
                      <>
                        The second is to{" "}
                        <strong>work through it in batches</strong>: repair
                        one folder, move the result elsewhere, move to the
                        next. Start with the year folders, which hold nearly
                        everything; albums are mostly copies of the same
                        photos.
                      </>
                    )}
                  </p>
                  <ul className="space-y-1">
                    {space.subfolders.map((folder) => (
                      <li
                        key={folder.path}
                        className="flex flex-wrap items-center justify-between gap-2 rounded border border-amber-200 bg-white/60 px-2 py-1 dark:border-amber-900 dark:bg-zinc-900/40"
                      >
                        <span className="min-w-0">
                          <span className="truncate">{folder.name}</span>
                          <span className="ml-2 text-xs opacity-70">
                            {formatBytes(folder.bytes)},{" "}
                            {formatCount(folder.fileCount)}{" "}
                            {it ? "file" : "files"}
                            {folder.isYear ? (it ? " · anno" : " · year") : ""}
                            {folder.isAlbum && folder.uniqueHere === 0
                              ? it
                                ? " · album, solo copie"
                                : " · album, copies only"
                              : ""}
                          </span>
                          {folder.isAlbum && folder.uniqueHere > 0 ? (
                            <span className="block text-xs font-medium">
                              {it
                                ? `${formatCount(folder.uniqueHere)} foto esistono solo qui: saltando questo album le perdi.`
                                : `${formatCount(folder.uniqueHere)} photos exist only here: skip this album and you lose them.`}
                            </span>
                          ) : null}
                        </span>
                        {folder.fits ? (
                          <button
                            type="button"
                            onClick={() => run(folder.path)}
                            disabled={running}
                            className="shrink-0 rounded border border-amber-500 px-2 py-0.5 text-xs font-medium transition-colors hover:bg-amber-100 disabled:opacity-50 dark:hover:bg-amber-900/40"
                          >
                            {it ? "Ripara solo questa" : "Repair this one only"}
                          </button>
                        ) : (
                          <span className="shrink-0 text-xs opacity-60">
                            {it ? "troppo grande" : "too large"}
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
          <span className="text-zinc-700 dark:text-zinc-300">
            {it ? "Disposizione:" : "Layout:"}
          </span>
          <select
            value={layout}
            onChange={(event) =>
              setLayout(event.target.value as OutputLayout)
            }
            disabled={running}
            className="rounded-lg border border-zinc-300 bg-white px-2 py-1 text-sm text-zinc-900 disabled:opacity-50 dark:border-zinc-700 dark:bg-zinc-800 dark:text-zinc-100"
          >
            {(Object.keys(layoutLabels) as OutputLayout[]).map((value) => (
              <option key={value} value={value}>
                {layoutLabels[value]}
              </option>
            ))}
          </select>
          {layout !== "preserve" ? (
            <span className="text-xs text-zinc-500 dark:text-zinc-400">
              {it ? (
                <>
                  I file senza data finiscono in{" "}
                  <span className="font-mono">no-date/</span>.
                </>
              ) : (
                <>
                  Files with no date go into{" "}
                  <span className="font-mono">no-date/</span>.
                </>
              )}
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
            {it
              ? "Ho un backup dei miei file e accetto che vengano riscritti."
              : "I have a backup of my files and I accept that they will be rewritten."}
          </span>
        </label>
      ) : null}

      <button
        type="button"
        onClick={() => run()}
        disabled={running || inPlaceBlocked || missingOutput || noSpace}
        className="rounded-lg bg-zinc-900 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-zinc-700 disabled:cursor-not-allowed disabled:opacity-50 dark:bg-zinc-100 dark:text-zinc-900 dark:hover:bg-white"
      >
        {running
          ? it
            ? "Elaborazione..."
            : "Working..."
          : it
            ? `Avvia: ${modeLabels[mode]}`
            : `Start: ${modeLabels[mode]}`}
      </button>

      {running || progress ? (
        <ProgressBar
          progress={progress}
          label={it ? "Foto elaborate" : "Photos processed"}
        />
      ) : null}

      {report ? (
        <RepairSummary report={report} onError={onError} sourcePath={path} />
      ) : null}
    </div>
  );
}

function RepairSummary({
  report,
  onError,
  sourcePath,
}: {
  report: RepairReport;
  onError: (message: string) => void;
  /** The repaired folder, where the sidecars remain after a rewrite. */
  sourcePath: string;
}) {
  const it = locale() === "it";
  const isDryRun = report.mode === "dryRun";

  return (
    <div className="space-y-2 rounded-lg bg-zinc-50 p-3 text-sm dark:bg-zinc-800/50">
      <p className="text-zinc-900 dark:text-zinc-100">
        {it
          ? isDryRun
            ? `Simulazione: ${formatCount(report.candidates)} file verrebbero aggiornati.`
            : `Tag EXIF scritti per ${formatCount(report.exifWritten)} file su ${formatCount(report.candidates)}.`
          : isDryRun
            ? `Dry run: ${formatCount(report.candidates)} files would be updated.`
            : `EXIF tags written for ${formatCount(report.exifWritten)} files out of ${formatCount(report.candidates)}.`}
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
            {it
              ? `Date dei file allineate: ${formatCount(report.fileTimesWritten)}`
              : `File dates aligned: ${formatCount(report.fileTimesWritten)}`}
          </li>
        ) : null}
        {report.sidecarsCopied > 0 ? (
          <li>
            {it
              ? `Sidecar JSON conservati accanto alle copie: ${formatCount(report.sidecarsCopied)}`
              : `JSON sidecars kept beside the copies: ${formatCount(report.sidecarsCopied)}`}
          </li>
        ) : null}
        {report.skippedUnsupported > 0 ? (
          <li>
            {it
              ? `${formatCount(report.skippedUnsupported)} file in un formato in cui non scriviamo EXIF: PNG, GIF e video tengono i metadati altrove. Per quelli si usa la data del file, e il sidecar JSON viene copiato accanto alla foto così la data non va persa.`
              : `${formatCount(report.skippedUnsupported)} files in a format we do not write EXIF into: PNG, GIF and video keep their metadata elsewhere. For those the file date is used, and the JSON sidecar is copied beside the photo so the date is not lost.`}
          </li>
        ) : null}
        {report.skippedTooLarge > 0 ? (
          <li>
            {it
              ? `File troppo grandi: ${formatCount(report.skippedTooLarge)}`
              : `Files too large: ${formatCount(report.skippedTooLarge)}`}
          </li>
        ) : null}
      </ul>

      {report.failures.length > 0 ? (
        <details className="text-xs">
          <summary className="cursor-pointer text-red-700 dark:text-red-400">
            {it
              ? `${formatCount(report.failures.length)} errori`
              : `${formatCount(report.failures.length)} errors`}
          </summary>
          <ul className="selectable mt-1 max-h-40 space-y-0.5 overflow-y-auto font-mono text-zinc-500 dark:text-zinc-400">
            {report.failures.slice(0, 50).map((failure) => (
              <li key={failure}>{failure}</li>
            ))}
          </ul>
        </details>
      ) : null}

      {/*
        Only after the originals have been rewritten: in a repaired copy the
        sidecar is kept on purpose beside the formats we do not write EXIF
        into, so there it is not surplus.
      */}
      {report.mode === "inPlace" && report.exifWritten > 0 ? (
        <SidecarSweep path={sourcePath} onError={onError} />
      ) : null}
    </div>
  );
}
