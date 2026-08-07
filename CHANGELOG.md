# Diario delle modifiche

Il formato segue [Keep a Changelog](https://keepachangelog.com/it-IT/1.1.0/) e la
numerazione [Semantic Versioning](https://semver.org/lang/it/).

## Non rilasciato

Nulla è stato ancora pubblicato: la `0.1.0` qui sotto descrive lo stato del
codice, non un binario distribuito. Manca la firma con un certificato Apple
Developer ID, senza la quale Gatekeeper rifiuta il pacchetto macOS.

## [0.1.0] non ancora pubblicata

Prima versione completa dell'applicazione.

### Sorgenti e archivi

- Riconoscimento di una cartella `Takeout/` estratta o di un archivio
  `takeout-*.zip`, con elenco delle sezioni, conteggi e dimensioni.
- Ricostruzione dell'intera serie a partire da un archivio qualsiasi
  (`takeout-...-001.zip`, `-002.zip`, ...) e unione in un solo albero, con
  segnalazione dei numeri mancanti in un download incompleto.
- Protezione zip-slip: i percorsi vengono normalizzati e le voci che tentano di
  uscire dalla destinazione sono scartate invece di essere scritte.

### Foto

- Lettura di EXIF e sidecar JSON, compresi gli schemi
  `.supplemental-metadata.json`, i duplicati con contatore e i nomi che Google
  accorcia a 46 caratteri: su un nome lungo il suffisso arriva mozzato
  (`.supplemental-m.json`) o sparisce del tutto, lasciando troncato il nome del
  media.
- Deduzione della data dal nome generato dalla fotocamera quando EXIF e sidecar
  mancano entrambi (`IMG_20200101_120000`, `PXL_...`, screenshot, Signal).
- Riscrittura di data e coordinate nei tag EXIF di JPEG, HEIC, TIFF e WebP senza
  ricomprimere l'immagine.
- **Ora locale del luogo invece dell'istante universale.** Quando la foto ha le
  coordinate, il fuso viene ricavato in locale e scritto insieme al suo scarto,
  tenendo conto dell'ora legale in vigore quel giorno. Scrivere l'istante UTC
  così com'è avrebbe spostato ogni foto della differenza di fuso.
- Disposizione dell'uscita a scelta: struttura originale, per anno, per anno e
  mese, o cartella unica. I file senza data finiscono in `senza-data/` invece di
  essere infilati in un mese inventato.
- Tre modalità: simulazione, copia riparata in un albero separato (predefinita) e
  riscrittura degli originali, che richiede una conferma esplicita.

### Album

- Riconoscimento degli album di Google Foto, che l'export non registra come
  informazione a parte ma come cartelle contenenti una seconda copia della foto.
- Distinzione fra cartelle per anno e album veri in qualsiasi lingua
  dell'account, ricavata dall'export stesso e non da un elenco di traduzioni:
  il prefisso delle annate (`Photos from`, `Foto da`, `Fotos de`) è identico per
  tutte le annate di uno stesso export, quindi si può dedurre invece di
  indovinarlo. Quando due prefissi diversi compaiono lo stesso numero di volte
  la distinzione non è possibile, e l'app lo dichiara invece di scegliere a
  caso.
- Manifest dell'appartenenza scrivibile su file. Finché non esiste, la deduplica
  sulla cartella foto resta bloccata: dalla quarantena i file tornano indietro,
  l'appartenenza a un album no.
- Riconoscimento delle versioni modificate (`-edited`, `-modificato`,
  `-modifié`, `-編集済み` e altre dodici lingue), che non vengono trattate come
  duplicati.

### Contatti e calendario

- Parser vCard con line folding, prefissi di gruppo ed escaping, deduplica per
  email o telefono normalizzato, export in vCard 3.0 standard.
- Parser iCalendar che non confonde gli allarmi con gli eventi, deduplica per
  UID e occorrenza, rimozione delle proprietà `X-GOOGLE-*`, export in un `.ics`
  conforme.

### Drive e pulizia

- Classificazione per categoria e rilevamento dei segnaposto `.gdoc`/`.gsheet`,
  che non contengono i dati ma un rimando.
- Deduplica **per contenuto**: due file con lo stesso nome e la stessa dimensione
  ma contenuto diverso restano entrambi.
- Nessuna modalità cancella: o si costruisce un albero pulito altrove, o si
  sposta in quarantena scrivendo un registro che permette di rimettere tutto a
  posto con un clic.
- Quando un media viene rimosso il suo sidecar JSON lo segue, per non lasciare
  file orfani.
- Il motore di pulizia vale per qualsiasi sezione, non solo Drive.

### Spazio su disco

- Conto dello spazio necessario prima di cominciare, con rifiuto dell'operazione
  se non ce n'è abbastanza, invece di riempire il disco a metà lavoro e lasciare
  un albero di uscita che sembra completo.
- Quando lo spazio manca l'app non si limita a dirlo: propone la riscrittura sul
  posto, che richiede poche decine di megabyte qualunque sia la libreria.
- Elenco delle sottocartelle che entrano nello spazio rimasto, con un pulsante
  per ripararne una per volta.
- Nell'elenco delle tranche annate e album sono distinti, e le cartelle che
  contengono file presenti solo lì vengono segnalate: non si possono rimandare
  senza perderli di vista.

### Interfaccia

- Guida integrata, raggiungibile dal pulsante nell'intestazione e dal menu Aiuto.
- Presentazione all'avvio con la casella "non mostrare più", unico dato che
  sopravvive alla sessione.
- Menu dell'applicazione con voci esplicite, perché in sviluppo macOS mostra
  altrimenti il nome dell'eseguibile.

### Privacy

- `deny.toml` che vieta le crate di rete, telemetria e updater, verificato in CI:
  aggiungere un client HTTP fa fallire la compilazione.
- CSP con `connect-src` limitato al canale IPC locale.
- Capability della finestra ridotta a `core:default` e ai due selettori di
  sistema. Il frontend non ha accesso diretto al filesystem.
- Nessun collegamento cliccabile in tutta l'interfaccia: gli indirizzi, compresi
  quelli dei segnaposto di Drive, sono testo selezionabile.
- `PRIVACY_AUDIT.md` con la verifica sul bundle di release.

### Verifiche

- 69 test, compresi end-to-end su Takeout sintetici. La riparazione EXIF viene
  verificata rileggendo i tag da un JPEG reale e confrontando le coordinate dopo
  il round trip attraverso gradi, primi e secondi. La quarantena viene verificata
  prendendo un'istantanea di percorsi e contenuti prima dell'operazione e
  pretendendo che il ripristino la riproduca identica.
- Quattro misure escluse dalla CI, da lanciare a mano: libreria da centomila
  foto, percorso dei byte veri, rubrica e calendario di grandi dimensioni,
  estrazione di una serie multi-archivio presa dal disco.
- `tools/genera_serie_takeout.py` costruisce il materiale per quest'ultima:
  quindici gigabyte divisi in otto archivi, con le stranezze note dell'export.
  Serve perché un test che genera i propri dati verifica anche le proprie
  assunzioni, e se un'assunzione è sbagliata resta verde lo stesso.
- `cargo clippy -- -D warnings`, `cargo deny check` e `npm run build` in CI.

### Correzioni durante lo sviluppo

- Le versioni modificate con nome in forma decomposta (NFD, come li scrive
  macOS) venivano troncate nel punto sbagliato, producendo nomi come
  `IMG_1-.jpg`. Il punto di taglio ora si cerca sulla stringa originale.
- I nomi Pixel con i millisecondi (`PXL_20200101_120000123`) venivano rifiutati
  dal riconoscimento della data.
- La ricerca dei file affiancati era quadratica sul numero di file: su centomila
  foto passava da 411 secondi a 0,7.
- I file in formato non supportato dalla riscrittura EXIF smettevano di essere
  copiati nell'albero riparato.
- I sidecar con il nome accorciato da Google non venivano riconosciuti, e la
  foto finiva per prendere la data dal proprio nome: una risorsa peggiore, che
  non porta le coordinate. Su una serie di prova da quindici gigabyte erano 87
  foto su 3237.
- Un album chiamato `Natale 2024` veniva scambiato per una cartella per anno, e
  la sua appartenenza non finiva nel manifest, cioè si perdeva proprio il dato
  che il manifest esiste per salvare.

Gli ultimi due sono emersi dalla misura su una serie multi-archivio presa dal
disco, e non dai test sintetici: quelli generavano il materiale con le stesse
assunzioni che stavano verificando.
