// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

import { useEffect, useRef, useState } from "react";

import type { AppInfo } from "../types";

interface WelcomeProps {
  info: AppInfo | null;
  /** Receives `true` if the user asked not to see the introduction again. */
  onStart: (hideNextTime: boolean) => void;
  onOpenHelp: () => void;
}

const POINTS: { title: string; body: string }[] = [
  {
    title: "Rimette in ordine i tuoi export",
    body: "Ripristina le date delle foto, unisce gli archivi spezzati, deduplica contatti e calendari e trova i file ripetuti.",
  },
  {
    title: "Non manda niente da nessuna parte",
    body: "L'applicazione non apre connessioni di rete. Non è una promessa: è un controllo che fa fallire la compilazione se qualcuno prova a introdurne una.",
  },
  {
    title: "Non cancella niente",
    body: "Le modifiche predefinite scrivono copie altrove e lasciano intatti gli originali. Le operazioni che spostano file sono annullabili.",
  },
  {
    title: "Controlla lo spazio prima di partire",
    body: "Una libreria da centinaia di gigabyte non ci sta due volte sullo stesso disco. L'app se ne accorge prima di cominciare e propone come procedere lo stesso.",
  },
];

/**
 * The introduction shown at startup.
 *
 * The "do not show again" box is the only piece of data outliving the session,
 * and it ends up in a preferences file declared in section 6 of
 * PRIVACY_AUDIT.md. Nothing else is remembered.
 */
export function Welcome({ info, onStart, onOpenHelp }: WelcomeProps) {
  const [hideNextTime, setHideNextTime] = useState(false);
  const startRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    startRef.current?.focus();

    const onKeyDown = (event: KeyboardEvent) => {
      // Closing with Esc does not save the choice: it is a quick way out, not a
      // confirmation of what is in the box.
      if (event.key === "Escape") onStart(false);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onStart]);

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="welcome-title"
      className="fixed inset-0 z-50 flex items-center justify-center bg-zinc-900/40 p-6 backdrop-blur-sm dark:bg-zinc-950/60"
    >
      <div className="max-h-full w-full max-w-lg overflow-y-auto rounded-2xl border border-zinc-200 bg-white p-6 shadow-xl dark:border-zinc-800 dark:bg-zinc-900">
        <div className="flex items-center gap-3">
          <svg
            viewBox="0 0 24 24"
            aria-hidden="true"
            className="h-8 w-8 shrink-0 text-emerald-600 dark:text-emerald-400"
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
          <div>
            <h2
              id="welcome-title"
              className="text-lg font-semibold text-zinc-900 dark:text-zinc-100"
            >
              Benvenuto in Open Takeout Hub
            </h2>
            <p className="text-sm text-zinc-500 dark:text-zinc-400">
              I tuoi dati Google, elaborati sul tuo computer.
            </p>
          </div>
        </div>

        <ul className="mt-5 space-y-4">
          {POINTS.map((point) => (
            <li key={point.title} className="flex gap-3">
              <span
                aria-hidden="true"
                className="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-emerald-500"
              />
              <div>
                <p className="text-sm font-medium text-zinc-900 dark:text-zinc-100">
                  {point.title}
                </p>
                <p className="text-sm text-zinc-500 dark:text-zinc-400">
                  {point.body}
                </p>
              </div>
            </li>
          ))}
        </ul>

        <p className="mt-5 rounded-lg bg-zinc-50 p-3 text-sm text-zinc-600 dark:bg-zinc-800/50 dark:text-zinc-300">
          Per iniziare, trascina nella finestra la cartella{" "}
          <span className="font-mono">Takeout</span> o uno degli archivi{" "}
          <span className="font-mono">takeout-...zip</span> che hai scaricato da
          Google.
        </p>

        <label className="mt-5 flex cursor-pointer items-center gap-2.5 text-sm text-zinc-600 dark:text-zinc-300">
          <input
            type="checkbox"
            checked={hideNextTime}
            onChange={(event) => setHideNextTime(event.target.checked)}
          />
          <span>Non mostrare più questa presentazione</span>
        </label>

        <div className="mt-4 flex flex-wrap items-center justify-between gap-3">
          <button
            type="button"
            onClick={onOpenHelp}
            className="rounded-lg border border-zinc-300 px-4 py-2 text-sm font-medium text-zinc-700 transition-colors hover:bg-zinc-100 dark:border-zinc-700 dark:text-zinc-200 dark:hover:bg-zinc-800"
          >
            Apri la guida
          </button>
          <button
            ref={startRef}
            type="button"
            onClick={() => onStart(hideNextTime)}
            className="rounded-lg bg-zinc-900 px-5 py-2 text-sm font-medium text-white transition-colors hover:bg-zinc-700 dark:bg-zinc-100 dark:text-zinc-900 dark:hover:bg-white"
          >
            Inizia
          </button>
        </div>

        {info ? (
          <p className="mt-4 text-center text-xs text-zinc-400 dark:text-zinc-500">
            Versione {info.version} · {info.license} · {info.author}
          </p>
        ) : null}
      </div>
    </div>
  );
}
