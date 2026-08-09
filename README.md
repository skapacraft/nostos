# Nostos

Applicazione desktop local-first per elaborare gli export di Google Takeout.
Analizza foto, contatti e Drive senza che un solo byte lasci il computer.

## Perché

Un Takeout è un archivio grezzo e poco navigabile: le foto perdono l'EXIF e
portano la data in un sidecar JSON, i contatti arrivano come vCard con
duplicati, Drive contiene segnaposto che non includono il contenuto. Gli
strumenti online che risolvono questi problemi chiedono di caricare l'intero
export su un server di terze parti, cioè esattamente i dati che si stava
cercando di riprendere in mano.

Nostos fa lo stesso lavoro in locale.

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
8. **Controllo dello spazio prima di scrivere.** La copia riparata duplica la
   libreria: su un export da sessanta gigabyte ne servono altrettanti.
   L'operazione viene rifiutata prima di cominciare se non ce n'è abbastanza,
   invece di riempire il disco a metà lavoro e lasciare un albero di uscita
   che sembra completo. Quando non ci sta, l'app non si limita a dirlo: propone
   la riscrittura sul posto, che richiede poche decine di megabyte qualunque
   sia la libreria, ed elenca le sottocartelle che entrano nello spazio
   rimasto, con un pulsante per ripararne una per volta.
9. **Nessuna cancellazione.** Nessuna funzione dell'applicazione elimina file.
   La pulizia di Drive costruisce un albero alternativo oppure sposta in
   quarantena scrivendo un registro che consente di annullare tutto. Vale anche
   per i sidecar rimasti indietro dopo una riparazione: vengono spostati, mai
   rimossi, e solo dopo aver riletto il file per accertarsi che il loro
   contenuto ci sia davvero.

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
    albums.rs        album, cartelle per anno, versioni modificate
    drive.rs         classificazione, segnaposto, deduplica e quarantena
                     (il motore di pulizia vale per qualsiasi cartella)

tools/
  genera_serie_takeout.py   costruisce una serie multi-archivio di prova

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
    AlbumPanel.tsx     album, manifest dell'appartenenza, versioni modificate
    FolderCleaner.tsx  pulizia di una cartella, con anteprima e annullamento
    Help.tsx           guida in-app e informazioni sulla licenza
    SidecarSweep.tsx   mette da parte i JSON il cui contenuto è nei file
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
- **Album**: Google non esporta gli album come informazione a parte, ma come
  cartelle contenenti una seconda copia della foto. L'app le riconosce e
  distingue le cartelle per anno dagli album veri in qualsiasi lingua, senza
  un elenco di traduzioni: ricava dall'export stesso il prefisso con cui quel
  export chiama le annate (`Photos from`, `Foto da`, `Fotos de`), così un album
  chiamato `Natale 2024` resta un album. Poi
  scrive un manifest dell'appartenenza. Finché quel manifest non esiste, la
  deduplica sulla cartella foto resta bloccata: i file tornerebbero dalla
  quarantena, l'appartenenza no.
- **Foto**: legge EXIF e sidecar JSON (compresi gli schemi
  `.supplemental-metadata.json`, i duplicati con contatore e i nomi che Google
  accorcia a 46 caratteri, dove il suffisso arriva mozzato o sparisce del
  tutto), e quando entrambi mancano deduce la data dal nome generato dalla
  fotocamera
  (`IMG_20200101_120000`, `PXL_...`, screenshot, Signal). **Riscrive nei tag EXIF** di
  JPEG, HEIC, TIFF e WebP, senza ricomprimere l'immagine, tutto ciò che il
  sidecar contiene e che ha una sede nei metadati: data, coordinate,
  descrizione (`ImageDescription`), volti riconosciuti (`XPKeywords`) e la
  stella dei preferiti (`Rating`). Restano fuori solo il conteggio delle
  visualizzazioni e l'indirizzo su Google Foto, che nei metadati non hanno dove
  stare: l'app lo dichiara invece di lasciarlo scoprire.
  Quando la foto ha le coordinate, ricava il fuso del
  luogo e scrive l'ora locale corretta con il suo scarto, tenendo conto
  dell'ora legale: `DateTimeOriginal` è l'ora dell'orologio sul posto, non
  l'ora universale, e scriverci dentro un istante UTC sposterebbe ogni foto.
  La copia riparata può conservare la struttura
  originale oppure essere riorganizzata per anno, per anno e mese, o in una
  cartella sola; i file senza data finiscono in `senza-data/` invece di essere
  infilati in un mese inventato. Riconosce le versioni modificate
  (`-edited`, `-modificato`, `-modifié`, `-編集済み` e altre) e non le tratta
  come duplicati. Tre modalità: simulazione, copia riparata in un
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
- **Sidecar messi da parte a riparazione conclusa**: dopo una riscrittura degli
  originali i `.json` restano nella cartella, e l'app propone di spostarli.
  Sposta solo quelli che non sono più l'unica copia di qualcosa, e non si fida
  di quanto ha riferito la riparazione: rilegge ogni file per verificare che il
  dato ci sia davvero. Restano dove sono quelli di PNG, GIF e video, quelli
  delle foto non riparate e quelli che portano dati senza una sede nei tag.
  Non è una cancellazione: scrive lo stesso registro della quarantena e si
  annulla con un clic.
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
| `cargo test` | 71 test, compresi end-to-end su Takeout sintetici |
| `npm run build` | tipi allineati alle struct serde |

Ci sono inoltre cinque misure escluse dalla CI, da lanciare a mano. La prima
lavora su una libreria grande, perché genera decine di migliaia di file:

```bash
FOTO=100000 cargo test --release --manifest-path src-tauri/Cargo.toml \
  misura_su_libreria_grande -- --ignored --nocapture
```

Non serve un export da cento gigabyte per trovare i guasti di scala: quello che
mette in difficoltà il codice è il numero di file, non di byte. Centomila foto
sintetiche occupano centocinquanta megabyte e sono una prova più severa di una
libreria reale della stessa consistenza.

Il percorso dei byte veri ha una misura a parte, che scrive qualche gigabyte:

```bash
GB=2 /usr/bin/time -l cargo test --release --manifest-path src-tauri/Cargo.toml \
  misura_su_file_grandi -- --ignored --nocapture
```

Una terza misura copre contatti e calendario, che hanno il profilo opposto:
pochissimi file ma grandi, letti interamente in memoria.

```bash
CONTATTI=20000 EVENTI=50000 cargo test --release --manifest-path src-tauri/Cargo.toml \
  misura_su_rubrica_grande -- --ignored --nocapture
```

La misura sui byte verifica la velocità di deduplica e riparazione, e soprattutto che un file
oltre la soglia di riscrittura venga saltato ma copiato lo stesso. Su due
gigabyte e mezzo di media la memoria allocata resta intorno ai cento megabyte
(`peak memory footprint`; il `maximum resident set size` comprende la cache
delle pagine dei file e non misura ciò che alloca il programma).

La quarta misura estrae una serie multi-archivio presa dal disco, invece di
costruirsela da sé. La differenza non è formale: un test che genera i propri
dati verifica anche le proprie assunzioni, e se un'assunzione è sbagliata resta
verde lo stesso. Il materiale si prepara con lo script in `tools/`:

```bash
tools/genera_serie_takeout.py ~/Downloads/prova-multiarchivio

SERIE=~/Downloads/prova-multiarchivio USCITA=~/Downloads/prova-estratta \
  cargo test --release --manifest-path src-tauri/Cargo.toml \
  estrazione_di_una_serie_reale -- --ignored --nocapture
```

Su quindici gigabyte divisi in otto archivi, come li produce Google con
l'opzione "2 GB": serie riconosciuta partendo da un archivio solo in 0,1 ms,
estrazione dei 6330 file in 40 secondi a 383 MB/s senza collisioni, scansione
delle 3237 foto in 1,4 secondi, memoria allocata 107 MB.

La quinta ripara una cartella vera e poi mette da parte i sidecar applicati,
lavorando su una copia così che l'originale resti intatto:

```bash
CARTELLA="~/Downloads/prova-estratta/Takeout/Google Foto/Foto da 2019" \
  cargo test --release --manifest-path src-tauri/Cargo.toml \
  ripara_e_mette_da_parte_i_sidecar -- --ignored --nocapture
```

Lo script non produce un Takeout di Google: riproduce la struttura, la
nomenclatura, la divisione a fette e le stranezze note dell'export, ma non le
scelte del suo scrittore zip. Quelle si verificano solo con un export vero,
chiesto a Google con la dimensione massima per archivio impostata bassa.
Vale la pena dirlo perché questa misura ha già trovato due difetti reali che i
test sintetici non vedevano: i sidecar con il nome accorciato da Google non
venivano riconosciuti, e un album chiamato `Natale 2024` finiva fra le cartelle
per anno.

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

Su macOS il passaggio finale, quello che impagina la finestra del `.dmg`, usa
AppleScript per parlare con il Finder. Va lanciato da un terminale autorizzato a
inviare Apple Events: da un processo che non lo è fallisce con l'errore -1743
dopo aver comunque prodotto il `.app`, che resta utilizzabile.

Per tutte le piattaforme c'è `.github/workflows/release.yml`, che su un tag
`v*` costruisce macOS (universale), Linux (`.deb` e `.AppImage`) e Windows
(`.msi`, `.nsis` e l'eseguibile portatile) sui rispettivi runner. Il runner
Linux è fissato a Ubuntu 22.04: compilare su una distro più recente produce
pacchetti che non partono su quelle in LTS.

## Attribuzioni

I confini dei fusi orari provengono da [OpenStreetMap](https://www.openstreetmap.org/copyright),
distribuiti dal pacchetto `tzf-dist` sotto
[Open Database License](https://opendatacommons.org/licenses/odbl/) (ODbL-1.0).
I dati sono inclusi nell'applicazione e consultati in locale.

## Autore

Sviluppato da **SkapaCraft** ([skapacraft.com](https://skapacraft.com)).

## Licenza

Copyright (C) 2026 SkapaCraft. GPL-3.0-or-later, vedi [LICENSE](LICENSE).

La scelta è deliberata: la GPL impedisce che qualcuno prenda questo codice, ci
aggiunga telemetria e lo ridistribuisca come binario proprietario. Per
un'applicazione la cui unica promessa è "non ti sorveglia", una licenza
permissiva sarebbe una contraddizione.
