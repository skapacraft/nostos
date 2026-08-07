// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

import type { ReactNode } from "react";

import type { AppInfo, PrivacyReport } from "../types";

interface HelpProps {
  info: AppInfo | null;
  privacy: PrivacyReport | null;
  onClose: () => void;
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="space-y-2">
      <h3 className="text-sm font-semibold uppercase tracking-wide text-zinc-500 dark:text-zinc-400">
        {title}
      </h3>
      <div className="space-y-2 text-sm text-zinc-700 dark:text-zinc-300">
        {children}
      </div>
    </section>
  );
}

/**
 * Guida dell'applicazione.
 *
 * Gli indirizzi sono resi come testo selezionabile e non come collegamenti:
 * l'app non ha un plugin per aprire URL, e un link che non fa nulla sarebbe
 * peggio di un indirizzo che si copia.
 */
export function Help({ info, privacy, onClose }: HelpProps) {
  return (
    <div className="space-y-8">
      <header className="flex items-start justify-between gap-4">
        <div>
          <h2 className="text-lg font-semibold text-zinc-900 dark:text-zinc-100">
            Guida
          </h2>
          <p className="text-sm text-zinc-500 dark:text-zinc-400">
            Come portare i tuoi dati fuori da Google e rimetterli in ordine.
          </p>
        </div>
        <button
          type="button"
          onClick={onClose}
          className="shrink-0 rounded-lg border border-zinc-300 px-3 py-1.5 text-sm font-medium text-zinc-700 transition-colors hover:bg-zinc-100 dark:border-zinc-700 dark:text-zinc-200 dark:hover:bg-zinc-800"
        >
          Chiudi
        </button>
      </header>

      <Section title="1. Ottenere l'export">
        <p>
          Vai su <span className="selectable font-mono">takeout.google.com</span>
          , scegli i servizi che ti interessano e avvia l'esportazione. Google
          impiega da qualche minuto a diversi giorni e ti manda un'email quando
          è pronto.
        </p>
        <p className="text-zinc-500 dark:text-zinc-400">
          Consiglio: scegli il formato <span className="font-mono">.zip</span> e
          la dimensione massima più grande disponibile. Meno archivi significano
          meno occasioni per un download interrotto.
        </p>
      </Section>

      <Section title="2. Caricare l'export">
        <p>
          Trascina nella finestra la cartella <span className="font-mono">Takeout</span>{" "}
          già estratta, oppure uno qualsiasi degli archivi{" "}
          <span className="font-mono">takeout-...zip</span>: l'app riconosce gli
          altri della stessa serie e li unisce in un solo albero, segnalando se
          ne manca qualcuno.
        </p>
        <p className="text-zinc-500 dark:text-zinc-400">
          Perché li trovi, gli archivi devono stare tutti nella stessa cartella.
          Google divide l'export in file autonomi che ripetono la stessa
          struttura, e le foto di una stessa annata possono essere sparse su più
          archivi: estrarli uno per uno in cartelle separate lascia il lavoro a
          metà.
        </p>
        <p>La struttura attesa è questa:</p>
        <pre className="selectable overflow-x-auto rounded-lg bg-zinc-100 p-3 font-mono text-xs text-zinc-700 dark:bg-zinc-800 dark:text-zinc-300">
          {`Takeout/
├── Google Foto/
│   └── Foto da 2026/
│       ├── IMG_0001.JPG
│       └── IMG_0001.JPG.supplemental-metadata.json
├── Contatti/
├── Calendario/
└── Drive/`}
        </pre>
      </Section>

      <Section title="3. Riparare le foto">
        <p>
          Google esporta la data di scatto e le coordinate in un file{" "}
          <span className="font-mono">.json</span> affiancato alla foto, non
          dentro la foto. Copiando le immagini altrove quel file resta indietro e
          la data è persa: è il motivo per cui un Takeout riversato in una
          galleria mostra tutte le foto con la data di download.
        </p>
        <p>
          La riparazione riscrive data e posizione nei tag EXIF, senza
          ricomprimere l'immagine. Funziona su JPEG, HEIC, TIFF e WebP. Per PNG,
          GIF e video l'EXIF non è la sede dei metadati: in quei casi viene
          allineata la data del file e il sidecar JSON viene copiato accanto,
          così il dato non si perde.
        </p>
        <p className="rounded-lg border border-amber-300 bg-amber-50 p-3 text-amber-900 dark:border-amber-800 dark:bg-amber-950/40 dark:text-amber-200">
          La modalità predefinita scrive copie riparate in una cartella separata
          e non tocca gli originali. La riscrittura degli originali esiste, ma va
          scelta a mano e confermata.
        </p>
      </Section>

      <Section title="4. Pulire i duplicati">
        <p>
          Gli export contengono spesso lo stesso file più volte, perché una foto
          in tre album viene esportata tre volte. Il confronto avviene sul
          contenuto, non sul nome: due file con nome e dimensione uguali ma
          contenuto diverso restano entrambi.
        </p>
        <p>
          <strong className="font-medium text-zinc-900 dark:text-zinc-100">
            Nessuna funzione cancella file.
          </strong>{" "}
          Puoi costruire un albero pulito altrove, oppure spostare le copie in
          eccesso in una cartella di quarantena: in quel caso viene scritto un
          registro e il pulsante di annullamento rimette ogni file al suo posto.
        </p>
      </Section>

      <Section title="5. Se lo spazio non basta">
        <p>
          La copia riparata è una seconda libreria: se l'export pesa duecento
          gigabyte, ne servono altri duecento liberi. L'app fa il conto prima di
          cominciare e si ferma se non ci stanno, invece di riempire il disco a
          metà lavoro e lasciare una cartella che sembra completa.
        </p>
        <p>Quando lo spazio manca ci sono due strade, entrambe proposte a schermo:</p>
        <ul className="list-inside list-disc space-y-1 text-zinc-600 dark:text-zinc-400">
          <li>
            <strong className="font-medium text-zinc-900 dark:text-zinc-100">
              Riscrivere gli originali sul posto
            </strong>
            : servono poche decine di megabyte qualunque sia la libreria, perché
            i file vengono modificati uno alla volta. In cambio non resta una
            copia intatta, quindi va confermata a mano.
          </li>
          <li>
            <strong className="font-medium text-zinc-900 dark:text-zinc-100">
              Procedere a tranche
            </strong>
            : l'app elenca le sottocartelle, dice quanto pesa ciascuna e quali
            entrano nello spazio rimasto, con un pulsante per ripararne una per
            volta. Liberato spazio, si passa alla successiva.
          </li>
        </ul>
        <p className="text-zinc-500 dark:text-zinc-400">
          Nell'elenco le annate e gli album sono distinti, perché non pesano allo
          stesso modo: un album contiene per lo più copie di foto che stanno già
          in una cartella per anno. Dove l'app trova file presenti solo lì, lo
          segnala: quella cartella non si può rimandare senza perderli di vista.
        </p>
      </Section>

      <Section title="6. Portare via contatti e calendario">
        <p>
          Le sezioni Contatti e Calendario producono un file unico e deduplicato,
          in vCard 3.0 e iCalendar 2.0 standard, senza le estensioni proprietarie
          di Google. Sono importabili su Proton, Tuta e Nextcloud senza passaggi
          intermedi.
        </p>
      </Section>

      <Section title="Privacy">
        <p>
          L'applicazione non apre connessioni di rete. Non è una promessa scritta
          qui: è un controllo che fa fallire la compilazione se qualcuno
          introduce una libreria capace di parlare con l'esterno.
        </p>
        {privacy ? (
          <ul className="space-y-1 text-zinc-600 dark:text-zinc-400">
            {privacy.notes.map((note) => (
              <li key={note} className="flex gap-2">
                <span className="text-emerald-600 dark:text-emerald-400">✓</span>
                <span>{note}</span>
              </li>
            ))}
          </ul>
        ) : null}
        <p className="text-zinc-500 dark:text-zinc-400">
          Gli indirizzi web che trovi nell'app, compresi quelli dei segnaposto di
          Drive, sono mostrati come testo e non sono cliccabili: aprirli
          significherebbe una connessione, ed è una decisione che spetta a te,
          fuori da qui.
        </p>
      </Section>

      <Section title="Fuso orario">
        <p>
          I tag EXIF registrano l'ora dell'orologio sul posto, senza dire quale
          fosse il fuso. Google invece esporta un istante in tempo universale.
          Scrivere quell'istante così com'è sposterebbe ogni foto: uno scatto
          fatto a Milano alle 14 comparirebbe alle 13.
        </p>
        <p>
          Quando la foto ha le coordinate, l'app ricava il fuso del luogo e
          scrive l'ora locale corretta insieme al suo scarto, tenendo conto
          anche dell'ora legale in vigore quel giorno. Senza coordinate scrive
          l'ora universale dichiarandola tale: diversa da quella dell'orologio,
          ma non ambigua.
        </p>
        <p className="text-zinc-500 dark:text-zinc-400">
          I confini dei fusi orari provengono da OpenStreetMap, distribuiti
          sotto Open Database License (ODbL). I dati sono inclusi
          nell'applicazione: la ricerca avviene sul tuo computer e non comporta
          alcuna connessione.
        </p>
      </Section>

      <Section title="Limiti noti">
        <ul className="list-inside list-disc space-y-1 text-zinc-600 dark:text-zinc-400">
          <li>PNG e GIF: nessuna scrittura EXIF, solo data del file e sidecar.</li>
          <li>Video: i metadati stanno nel contenitore, non in EXIF.</li>
          <li>Mail e YouTube: riconosciute ma senza analizzatore dedicato.</li>
          <li>Gli archivi vanno estratti prima di analizzarne le sezioni.</li>
        </ul>
      </Section>

      {info ? (
        <Section title="Informazioni">
          <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-zinc-600 dark:text-zinc-400">
            <dt>Versione</dt>
            <dd className="selectable font-mono">{info.version}</dd>
            <dt>Autore</dt>
            <dd className="selectable">{info.author}</dd>
            <dt>Sito</dt>
            <dd className="selectable font-mono">{info.homepage}</dd>
            <dt>Codice</dt>
            <dd className="selectable font-mono break-all">{info.repository}</dd>
            <dt>Licenza</dt>
            <dd className="selectable font-mono">{info.license}</dd>
          </dl>
          <p className="text-zinc-500 dark:text-zinc-400">
            Software libero: puoi usarlo, studiarlo, modificarlo e
            ridistribuirlo. La licenza impone che ogni versione derivata resti
            altrettanto libera, così nessuno può prendere questo codice,
            aggiungerci tracciamento e distribuirlo come programma chiuso.
          </p>
        </Section>
      ) : null}
    </div>
  );
}
