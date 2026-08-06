// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

/** Formattazioni condivise, tutte in locale italiano e senza dipendenze. */

const NUMBER = new Intl.NumberFormat("it-IT");
const DATE = new Intl.DateTimeFormat("it-IT", {
  dateStyle: "medium",
  timeStyle: "short",
  timeZone: "UTC",
});

const UNITS = ["B", "kB", "MB", "GB", "TB"] as const;

/** Dimensione leggibile in unità decimali, come le mostra il sistema. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";

  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < UNITS.length - 1) {
    value /= 1000;
    unit += 1;
  }

  const decimals = unit === 0 || value >= 100 ? 0 : 1;
  return `${value.toFixed(decimals)} ${UNITS[unit]}`;
}

export function formatCount(value: number): string {
  return NUMBER.format(value);
}

/**
 * Le date arrivano in UTC dal backend e vengono mostrate in UTC: convertirle al
 * fuso locale falserebbe l'ora di scatto, che nell'EXIF non ha fuso orario.
 */
export function formatDate(iso: string | null): string {
  if (!iso) return "non disponibile";
  const parsed = new Date(iso);
  if (Number.isNaN(parsed.getTime())) return "non disponibile";
  return `${DATE.format(parsed)} UTC`;
}

/** Accorcia un percorso lungo tenendo inizio e fine leggibili. */
export function shortenPath(path: string, maxLength = 64): string {
  if (path.length <= maxLength) return path;
  const head = path.slice(0, Math.floor(maxLength / 2) - 2);
  const tail = path.slice(-Math.floor(maxLength / 2) + 1);
  return `${head}...${tail}`;
}

export function percent(part: number, total: number): string {
  if (total <= 0) return "0%";
  return `${Math.round((part / total) * 100)}%`;
}
