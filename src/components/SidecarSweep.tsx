// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import * as api from "../lib/api";
import { toMessage } from "../lib/api";
import { formatBytes, formatCount, shortenPath } from "../lib/format";
import { SIDECAR_KEPT_LABELS } from "../lib/messages";
import type { SidecarSweepReport } from "../types";
import { RevealButton } from "./RevealButton";

interface SidecarSweepProps {
  /** The repaired folder, where the sidecars to assess are found. */
  path: string;
  onError: (message: string) => void;
}

/**
 * Moves the JSON sidecars whose content is now inside the photos.
 *
 * It appears only after rewriting the originals, the one case where those
 * JSONs really are surplus: in the repaired copy the sidecar is kept on
 * purpose beside the formats where we write no EXIF.
 *
 * It is not a cleanup button and must not be presented as one. It moves,
 * writes a ledger and undoes: deleting would have meant throwing away the data
 * that has no home in the EXIF tags, that is doing in reverse the very damage
 * this application repairs.
 */
export function SidecarSweep({ path, onError }: SidecarSweepProps) {
  const [destination, setDestination] = useState<string | null>(null);
  const [report, setReport] = useState<SidecarSweepReport | null>(null);
  const [running, setRunning] = useState(false);
  const [restored, setRestored] = useState(false);

  const chooseDestination = async () => {
    try {
      const chosen = await open({
        directory: true,
        multiple: false,
        title: "Where to move the sidecars already applied",
      });
      if (typeof chosen === "string") setDestination(chosen);
    } catch (error) {
      onError(toMessage(error));
    }
  };

  const run = async () => {
    if (!destination) return;
    setRunning(true);
    try {
      setReport(await api.sweepSidecars(path, destination));
      setRestored(false);
    } catch (error) {
      onError(toMessage(error));
    } finally {
      setRunning(false);
    }
  };

  const undo = async () => {
    if (!report?.manifest) return;
    setRunning(true);
    try {
      const outcome = await api.restoreQuarantine(report.manifest);
      setRestored(true);
      if (outcome.failures.length > 0) onError(outcome.failures[0]);
    } catch (error) {
      onError(toMessage(error));
    } finally {
      setRunning(false);
    }
  };

  return (
    <details className="rounded-lg border border-zinc-200 p-3 dark:border-zinc-700">
      <summary className="cursor-pointer text-sm font-medium text-zinc-900 dark:text-zinc-100">
        Set aside the JSON files that are now redundant
      </summary>

      <div className="mt-3 space-y-3 text-sm text-zinc-600 dark:text-zinc-300">
        <p>
          Every repaired photograph now carries inside it what used to be in
          its <span className="font-mono">.json</span> file: date, coordinates,
          description, recognised faces and the favourite star. Those JSON
          files can be moved elsewhere to leave the folder clean.
        </p>
        <p className="rounded-lg bg-zinc-50 p-3 text-xs dark:bg-zinc-800/50">
          Only the JSON files that are no longer the sole copy of anything are
          moved, checking inside each file that the data really is there. The
          ones for PNG, GIF and video stay where they are, since those are
          formats we do not write EXIF into, and so do the ones for photographs
          that were not repaired and the ones holding data with no tag to live
          in, such as the Google Photos view count. No file is deleted: one
          click undoes the move.
        </p>

        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={chooseDestination}
            className="rounded-lg border border-zinc-300 px-3 py-1.5 text-sm font-medium text-zinc-700 transition-colors hover:bg-zinc-100 dark:border-zinc-700 dark:text-zinc-200 dark:hover:bg-zinc-800"
          >
            {destination ? "Change destination" : "Choose where to move them"}
          </button>
          {destination ? (
            <span
              className="selectable truncate font-mono text-xs text-zinc-500 dark:text-zinc-400"
              title={destination}
            >
              {shortenPath(destination, 48)}
            </span>
          ) : null}
        </div>

        <button
          type="button"
          onClick={run}
          disabled={!destination || running}
          className="rounded-lg bg-zinc-900 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-zinc-700 disabled:cursor-not-allowed disabled:opacity-50 dark:bg-zinc-100 dark:text-zinc-900 dark:hover:bg-white"
        >
          {running ? "Working..." : "Move the JSON files already applied"}
        </button>

        {report ? (
          <div className="space-y-2 rounded-lg bg-zinc-50 p-3 text-sm dark:bg-zinc-800/50">
            <p className="text-zinc-900 dark:text-zinc-100">
              {restored
                ? `Put back ${formatCount(report.moved)} JSON files.`
                : `Moved ${formatCount(report.moved)} JSON files, ${formatBytes(report.bytesMoved)}.`}
            </p>

            {report.kept > 0 ? (
              <div className="space-y-1 text-xs text-zinc-500 dark:text-zinc-400">
                <p>
                  Left where they were: {formatCount(report.kept)}. The reasons:
                </p>
                <ul className="list-inside list-disc space-y-0.5">
                  {report.keptReasons.map(({ reason, count }) => (
                    <li key={reason}>
                      {SIDECAR_KEPT_LABELS[reason]}: {formatCount(count)}
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}

            {!restored && report.manifest ? (
              <div className="flex flex-wrap items-center gap-2 pt-1">
                <button
                  type="button"
                  onClick={undo}
                  disabled={running}
                  className="rounded-lg border border-zinc-300 px-3 py-1.5 text-xs font-medium text-zinc-700 transition-colors hover:bg-zinc-100 disabled:opacity-50 dark:border-zinc-700 dark:text-zinc-200 dark:hover:bg-zinc-800"
                >
                  Undo the move
                </button>
                <RevealButton path={report.destination} onError={onError} />
              </div>
            ) : null}

            {report.failures.length > 0 ? (
              <details className="text-xs">
                <summary className="cursor-pointer text-amber-700 dark:text-amber-400">
                  {formatCount(report.failures.length)} files not moved
                </summary>
                <ul className="selectable mt-1 space-y-0.5 font-mono text-zinc-500 dark:text-zinc-400">
                  {report.failures.slice(0, 20).map((failure) => (
                    <li key={failure}>{failure}</li>
                  ))}
                </ul>
              </details>
            ) : null}
          </div>
        ) : null}
      </div>
    </details>
  );
}
