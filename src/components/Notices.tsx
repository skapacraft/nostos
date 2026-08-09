// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

import { noticeDetail, noticeText } from "../lib/messages";
import type { Notice } from "../types";

interface NoticesProps {
  items: Notice[];
}

/**
 * Avvisi non bloccanti emessi dal backend.
 *
 * Esiste come componente unico perché il riquadro giallo compariva identico in
 * tre pannelli: tre copie da tenere allineate, e tre punti da ritoccare a ogni
 * lingua nuova.
 *
 * Il dettaglio tecnico, quando c'è, sta su una riga a parte e più smorta: è il
 * messaggio del sistema operativo, nella sua lingua, e non va confuso con la
 * frase rivolta all'utente.
 */
export function Notices({ items }: NoticesProps) {
  if (items.length === 0) return null;

  return (
    <ul className="space-y-2">
      {items.map((notice, index) => {
        const detail = noticeDetail(notice);
        return (
          <li
            key={`${notice.code}-${index}`}
            className="rounded-lg border border-amber-300 bg-amber-50 px-4 py-3 text-sm text-amber-900 dark:border-amber-800 dark:bg-amber-950/40 dark:text-amber-200"
          >
            {noticeText(notice)}
            {detail ? (
              <span className="selectable mt-1 block font-mono text-xs opacity-70">
                {detail}
              </span>
            ) : null}
          </li>
        );
      })}
    </ul>
  );
}
