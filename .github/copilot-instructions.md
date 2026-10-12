# Istruzioni per Copilot: Nostos

App desktop local-first che elabora gli export di Google Takeout (foto, contatti, calendario, Drive) senza che un byte lasci il computer. Tauri 2 con backend Rust e frontend React 19 + TypeScript + Tailwind 4. Funzioni, struttura dei moduli e garanzie di privacy sono nel [README](../README.md) e in [PRIVACY_AUDIT.md](../PRIVACY_AUDIT.md).

## Struttura
- `src-tauri/`: backend Rust (`src/`), `deny.toml` (il divieto di rete in forma eseguibile), `fixtures/`
- `src/`: frontend React; `lib/api.ts` è l'unico punto di contatto con il backend, `lib/messages.ts` contiene i testi
- `tools/`: script per generare serie di archivi di prova
- `packaging/`: impacchettamento per le piattaforme

## Comandi
```bash
npm install
npm run tauri dev
npm run build                     # tsc + vite: tipi allineati alle struct serde
cd src-tauri
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo deny check                  # niente crate di rete, telemetria o updater
```
Le misure su librerie grandi sono test `#[ignore]` da lanciare a mano (comandi nel README, sezione Quality).

## Regole del progetto
- L'app non apre connessioni di rete, mai. `deny.toml` vieta per nome crate HTTP, TLS, telemetria e i plugin Tauri http, updater, opener e shell; la CI rifiuta anche `fetch(`, `XMLHttpRequest`, `WebSocket`, `EventSource` nel frontend. Se un controllo fallisce, la soluzione non è allentarlo: prima di aggiungere una dipendenza, `cargo deny check`.
- Non allentare CSP, capability della finestra (solo `core:default` e i due selettori di file) o l'assenza di updater: se una modifica lo richiede, va discussa prima di scrivere codice.
- Nessuna funzione cancella file, e il default non è mai distruttivo. Pulizia e riparazione costruiscono un albero alternativo, spostano in quarantena con registro reversibile o riscrivono sul posto solo dopo conferma e controllo dello spazio. Un export è spesso l'unica copia delle foto.
- La deduplica è per contenuto (BLAKE3), mai per nome e dimensione: negli export reali esistono file uguali per dimensione e diversi per contenuto.
- PNG resta escluso dalla scrittura EXIF: in `little_exif` il parser XML vulnerabile è raggiungibile solo da quel percorso. Il test `png_resta_fuori_dalla_scrittura_exif` protegge l'eccezione in `deny.toml`.
- `reqwest` compare in `Cargo.lock` ma non è compilato (dipendenza opzionale di `tauri` mai attivata): non è una violazione, e non va "corretto".
- Il backend Rust non compone frasi: restituisce codici e numeri, i testi stanno in `src/lib/messages.ts` e nei componenti. Le traduzioni sono incluse nel pacchetto, mai scaricate.
- Ogni modifica rilevante ha una voce in `CHANGELOG.md` (Keep a Changelog, SemVer). La versione vive in `src-tauri/Cargo.toml` e `src-tauri/tauri.conf.json`.
- Un tag `v*` avvia `release.yml` su tre piattaforme. I minuti di GitHub Actions dell'account sono contati e la CI di Nostos ne consuma la maggior parte: niente esecuzioni inutili.
- Niente file generati o di pacchetto nei commit (`dist/`, `dist-release/`, `.pkg`, `.dmg`) se il progetto non li traccia già.
- Niente credenziali, chiavi o dati personali, nemmeno nei fixture di test: i test usano alberi Takeout sintetici.

## Regole generali
- Rispondi in italiano. Codice, commenti, messaggi di commit e testi dell'app in inglese, come da CONTRIBUTING (l'interfaccia oggi è ancora in italiano; l'inglese è il default previsto).
- Commit, push, tag e release solo se Daniele lo chiede. Mai righe `Co-Authored-By` o riferimenti a Copilot/Claude/AI nei commit o nei file del repo: un hook rifiuta il commit.
- Non eseguire comandi git che riscrivono i file (`checkout`, `restore`, `reset`, `stash`, `clean`): possono cancellare lavoro non committato.
- Niente trattino lungo né medio, puntino centrale, frecce o tilde per "circa" nei testi; niente emoji nel codice e nei testi.
- Non leggere, stampare o committare segreti (`.env`, certificati e chiavi di firma, `.p8`, `.pem`, profili di provisioning).
- Numero di versione cambia solo quando Daniele lo chiede, insieme alla voce di CHANGELOG.
- Se un'informazione manca o non hai accesso a qualcosa, dillo: non inventare.
