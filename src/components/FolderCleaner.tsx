// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import * as api from "../lib/api";
import { toMessage } from "../lib/api";
import { formatBytes, formatCount, shortenPath } from "../lib/format";
import { locale } from "../lib/locale";
import type {
  CleanMode,
  CleanPlan,
  CleanReport,
  Progress,
  RestoreReport,
} from "../types";
import { ProgressBar } from "./ProgressBar";
import { RevealButton } from "./RevealButton";

interface FolderCleanerProps {
  path: string;
  /**
   * True if the folder holds albums whose membership has not been saved
   * yet. In that case deduplicating would destroy the only trace of which
   * photos were in which album.
   */
  albumRisk?: boolean;
  /** Working folder shared with the other panels. */
  destination: string | null;
  onDestination: (path: string) => void;
  onCleaned: () => void;
  onError: (message: string) => void;
}

const MODE_LABELS: Record<"en" | "it", Record<CleanMode, string>> = {
  en: {
    dryRun: "Analyse only",
    copyToOutput: "Clean tree elsewhere",
    quarantine: "Move to quarantine",
  },
  it: {
    dryRun: "Solo analisi",
    copyToOutput: "Albero pulito altrove",
    quarantine: "Sposta in quarantena",
  },
};

const MODE_HINTS: Record<"en" | "it", Record<CleanMode, string>> = {
  en: {
    dryRun: "Works out what would change. Writes nothing.",
    copyToOutput:
      "Rebuilds the tree elsewhere, keeping one file per distinct content. The original is left as it is.",
    quarantine:
      "Moves the surplus copies and the junk into a separate folder, with a ledger that puts everything back.",
  },
  it: {
    dryRun: "Calcola cosa cambierebbe. Non scrive nulla.",
    copyToOutput:
      "Ricostruisce l'albero altrove, tenendo un file per ogni contenuto distinto. L'originale resta com'è.",
    quarantine:
      "Sposta le copie in eccesso e la spazzatura in una cartella separata, con un registro che rimette tutto a posto.",
  },
};

/**
 * Cleanup of an export folder.
 *
 * There is no button that deletes, and that is a choice: an export is often
 * the only copy left of that data, and a botched deduplication carried out as
 * a deletion cannot be undone. Both working modes produce something
 * reversible.
 */
export function FolderCleaner({
  path,
  albumRisk = false,
  destination,
  onDestination,
  onCleaned,
  onError,
}: FolderCleanerProps) {
  const it = locale() === "it";
  const modeLabels = MODE_LABELS[it ? "it" : "en"];
  const modeHints = MODE_HINTS[it ? "it" : "en"];
  const [mode, setMode] = useState<CleanMode>("dryRun");
  const [removeJunk, setRemoveJunk] = useState(true);
  const [removeDuplicates, setRemoveDuplicates] = useState(true);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [plan, setPlan] = useState<CleanPlan | null>(null);
  const [report, setReport] = useState<CleanReport | null>(null);
  const [restore, setRestore] = useState<RestoreReport | null>(null);
  const [running, setRunning] = useState(false);

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

  const pickDestination = useCallback(async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title:
          mode === "quarantine"
            ? it
              ? "Dove mettere la quarantena"
              : "Where to put the quarantine"
            : it
              ? "Dove costruire l'albero pulito"
              : "Where to build the clean tree",
      });
      if (typeof selected === "string") onDestination(selected);
    } catch (error) {
      onError(toMessage(error));
    }
  }, [mode, onDestination, onError]);

  const run = useCallback(async () => {
    if (mode !== "dryRun" && !destination) {
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
    setRestore(null);
    setProgress(null);

    const options = {
      mode,
      destination: mode === "dryRun" ? null : destination,
      removeJunk,
      removeDuplicates,
      moveCompanions: true,
    };

    try {
      // The plan is always computed and shown: even before a real action the
      // user has to be able to see what is about to happen.
      setPlan(await api.planDriveClean(path, { ...options, mode: "dryRun" }));
      if (mode !== "dryRun") {
        setReport(await api.cleanDrive(path, options));
        onCleaned();
      }
    } catch (error) {
      onError(toMessage(error));
    } finally {
      runningRef.current = false;
      setRunning(false);
    }
  }, [mode, destination, removeJunk, removeDuplicates, path, onCleaned, onError]);

  const undo = useCallback(async () => {
    if (!report?.manifest) return;
    setRunning(true);
    try {
      setRestore(await api.restoreQuarantine(report.manifest));
      onCleaned();
    } catch (error) {
      onError(toMessage(error));
    } finally {
      setRunning(false);
    }
  }, [report, onCleaned, onError]);

  const needsDestination = mode !== "dryRun" && !destination;
  // The dry run is always allowed: it touches nothing.
  const blockedByAlbums = albumRisk && mode !== "dryRun";

  return (
    <div className="space-y-4 rounded-xl border border-zinc-200 bg-white p-4 dark:border-zinc-800 dark:bg-zinc-900">
      <div>
        <h4 className="text-sm font-semibold text-zinc-900 dark:text-zinc-100">
          {it ? "Pulizia" : "Cleanup"}
        </h4>
        <p className="mt-1 text-sm text-zinc-500 dark:text-zinc-400">
          {it
            ? "I duplicati sono confrontati per contenuto e non per nome: due file con lo stesso nome e la stessa dimensione ma contenuto diverso sopravvivono entrambi."
            : "Duplicates are compared by content and not by name: two files with the same name and the same size but different content both survive."}
        </p>
      </div>

      <fieldset disabled={running} className="space-y-2">
        <legend className="sr-only">{it ? "Modalità di pulizia" : "Cleanup mode"}</legend>
        {(Object.keys(modeLabels) as CleanMode[]).map((value) => (
          <label
            key={value}
            className="flex cursor-pointer items-start gap-2.5 text-sm"
          >
            <input
              type="radio"
              name="clean-mode"
              value={value}
              checked={mode === value}
              onChange={() => setMode(value)}
              className="mt-0.5"
            />
            <span>
              <span className="font-medium text-zinc-900 dark:text-zinc-100">
                {modeLabels[value]}
              </span>
              <span className="block text-xs text-zinc-500 dark:text-zinc-400">
                {modeHints[value]}
              </span>
            </span>
          </label>
        ))}
      </fieldset>

      <div className="flex flex-wrap gap-4 text-sm">
        <label className="flex items-center gap-2">
          <input
            type="checkbox"
            checked={removeDuplicates}
            onChange={(e) => setRemoveDuplicates(e.target.checked)}
            disabled={running}
          />
          <span className="text-zinc-700 dark:text-zinc-300">
            {it ? "Duplicati" : "Duplicates"}
          </span>
        </label>
        <label className="flex items-center gap-2">
          <input
            type="checkbox"
            checked={removeJunk}
            onChange={(e) => setRemoveJunk(e.target.checked)}
            disabled={running}
          />
          <span className="text-zinc-700 dark:text-zinc-300">
            {it
              ? "File di sistema (.DS_Store, Thumbs.db, __MACOSX)"
              : "System files (.DS_Store, Thumbs.db, __MACOSX)"}
          </span>
        </label>
      </div>

      {mode !== "dryRun" ? (
        <div className="flex flex-wrap items-center gap-3">
          <button
            type="button"
            onClick={pickDestination}
            disabled={running}
            className="rounded-lg border border-zinc-300 px-3 py-1.5 text-sm font-medium text-zinc-700 transition-colors hover:bg-zinc-100 disabled:opacity-50 dark:border-zinc-700 dark:text-zinc-200 dark:hover:bg-zinc-800"
          >
            {destination
              ? it
                ? "Cambia cartella"
                : "Change folder"
              : it
                ? "Scegli cartella"
                : "Choose folder"}
          </button>
          <span
            className="selectable min-w-0 truncate font-mono text-xs text-zinc-500 dark:text-zinc-400"
            title={destination ?? undefined}
          >
            {destination
              ? shortenPath(destination, 56)
              : it
                ? "nessuna cartella scelta"
                : "no folder chosen"}
          </span>
        </div>
      ) : null}

      {blockedByAlbums ? (
        <p className="rounded-lg border border-amber-300 bg-amber-50 p-3 text-sm text-amber-900 dark:border-amber-800 dark:bg-amber-950/40 dark:text-amber-200">
          {it
            ? "Questa cartella contiene album. Salva prima il manifest nel pannello sopra: la deduplica rimuove le copie negli album, e con esse l'unica traccia di quali foto appartenevano a quale album. I file tornano dalla quarantena, quell'informazione no."
            : "This folder contains albums. Save the manifest in the panel above first: deduplication removes the copies inside the albums, and with them the only trace of which photos belonged to which. Files come back from quarantine, that information does not."}
        </p>
      ) : null}

      <button
        type="button"
        onClick={run}
        disabled={running || needsDestination || blockedByAlbums}
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
          label={it ? "File verificati" : "Files checked"}
        />
      ) : null}

      {plan ? <PlanSummary plan={plan} /> : null}

      {report ? (
        <div className="space-y-2 rounded-lg border border-emerald-300 bg-emerald-50 p-3 text-sm dark:border-emerald-800 dark:bg-emerald-950/30">
          <p className="text-emerald-900 dark:text-emerald-200">
            {it
              ? `Spostati ${formatCount(report.duplicatesHandled)} duplicati e ${formatCount(report.junkHandled)} file di sistema. ${formatBytes(report.bytesReclaimed)} liberati.`
              : `Moved ${formatCount(report.duplicatesHandled)} duplicates and ${formatCount(report.junkHandled)} system files. ${formatBytes(report.bytesReclaimed)} freed.`}
          </p>
          {report.manifest ? (
            <>
              <p
                className="selectable truncate font-mono text-xs text-emerald-800 dark:text-emerald-300"
                title={report.manifest}
              >
                {shortenPath(report.manifest, 72)}
              </p>
              <RevealButton path={report.manifest} onError={onError} />
              <button
                type="button"
                onClick={undo}
                disabled={running || restore !== null}
                className="rounded-lg border border-emerald-500 px-3 py-1.5 text-sm font-medium text-emerald-900 transition-colors hover:bg-emerald-100 disabled:opacity-50 dark:border-emerald-700 dark:text-emerald-200 dark:hover:bg-emerald-900/40"
              >
                {it ? "Annulla e rimetti tutto a posto" : "Undo and put everything back"}
              </button>
            </>
          ) : null}
          {restore ? (
            <p className="text-emerald-900 dark:text-emerald-200">
              {it
                ? `Ripristinati ${formatCount(restore.restored)} file.`
                : `Restored ${formatCount(restore.restored)} files.`}
              {restore.skippedExisting > 0
                ? it
                  ? ` ${formatCount(restore.skippedExisting)} saltati perché già presenti alla sorgente.`
                  : ` ${formatCount(restore.skippedExisting)} skipped because they were already back at the source.`
                : ""}
            </p>
          ) : null}
          {report.failures.length > 0 ? (
            <p className="text-red-700 dark:text-red-400">
              {it
                ? `${formatCount(report.failures.length)} errori.`
                : `${formatCount(report.failures.length)} errors.`}
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function PlanSummary({ plan }: { plan: CleanPlan }) {
  const it = locale() === "it";
  const nothingToDo = plan.duplicateCopies === 0 && plan.junkFiles === 0;

  return (
    <div className="space-y-2 rounded-lg bg-zinc-50 p-3 text-sm dark:bg-zinc-800/50">
      {nothingToDo ? (
        <p className="text-zinc-900 dark:text-zinc-100">
          {it
            ? `Niente da rimuovere: ${formatCount(plan.filesScanned)} file, tutti distinti, nessuna spazzatura di sistema.`
            : `Nothing to remove: ${formatCount(plan.filesScanned)} files, all distinct, and no system junk.`}
        </p>
      ) : it ? (
        <p className="text-zinc-900 dark:text-zinc-100">
          {formatCount(plan.duplicateCopies)} copie in eccesso e{" "}
          {formatCount(plan.junkFiles)} file di sistema su{" "}
          {formatCount(plan.filesScanned)} esaminati.{" "}
          <span className="font-medium">
            {formatBytes(plan.reclaimableBytes)} recuperabili.
          </span>
        </p>
      ) : (
        <p className="text-zinc-900 dark:text-zinc-100">
          {formatCount(plan.duplicateCopies)} surplus copies and{" "}
          {formatCount(plan.junkFiles)} system files out of{" "}
          {formatCount(plan.filesScanned)} examined.{" "}
          <span className="font-medium">
            {formatBytes(plan.reclaimableBytes)} recoverable.
          </span>
        </p>
      )}

      {plan.companionFiles > 0 ? (
        <p className="text-xs text-zinc-500 dark:text-zinc-400">
          {it
            ? `${formatCount(plan.companionFiles)} sidecar JSON seguiranno i file multimediali a cui appartengono, così nessun orfano resta indietro.`
            : `${formatCount(plan.companionFiles)} JSON sidecars will follow the media they belong to, so no orphans are left behind.`}
        </p>
      ) : null}

      <p className="text-xs text-zinc-500 dark:text-zinc-400">
        {it
          ? `${formatBytes(plan.hashedBytes)} letti per verificare il contenuto: vengono confrontati davvero solo i file di dimensione uguale.`
          : `${formatBytes(plan.hashedBytes)} read to verify the content: only files of equal size are actually compared.`}
      </p>

      {plan.duplicateGroups.length > 0 ? (
        <details className="text-xs">
          <summary className="cursor-pointer text-zinc-600 dark:text-zinc-300">
            {it
              ? `Vedi i ${formatCount(plan.duplicateGroups.length)} gruppi di duplicati`
              : `See the ${formatCount(plan.duplicateGroups.length)} duplicate groups`}
          </summary>
          <ul className="mt-2 max-h-52 space-y-2 overflow-y-auto">
            {plan.duplicateGroups.slice(0, 20).map((group) => (
              <li key={group.hash} className="selectable">
                <p className="truncate text-zinc-700 dark:text-zinc-300">
                  {it ? "conservato" : "kept"}: {shortenPath(group.kept, 60)}
                </p>
                {group.copies.map((copy) => (
                  <p
                    key={copy}
                    className="truncate text-zinc-400 line-through dark:text-zinc-500"
                  >
                    {shortenPath(copy, 60)}
                  </p>
                ))}
                <p className="text-zinc-400 dark:text-zinc-500">
                  {it
                    ? `${formatBytes(group.sizeBytes)} ciascuno`
                    : `${formatBytes(group.sizeBytes)} each`}
                </p>
              </li>
            ))}
          </ul>
        </details>
      ) : null}
    </div>
  );
}
