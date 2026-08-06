// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

import { formatBytes, formatCount, shortenPath } from "../lib/format";
import type { SectionSummary, SourceSummary, TakeoutSectionId } from "../types";
import { Stat } from "./Stat";

/** Sezioni per cui esiste un analizzatore dedicato. */
const ANALYZABLE: TakeoutSectionId[] = [
  "googlePhotos",
  "contacts",
  "drive",
  "calendar",
];

export function isAnalyzable(section: TakeoutSectionId): boolean {
  return ANALYZABLE.includes(section);
}

interface SourcePanelProps {
  summary: SourceSummary;
  activeSection: string | null;
  busy: boolean;
  onAnalyze: (section: SectionSummary) => void;
  onClose: () => void;
}

export function SourcePanel({
  summary,
  activeSection,
  busy,
  onAnalyze,
  onClose,
}: SourcePanelProps) {
  const isArchive = summary.kind === "archive";

  return (
    <section className="space-y-6">
      <header className="flex flex-wrap items-start justify-between gap-4">
        <div className="min-w-0">
          <h2 className="truncate text-lg font-semibold text-zinc-900 dark:text-zinc-100">
            {summary.displayName}
          </h2>
          <p
            className="selectable mt-1 truncate font-mono text-xs text-zinc-500 dark:text-zinc-400"
            title={summary.root}
          >
            {shortenPath(summary.root, 96)}
          </p>
        </div>
        <button
          type="button"
          onClick={onClose}
          disabled={busy}
          className="shrink-0 rounded-lg border border-zinc-300 px-3 py-1.5 text-sm font-medium text-zinc-700 transition-colors hover:bg-zinc-100 disabled:opacity-50 dark:border-zinc-700 dark:text-zinc-200 dark:hover:bg-zinc-800"
        >
          Chiudi
        </button>
      </header>

      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
        <Stat
          label="Tipo"
          value={isArchive ? "Archivio .zip" : "Cartella"}
          hint={isArchive ? "non estratto" : undefined}
        />
        <Stat label="File" value={formatCount(summary.fileCount)} />
        <Stat
          label="Dimensione"
          value={formatBytes(summary.totalBytes)}
          hint={isArchive ? "decompressa" : undefined}
        />
      </div>

      {summary.warnings.length > 0 ? (
        <ul className="space-y-2">
          {summary.warnings.map((warning) => (
            <li
              key={warning}
              className="rounded-lg border border-amber-300 bg-amber-50 px-4 py-3 text-sm text-amber-900 dark:border-amber-800 dark:bg-amber-950/40 dark:text-amber-200"
            >
              {warning}
            </li>
          ))}
        </ul>
      ) : null}

      <div className="space-y-3">
        <h3 className="text-sm font-semibold uppercase tracking-wide text-zinc-500 dark:text-zinc-400">
          Sezioni trovate
        </h3>

        {summary.sections.length === 0 ? (
          <p className="text-sm text-zinc-500 dark:text-zinc-400">
            Nessuna sezione riconosciuta in questa sorgente.
          </p>
        ) : (
          <ul className="divide-y divide-zinc-200 overflow-hidden rounded-xl border border-zinc-200 dark:divide-zinc-800 dark:border-zinc-800">
            {summary.sections.map((section) => {
              const analyzable = !isArchive && isAnalyzable(section.section);
              const isActive = activeSection === section.path;

              return (
                <li
                  key={section.path}
                  className="flex flex-wrap items-center justify-between gap-3 bg-white px-4 py-3 dark:bg-zinc-900"
                >
                  <div className="min-w-0">
                    <p className="truncate text-sm font-medium text-zinc-900 dark:text-zinc-100">
                      {section.dirName}
                    </p>
                    <p className="text-xs text-zinc-500 dark:text-zinc-400">
                      {section.fileCount > 0
                        ? `${formatCount(section.fileCount)} file, ${formatBytes(section.totalBytes)}`
                        : section.label}
                    </p>
                  </div>

                  {analyzable ? (
                    <button
                      type="button"
                      onClick={() => onAnalyze(section)}
                      disabled={busy}
                      className={[
                        "shrink-0 rounded-lg px-3 py-1.5 text-sm font-medium transition-colors disabled:opacity-50",
                        isActive
                          ? "bg-emerald-600 text-white hover:bg-emerald-500"
                          : "bg-zinc-900 text-white hover:bg-zinc-700 dark:bg-zinc-100 dark:text-zinc-900 dark:hover:bg-white",
                      ].join(" ")}
                    >
                      {isActive ? "Aggiorna" : "Analizza"}
                    </button>
                  ) : (
                    <span className="shrink-0 text-xs text-zinc-400 dark:text-zinc-500">
                      {isArchive ? "estrai prima" : "analizzatore non disponibile"}
                    </span>
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </section>
  );
}
