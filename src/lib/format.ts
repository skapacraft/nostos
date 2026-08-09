// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

/** Shared formatting helpers, all in Italian locale and dependency-free. */

const NUMBER = new Intl.NumberFormat("it-IT");
const DATE = new Intl.DateTimeFormat("it-IT", {
  dateStyle: "medium",
  timeStyle: "short",
  timeZone: "UTC",
});

const UNITS = ["B", "kB", "MB", "GB", "TB"] as const;

/** Human-readable size in decimal units, as the system shows them. */
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
 * Dates arrive in UTC from the backend and are shown in UTC: converting them to
 * the local zone would falsify the capture time, which in EXIF has no zone.
 */
export function formatDate(iso: string | null): string {
  if (!iso) return "non disponibile";
  const parsed = new Date(iso);
  if (Number.isNaN(parsed.getTime())) return "non disponibile";
  return `${DATE.format(parsed)} UTC`;
}

/** Shortens a long path keeping the start and the end readable. */
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
