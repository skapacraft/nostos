// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

import { useEffect, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";

import * as api from "../lib/api";
import { toMessage } from "../lib/api";
import type { AppInfo } from "../types";

interface ProblemReportProps {
  info: AppInfo | null;
  /** Errors seen this session, oldest first, already bounded by `App`. */
  errors: string[];
  onError: (message: string) => void;
}

/**
 * Reporting a problem, from an application that cannot send anything.
 *
 * There is no API to post to and no browser to open: `tauri-plugin-http` and
 * `tauri-plugin-opener` are both banned in `deny.toml`. That is not an obstacle
 * to work around, it is the reason this screen looks the way it does.
 *
 * So the application prepares the report and the person carries it. The text is
 * shown in full before anything happens, because a report about privacy
 * software that the sender cannot read first would be a poor joke.
 *
 * Paths are redacted by the backend, which replaces the home directory with
 * `~`: the shape of a path is what helps diagnose, the account name in it is
 * not. Folder names the user chose are left alone, and that is precisely why
 * the text is on screen to be read and edited before sending.
 */
export function ProblemReport({ info, errors, onError }: ProblemReportProps) {
  const [environment, setEnvironment] = useState("");
  const [redacted, setRedacted] = useState<string[]>([]);
  const [description, setDescription] = useState("");

  useEffect(() => {
    api.reportEnvironment().then(setEnvironment).catch(() => setEnvironment(""));
  }, []);

  useEffect(() => {
    if (errors.length === 0) {
      setRedacted([]);
      return;
    }
    api.redactHome(errors).then(setRedacted).catch(() => setRedacted(errors));
  }, [errors]);

  const subject = `Nostos ${info?.version ?? ""} on ${environment}${
    redacted.length > 0 ? ` (${redacted.length} errors)` : ""
  }`;

  const report = [
    "## What happened",
    "",
    description.trim() || "(describe what you were doing and what you expected)",
    "",
    "## Environment",
    "",
    `- Version: ${info?.version ?? "unknown"}`,
    `- System: ${environment || "unknown"}`,
    "",
    "## Errors seen this session",
    "",
    redacted.length > 0
      ? redacted.map((e) => `- ${e}`).join("\n")
      : "- none recorded",
  ].join("\n");

  const sendByEmail = async () => {
    try {
      await api.composeSupportEmail(subject, report);
    } catch (error) {
      onError(toMessage(error));
    }
  };

  const saveToFile = async () => {
    try {
      const target = await save({
        title: "Dove salvare il rapporto",
        defaultPath: "nostos-segnalazione.md",
        filters: [{ name: "Markdown", extensions: ["md"] }],
      });
      if (typeof target === "string") {
        await api.saveTextFile(target, report);
      }
    } catch (error) {
      onError(toMessage(error));
    }
  };

  return (
    <div className="space-y-3 text-sm text-zinc-700 dark:text-zinc-300">
      <p>
        L'applicazione non invia niente da sola, nemmeno una segnalazione.
        Prepara il testo, tu lo leggi, e decidi se mandarlo.
      </p>

      <label className="block space-y-1">
        <span className="text-xs font-medium text-zinc-500 dark:text-zinc-400">
          Che cosa stavi facendo, e che cosa ti aspettavi?
        </span>
        <textarea
          value={description}
          onChange={(event) => setDescription(event.target.value)}
          rows={3}
          placeholder="Ho trascinato la cartella Takeout e..."
          className="w-full rounded-lg border border-zinc-300 bg-white p-2 text-sm text-zinc-900 dark:border-zinc-700 dark:bg-zinc-900 dark:text-zinc-100"
        />
      </label>

      <details open>
        <summary className="cursor-pointer text-xs font-medium text-zinc-500 dark:text-zinc-400">
          Questo è esattamente ciò che verrebbe mandato
        </summary>
        <pre className="selectable mt-2 max-h-56 overflow-auto whitespace-pre-wrap rounded-lg bg-zinc-100 p-3 font-mono text-xs text-zinc-700 dark:bg-zinc-800 dark:text-zinc-300">
          {report}
        </pre>
      </details>

      <p className="rounded-lg bg-zinc-50 p-3 text-xs text-zinc-600 dark:bg-zinc-800/50 dark:text-zinc-400">
        I percorsi sono accorciati: la tua cartella personale diventa{" "}
        <span className="font-mono">~</span>. Restano però i nomi delle cartelle
        che hai scelto tu, ed è il motivo per cui il testo sta qui sopra invece
        di partire di nascosto: leggilo, e togli quello che non vuoi mandare.
      </p>

      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          onClick={sendByEmail}
          className="rounded-lg bg-zinc-900 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-zinc-700 dark:bg-zinc-100 dark:text-zinc-900 dark:hover:bg-white"
        >
          Apri l'email già compilata
        </button>
        <button
          type="button"
          onClick={saveToFile}
          className="rounded-lg border border-zinc-300 px-4 py-2 text-sm font-medium text-zinc-700 transition-colors hover:bg-zinc-100 dark:border-zinc-700 dark:text-zinc-200 dark:hover:bg-zinc-800"
        >
          Salva come file
        </button>
      </div>

      <p className="text-xs text-zinc-500 dark:text-zinc-400">
        L'indirizzo è{" "}
        <span className="selectable font-mono">support@skapacraft.com</span>. Se
        preferisci GitHub, il progetto sta su{" "}
        <span className="selectable font-mono break-all">
          {info?.repository ?? "github.com/skapacraft/nostos"}
        </span>
        , e per un problema di sicurezza c'è la segnalazione riservata invece di
        una issue pubblica.
      </p>
    </div>
  );
}
