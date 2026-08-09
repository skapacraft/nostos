// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

import { useCallback } from "react";

import * as api from "../lib/api";
import { toMessage } from "../lib/api";

interface RevealButtonProps {
  /** Percorso da rivelare. Deve essere uno prodotto dall'applicazione. */
  path: string;
  onError: (message: string) => void;
  label?: string;
}

/**
 * Mostra un percorso nel gestore file del sistema.
 *
 * Non apre il file e non apre indirizzi: il backend invoca un programma fisso
 * su un percorso che deve già esistere. È l'unica azione dell'app che esce
 * verso il sistema operativo, e resta ben distinta dall'apertura di URL, che
 * qui non esiste.
 */
export function RevealButton({
  path,
  onError,
  label = "Mostra nel Finder",
}: RevealButtonProps) {
  const reveal = useCallback(async () => {
    try {
      await api.revealInFileManager(path);
    } catch (error) {
      onError(toMessage(error));
    }
  }, [path, onError]);

  return (
    <button
      type="button"
      onClick={reveal}
      className="inline-flex items-center gap-1.5 rounded-lg border border-zinc-300 px-3 py-1.5 text-sm font-medium text-zinc-700 transition-colors hover:bg-zinc-100 dark:border-zinc-700 dark:text-zinc-200 dark:hover:bg-zinc-800"
    >
      <svg
        viewBox="0 0 24 24"
        aria-hidden="true"
        className="h-4 w-4"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <path d="M3 7.5A1.5 1.5 0 0 1 4.5 6h4.2l1.8 2.4h9A1.5 1.5 0 0 1 21 9.9v8.6a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18.5z" />
      </svg>
      {label}
    </button>
  );
}
