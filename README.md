# Open Takeout Hub

Applicazione desktop local-first per elaborare gli export di Google Takeout.
Analizza foto, contatti e Drive senza che un solo byte lasci il computer.

## Perché

Un Takeout è un archivio grezzo e poco navigabile: le foto perdono l'EXIF e
portano la data in un sidecar JSON, i contatti arrivano come vCard con
duplicati, Drive contiene segnaposto che non includono il contenuto. Gli
strumenti online che risolvono questi problemi chiedono di caricare l'intero
export su un server di terze parti, cioè esattamente i dati che si stava
cercando di riprendere in mano.

Open Takeout Hub fa lo stesso lavoro in locale.

## Stack

| Livello  | Tecnologia                                   |
| -------- | -------------------------------------------- |
| Shell    | Tauri 2                                      |
| Backend  | Rust stable, edition 2021                    |
| Frontend | React 19, TypeScript 5.8, Vite 7             |
| Stili    | Tailwind CSS 4 (plugin Vite, nessun PostCSS) |

## Garanzie di privacy

Non sono buone intenzioni: sono vincoli verificabili nel codice.

1. **Nessuna crate di rete.** Nel grafo delle dipendenze Rust non esiste un
   client HTTP. Verificabile con `cargo tree`.
2. **Nessuna telemetria e nessun crash reporter.** Nessun identificativo di
   installazione viene generato o salvato.
3. **Nessun updater automatico.** `createUpdaterArtifacts` è disattivato e il
   plugin updater non è installato.
4. **CSP restrittiva.** `connect-src` è limitato al canale IPC locale: anche una
   `fetch` introdotta per errore nel frontend verrebbe bloccata dal webview.
   Vedi `app.security.csp` in `src-tauri/tauri.conf.json`.
5. **Nessun apri-collegamenti.** Il plugin `opener` incluso nel template è stato
   rimosso: gli URL trovati nei segnaposto di Drive sono mostrati come testo e
   non sono cliccabili.
6. **Permessi minimi.** La capability della finestra concede `core:default`,
   `dialog:allow-open` e `dialog:allow-save`, cioè i due selettori di sistema.
   Il frontend non ha accesso diretto al filesystem: ogni lettura e ogni
   scrittura passano da un comando Rust esplicito, e il percorso arriva sempre
   da una finestra scelta dall'utente.
7. **Nessuna persistenza implicita.** I risultati vivono in memoria per la
   durata della sessione. Ogni scrittura su disco nasce da un'azione esplicita:
   estrazione di un archivio, riparazione delle foto, export di contatti o
   calendario, quarantena di Drive.
8. **Nessuna cancellazione.** Nessuna funzione dell'applicazione elimina file.
   La pulizia di Drive costruisce un albero alternativo oppure sposta in
   quarantena scrivendo un registro che consente di annullare tutto.

Il comando `privacy_report` espone questa dichiarazione alla UI, che la mostra
nel badge "Offline" dell'intestazione.

## Struttura

```
src-tauri/
  deny.toml          il divieto di rete, in forma eseguibile
  fixtures/          JPEG reale usato dai test di scrittura EXIF
  src/
    lib.rs           composition root: stato, comandi, eventi, plugin
    app_state.rs     stato condiviso, errori, avanzamento, sezioni
    zip_handler.rs   serie di archivi, merge, protezione zip-slip
    exif_parser.rs   EXIF e sidecar, riconciliazione e riscrittura
    contacts.rs      parser vCard 3.0, deduplica, export
    calendar.rs      parser iCalendar, pulizia, export
    drive.rs         classificazione, segnaposto, deduplica e quarantena
                     (il motore di pulizia vale per qualsiasi cartella)

src/
  App.tsx              orchestrazione della sessione
  types.ts             controparte TypeScript delle struct serde
  lib/api.ts           unico punto di contatto con il backend (IPC)
  lib/format.ts        formattazioni condivise
  components/
    Dropzone.tsx       area di trascinamento su eventi nativi Tauri
    SourcePanel.tsx    riepilogo sorgente ed elenco sezioni
    Reports.tsx        viste dei report foto, contatti, calendario, Drive
    PhotoFixer.tsx     riparazione metadati con scelta della modalità
    ProgressBar.tsx    avanzamento alimentato dagli eventi del backend
    ExportButton.tsx   salvataggio dei file esportati
    FolderCleaner.tsx  pulizia di una cartella, con anteprima e annullamento
    Help.tsx           guida in-app e informazioni sulla licenza
    Welcome.tsx        presentazione all'avvio, una volta per sessione
    Stat.tsx           riquadro numerico
```

## Cosa fa oggi

- **Sorgente**: riconosce una cartella `Takeout/` estratta o un archivio
  `takeout-*.zip`, elenca le sezioni con conteggi e dimensioni.
- **Archivi**: dato un archivio qualsiasi ricostruisce l'intera serie
  (`takeout-...-001.zip`, `-002.zip`, ...) e la unisce in un solo albero,
  segnalando i numeri mancanti di un download incompleto. I percorsi vengono
  normalizzati e le voci che tentano di uscire dalla destinazione rifiutate.
- **Foto**: legge EXIF e sidecar JSON (compresi gli schemi
  `.supplemental-metadata.json` e i duplicati con contatore), riconcilia data e
  coordinate e **le riscrive nei tag EXIF** di JPEG, HEIC, TIFF e WebP senza
  ricomprimere l'immagine. Tre modalità: simulazione, copia riparata in un
  albero separato (predefinita) e riscrittura degli originali, che richiede una
  conferma esplicita.
- **Contatti**: parser vCard con line folding, prefissi di gruppo ed escaping,
  deduplica per email o telefono normalizzato, export in un vCard 3.0 standard.
- **Calendario**: parser iCalendar che non confonde gli allarmi con gli eventi,
  deduplica per UID e occorrenza, rimuove le proprietà `X-GOOGLE-*` ed esporta
  un `.ics` conforme, con line folding corretto.
- **Drive**: classificazione per categoria, rilevamento dei segnaposto
  `.gdoc`/`.gsheet` che non contengono dati, e pulizia con deduplica **per
  contenuto**: due file con lo stesso nome e la stessa dimensione ma contenuto
  diverso restano entrambi. Nessuna modalità cancella: o si costruisce un albero
  pulito altrove, o si sposta in quarantena con un registro che permette di
  rimettere tutto a posto con un clic. Quando un media viene rimosso il suo
  sidecar JSON lo segue, per non lasciare file orfani.
- **La pulizia vale per qualsiasi sezione**, non solo Drive: è disponibile anche
  su Google Foto, dove gli export contengono spesso lo stesso scatto duplicato
  perché presente in più album.
- **Guida integrata**, raggiungibile dal pulsante nell'intestazione e dal menu
  Aiuto, con una presentazione all'avvio per chi apre l'app la prima volta.

## Qualità

Ogni modifica passa da quattro controlli, eseguiti in CI:

| Controllo | Cosa garantisce |
| --- | --- |
| `cargo deny check` | nessuna crate di rete, telemetria o updater |
| `cargo clippy -- -D warnings` | zero warning |
| `cargo test` | 48 test, compresi end-to-end su Takeout sintetici |
| `npm run build` | tipi allineati alle struct serde |

I test non si fermano al "non è esploso". La riparazione EXIF viene verificata
rileggendo i tag scritti da un JPEG reale e confrontando le coordinate dopo il
round trip attraverso gradi, primi e secondi. La quarantena viene verificata
prendendo un'istantanea di percorsi e contenuti prima dell'operazione e
pretendendo che il ripristino la riproduca identica.

## Cosa non fa ancora

- **PNG**: escluso di proposito dalla riscrittura EXIF. Vedi
  [PRIVACY_AUDIT.md](PRIVACY_AUDIT.md), sezione 8.
- **Video**: i metadati stanno negli atomi del contenitore, non in EXIF. Per
  loro resta l'allineamento della data di modifica.
- **Mail e YouTube**: riconosciute nel riepilogo, senza analizzatore. Un `.mbox`
  di Gmail richiede un indice su disco, cioè un progetto a sé.
- Analisi diretta dentro l'archivio: le sezioni di uno ZIP vanno estratte prima.

## Sviluppo

Prerequisiti: Node 20 o superiore, toolchain Rust stable, Xcode Command Line
Tools su macOS.

```bash
npm install
npm run tauri dev
```

Altri comandi utili:

```bash
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo deny --manifest-path src-tauri/Cargo.toml check
```

## Distribuzione

I bundle di Tauri non si compilano da una piattaforma all'altra: da macOS non
escono `.msi` né `.deb`. La build locale produce il pacchetto del solo sistema
su cui gira:

```bash
npm run tauri build
```

Per tutte le piattaforme c'è `.github/workflows/release.yml`, che su un tag
`v*` costruisce macOS (universale), Linux (`.deb` e `.AppImage`) e Windows
(`.msi`, `.nsis` e l'eseguibile portatile) sui rispettivi runner. Il runner
Linux è fissato a Ubuntu 22.04: compilare su una distro più recente produce
pacchetti che non partono su quelle in LTS.

## Autore

Sviluppato da **SkapaCraft** ([skapacraft.com](https://skapacraft.com)).

## Licenza

Copyright (C) 2026 SkapaCraft. GPL-3.0-or-later, vedi [LICENSE](LICENSE).

La scelta è deliberata: la GPL impedisce che qualcuno prenda questo codice, ci
aggiunga telemetria e lo ridistribuisca come binario proprietario. Per
un'applicazione la cui unica promessa è "non ti sorveglia", una licenza
permissiva sarebbe una contraddizione.
