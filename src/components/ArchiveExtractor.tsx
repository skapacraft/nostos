// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import * as api from "../lib/api";
import { toMessage } from "../lib/api";
import { shortenPath } from "../lib/format";
import { locale } from "../lib/locale";
import type { Progress } from "../types";
import { ProgressBar } from "./ProgressBar";

interface ArchiveExtractorProps {
  /** Path of the archive the user loaded. */
  path: string;
  /** Called with the extracted folder once extraction succeeds. */
  onExtracted: (destination: string) => void;
  onError: (message: string) => void;
}

/**
 * Extracts a `.zip` Takeout export into a folder the user chooses.
 *
 * `extractTakeout` already reconstructs the whole archive series from any one
 * member, so this only has to ask for a destination and hand the result back.
 */
export function ArchiveExtractor({
  path,
  onExtracted,
  onError,
}: ArchiveExtractorProps) {
  const it = locale() === "it";
  const [destination, setDestination] = useState<string | null>(null);
  const [progress, setProgress] = useState<Progress | null>(null);
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
        title: it
          ? "Dove estrarre l'archivio"
          : "Where to extract the archive",
      });
      if (typeof selected === "string") setDestination(selected);
    } catch (error) {
      onError(toMessage(error));
    }
  }, [it, onError]);

  const run = useCallback(async () => {
    if (!destination) return;

    setRunning(true);
    runningRef.current = true;
    setProgress(null);

    try {
      const report = await api.extractTakeout(path, destination);
      onExtracted(report.destination);
    } catch (error) {
      onError(toMessage(error));
    } finally {
      runningRef.current = false;
      setRunning(false);
    }
  }, [path, destination, onExtracted, onError]);

  return (
    <div className="space-y-4 rounded-xl border border-zinc-200 bg-white p-4 dark:border-zinc-800 dark:bg-zinc-900">
      <div>
        <h4 className="text-sm font-semibold text-zinc-900 dark:text-zinc-100">
          {it ? "Estrai l'archivio" : "Extract the archive"}
        </h4>
        <p className="mt-1 text-sm text-zinc-500 dark:text-zinc-400">
          {it
            ? "Le sezioni si possono esaminare solo dopo l'estrazione. Se ci sono altri archivi della stessa serie nella stessa cartella, vengono uniti insieme."
            : "Sections can only be examined after extraction. Any other archives from the same series sitting in the same folder are merged in."}
        </p>
      </div>

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

      <button
        type="button"
        onClick={run}
        disabled={running || !destination}
        className="rounded-lg bg-zinc-900 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-zinc-700 disabled:cursor-not-allowed disabled:opacity-50 dark:bg-zinc-100 dark:text-zinc-900 dark:hover:bg-white"
      >
        {running
          ? it
            ? "Estrazione..."
            : "Extracting..."
          : it
            ? "Estrai"
            : "Extract"}
      </button>

      {running || progress ? (
        <ProgressBar
          progress={progress}
          label={it ? "File estratti" : "Files extracted"}
        />
      ) : null}
    </div>
  );
}
