// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

import { noticeDetail, noticeText } from "../lib/messages";
import type { Notice } from "../types";

interface NoticesProps {
  items: Notice[];
}

/**
 * Non-blocking notices emitted by the backend.
 *
 * It exists as a single component because the amber box appeared identically
 * in three panels: three copies to keep aligned, and three places to touch up
 * for every new language.
 *
 * The technical detail, when there is one, sits on a separate and dimmer line:
 * it is the operating system message, in its own language, and must not be
 * confused with the sentence addressed to the user.
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
