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
  /** Cartella riparata, dove si trovano i sidecar da valutare. */
  path: string;
  onError: (message: string) => void;
}

/**
 * Sposta i sidecar JSON il cui contenuto è ormai dentro alle foto.
 *
 * Compare solo dopo una riscrittura degli originali, che è l'unico caso in cui
 * quei JSON siano davvero di troppo: nella copia riparata il sidecar viene
 * conservato apposta accanto ai formati in cui non scriviamo l'EXIF.
 *
 * Non è un pulsante di pulizia e non va presentato come tale. Sposta, scrive un
 * registro e si annulla: cancellare avrebbe voluto dire buttare via i dati che
 * non hanno una sede nei tag EXIF, cioè fare al contrario proprio il danno che
 * questa applicazione ripara.
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
        title: "Dove spostare i sidecar già applicati",
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
      const esito = await api.restoreQuarantine(report.manifest);
      setRestored(true);
      if (esito.failures.length > 0) onError(esito.failures[0]);
    } catch (error) {
      onError(toMessage(error));
    } finally {
      setRunning(false);
    }
  };

  return (
    <details className="rounded-lg border border-zinc-200 p-3 dark:border-zinc-700">
      <summary className="cursor-pointer text-sm font-medium text-zinc-900 dark:text-zinc-100">
        Mettere da parte i file JSON ora inutili
      </summary>

      <div className="mt-3 space-y-3 text-sm text-zinc-600 dark:text-zinc-300">
        <p>
          Ogni foto riparata porta ora dentro di sé quello che stava nel suo file{" "}
          <span className="font-mono">.json</span>: data, coordinate,
          descrizione, volti riconosciuti e la stella dei preferiti. Quei JSON
          si possono spostare altrove per lasciare pulita la cartella.
        </p>
        <p className="rounded-lg bg-zinc-50 p-3 text-xs dark:bg-zinc-800/50">
          Vengono spostati solo i JSON che non sono più l'unica copia di
          qualcosa, verificando dentro a ogni file che il dato ci sia davvero.
          Restano dove sono quelli di PNG, GIF e video, formati in cui non
          scriviamo l'EXIF, quelli delle foto non riparate, e quelli che
          contengono dati senza una sede nei tag, come il conteggio delle
          visualizzazioni di Google Foto. Nessun file viene eliminato: lo
          spostamento si annulla con un clic.
        </p>

        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={chooseDestination}
            className="rounded-lg border border-zinc-300 px-3 py-1.5 text-sm font-medium text-zinc-700 transition-colors hover:bg-zinc-100 dark:border-zinc-700 dark:text-zinc-200 dark:hover:bg-zinc-800"
          >
            {destination ? "Cambia destinazione" : "Scegli dove spostarli"}
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
          {running ? "Elaborazione..." : "Sposta i JSON già applicati"}
        </button>

        {report ? (
          <div className="space-y-2 rounded-lg bg-zinc-50 p-3 text-sm dark:bg-zinc-800/50">
            <p className="text-zinc-900 dark:text-zinc-100">
              {restored
                ? `Rimessi al loro posto ${formatCount(report.moved)} file JSON.`
                : `Spostati ${formatCount(report.moved)} file JSON, ${formatBytes(report.bytesMoved)}.`}
            </p>

            {report.kept > 0 ? (
              <div className="space-y-1 text-xs text-zinc-500 dark:text-zinc-400">
                <p>
                  Lasciati dov'erano: {formatCount(report.kept)}. I motivi:
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
                  Annulla lo spostamento
                </button>
                <RevealButton path={report.destination} onError={onError} />
              </div>
            ) : null}

            {report.failures.length > 0 ? (
              <details className="text-xs">
                <summary className="cursor-pointer text-amber-700 dark:text-amber-400">
                  {formatCount(report.failures.length)} file non spostati
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
