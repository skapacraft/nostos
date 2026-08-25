// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

/**
 * The only point of contact between frontend and backend.
 *
 * Everything goes through Tauri's local IPC channel: there is no HTTP client in
 * the frontend and there must not be one. Any `fetch` added here would be
 * blocked by the CSP anyway (`connect-src` limited to `ipc:`).
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { locale } from "./locale";
import { errorDetail, errorText } from "./messages";
import type {
  AlbumIndex,
  AppInfo,
  ArchiveEntry,
  ArchiveSeries,
  ArchiveSummary,
  CalendarReport,
  CleanOptions,
  CleanPlan,
  CleanReport,
  ContactsReport,
  DriveReport,
  ErrorPayload,
  ExportReport,
  ExtractReport,
  PhotoScanReport,
  Preferences,
  PrivacyReport,
  Progress,
  RepairReport,
  RestoreReport,
  SidecarSweepReport,
  SourceSummary,
  SpaceEstimate,
  WriteOptions,
} from "../types";

/** Progress event emitted by the long-running commands. */
const PROGRESS_EVENT = "takeout://progress";

/** Event with which the "Report a problem" menu item asks for the report. */
const SHOW_REPORT_EVENT = "takeout://mostra-segnalazione";

/** Listens for the request to open the problem report from the menu. */
export function onShowReport(handler: () => void): Promise<UnlistenFn> {
  return listen(SHOW_REPORT_EVENT, () => handler());
}

/** Event with which the "Guide" menu item asks for the guide to be shown. */
const SHOW_HELP_EVENT = "takeout://mostra-guida";

/** Listens for the request to open the guide from the menu. */
export function onShowHelp(handler: () => void): Promise<UnlistenFn> {
  return listen(SHOW_HELP_EVENT, () => handler());
}

/** Event with which the "Version and updates" menu item asks for the panel. */
const SHOW_VERSION_EVENT = "takeout://mostra-versione";

/** Listens for the request to open version and updates from the menu. */
export function onShowVersion(handler: () => void): Promise<UnlistenFn> {
  return listen(SHOW_VERSION_EVENT, () => handler());
}

/**
 * Listens for progress.
 *
 * The backend already caps the rate at one event every 80 ms, so the handler
 * can update React state directly with no further defences.
 */
export function onProgress(
  handler: (progress: Progress) => void,
): Promise<UnlistenFn> {
  return listen<Progress>(PROGRESS_EVENT, (event) => handler(event.payload));
}

/** Loads a Takeout folder or a `takeout-*.zip` archive. */
export const loadSource = (path: string) =>
  invoke<SourceSummary>("load_source", { path });

/** Summary of the source already loaded in the session. */
export const currentSource = () => invoke<SourceSummary>("current_source");

/** Forgets the current source. */
export const closeSource = () => invoke<void>("close_source");

/** Inspects an archive without extracting it. */
export const inspectArchive = (path: string) =>
  invoke<ArchiveSummary>("inspect_archive", { path });

/** First entries of an archive, for the preview. */
export const listArchiveEntries = (path: string, limit?: number) =>
  invoke<ArchiveEntry[]>("list_archive_entries", { path, limit });

/** Extracts an archive into the folder chosen by the user. */
export const extractArchive = (path: string, destination: string) =>
  invoke<ExtractReport>("extract_archive", { path, destination });

/** Finds every archive making up the same export. */
export const discoverArchiveSeries = (path: string) =>
  invoke<ArchiveSeries>("discover_archive_series", { path });

/** Extracts the whole series of archives into one tree, with progress. */
export const extractTakeout = (path: string, destination: string) =>
  invoke<ExtractReport>("extract_takeout", { path, destination });

/** Analyses the Google Photos media. Without `path` it uses the whole source. */
export const scanPhotos = (path?: string) =>
  invoke<PhotoScanReport>("scan_photos", { path });

/** Repairs date and coordinates of the media per the options given. */
export const repairPhotos = (path: string, options: WriteOptions) =>
  invoke<RepairReport>("repair_photos", { path, options });

/** Analyses the Contacts export. */
export const scanContacts = (path?: string) =>
  invoke<ContactsReport>("scan_contacts", { path });

/** Reconstructs albums, year folders and edited versions. */
export const scanAlbums = (path: string) =>
  invoke<AlbumIndex>("scan_albums", { path });

/** Writes the album manifest, to be done before deduplicating. */
export const exportAlbumManifest = (path: string, destination: string) =>
  invoke<ExportReport>("export_album_manifest", { path, destination });

/** Analyses the Calendar export. */
export const scanCalendar = (path?: string) =>
  invoke<CalendarReport>("scan_calendar", { path });

/** Writes a clean, deduplicated vCard 3.0. */
export const exportContacts = (path: string, destination: string) =>
  invoke<ExportReport>("export_contacts", { path, destination });

/** Writes a clean, deduplicated iCalendar 2.0. */
export const exportCalendar = (path: string, destination: string) =>
  invoke<ExportReport>("export_calendar", { path, destination });

/** Analyses the Drive export. */
export const scanDrive = (path?: string) =>
  invoke<DriveReport>("scan_drive", { path });

/** Computes the cleanup plan without touching anything. */
export const planDriveClean = (path: string, options: CleanOptions) =>
  invoke<CleanPlan>("plan_drive_clean", { path, options });

/** Performs the cleanup: a clean tree elsewhere, or reversible quarantine. */
export const cleanDrive = (path: string, options: CleanOptions) =>
  invoke<CleanReport>("clean_drive", { path, options });

/**
 * Moves the sidecars whose content is now inside the files.
 *
 * It does not delete: it writes the same ledger as quarantine, so
 * `restoreQuarantine` puts every JSON back where it was.
 */
export const sweepSidecars = (path: string, destination: string) =>
  invoke<SidecarSweepReport>("sweep_sidecars", { path, destination });

/** Puts the files moved to quarantine back where they were. */
export const restoreQuarantine = (manifest: string) =>
  invoke<RestoreReport>("restore_quarantine", { manifest });

/** Privacy profile declared by the backend. */
export const privacyReport = () => invoke<PrivacyReport>("privacy_report");

/** Space arithmetic between a source and a destination. */
export const estimateSpace = (source: string, destination: string) =>
  invoke<SpaceEstimate>("estimate_space", { source, destination });

/** Application metadata, for the guide. */
export const appInfo = () => invoke<AppInfo>("app_info");

/** Reads the stored preferences. */
export const readPreferences = () => invoke<Preferences>("read_preferences");

/** Saves the stored preferences. */
export const writePreferences = (preferences: Preferences) =>
  invoke<void>("write_preferences", { preferences });

/**
 * Opens the system mail client on a pre-filled problem report.
 *
 * Nothing is sent from here: the address is compiled into the backend, the mail
 * client opens with the text visible, and the user decides whether to press
 * send. That is why this does not contradict the promise the whole application
 * rests on.
 */
export const composeSupportEmail = (subject: string, body: string) =>
  invoke<void>("compose_support_email", { subject, body });

/** Operating system, architecture and version, for a problem report. */
export const reportEnvironment = () => invoke<string>("report_environment");

/** Writes text to a path the user chose in the save dialog. */
export const saveTextFile = (path: string, content: string) =>
  invoke<void>("save_text_file", { path, content });

/** Replaces the user's home directory with `~` in the collected lines. */
export const redactHome = (lines: string[]) =>
  invoke<string[]>("redact_home", { lines });

/**
 * Reveals a path in the system file manager.
 *
 * It does not open the file: it reveals it in its folder. The program the
 * backend invokes is fixed and accepts only an existing path.
 */
export const revealInFileManager = (path: string) =>
  invoke<void>("reveal_in_file_manager", { path });

/**
 * Renders what a command rejected with into something readable.
 *
 * The backend rejects with an [`ErrorPayload`], that is a code plus the data
 * needed to compose the sentence: the text is decided in `lib/messages.ts`. The
 * two cases that do not come from there are still handled, namely a fault in
 * the IPC channel itself and any unexpected value, because an error that
 * cannot be stated must not turn into a blank screen.
 */
export function toMessage(error: unknown): string {
  if (isErrorPayload(error)) {
    const detail = errorDetail(error);
    return detail ? `${errorText(error)} ${detail}` : errorText(error);
  }
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return locale() === "it"
    ? "Errore imprevisto durante l'elaborazione."
    : "Unexpected error while processing.";
}

function isErrorPayload(value: unknown): value is ErrorPayload {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as { code?: unknown }).code === "string"
  );
}
