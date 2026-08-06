// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

import { formatCount } from "../lib/format";
import type { Progress } from "../types";

const PHASE_LABELS: Record<Progress["phase"], string> = {
  scanning: "Scansione",
  extracting: "Estrazione",
  writing: "Scrittura",
  done: "Completato",
};

interface ProgressBarProps {
  progress: Progress | null;
  label: string;
}

/**
 * Barra di avanzamento alimentata dagli eventi del backend.
 *
 * L'animazione è affidata a una transizione CSS sulla larghezza: il compositor
 * la gestisce fuori dal thread principale, quindi la finestra resta fluida
 * anche mentre React ridisegna i contatori.
 */
export function ProgressBar({ progress, label }: ProgressBarProps) {
  const total = progress?.total ?? 0;
  const done = progress?.done ?? 0;
  const ratio = total > 0 ? Math.min(done / total, 1) : 0;
  const indeterminate = !progress || total === 0;

  return (
    <div className="space-y-1.5">
      <div className="flex items-baseline justify-between gap-3 text-xs">
        <span className="text-zinc-600 dark:text-zinc-300">
          {progress ? PHASE_LABELS[progress.phase] : "Avvio..."}
          {progress?.current ? (
            <span className="ml-2 truncate font-mono text-zinc-400 dark:text-zinc-500">
              {progress.current}
            </span>
          ) : null}
        </span>
        <span className="shrink-0 tabular-nums text-zinc-500 dark:text-zinc-400">
          {indeterminate
            ? label
            : `${formatCount(done)} / ${formatCount(total)} ${label.toLowerCase()}`}
        </span>
      </div>

      <div
        role="progressbar"
        aria-valuenow={indeterminate ? undefined : done}
        aria-valuemin={0}
        aria-valuemax={total}
        aria-label={label}
        className="h-2 w-full overflow-hidden rounded-full bg-zinc-200 dark:bg-zinc-700"
      >
        <div
          className="h-full rounded-full bg-emerald-500 transition-[width] duration-150 ease-out"
          style={{ width: `${ratio * 100}%` }}
        />
      </div>

      {progress && progress.errors > 0 ? (
        <p className="text-xs text-amber-600 dark:text-amber-400">
          {formatCount(progress.errors)} file con problemi
        </p>
      ) : null}
    </div>
  );
}
