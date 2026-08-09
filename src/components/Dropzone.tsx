// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

import { useCallback, useEffect, useState } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";

import { toMessage } from "../lib/api";

interface DropzoneProps {
  onSelect: (path: string) => void;
  onError: (message: string) => void;
  busy: boolean;
}

/**
 * Drop area for Takeout folders and archives.
 *
 * HTML5 drag and drop inside the webview does not expose the real file path,
 * only a browser handle: the paths come from the native Tauri event, which
 * covers the whole window. The HTML handlers exist only to prevent the
 * webview default behaviour.
 */
export function Dropzone({ onSelect, onError, busy }: DropzoneProps) {
  const [hovering, setHovering] = useState(false);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;

    getCurrentWebview()
      .onDragDropEvent((event) => {
        if (busy) return;

        switch (event.payload.type) {
          case "enter":
          case "over":
            setHovering(true);
            break;
          case "drop": {
            setHovering(false);
            const [first] = event.payload.paths;
            if (first) onSelect(first);
            break;
          }
          default:
            setHovering(false);
        }
      })
      .then((fn) => {
        // In StrictMode the effect is mounted twice: if the cleanup has already
        // run, a registration arriving late has to be cancelled at once.
        if (cancelled) {
          void fn();
          return;
        }
        unlisten = fn;
      })
      .catch((error) => onError(toMessage(error)));

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [busy, onSelect, onError]);

  const pickFolder = useCallback(async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Seleziona la cartella Takeout",
      });
      if (typeof selected === "string") onSelect(selected);
    } catch (error) {
      onError(toMessage(error));
    }
  }, [onSelect, onError]);

  const pickArchive = useCallback(async () => {
    try {
      const selected = await open({
        multiple: false,
        title: "Seleziona un archivio Takeout",
        filters: [{ name: "Archivio Takeout", extensions: ["zip"] }],
      });
      if (typeof selected === "string") onSelect(selected);
    } catch (error) {
      onError(toMessage(error));
    }
  }, [onSelect, onError]);

  return (
    <div
      onDragOver={(event) => event.preventDefault()}
      onDrop={(event) => event.preventDefault()}
      className={[
        "flex flex-col items-center justify-center gap-5 rounded-2xl border-2 border-dashed px-8 py-16 text-center transition-colors",
        hovering
          ? "border-emerald-500 bg-emerald-50 dark:border-emerald-400 dark:bg-emerald-950/30"
          : "border-zinc-300 bg-white dark:border-zinc-700 dark:bg-zinc-900",
        busy ? "opacity-60" : "",
      ].join(" ")}
    >
      <svg
        viewBox="0 0 24 24"
        aria-hidden="true"
        className={`h-12 w-12 ${
          hovering
            ? "text-emerald-600 dark:text-emerald-400"
            : "text-zinc-400 dark:text-zinc-600"
        }`}
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <path d="M3 7.5A1.5 1.5 0 0 1 4.5 6h4.2l1.8 2.4h9A1.5 1.5 0 0 1 21 9.9v8.6a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18.5z" />
        <path d="M12 11v6" />
        <path d="m9.5 13.5 2.5-2.5 2.5 2.5" />
      </svg>

      <div className="space-y-1">
        <p className="text-base font-medium text-zinc-900 dark:text-zinc-100">
          {busy
            ? "Analisi in corso..."
            : hovering
              ? "Rilascia per analizzare"
              : "Trascina qui la cartella Takeout o un archivio .zip"}
        </p>
        <p className="text-sm text-zinc-500 dark:text-zinc-400">
          I file restano sul tuo computer. Nulla viene caricato, copiato o
          inviato.
        </p>
      </div>

      <div className="flex flex-wrap items-center justify-center gap-3">
        <button
          type="button"
          onClick={pickFolder}
          disabled={busy}
          className="rounded-lg bg-zinc-900 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-zinc-700 disabled:cursor-not-allowed disabled:opacity-50 dark:bg-zinc-100 dark:text-zinc-900 dark:hover:bg-white"
        >
          Scegli cartella
        </button>
        <button
          type="button"
          onClick={pickArchive}
          disabled={busy}
          className="rounded-lg border border-zinc-300 px-4 py-2 text-sm font-medium text-zinc-700 transition-colors hover:bg-zinc-100 disabled:cursor-not-allowed disabled:opacity-50 dark:border-zinc-700 dark:text-zinc-200 dark:hover:bg-zinc-800"
        >
          Scegli archivio .zip
        </button>
      </div>
    </div>
  );
}
