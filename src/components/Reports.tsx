// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

import { useState, type ReactNode } from "react";

import {
  formatBytes,
  formatCount,
  formatDate,
  percent,
  shortenPath,
} from "../lib/format";
import * as api from "../lib/api";
import type {
  CalendarReport,
  ContactsReport,
  DriveReport,
  PhotoScanReport,
} from "../types";
import { AlbumPanel } from "./AlbumPanel";
import { FolderCleaner } from "./FolderCleaner";
import { ExportButton } from "./ExportButton";
import { PhotoFixer } from "./PhotoFixer";
import { Stat } from "./Stat";
import { Notices } from "./Notices";
import { CATEGORY_LABELS } from "../lib/messages";

function SectionTitle({ children }: { children: ReactNode }) {
  return (
    <h4 className="text-sm font-semibold uppercase tracking-wide text-zinc-500 dark:text-zinc-400">
      {children}
    </h4>
  );
}

function Card({ children }: { children: ReactNode }) {
  return (
    <div className="overflow-hidden rounded-xl border border-zinc-200 bg-white dark:border-zinc-800 dark:bg-zinc-900">
      {children}
    </div>
  );
}

// --- Foto ----------------------------------------------------------------

interface PhotoReportProps {
  report: PhotoScanReport;
  /** Cartella analizzata, su cui agisce la riparazione. */
  path: string;
  workingFolder: string | null;
  onWorkingFolder: (path: string) => void;
  onRepaired: () => void;
  onError: (message: string) => void;
}

export function PhotoReportView({
  report,
  path,
  workingFolder,
  onWorkingFolder,
  onRepaired,
  onError,
}: PhotoReportProps) {
  const [albumRisk, setAlbumRisk] = useState(false);

  return (
    <div className="space-y-5">
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">
        <Stat label="Media" value={formatCount(report.mediaCount)} />
        <Stat
          label="Con sidecar"
          value={formatCount(report.withSidecar)}
          hint={percent(report.withSidecar, report.mediaCount)}
        />
        <Stat
          label="Data EXIF"
          value={formatCount(report.withExifDate)}
          hint={percent(report.withExifDate, report.mediaCount)}
        />
        <Stat label="Con GPS" value={formatCount(report.withGeo)} />
        <Stat
          label="Da riparare"
          value={formatCount(report.needsRepair)}
          hint="data non nell'EXIF"
          tone={report.needsRepair > 0 ? "warning" : "neutral"}
        />
        <Stat label="Totale" value={formatBytes(report.totalBytes)} />
      </div>

      <AlbumPanel path={path} onError={onError} onRisk={setAlbumRisk} />

      {report.dateFromFilename > 0 ? (
        <p className="text-sm text-zinc-500 dark:text-zinc-400">
          Per {formatCount(report.dateFromFilename)} file la data è stata
          dedotta dal nome, perché mancavano sia l'EXIF sia il sidecar. È
          l'orario che l'app fotocamera ha scritto al momento dello scatto.
        </p>
      ) : null}

      {report.mediaCount > 0 ? (
        <PhotoFixer
          path={path}
          repairable={report.needsRepair}
          outputRoot={workingFolder}
          onOutputRoot={onWorkingFolder}
          onDone={onRepaired}
          onError={onError}
        />
      ) : null}

      <FolderCleaner
        path={path}
        albumRisk={albumRisk}
        destination={workingFolder}
        onDestination={onWorkingFolder}
        onCleaned={onRepaired}
        onError={onError}
      />

      {report.sample.length > 0 ? (
        <div className="space-y-2">
          <SectionTitle>Anteprima</SectionTitle>
          <Card>
            <ul className="divide-y divide-zinc-200 dark:divide-zinc-800">
              {report.sample.map((media) => (
                <li key={media.path} className="px-4 py-3">
                  <div className="flex flex-wrap items-baseline justify-between gap-2">
                    <p className="selectable truncate text-sm font-medium text-zinc-900 dark:text-zinc-100">
                      {media.fileName}
                    </p>
                    <span className="text-xs text-zinc-500 dark:text-zinc-400">
                      {formatBytes(media.sizeBytes)}
                    </span>
                  </div>
                  <p className="mt-0.5 text-xs text-zinc-500 dark:text-zinc-400">
                    {formatDate(media.resolvedTakenAt)}
                    {media.takenAtSource !== "missing"
                      ? ` (da ${media.takenAtSource === "exif" ? "EXIF" : "sidecar"})`
                      : ""}
                    {media.resolvedGeo
                      ? ` · ${media.resolvedGeo.latitude.toFixed(4)}, ${media.resolvedGeo.longitude.toFixed(4)}`
                      : ""}
                  </p>
                </li>
              ))}
            </ul>
          </Card>
        </div>
      ) : null}

      {report.unreadableCount > 0 ? (
        <p className="text-xs text-zinc-500 dark:text-zinc-400">
          {formatCount(report.unreadableCount)} file non leggibili sono stati
          ignorati.
        </p>
      ) : null}
    </div>
  );
}

// --- Contatti ------------------------------------------------------------

interface ContactsReportProps {
  report: ContactsReport;
  path: string;
  onError: (message: string) => void;
}

export function ContactsReportView({
  report,
  path,
  onError,
}: ContactsReportProps) {
  return (
    <div className="space-y-5">
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-5">
        <Stat label="Schede lette" value={formatCount(report.total)} />
        <Stat label="Contatti unici" value={formatCount(report.unique)} />
        <Stat
          label="Duplicati"
          value={formatCount(report.duplicates)}
          tone={report.duplicates > 0 ? "warning" : "neutral"}
        />
        <Stat label="Con email" value={formatCount(report.withEmail)} />
        <Stat label="Con telefono" value={formatCount(report.withPhone)} />
      </div>

      {report.withoutContactInfo > 0 ? (
        <p className="text-sm text-zinc-500 dark:text-zinc-400">
          {formatCount(report.withoutContactInfo)} schede non hanno né email né
          telefono.
        </p>
      ) : null}

      {report.unique > 0 ? (
        <ExportButton
          label="Esporta vCard pulito"
          defaultName="contacts_cleaned.vcf"
          extension="vcf"
          filterName="vCard"
          hint={`Scrive ${formatCount(report.unique)} contatti deduplicati in un vCard 3.0 standard, importabile su Proton, Tuta e Nextcloud.`}
          onExport={(destination) => api.exportContacts(path, destination)}
          onError={onError}
        />
      ) : null}

      {report.sample.length > 0 ? (
        <div className="space-y-2">
          <SectionTitle>Anteprima</SectionTitle>
          <Card>
            <ul className="divide-y divide-zinc-200 dark:divide-zinc-800">
              {report.sample.map((contact, index) => (
                <li
                  key={`${contact.displayName ?? "senza-nome"}-${index}`}
                  className="px-4 py-3"
                >
                  <p className="selectable text-sm font-medium text-zinc-900 dark:text-zinc-100">
                    {contact.displayName ??
                      [contact.givenName, contact.familyName]
                        .filter(Boolean)
                        .join(" ") ??
                      "Senza nome"}
                  </p>
                  <p className="selectable mt-0.5 text-xs text-zinc-500 dark:text-zinc-400">
                    {[contact.emails[0], contact.phones[0], contact.organization]
                      .filter(Boolean)
                      .join(" · ") || "nessun recapito"}
                  </p>
                </li>
              ))}
            </ul>
          </Card>
        </div>
      ) : null}
    </div>
  );
}

// --- Drive ---------------------------------------------------------------

interface DriveReportProps {
  report: DriveReport;
  path: string;
  workingFolder: string | null;
  onWorkingFolder: (path: string) => void;
  onCleaned: () => void;
  onError: (message: string) => void;
}

export function DriveReportView({
  report,
  path,
  workingFolder,
  onWorkingFolder,
  onCleaned,
  onError,
}: DriveReportProps) {
  return (
    <div className="space-y-5">
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <Stat label="File" value={formatCount(report.fileCount)} />
        <Stat label="Cartelle" value={formatCount(report.dirCount)} />
        <Stat label="Totale" value={formatBytes(report.totalBytes)} />
        <Stat
          label="Segnaposto"
          value={formatCount(report.placeholderCount)}
          hint="senza contenuto"
          tone={report.placeholderCount > 0 ? "warning" : "neutral"}
        />
      </div>

      <Notices items={report.warnings} />

      <FolderCleaner
        path={path}
        destination={workingFolder}
        onDestination={onWorkingFolder}
        onCleaned={onCleaned}
        onError={onError}
      />

      {report.categories.length > 0 ? (
        <div className="space-y-2">
          <SectionTitle>Per categoria</SectionTitle>
          <Card>
            <ul className="divide-y divide-zinc-200 dark:divide-zinc-800">
              {report.categories.map((category) => (
                <li
                  key={category.category}
                  className="flex items-center justify-between px-4 py-2.5 text-sm"
                >
                  <span className="text-zinc-700 dark:text-zinc-300">
                    {CATEGORY_LABELS[category.category]}
                  </span>
                  <span className="tabular-nums text-zinc-500 dark:text-zinc-400">
                    {formatCount(category.fileCount)} file ·{" "}
                    {formatBytes(category.totalBytes)}
                  </span>
                </li>
              ))}
            </ul>
          </Card>
        </div>
      ) : null}

      {report.duplicateGroups.length > 0 ? (
        <div className="space-y-2">
          <SectionTitle>
            Duplicati ({formatBytes(report.duplicateBytes)} recuperabili)
          </SectionTitle>
          <Card>
            <ul className="divide-y divide-zinc-200 dark:divide-zinc-800">
              {report.duplicateGroups.slice(0, 10).map((group) => (
                <li key={`${group.fileName}-${group.sizeBytes}`} className="px-4 py-2.5">
                  <p className="selectable truncate text-sm text-zinc-900 dark:text-zinc-100">
                    {group.fileName}
                  </p>
                  <p className="text-xs text-zinc-500 dark:text-zinc-400">
                    {group.paths.length} copie · {formatBytes(group.sizeBytes)}{" "}
                    ciascuna
                  </p>
                </li>
              ))}
            </ul>
          </Card>
        </div>
      ) : null}

      {report.placeholders.length > 0 ? (
        <div className="space-y-2">
          <SectionTitle>Segnaposto senza contenuto</SectionTitle>
          <Card>
            <ul className="divide-y divide-zinc-200 dark:divide-zinc-800">
              {report.placeholders.slice(0, 10).map((placeholder) => (
                <li key={placeholder.path} className="px-4 py-2.5">
                  <p className="selectable truncate text-sm text-zinc-900 dark:text-zinc-100">
                    {placeholder.fileName}
                  </p>
                  {/* L'URL è mostrato come testo e mai reso cliccabile: aprirlo
                      significherebbe una connessione verso Google. */}
                  <p
                    className="selectable truncate font-mono text-xs text-zinc-500 dark:text-zinc-400"
                    title={placeholder.targetUrl ?? undefined}
                  >
                    {placeholder.targetUrl
                      ? shortenPath(placeholder.targetUrl, 72)
                      : "riferimento non leggibile"}
                  </p>
                </li>
              ))}
            </ul>
          </Card>
        </div>
      ) : null}
    </div>
  );
}

// --- Calendario ----------------------------------------------------------

interface CalendarReportProps {
  report: CalendarReport;
  path: string;
  onError: (message: string) => void;
}

/** Rende leggibile una data iCalendar grezza (`20200101T120000Z`). */
function formatIcsDate(raw: string | null): string {
  if (!raw || raw.length < 8) return "data non disponibile";
  const day = `${raw.slice(6, 8)}/${raw.slice(4, 6)}/${raw.slice(0, 4)}`;
  if (raw.length < 15 || raw[8] !== "T") return day;
  return `${day} ${raw.slice(9, 11)}:${raw.slice(11, 13)}`;
}

export function CalendarReportView({
  report,
  path,
  onError,
}: CalendarReportProps) {
  return (
    <div className="space-y-5">
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-5">
        <Stat label="Eventi letti" value={formatCount(report.total)} />
        <Stat label="Eventi unici" value={formatCount(report.unique)} />
        <Stat
          label="Duplicati"
          value={formatCount(report.duplicates)}
          tone={report.duplicates > 0 ? "warning" : "neutral"}
        />
        <Stat label="Ricorrenti" value={formatCount(report.recurring)} />
        <Stat label="Giornata intera" value={formatCount(report.allDay)} />
      </div>

      {report.droppedProperties > 0 ? (
        <p className="text-sm text-zinc-500 dark:text-zinc-400">
          {formatCount(report.droppedProperties)} proprietà proprietarie di
          Google verranno rimosse dall'export: non hanno significato fuori dai
          suoi servizi.
        </p>
      ) : null}

      {report.unique > 0 ? (
        <ExportButton
          label="Esporta calendario pulito"
          defaultName="calendar_cleaned.ics"
          extension="ics"
          filterName="iCalendar"
          hint={`Scrive ${formatCount(report.unique)} eventi deduplicati in un iCalendar 2.0 standard, senza le estensioni proprietarie.`}
          onExport={(destination) => api.exportCalendar(path, destination)}
          onError={onError}
        />
      ) : null}

      {report.sample.length > 0 ? (
        <div className="space-y-2">
          <SectionTitle>Anteprima</SectionTitle>
          <Card>
            <ul className="divide-y divide-zinc-200 dark:divide-zinc-800">
              {report.sample.map((event, index) => (
                <li key={`${event.uid ?? "senza-uid"}-${index}`} className="px-4 py-3">
                  <p className="selectable truncate text-sm font-medium text-zinc-900 dark:text-zinc-100">
                    {event.summary ?? "Senza titolo"}
                  </p>
                  <p className="mt-0.5 text-xs text-zinc-500 dark:text-zinc-400">
                    {event.isAllDay
                      ? `${formatIcsDate(event.start)} (giornata intera)`
                      : formatIcsDate(event.start)}
                    {event.isRecurring ? " · ricorrente" : ""}
                    {event.location ? ` · ${event.location}` : ""}
                  </p>
                </li>
              ))}
            </ul>
          </Card>
        </div>
      ) : null}

      <Notices items={report.warnings} />
    </div>
  );
}
