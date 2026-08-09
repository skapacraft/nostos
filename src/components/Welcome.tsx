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
    title: "Puts your export back in order",
    body: "Restores the dates of your photos, merges split archives, deduplicates contacts and calendars, and finds the repeated files.",
  },
  {
    title: "Sends nothing anywhere",
    body: "The application opens no network connections. That is not a promise: it is a check that fails the build if anyone tries to add one.",
  },
  {
    title: "Deletes nothing",
    body: "The default changes write copies elsewhere and leave your originals untouched. Anything that moves files can be undone.",
  },
  {
    title: "Checks the room before starting",
    body: "A library of several hundred gigabytes does not fit twice on the same disk. The application works this out before starting, and offers a way to proceed anyway.",
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
            {/* The mark from the application icon: a photo that lost its date. */}
            <rect x="2.5" y="4.5" width="14" height="11.5" rx="2" />
            <path d="M2.5 12.5l3.5-3.5 4 4" />
            <circle cx="12.5" cy="8.5" r="1.2" />
            <circle cx="16" cy="16" r="5" fill="currentColor" fillOpacity="0" />
            <path d="M16 13v3l2 1.5" />
          </svg>
          <div>
            <h2
              id="welcome-title"
              className="text-lg font-semibold text-zinc-900 dark:text-zinc-100"
            >
              Welcome to Nostos
            </h2>
            <p className="text-sm text-zinc-500 dark:text-zinc-400">
              Your Google data, processed on your own computer.
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
          To begin, drag the <span className="font-mono">Takeout</span> folder
          into this window, or one of the{" "}
          <span className="font-mono">takeout-....zip</span> archives you
          downloaded from Google.
        </p>

        <label className="mt-5 flex cursor-pointer items-center gap-2.5 text-sm text-zinc-600 dark:text-zinc-300">
          <input
            type="checkbox"
            checked={hideNextTime}
            onChange={(event) => setHideNextTime(event.target.checked)}
          />
          <span>Do not show this introduction again</span>
        </label>

        <div className="mt-4 flex flex-wrap items-center justify-between gap-3">
          <button
            type="button"
            onClick={onOpenHelp}
            className="rounded-lg border border-zinc-300 px-4 py-2 text-sm font-medium text-zinc-700 transition-colors hover:bg-zinc-100 dark:border-zinc-700 dark:text-zinc-200 dark:hover:bg-zinc-800"
          >
            Open the guide
          </button>
          <button
            ref={startRef}
            type="button"
            onClick={() => onStart(hideNextTime)}
            className="rounded-lg bg-zinc-900 px-5 py-2 text-sm font-medium text-white transition-colors hover:bg-zinc-700 dark:bg-zinc-100 dark:text-zinc-900 dark:hover:bg-white"
          >
            Start
          </button>
        </div>

        {info ? (
          <p className="mt-4 text-center text-xs text-zinc-400 dark:text-zinc-500">
            Version {info.version} · {info.license} · {info.author}
          </p>
        ) : null}
      </div>
    </div>
  );
}
