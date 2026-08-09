// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

import { useCallback, useEffect, useState } from "react";

import * as api from "../lib/api";
import { toMessage } from "../lib/api";
import { formatCount } from "../lib/format";
import type { AlbumIndex } from "../types";
import { ExportButton } from "./ExportButton";
import { Stat } from "./Stat";
import { Notices } from "./Notices";

interface AlbumPanelProps {
  path: string;
  onError: (message: string) => void;
  /** Tells the cleanup panel whether membership is still at risk. */
  onRisk: (unsaved: boolean) => void;
}

/**
 * Google Photos albums.
 *
 * Google does not export albums as metadata: it exports them as folders
 * containing a second copy of the photo. Deduplicating without having saved
 * the membership first erases the only remaining trace of which photos were
 * in which album, and that does not come back from quarantine.
 */
export function AlbumPanel({ path, onError, onRisk }: AlbumPanelProps) {
  const [index, setIndex] = useState<AlbumIndex | null>(null);
  const [saved, setSaved] = useState(false);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    api
      .scanAlbums(path)
      .then((result) => {
        if (cancelled) return;
        setIndex(result);
        onRisk(result.membershipCount > 0);
      })
      .catch((error) => {
        if (!cancelled) onError(toMessage(error));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [path, onError, onRisk]);

  const handleExported = useCallback(() => {
    setSaved(true);
    onRisk(false);
  }, [onRisk]);

  if (loading) {
    return (
      <p className="text-sm text-zinc-500 dark:text-zinc-400">
        Lettura della struttura degli album...
      </p>
    );
  }
  if (!index || (index.albums.length === 0 && index.editedCount === 0)) {
    return null;
  }

  return (
    <div className="space-y-4 rounded-xl border border-zinc-200 bg-white p-4 dark:border-zinc-800 dark:bg-zinc-900">
      <div>
        <h4 className="text-sm font-semibold text-zinc-900 dark:text-zinc-100">
          Album
        </h4>
        <p className="mt-1 text-sm text-zinc-500 dark:text-zinc-400">
          Google non esporta gli album come informazione a parte: crea una
          cartella per album e ci mette dentro una seconda copia della foto.
        </p>
      </div>

      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <Stat label="Album" value={formatCount(index.albums.length)} />
        <Stat
          label="Cartelle per anno"
          value={formatCount(index.yearFolders.length)}
        />
        <Stat
          label="Foto in album"
          value={formatCount(index.membershipCount)}
          hint="duplicate altrove"
          tone={index.membershipCount > 0 ? "warning" : "neutral"}
        />
        <Stat
          label="Solo in album"
          value={formatCount(index.albumOnly)}
          hint="senza copia per anno"
          tone={index.albumOnly > 0 ? "warning" : "neutral"}
        />
      </div>

      <Notices items={index.warnings} />

      {index.membershipCount > 0 ? (
        <ExportButton
          label="Salva il manifest degli album"
          defaultName="album.json"
          extension="json"
          filterName="JSON"
          hint={`Registra a quale album appartiene ciascuna delle ${formatCount(index.membershipCount)} foto duplicate. Da fare prima della deduplica: i file si recuperano dalla quarantena, questa informazione no.`}
          onExport={async (destination) => {
            const report = await api.exportAlbumManifest(path, destination);
            handleExported();
            return report;
          }}
          onError={onError}
        />
      ) : null}

      {saved ? (
        <p className="rounded-lg border border-emerald-300 bg-emerald-50 px-4 py-2 text-sm text-emerald-900 dark:border-emerald-800 dark:bg-emerald-950/30 dark:text-emerald-200">
          Appartenenza salvata: ora la deduplica è sicura.
        </p>
      ) : null}

      {index.editedCount > 0 ? (
        <p className="text-sm text-zinc-500 dark:text-zinc-400">
          {formatCount(index.editedCount)} file sono versioni modificate
          affiancate all'originale (suffisso{" "}
          <span className="font-mono">{index.editedPairs[0].suffix}</span>). Non
          sono duplicati: hanno pixel diversi e restano entrambi.
        </p>
      ) : null}

      {index.albums.length > 0 ? (
        <details className="text-sm">
          <summary className="cursor-pointer text-zinc-600 dark:text-zinc-300">
            Vedi i {formatCount(index.albums.length)} album
          </summary>
          <ul className="mt-2 max-h-52 space-y-1 overflow-y-auto text-xs">
            {index.albums.slice(0, 100).map((album) => (
              <li
                key={album.path}
                className="selectable flex justify-between gap-3 text-zinc-600 dark:text-zinc-400"
              >
                <span className="truncate">{album.name}</span>
                <span className="shrink-0 tabular-nums">
                  {formatCount(album.fileCount)}
                </span>
              </li>
            ))}
          </ul>
        </details>
      ) : null}
    </div>
  );
}
