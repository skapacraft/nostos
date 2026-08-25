// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

/**
 * The interface language, decided once from the device and held fixed for
 * the session.
 *
 * `navigator.language` is what the system webview already resolves from the
 * OS locale, so this asks for nothing extra: no Tauri plugin, no permission
 * dialog. Only English and Italian have translations; anything else falls
 * back to English rather than mixing an unfinished language into the UI.
 *
 * Read once and cached: the OS locale does not change while the app is
 * running, and re-reading it on every call would just be the same string
 * parsed again.
 */

export type Locale = "en" | "it";

let cached: Locale | undefined;

export function locale(): Locale {
  if (cached === undefined) {
    cached = navigator.language.toLowerCase().startsWith("it") ? "it" : "en";
  }
  return cached;
}
