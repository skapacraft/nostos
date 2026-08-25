// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

import { useEffect, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";

import * as api from "../lib/api";
import { toMessage } from "../lib/api";
import { locale } from "../lib/locale";
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
  const it = locale() === "it";
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

  const subject = it
    ? `Nostos ${info?.version ?? ""} su ${environment}${
        redacted.length > 0 ? ` (${redacted.length} errori)` : ""
      }`
    : `Nostos ${info?.version ?? ""} on ${environment}${
        redacted.length > 0 ? ` (${redacted.length} errors)` : ""
      }`;

  const report = it
    ? [
        "## Cosa è successo",
        "",
        description.trim() ||
          "(descrivi cosa stavi facendo e cosa ti aspettavi)",
        "",
        "## Ambiente",
        "",
        `- Versione: ${info?.version ?? "sconosciuta"}`,
        `- Sistema: ${environment || "sconosciuto"}`,
        "",
        "## Errori visti in questa sessione",
        "",
        redacted.length > 0
          ? redacted.map((e) => `- ${e}`).join("\n")
          : "- nessuno registrato",
      ].join("\n")
    : [
        "## What happened",
        "",
        description.trim() ||
          "(describe what you were doing and what you expected)",
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
        title: it ? "Dove salvare il rapporto" : "Where to save the report",
        defaultPath: "nostos-report.md",
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
        {it
          ? "L'applicazione non invia nulla da sola, nemmeno una segnalazione. Prepara il testo, tu lo leggi e decidi se inviarlo."
          : "The application sends nothing on its own, not even a bug report. It prepares the text, you read it, and you decide whether to send it."}
      </p>

      <label className="block space-y-1">
        <span className="text-xs font-medium text-zinc-500 dark:text-zinc-400">
          {it
            ? "Cosa stavi facendo, e cosa ti aspettavi?"
            : "What were you doing, and what did you expect?"}
        </span>
        <textarea
          value={description}
          onChange={(event) => setDescription(event.target.value)}
          rows={3}
          placeholder={
            it
              ? "Ho trascinato la cartella Takeout e..."
              : "I dragged the Takeout folder in and..."
          }
          className="w-full rounded-lg border border-zinc-300 bg-white p-2 text-sm text-zinc-900 dark:border-zinc-700 dark:bg-zinc-900 dark:text-zinc-100"
        />
      </label>

      <details open>
        <summary className="cursor-pointer text-xs font-medium text-zinc-500 dark:text-zinc-400">
          {it
            ? "Questo è esattamente ciò che verrebbe inviato"
            : "This is exactly what would be sent"}
        </summary>
        <pre className="selectable mt-2 max-h-56 overflow-auto whitespace-pre-wrap rounded-lg bg-zinc-100 p-3 font-mono text-xs text-zinc-700 dark:bg-zinc-800 dark:text-zinc-300">
          {report}
        </pre>
      </details>

      <p className="rounded-lg bg-zinc-50 p-3 text-xs text-zinc-600 dark:bg-zinc-800/50 dark:text-zinc-400">
        {it ? (
          <>
            I percorsi sono abbreviati: la tua cartella utente diventa{" "}
            <span className="font-mono">~</span>. I nomi delle cartelle che
            hai scelto restano invece invariati, ed è per questo che il testo
            resta qui sopra invece di partire in silenzio: leggilo, e togli
            ciò che preferisci non inviare.
          </>
        ) : (
          <>
            Paths are shortened: your home folder becomes{" "}
            <span className="font-mono">~</span>. The names of the folders you
            chose do remain, which is why the text sits above rather than
            leaving quietly: read it, and take out anything you would rather
            not send.
          </>
        )}
      </p>

      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          onClick={sendByEmail}
          className="rounded-lg bg-zinc-900 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-zinc-700 dark:bg-zinc-100 dark:text-zinc-900 dark:hover:bg-white"
        >
          {it ? "Apri l'email, già compilata" : "Open the email, already filled in"}
        </button>
        <button
          type="button"
          onClick={saveToFile}
          className="rounded-lg border border-zinc-300 px-4 py-2 text-sm font-medium text-zinc-700 transition-colors hover:bg-zinc-100 dark:border-zinc-700 dark:text-zinc-200 dark:hover:bg-zinc-800"
        >
          {it ? "Salva come file" : "Save as a file"}
        </button>
      </div>

      <p className="text-xs text-zinc-500 dark:text-zinc-400">
        {it ? (
          <>
            L'indirizzo è{" "}
            <span className="selectable font-mono">support@skapacraft.com</span>
            . Se preferisci GitHub, il progetto si trova su{" "}
            <span className="selectable font-mono break-all">
              {info?.repository ?? "github.com/skapacraft/nostos"}
            </span>
            , e per un problema di sicurezza esiste la segnalazione privata
            invece di una issue pubblica.
          </>
        ) : (
          <>
            The address is{" "}
            <span className="selectable font-mono">support@skapacraft.com</span>
            . If you prefer GitHub, the project lives at{" "}
            <span className="selectable font-mono break-all">
              {info?.repository ?? "github.com/skapacraft/nostos"}
            </span>
            , and for a security problem there is private reporting instead of
            a public issue.
          </>
        )}
      </p>
    </div>
  );
}
