# Audit privacy e sicurezza

Questo documento descrive l'architettura offline di Open Takeout Hub e come
verificarla da soli. Non chiede di fidarsi: ogni affermazione ha accanto il
comando che la conferma o la smentisce.

Ultima verifica: 2026-08-05, su commit iniziale.

## 1. Il vincolo

Open Takeout Hub tratta gli archivi che una persona scarica proprio perché
vuole riprendersi i propri dati. Se l'applicazione contattasse un server,
qualunque server, per qualunque motivo, vanificherebbe il gesto. Da qui una
regola sola, che vince su ogni altra considerazione di comodità:

> Il processo dell'applicazione non apre connessioni di rete.

## 2. Come il vincolo è reso esecutivo

Una promessa scritta in un README invecchia al primo `cargo add`. Per questo il
vincolo è codificato in `src-tauri/deny.toml`, sezione `[bans]`, ed eseguito in
CI dal job `Vincolo local-first`.

Sono vietate per nome le crate di rete (`reqwest`, `hyper`, `ureq`, `curl`,
`axum`, `tungstenite`, `quinn`), gli stack TLS (`rustls`, `native-tls`,
`openssl`), la telemetria (`sentry`, `opentelemetry`) e i plugin Tauri che
aprirebbero superfici verso l'esterno (`tauri-plugin-http`,
`tauri-plugin-updater`, `tauri-plugin-opener`, `tauri-plugin-shell`).

Verifica:

```bash
cargo deny --manifest-path src-tauri/Cargo.toml check
```

Il divieto è stato provato al contrario: aggiungendo `reqwest` alle dipendenze,
`cargo deny check bans` fallisce segnalando tre crate vietate (`reqwest`,
`hyper`, `hyper-util`). Non è un controllo decorativo.

Sul lato frontend la CI rifiuta `fetch(`, `XMLHttpRequest`, `WebSocket` ed
`EventSource` nei sorgenti. La CSP li bloccherebbe comunque a runtime, ma una
build che fallisce è una diagnosi migliore di un errore in console.

## 3. Una cosa che sembra una violazione e non lo è

**`reqwest` compare in `src-tauri/Cargo.lock`.** Chi fa un audit veloce lo trova
con un `grep` e conclude che l'app telefona a casa. Non è così, ed è giusto
spiegare perché invece di lasciare il dubbio.

`tauri` dichiara `reqwest` come dipendenza **opzionale**, attivata solo dalle
feature `native-tls` e `rustls-tls`, che questo progetto non abilita. Il
lockfile registra l'intero universo delle dipendenze risolvibili, comprese
quelle mai compilate.

Le tre verifiche indipendenti:

```bash
# 1. Non è nel grafo delle dipendenze attive: stampa "nothing to print".
cargo tree --manifest-path src-tauri/Cargo.toml --edges normal -i reqwest

# 2. cargo-deny valuta le feature realmente attive, e infatti passa.
cargo deny --manifest-path src-tauri/Cargo.toml check bans

# 3. Il binario compilato non contiene un solo simbolo di rete.
nm -aj src-tauri/target/debug/open-takeout-hub | grep -ci reqwest   # 0
nm -u  src-tauri/target/debug/open-takeout-hub | grep -c '^_socket$' # 0
```

L'ultimo è il più difficile da aggirare: il binario non importa nemmeno
`socket`, `connect` o `getaddrinfo` dalla libreria di sistema.

## 4. Perimetro onesto: il webview

Una dichiarazione di questo tipo sarebbe disonesta se si fermasse al codice
Rust. L'applicazione incorpora il webview di sistema (WKWebView su macOS,
WebKit2GTK su Linux, WebView2 su Windows), che è un componente del sistema
operativo e, in astratto, sa parlare in rete.

A contenerlo c'è la Content Security Policy dichiarata in
`src-tauri/tauri.conf.json`, che in produzione vale:

```
default-src 'self'; script-src 'self'; connect-src 'self' ipc: http://ipc.localhost;
object-src 'none'; frame-src 'none'; form-action 'none'
```

`connect-src` ammette solo il canale IPC locale verso il backend Rust. Non
esistono origini remote consentite, `form-action` è vietata e non c'è alcun
frame. Il contenuto caricato è esclusivamente quello impacchettato nel bundle:
nessun CDN, nessun font remoto, nessuna immagine esterna.

In sviluppo vale una `devCsp` separata che riapre soltanto
`ws://localhost:1420` per l'hot reload di Vite. Non è la policy che finisce nel
binario distribuito.

## 5. Superficie concessa al frontend

Il codice React non ha accesso diretto al filesystem. La capability della
finestra, in `src-tauri/capabilities/default.json`, concede tre sole voci:

| Permesso | Perché |
| --- | --- |
| `core:default` | eventi, finestra, drag & drop, menu |
| `dialog:allow-open` | selettore file e cartelle |
| `dialog:allow-save` | selettore di salvataggio per gli export |

Ogni lettura e ogni scrittura passano da un comando Rust esplicito e nominato.
Non esiste un permesso che consenta al frontend di leggere un percorso
arbitrario, aprire un URL o eseguire un processo a sua scelta.

## 6. Dati scritti su disco

| Cosa | Dove | Quando |
| --- | --- | --- |
| Risultati delle analisi | memoria del processo | fino alla chiusura |
| File estratti da un archivio | cartella scelta dall'utente | solo su azione esplicita |
| Date di modifica dei media | file originali | solo su azione esplicita |
| File spostati in quarantena | cartella scelta dall'utente | solo su azione esplicita |
| Registro della quarantena | dentro la quarantena stessa | insieme allo spostamento |
| `preferences.json` | cartella di configurazione di sistema | solo se spunti "non mostrare più" |

### L'unico file che l'app scrive per sé

`preferences.json`, nella cartella di configurazione del sistema, contiene un
solo campo booleano:

```json
{ "hideWelcome": true }
```

Serve a ricordare che hai spuntato "non mostrare più" nella presentazione
iniziale. Viene creato **solo** se spunti quella casella: se non la tocchi, il
file non esiste.

Non contiene percorsi, non contiene cronologie, non contiene identificativi. La
struttura in `app_state.rs` ha un campo solo, e ogni campo aggiunto in futuro va
dichiarato qui: è il motivo per cui il commento sopra quella struct lo dice
esplicitamente.

Non vengono scritti cache, log su disco, cronologie dei percorsi aperti né
identificativi di installazione.

La diagnostica di sviluppo va su stderr ed è racchiusa in
`#[cfg(debug_assertions)]`: nel binario distribuito quelle righe non esistono.
Non è un logger su file, di proposito.

Il registro della quarantena viene scritto dentro la cartella che l'utente ha
appena scelto, insieme ai file spostati. Contiene i loro percorsi originali, e
senza di esso l'operazione non sarebbe annullabile.

## 7. Altri assenti

- **Nessun updater automatico.** `createUpdaterArtifacts` è disattivato e il
  plugin non è installato. Gli aggiornamenti si scaricano a mano dalle release.
- **Nessun crash reporter.** Un panic resta sulla macchina.
- **Nessun link cliccabile verso l'esterno.** Gli URL trovati nei segnaposto di
  Google Drive sono mostrati come testo. Aprirli significherebbe una
  connessione verso Google, ed è una decisione che spetta all'utente, fuori da
  questa applicazione.

## 7-bis. L'unica azione verso il sistema operativo

Il pulsante "Mostra nel Finder" invoca un programma esterno: `open -R` su macOS,
`explorer /select,` su Windows, `xdg-open` sulla cartella su Linux.

È l'unico punto in cui l'applicazione esce verso il sistema, e i vincoli sono
stretti apposta:

- il programma invocato è **fisso nel codice**, non è una stringa che qualcuno
  possa influenzare;
- l'unico argomento è un percorso che deve **già esistere** e che viene
  canonicalizzato prima dell'uso;
- non passa da una shell, quindi non esiste iniezione di comandi;
- su Linux si apre la **cartella** e non il file, perché `xdg-open` su un file
  lo aprirebbe con l'applicazione predefinita, che è un'altra cosa dal mostrarlo.

`tauri-plugin-opener` e `tauri-plugin-shell` restano vietati in `deny.toml`: il
primo sa aprire anche URL nel browser, il secondo eseguire comandi arbitrari.
Rivelare una cartella nel gestore file è un'azione locale e non comporta alcuna
connessione, quindi non intacca la promessa del punto 1.

## 8. Avvisi noti e accettati

`cargo deny check advisories` tratta le vulnerabilità come errore bloccante,
mentre gli avvisi di tipo *unmaintained* valgono solo per le dipendenze dirette
(`unmaintained = "workspace"`).

La ragione: quindici avvisi *unmaintained* arrivano dalle binding GTK3, che
Tauri usa obbligatoriamente su Linux, e da crate interne di `tauri-utils`. Non
descrivono falle sfruttabili e non esiste un aggiornamento sicuro. Elencarli a
mano produrrebbe una lista da rinnovare a ogni release di Tauri, e una lista che
si aggiorna per abitudine prima o poi copre anche l'avviso che conta.

### Due vulnerabilità con eccezione motivata

`quick-xml` 0.37.5, tirato dentro da `little_exif` (la libreria che scrive i tag
EXIF), ha due denial of service noti: RUSTSEC-2026-0194 (tempo quadratico su
attributi duplicati) e RUSTSEC-2026-0195 (allocazione illimitata di
dichiarazioni di namespace). Non esiste una 0.37.x corretta e `little_exif` non
espone una feature per disattivare l'XMP.

Sono elencate in `ignore` per una ragione verificata, non per comodità: dentro
`little_exif` quel parser XML è usato solo da `xmp.rs`, che è raggiungibile solo
dal percorso di scrittura PNG. Questo progetto esclude PNG da
`EXIF_WRITABLE_EXTENSIONS`, quindi il codice vulnerabile non viene mai eseguito.

L'esclusione non è lasciata alla memoria di chi scriverà il prossimo commit: il
test `png_resta_fuori_dalla_scrittura_exif` fallisce se qualcuno aggiunge PNG
all'elenco, e il suo messaggio rimanda a questa eccezione.

L'eccezione va rimossa quando `little_exif` passerà a `quick-xml` 0.41.

A parte queste due, nessuna vulnerabilità nota è presente al momento della
verifica.

## 9. Rifare l'audit

```bash
cargo deny --manifest-path src-tauri/Cargo.toml check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
npm audit --audit-level=high
```

Se una di queste fallisce, la promessa di questo documento non vale più.
