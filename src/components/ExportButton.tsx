// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

import { useCallback, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";

import { toMessage } from "../lib/api";
import { formatBytes, formatCount, shortenPath } from "../lib/format";
import type { ExportReport } from "../types";

interface ExportButtonProps {
  label: string;
  /** Nome proposto nella finestra di salvataggio. */
  defaultName: string;
  extension: string;
  filterName: string;
  /** Descrizione di cosa contiene il file prodotto. */
  hint: string;
  onExport: (destination: string) => Promise<ExportReport>;
  onError: (message: string) => void;
}

/**
 * Salvataggio di un file esportato.
 *
 * Il percorso arriva sempre dalla finestra di sistema: l'applicazione non
 * sceglie mai da sé dove scrivere, e non ha permessi per farlo.
 */
export function ExportButton({
  label,
  defaultName,
  extension,
  filterName,
  hint,
  onExport,
  onError,
}: ExportButtonProps) {
  const [report, setReport] = useState<ExportReport | null>(null);
  const [busy, setBusy] = useState(false);

  const run = useCallback(async () => {
    try {
      const destination = await save({
        title: label,
        defaultPath: defaultName,
        filters: [{ name: filterName, extensions: [extension] }],
      });
      if (typeof destination !== "string") return;

      setBusy(true);
      setReport(await onExport(destination));
    } catch (error) {
      onError(toMessage(error));
    } finally {
      setBusy(false);
    }
  }, [label, defaultName, extension, filterName, onExport, onError]);

  return (
    <div className="space-y-2 rounded-xl border border-zinc-200 bg-white p-4 dark:border-zinc-800 dark:bg-zinc-900">
      <p className="text-sm text-zinc-600 dark:text-zinc-300">{hint}</p>

      <button
        type="button"
        onClick={run}
        disabled={busy}
        className="rounded-lg bg-zinc-900 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-zinc-700 disabled:cursor-not-allowed disabled:opacity-50 dark:bg-zinc-100 dark:text-zinc-900 dark:hover:bg-white"
      >
        {busy ? "Scrittura..." : label}
      </button>

      {report ? (
        <div className="text-sm">
          <p className="text-zinc-900 dark:text-zinc-100">
            Scritti {formatCount(report.written)} elementi,{" "}
            {formatBytes(report.bytes)}.
          </p>
          <p
            className="selectable truncate font-mono text-xs text-zinc-500 dark:text-zinc-400"
            title={report.path}
          >
            {shortenPath(report.path, 72)}
          </p>
        </div>
      ) : null}
    </div>
  );
}
