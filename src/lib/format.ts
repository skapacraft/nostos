// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

/**
 * Shared formatting helpers, dependency-free.
 *
 * The locale is fixed to the one the interface is written in rather than taken
 * from the system: an English sentence with an Italian month inside it reads as
 * a bug, and the machine's regional settings say nothing about which language
 * this application speaks.
 */

const NUMBER = new Intl.NumberFormat("en-GB");
const DATE = new Intl.DateTimeFormat("en-GB", {
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

/**
 * A span of days in words, for the age of the running build.
 *
 * Rounded on purpose: the point of the sentence is whether the copy is recent
 * or has been sitting there a while, and "7 months" carries that better than
 * "213 days".
 */
export function formatAge(days: number): string {
  if (!Number.isFinite(days) || days < 1) return "less than a day";
  if (days === 1) return "1 day";
  if (days < 60) return `${days} days`;

  const months = Math.round(days / 30.44);
  if (months < 18) return `${months} months`;

  const years = Math.floor(days / 365.25);
  const rest = Math.round((days - years * 365.25) / 30.44);
  const yearPart = years === 1 ? "1 year" : `${years} years`;
  if (rest < 1) return yearPart;
  return `${yearPart} and ${rest === 1 ? "1 month" : `${rest} months`}`;
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
