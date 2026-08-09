// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

/**
 * Unico punto di contatto tra frontend e backend.
 *
 * Tutto passa dal canale IPC locale di Tauri: non esiste un client HTTP nel
 * frontend e non deve essercene uno. Qualsiasi `fetch` aggiunto qui sarebbe
 * comunque bloccato dalla CSP (`connect-src` limitato a `ipc:`).
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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

/** Evento di avanzamento emesso dai comandi lunghi. */
const PROGRESS_EVENT = "takeout://progress";

/** Evento con cui la voce di menu "Guida" chiede di mostrare la guida. */
const SHOW_HELP_EVENT = "takeout://mostra-guida";

/** Si mette in ascolto della richiesta di apertura guida dal menu. */
export function onShowHelp(handler: () => void): Promise<UnlistenFn> {
  return listen(SHOW_HELP_EVENT, () => handler());
}

/**
 * Si mette in ascolto dell'avanzamento.
 *
 * Il backend limita già la frequenza a un evento ogni 80 ms, quindi il
 * gestore può aggiornare lo stato React direttamente senza altre difese.
 */
export function onProgress(
  handler: (progress: Progress) => void,
): Promise<UnlistenFn> {
  return listen<Progress>(PROGRESS_EVENT, (event) => handler(event.payload));
}

/** Carica una cartella Takeout o un archivio `takeout-*.zip`. */
export const loadSource = (path: string) =>
  invoke<SourceSummary>("load_source", { path });

/** Riepilogo della sorgente già caricata in sessione. */
export const currentSource = () => invoke<SourceSummary>("current_source");

/** Dimentica la sorgente corrente. */
export const closeSource = () => invoke<void>("close_source");

/** Ispeziona un archivio senza estrarlo. */
export const inspectArchive = (path: string) =>
  invoke<ArchiveSummary>("inspect_archive", { path });

/** Prime voci di un archivio, per l'anteprima. */
export const listArchiveEntries = (path: string, limit?: number) =>
  invoke<ArchiveEntry[]>("list_archive_entries", { path, limit });

/** Estrae un archivio nella cartella scelta dall'utente. */
export const extractArchive = (path: string, destination: string) =>
  invoke<ExtractReport>("extract_archive", { path, destination });

/** Individua tutti gli archivi che compongono lo stesso export. */
export const discoverArchiveSeries = (path: string) =>
  invoke<ArchiveSeries>("discover_archive_series", { path });

/** Estrae l'intera serie di archivi in un unico albero, con avanzamento. */
export const extractTakeout = (path: string, destination: string) =>
  invoke<ExtractReport>("extract_takeout", { path, destination });

/** Analizza i media di Google Foto. Senza `path` usa l'intera sorgente. */
export const scanPhotos = (path?: string) =>
  invoke<PhotoScanReport>("scan_photos", { path });

/** Ripara data e coordinate dei media secondo le opzioni indicate. */
export const repairPhotos = (path: string, options: WriteOptions) =>
  invoke<RepairReport>("repair_photos", { path, options });

/** Analizza l'export Contatti. */
export const scanContacts = (path?: string) =>
  invoke<ContactsReport>("scan_contacts", { path });

/** Ricostruisce album, cartelle per anno e versioni modificate. */
export const scanAlbums = (path: string) =>
  invoke<AlbumIndex>("scan_albums", { path });

/** Scrive il manifest degli album, da fare prima di deduplicare. */
export const exportAlbumManifest = (path: string, destination: string) =>
  invoke<ExportReport>("export_album_manifest", { path, destination });

/** Analizza l'export Calendario. */
export const scanCalendar = (path?: string) =>
  invoke<CalendarReport>("scan_calendar", { path });

/** Scrive un vCard 3.0 pulito e deduplicato. */
export const exportContacts = (path: string, destination: string) =>
  invoke<ExportReport>("export_contacts", { path, destination });

/** Scrive un iCalendar 2.0 pulito e deduplicato. */
export const exportCalendar = (path: string, destination: string) =>
  invoke<ExportReport>("export_calendar", { path, destination });

/** Analizza l'export Drive. */
export const scanDrive = (path?: string) =>
  invoke<DriveReport>("scan_drive", { path });

/** Calcola il piano di pulizia senza toccare nulla. */
export const planDriveClean = (path: string, options: CleanOptions) =>
  invoke<CleanPlan>("plan_drive_clean", { path, options });

/** Esegue la pulizia: albero pulito altrove, oppure quarantena reversibile. */
export const cleanDrive = (path: string, options: CleanOptions) =>
  invoke<CleanReport>("clean_drive", { path, options });

/**
 * Sposta i sidecar il cui contenuto è ormai dentro ai file.
 *
 * Non cancella: scrive lo stesso registro della quarantena, quindi
 * `restoreQuarantine` rimette ogni JSON dov'era.
 */
export const sweepSidecars = (path: string, destination: string) =>
  invoke<SidecarSweepReport>("sweep_sidecars", { path, destination });

/** Rimette al loro posto i file spostati in quarantena. */
export const restoreQuarantine = (manifest: string) =>
  invoke<RestoreReport>("restore_quarantine", { manifest });

/** Profilo privacy dichiarato dal backend. */
export const privacyReport = () => invoke<PrivacyReport>("privacy_report");

/** Conti sullo spazio tra una sorgente e una destinazione. */
export const estimateSpace = (source: string, destination: string) =>
  invoke<SpaceEstimate>("estimate_space", { source, destination });

/** Metadati dell'applicazione, per la guida. */
export const appInfo = () => invoke<AppInfo>("app_info");

/** Legge le preferenze conservate. */
export const readPreferences = () => invoke<Preferences>("read_preferences");

/** Salva le preferenze conservate. */
export const writePreferences = (preferences: Preferences) =>
  invoke<void>("write_preferences", { preferences });

/**
 * Mostra un percorso nel gestore file del sistema.
 *
 * Non apre il file: lo rivela nella sua cartella. Il comando invocato dal
 * backend è fisso e accetta solo un percorso esistente.
 */
export const revealInFileManager = (path: string) =>
  invoke<void>("reveal_in_file_manager", { path });

/**
 * Rende leggibile ciò con cui un comando ha rifiutato.
 *
 * Il backend rifiuta con un [`ErrorPayload`], cioè un codice più i dati che
 * servono a comporre la frase: il testo si decide in `lib/messages.ts`. Restano
 * gestiti anche i due casi che non passano da lì, cioè un guasto del canale IPC
 * stesso e qualunque valore inatteso, perché un errore che non si sa dire non
 * deve diventare una schermata vuota.
 */
export function toMessage(error: unknown): string {
  if (isErrorPayload(error)) {
    const detail = errorDetail(error);
    return detail ? `${errorText(error)} ${detail}` : errorText(error);
  }
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "Errore imprevisto durante l'elaborazione.";
}

function isErrorPayload(value: unknown): value is ErrorPayload {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as { code?: unknown }).code === "string"
  );
}
