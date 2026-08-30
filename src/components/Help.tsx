// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

import { useEffect, useRef, type ReactNode } from "react";

import { locale } from "../lib/locale";
import { PRIVACY_NOTES } from "../lib/messages";
import { ProblemReport } from "./ProblemReport";
import type { AppInfo, PrivacyReport } from "../types";

interface HelpProps {
  info: AppInfo | null;
  privacy: PrivacyReport | null;
  /** Errors seen this session, for the problem report. */
  errors: string[];
  /** True when the guide was opened from "Report a problem" in the menu. */
  openReport: boolean;
  /** True when the guide was opened from "Version and updates" in the menu. */
  openVersion: boolean;
  onError: (message: string) => void;
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
 * The application guide.
 *
 * Addresses are rendered as selectable text and not as links: the app has no
 * plugin for opening URLs, and a link that does nothing would be worse than
 * an address you can copy.
 */
export function Help({
  info,
  privacy,
  errors,
  openReport,
  openVersion,
  onError,
  onClose,
}: HelpProps) {
  const it = locale() === "it";
  // Opened from a menu item, the guide should land on the section that was
  // asked for rather than making the reader scroll past five others to find it.
  const reportRef = useRef<HTMLDivElement>(null);
  const versionRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (openReport) {
      reportRef.current?.scrollIntoView({ behavior: "smooth", block: "start" });
    } else if (openVersion) {
      versionRef.current?.scrollIntoView({ behavior: "smooth", block: "start" });
    }
  }, [openReport, openVersion]);

  return (
    <div className="space-y-8">
      <header className="flex items-start justify-between gap-4">
        <div>
          <h2 className="text-lg font-semibold text-zinc-900 dark:text-zinc-100">
            {it ? "Guida" : "Guide"}
          </h2>
          <p className="text-sm text-zinc-500 dark:text-zinc-400">
            {it
              ? "Come portare fuori i tuoi dati da Google e rimetterli in ordine."
              : "How to get your data out of Google and put it back in order."}
          </p>
        </div>
        <button
          type="button"
          onClick={onClose}
          className="shrink-0 rounded-lg border border-zinc-300 px-3 py-1.5 text-sm font-medium text-zinc-700 transition-colors hover:bg-zinc-100 dark:border-zinc-700 dark:text-zinc-200 dark:hover:bg-zinc-800"
        >
          {it ? "Chiudi" : "Close"}
        </button>
      </header>

      {it ? (
        <>
          <Section title="1. Ottenere l'export">
            <p>
              Vai su{" "}
              <span className="selectable font-mono">takeout.google.com</span>,
              scegli i servizi che ti interessano e avvia l'export. Google ci
              mette da pochi minuti a diversi giorni, e ti manda un'email
              quando è pronto.
            </p>
            <p className="text-zinc-500 dark:text-zinc-400">
              Conviene: scegli il formato{" "}
              <span className="font-mono">.zip</span> e la dimensione massima
              disponibile. Meno archivi significano meno occasioni di
              interruzione del download.
            </p>
          </Section>

          <Section title="2. Caricare l'export">
            <p>
              Trascina la cartella <span className="font-mono">Takeout</span>{" "}
              estratta nella finestra, oppure uno qualsiasi degli archivi{" "}
              <span className="font-mono">takeout-....zip</span>:
              l'applicazione riconosce gli altri della stessa serie e li unisce
              in un unico albero, dicendoti se ne manca uno.
            </p>
            <p className="text-zinc-500 dark:text-zinc-400">
              Perché li trovi, gli archivi devono stare nella stessa cartella.
              Google divide l'export in file autonomi che ripetono la stessa
              struttura, e le foto di un anno possono essere sparse su più
              archivi: estrarli uno per uno in cartelle separate lascia il
              lavoro a metà.
            </p>
            <p>Questa è la struttura che si aspetta:</p>
            <pre className="selectable overflow-x-auto rounded-lg bg-zinc-100 p-3 font-mono text-xs text-zinc-700 dark:bg-zinc-800 dark:text-zinc-300">
              {`Takeout/
├── Google Foto/
│   └── Foto del 2026/
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
              <span className="font-mono">.json</span> accanto alla
              fotografia, non al suo interno. Copia le immagini altrove e
              quel file resta indietro, e la data sparisce: è per questo che
              un Takeout riversato in una galleria mostra ogni fotografia
              datata il giorno in cui l'hai scaricata.
            </p>
            <p>
              La riparazione scrive nei tag EXIF tutto ciò che il file{" "}
              <span className="font-mono">.json</span> contiene e che ha una
              sede nei metadati: data di scatto con il suo fuso orario,
              coordinate, descrizione, volti riconosciuti e la stella dei
              preferiti. L'immagine non viene ricompressa. Funziona con JPEG,
              HEIC, TIFF e WebP. Per PNG, GIF e video, EXIF non è dove
              vivono i metadati: lì viene invece allineata la data del file e
              il sidecar JSON viene copiato accanto, così nulla va perso.
            </p>
            <p className="text-zinc-500 dark:text-zinc-400">
              Ciò che resta fuori è solo ciò che non ha dove andare nei
              metadati: il conteggio delle visualizzazioni e l'indirizzo della
              foto su Google Foto. Non sono proprietà della fotografia, ma
              finché quel JSON esiste ci sono, e l'applicazione te lo dice
              invece di lasciarti scoprirlo.
            </p>
            <p className="rounded-lg border border-amber-300 bg-amber-50 p-3 text-amber-900 dark:border-amber-800 dark:bg-amber-950/40 dark:text-amber-200">
              La modalità predefinita scrive copie riparate in una cartella
              separata e non tocca i tuoi originali. Riscrivere gli originali
              è disponibile, ma va scelto a mano e confermato.
            </p>
            <p>
              Dopo che gli originali sono stati riscritti, i file{" "}
              <span className="font-mono">.json</span> restano nella
              cartella, e l'applicazione offre di metterli da parte. Sposta
              solo quelli che non sono più l'unica copia di qualcosa,
              verificando dentro ogni file che il dato ci sia davvero: quelli
              per PNG, GIF e video restano, così come quelli per fotografie non
              riparate e quelli che contengono dati senza un tag dove stare.
              Non è una cancellazione, e un clic la annulla.
            </p>
          </Section>

          <Section title="4. Ripulire i duplicati">
            <p>
              Gli export contengono spesso lo stesso file più volte, perché
              una foto in tre album viene esportata tre volte. Il confronto
              avviene per contenuto e non per nome: due file con lo stesso
              nome e la stessa dimensione ma contenuto diverso sopravvivono
              entrambi.
            </p>
            <p>
              <strong className="font-medium text-zinc-900 dark:text-zinc-100">
                Nessuna funzione qui elimina un file.
              </strong>{" "}
              Puoi costruire un albero pulito altrove, oppure spostare le
              copie in eccesso in una cartella di quarantena: in quel caso
              viene scritto un registro e il pulsante di annullamento
              rimette ogni file dov'era.
            </p>
          </Section>

          <Section title="5. Quando lo spazio non basta">
            <p>
              Una copia riparata è una seconda libreria: se l'export pesa
              duecento gigabyte, altri duecento devono essere liberi.
              L'applicazione fa i conti prima di iniziare e si ferma se non
              ci sta, invece di riempire il disco a metà e lasciare una
              cartella che sembra completa.
            </p>
            <p>
              Quando lo spazio non c'è, si aprono due strade, entrambe offerte
              a schermo:
            </p>
            <ul className="list-inside list-disc space-y-1 text-zinc-600 dark:text-zinc-400">
              <li>
                <strong className="font-medium text-zinc-900 dark:text-zinc-100">
                  Riscrivere gli originali sul posto
                </strong>
                : bastano poche decine di megabyte qualunque sia la
                dimensione della libreria, perché i file vengono modificati
                uno alla volta. In cambio non resta una copia intatta, per
                questo va confermato a mano.
              </li>
              <li>
                <strong className="font-medium text-zinc-900 dark:text-zinc-100">
                  Procedere per lotti
                </strong>
                : l'applicazione elenca le sottocartelle, dice quanto pesa
                ciascuna e quali entrano nello spazio rimasto, con un
                pulsante per riparane una alla volta. Libera spazio, passa
                alla successiva.
              </li>
            </ul>
            <p className="text-zinc-500 dark:text-zinc-400">
              In quell'elenco anni e album restano separati, perché non
              pesano allo stesso modo: un album è per lo più copie di
              fotografie che stanno già in una cartella per anno. Dove
              l'applicazione trova file che esistono solo lì, lo dice: quella
              cartella non si può rimandare senza perderli di vista.
            </p>
          </Section>

          <Section title="6. Portare via contatti e calendari">
            <p>
              Le sezioni Contatti e Calendario producono ciascuna un unico
              file deduplicato, in vCard 3.0 e iCalendar 2.0 standard, senza
              le estensioni proprietarie di Google. Si importano in Proton,
              Tuta e Nextcloud senza passaggi intermedi.
            </p>
          </Section>

          <div ref={reportRef}>
            <Section title="Segnala un problema">
              <ProblemReport info={info} errors={errors} onError={onError} />
            </Section>
          </div>

          <Section title="Privacy">
            <p>
              L'applicazione non apre connessioni di rete. Non è una
              promessa scritta qui: è un controllo che fa fallire la build
              se qualcuno introduce una libreria capace di parlare con
              l'esterno.
            </p>
            {privacy ? (
              <ul className="space-y-1 text-zinc-600 dark:text-zinc-400">
                {privacy.notes.map((note) => (
                  <li key={note} className="flex gap-2">
                    <span className="text-emerald-600 dark:text-emerald-400">✓</span>
                    <span>{PRIVACY_NOTES[note]}</span>
                  </li>
                ))}
              </ul>
            ) : null}
            <p className="text-zinc-500 dark:text-zinc-400">
              Gli indirizzi web che trovi nell'applicazione, compresi quelli
              nei segnaposto Drive, sono mostrati come testo e non sono
              cliccabili: aprirli significherebbe una connessione, ed è una
              decisione tua da prendere fuori di qui.
            </p>
          </Section>

          <Section title="Fusi orari">
            <p>
              I tag EXIF registrano l'ora che l'orologio segnava sul posto,
              senza dire quale fosse il fuso. Google, invece, esporta un
              istante in tempo universale. Scrivere quell'istante così com'è
              sposterebbe ogni fotografia: uno scatto fatto a Milano alle
              due del pomeriggio comparirebbe all'una.
            </p>
            <p>
              Quando la fotografia ha coordinate, l'applicazione calcola il
              fuso del luogo e scrive l'ora locale corretta insieme al suo
              scarto, tenendo conto dell'ora legale in vigore quel giorno.
              Senza coordinate scrive l'ora universale e lo dichiara: diversa
              dall'orologio, ma non ambigua.
            </p>
            <p className="text-zinc-500 dark:text-zinc-400">
              I confini dei fusi orari vengono da OpenStreetMap, distribuiti
              sotto Open Database License (ODbL). I dati viaggiano dentro
              l'applicazione: la ricerca avviene sul tuo computer e non
              comporta alcuna connessione.
            </p>
          </Section>

          <Section title="Limiti noti">
            <ul className="list-inside list-disc space-y-1 text-zinc-600 dark:text-zinc-400">
              <li>PNG e GIF: nessuna scrittura EXIF, solo data del file e sidecar.</li>
              <li>Video: i metadati vivono nel contenitore, non in EXIF.</li>
              <li>Mail e YouTube: riconosciuti, ma senza analizzatore dedicato.</li>
              <li>Gli archivi vanno estratti prima che le loro sezioni possano essere lette.</li>
            </ul>
          </Section>

          {info ? (
            <div ref={versionRef}>
              <Section title="Versione e aggiornamenti">
                <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-zinc-600 dark:text-zinc-400">
                  <dt>Versione</dt>
                  <dd className="selectable font-mono">{info.version}</dd>
                  <dt>Compilata il</dt>
                  <dd className="selectable font-mono">{info.buildDate}</dd>
                </dl>
                <p>
                  L'applicazione non controlla mai da sola se esiste un
                  aggiornamento. È lo store da cui l'hai installata a tenerla
                  aggiornata, ed è l'unica parte del sistema che parla con la
                  rete.
                </p>
              </Section>
            </div>
          ) : null}

          {info ? (
            <Section title="Informazioni">
              <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-zinc-600 dark:text-zinc-400">
                <dt>Autore</dt>
                <dd className="selectable">{info.author}</dd>
                <dt>Sito</dt>
                <dd className="selectable font-mono">{info.homepage}</dd>
                <dt>Sorgente</dt>
                <dd className="selectable font-mono break-all">{info.repository}</dd>
                <dt>Licenza</dt>
                <dd className="selectable font-mono">{info.license}</dd>
              </dl>
              <p className="text-zinc-500 dark:text-zinc-400">
                Software libero: puoi usarlo, studiarlo, modificarlo e
                ridistribuirlo. La licenza richiede che ogni versione derivata
                resti altrettanto libera, così nessuno può prendere questo
                codice, aggiungerci tracciamento e distribuirlo come programma
                chiuso.
              </p>
              <p className="text-zinc-500 dark:text-zinc-400">
                Questa applicazione non è affiliata, approvata o
                sponsorizzata da Google LLC. Google, Google Foto, Google
                Drive e Google Takeout sono marchi di Google LLC, citati qui
                solo per identificare l'export che questo software legge.
              </p>
            </Section>
          ) : null}
        </>
      ) : (
        <>
          <Section title="1. Getting the export">
            <p>
              Go to <span className="selectable font-mono">takeout.google.com</span>
              , pick the services you care about and start the export. Google takes
              anywhere from a few minutes to several days, and sends you an email
              when it is ready.
            </p>
            <p className="text-zinc-500 dark:text-zinc-400">
              Worth doing: choose the <span className="font-mono">.zip</span> format
              and the largest size available. Fewer archives means fewer chances for
              an interrupted download.
            </p>
          </Section>

          <Section title="2. Loading the export">
            <p>
              Drag the extracted <span className="font-mono">Takeout</span> folder
              into the window, or any one of the{" "}
              <span className="font-mono">takeout-....zip</span> archives: the
              application recognises the others in the same series and merges them
              into a single tree, telling you if one is missing.
            </p>
            <p className="text-zinc-500 dark:text-zinc-400">
              For it to find them, the archives have to sit in the same folder.
              Google splits the export into self contained files that repeat the
              same structure, and the photos of one year can be spread across
              several archives: extracting them one by one into separate folders
              leaves the job half done.
            </p>
            <p>This is the structure it expects:</p>
            <pre className="selectable overflow-x-auto rounded-lg bg-zinc-100 p-3 font-mono text-xs text-zinc-700 dark:bg-zinc-800 dark:text-zinc-300">
              {`Takeout/
├── Google Photos/
│   └── Photos from 2026/
│       ├── IMG_0001.JPG
│       └── IMG_0001.JPG.supplemental-metadata.json
├── Contacts/
├── Calendar/
└── Drive/`}
            </pre>
          </Section>

          <Section title="3. Repairing the photos">
            <p>
              Google exports the capture date and the coordinates into a{" "}
              <span className="font-mono">.json</span> file beside the photograph,
              not inside it. Copy the images anywhere else and that file stays
              behind, and the date is gone: it is why a Takeout poured into a
              gallery shows every photograph dated the day you downloaded it.
            </p>
            <p>
              The repair writes into the EXIF tags everything the{" "}
              <span className="font-mono">.json</span> file holds that has a home in
              metadata: capture date with its time zone, coordinates, description,
              recognised faces and the favourite star. The image is not
              recompressed. It works on JPEG, HEIC, TIFF and WebP. For PNG, GIF and
              video, EXIF is not where metadata lives: there the file date is
              aligned instead and the JSON sidecar is copied alongside, so nothing
              is lost.
            </p>
            <p className="text-zinc-500 dark:text-zinc-400">
              What stays out is only what has nowhere to go in metadata: the view
              count and the address of the photo on Google Photos. They are not
              properties of the photograph, but as long as that JSON exists they are
              there, and the application tells you rather than letting you find out.
            </p>
            <p className="rounded-lg border border-amber-300 bg-amber-50 p-3 text-amber-900 dark:border-amber-800 dark:bg-amber-950/40 dark:text-amber-200">
              The default mode writes repaired copies into a separate folder and
              does not touch your originals. Rewriting the originals is available,
              but it has to be chosen by hand and confirmed.
            </p>
            <p>
              After the originals have been rewritten the{" "}
              <span className="font-mono">.json</span> files remain in the folder,
              and the application offers to set them aside. It moves only the ones
              that are no longer the sole copy of anything, checking inside each
              photograph that the data really is there: the ones belonging to PNG,
              GIF and video stay, so do the ones for photographs that were not
              repaired and the ones holding data with no tag to live in. It is not a
              deletion, and one click undoes it.
            </p>
          </Section>

          <Section title="4. Cleaning up duplicates">
            <p>
              Exports often contain the same file several times, because a photo in
              three albums is exported three times. The comparison is made on
              content and not on the name: two files with the same name and size but
              different content both survive.
            </p>
            <p>
              <strong className="font-medium text-zinc-900 dark:text-zinc-100">
                No function here deletes a file.
              </strong>{" "}
              You can build a clean tree elsewhere, or move the surplus copies into
              a quarantine folder: in that case a ledger is written and the undo
              button puts every file back where it was.
            </p>
          </Section>

          <Section title="5. When there is not enough room">
            <p>
              A repaired copy is a second library: if the export weighs two hundred
              gigabytes, another two hundred have to be free. The application does
              the arithmetic before starting and stops if it does not fit, rather
              than filling the disk halfway through and leaving a folder that looks
              complete.
            </p>
            <p>
              When the room is not there, two routes are open, and both are offered
              on screen:
            </p>
            <ul className="list-inside list-disc space-y-1 text-zinc-600 dark:text-zinc-400">
              <li>
                <strong className="font-medium text-zinc-900 dark:text-zinc-100">
                  Rewrite the originals in place
                </strong>
                : a few tens of megabytes suffice whatever the size of the library,
                because the files are modified one at a time. In exchange no
                untouched copy is left, which is why it has to be confirmed by hand.
              </li>
              <li>
                <strong className="font-medium text-zinc-900 dark:text-zinc-100">
                  Work through it in batches
                </strong>
                : the application lists the subfolders, says how much each one
                weighs and which ones fit in the room left, with a button to repair
                one at a time. Free some space, move to the next.
              </li>
            </ul>
            <p className="text-zinc-500 dark:text-zinc-400">
              In that list years and albums are kept apart, because they do not
              weigh the same: an album is mostly copies of photographs that already
              sit in a year folder. Where the application finds files that exist
              only there, it says so: that folder cannot be postponed without losing
              sight of them.
            </p>
          </Section>

          <Section title="6. Taking contacts and calendars with you">
            <p>
              The Contacts and Calendar sections produce a single deduplicated file
              each, in standard vCard 3.0 and iCalendar 2.0, without Google's
              proprietary extensions. They import into Proton, Tuta and Nextcloud
              with nothing in between.
            </p>
          </Section>

          <div ref={reportRef}>
            <Section title="Report a problem">
              <ProblemReport info={info} errors={errors} onError={onError} />
            </Section>
          </div>

          <Section title="Privacy">
            <p>
              The application opens no network connections. That is not a promise
              written here: it is a check that fails the build if anyone introduces
              a library capable of talking to the outside.
            </p>
            {privacy ? (
              <ul className="space-y-1 text-zinc-600 dark:text-zinc-400">
                {privacy.notes.map((note) => (
                  <li key={note} className="flex gap-2">
                    <span className="text-emerald-600 dark:text-emerald-400">✓</span>
                    <span>{PRIVACY_NOTES[note]}</span>
                  </li>
                ))}
              </ul>
            ) : null}
            <p className="text-zinc-500 dark:text-zinc-400">
              The web addresses you find in the application, including the ones in
              Drive placeholders, are shown as text and are not clickable: opening
              them would mean a connection, and that is your decision to make,
              outside of here.
            </p>
          </Section>

          <Section title="Time zones">
            <p>
              EXIF tags record the time the clock showed on the spot, without saying
              which zone that was. Google, on the other hand, exports an instant in
              universal time. Writing that instant as it stands would shift every
              photograph: a picture taken in Milan at two in the afternoon would
              appear at one.
            </p>
            <p>
              When the photograph has coordinates, the application works out the
              zone of the place and writes the correct local time together with its
              offset, taking into account the daylight saving in force that day.
              Without coordinates it writes universal time and says so: different
              from the clock, but not ambiguous.
            </p>
            <p className="text-zinc-500 dark:text-zinc-400">
              The time zone boundaries come from OpenStreetMap, distributed under
              the Open Database License (ODbL). The data ships inside the
              application: the lookup happens on your computer and involves no
              connection.
            </p>
          </Section>

          <Section title="Known limits">
            <ul className="list-inside list-disc space-y-1 text-zinc-600 dark:text-zinc-400">
              <li>PNG and GIF: no EXIF writing, only the file date and the sidecar.</li>
              <li>Video: the metadata lives in the container, not in EXIF.</li>
              <li>Mail and YouTube: recognised, but with no dedicated analyser.</li>
              <li>Archives have to be extracted before their sections can be read.</li>
            </ul>
          </Section>

          {info ? (
            <div ref={versionRef}>
              <Section title="Version and updates">
                <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-zinc-600 dark:text-zinc-400">
                  <dt>Version</dt>
                  <dd className="selectable font-mono">{info.version}</dd>
                  <dt>Built on</dt>
                  <dd className="selectable font-mono">{info.buildDate}</dd>
                </dl>
                <p>
                  The application never checks for itself whether an update
                  exists. The store you installed it from is what keeps it
                  current, and it is the only part of the system that talks to
                  the network.
                </p>
              </Section>
            </div>
          ) : null}

          {info ? (
            <Section title="About">
              <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-zinc-600 dark:text-zinc-400">
                <dt>Author</dt>
                <dd className="selectable">{info.author}</dd>
                <dt>Website</dt>
                <dd className="selectable font-mono">{info.homepage}</dd>
                <dt>Source</dt>
                <dd className="selectable font-mono break-all">{info.repository}</dd>
                <dt>Licence</dt>
                <dd className="selectable font-mono">{info.license}</dd>
              </dl>
              <p className="text-zinc-500 dark:text-zinc-400">
                Free software: you may use it, study it, modify it and redistribute
                it. The licence requires every derived version to stay just as free,
                so nobody can take this code, add tracking to it and ship it as a
                closed program.
              </p>
              <p className="text-zinc-500 dark:text-zinc-400">
                This application is not affiliated with, endorsed by or sponsored by
                Google LLC. Google, Google Photos, Google Drive and Google Takeout
                are trademarks of Google LLC, named here only to identify the export
                this software reads.
              </p>
            </Section>
          ) : null}
        </>
      )}
    </div>
  );
}
