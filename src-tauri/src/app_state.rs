// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Stato applicativo condiviso e tipi comuni.
//!
//! Tutto lo stato vive in memoria per la durata della sessione: nessuna
//! persistenza implicita, nessun file di configurazione scritto di nascosto,
//! nessun identificativo di installazione generato.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errore unico propagato ai comandi Tauri.
#[derive(Debug, Error)]
pub enum TakeoutError {
    #[error("errore di I/O su {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("archivio non valido: {0}")]
    Archive(String),

    #[error("voce di archivio non sicura (path traversal): {0}")]
    UnsafeEntry(String),

    #[error("metadati non interpretabili: {0}")]
    Metadata(String),

    #[error("percorso non trovato: {0}")]
    NotFound(PathBuf),

    #[error("nessuna sorgente Takeout caricata")]
    NoSource,

    #[error(
        "spazio insufficiente sulla destinazione: servono {} ma ne restano {}",
        crate::app_state::formatta_byte(*needed),
        crate::app_state::formatta_byte(*available)
    )]
    NotEnoughSpace { needed: u64, available: u64 },

    #[error("elaborazione in background interrotta: {0}")]
    Task(String),

    #[error("stato interno corrotto: lock avvelenato")]
    Poisoned,
}

impl TakeoutError {
    /// Costruisce un errore di I/O conservando il percorso che lo ha causato.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

// I comandi Tauri richiedono un errore serializzabile: lo appiattiamo a stringa
// leggibile, senza esporre stack o dettagli interni al frontend.
impl Serialize for TakeoutError {
    // `Result` in questo modulo è l'alias di crate, quindi qui serve la forma
    // completa di quello standard.
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, TakeoutError>;

/// Diagnostica di sviluppo, stampata sul terminale di `tauri dev`.
///
/// Volutamente non è un logger su file: scrivere log su disco
/// contraddirebbe la sezione 6 di `PRIVACY_AUDIT.md`, che promette di non
/// lasciare traccia della sessione. Qui l'output va su stderr e l'intero blocco
/// viene compilato via nelle build di release, quindi nel binario distribuito
/// queste righe non esistono.
macro_rules! trace_dev {
    ($($arg:tt)*) => {{
        #[cfg(debug_assertions)]
        {
            eprintln!("[oth] {}", format_args!($($arg)*));
        }
    }};
}

pub(crate) use trace_dev;

/// Dati identificativi dell'applicazione, mostrati nella guida.
///
/// I valori arrivano dalle variabili che Cargo espone a compilazione: sono le
/// stesse di `Cargo.toml`, quindi non possono divergere dai metadati del
/// pacchetto distribuito.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub author: String,
    pub homepage: String,
    pub repository: String,
    pub license: String,
}

impl Default for AppInfo {
    fn default() -> Self {
        Self {
            name: "Open Takeout Hub".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            author: env!("CARGO_PKG_AUTHORS").to_string(),
            homepage: env!("CARGO_PKG_HOMEPAGE").to_string(),
            repository: env!("CARGO_PKG_REPOSITORY").to_string(),
            license: env!("CARGO_PKG_LICENSE").to_string(),
        }
    }
}

/// Le uniche preferenze che l'applicazione conserva tra un avvio e l'altro.
///
/// Il file contiene solo questo campo, ed è l'unica eccezione alla regola di
/// non scrivere nulla di implicito. La struttura resta volutamente minima: ogni
/// campo aggiunto qui è un dato in più che sopravvive alla sessione e va
/// dichiarato in `PRIVACY_AUDIT.md`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Preferences {
    /// L'utente ha chiesto di non rivedere la presentazione all'avvio.
    pub hide_welcome: bool,
}

/// Esito della scrittura di un file esportato.
///
/// Condiviso da contatti e calendario: entrambi producono un singolo file
/// standard pronto per essere importato altrove.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub path: PathBuf,
    /// Elementi scritti nel file.
    pub written: usize,
    pub bytes: u64,
}

/// Fase di un'operazione lunga.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Scanning,
    Extracting,
    Writing,
    Done,
}

/// Avanzamento inviato alla UI durante un'operazione lunga.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: Phase,
    pub done: usize,
    pub total: usize,
    pub errors: usize,
    /// Nome del file in lavorazione, senza il percorso completo.
    pub current: Option<String>,
}

impl Progress {
    pub fn new(phase: Phase, done: usize, total: usize, errors: usize) -> Self {
        Self {
            phase,
            done,
            total,
            errors,
            current: None,
        }
    }

    pub fn with_current(mut self, current: impl Into<String>) -> Self {
        self.current = Some(current.into());
        self
    }
}

/// Canale di avanzamento passato ai moduli di dominio.
///
/// I moduli non conoscono Tauri: ricevono una chiusura e non sanno se dietro ci
/// sia un evento verso il webview, un contatore in un test o nulla. `Send` e
/// `Sync` servono perché l'elaborazione delle foto gira su più thread.
pub type ProgressSink<'a> = &'a (dyn Fn(Progress) + Send + Sync);

/// Sink che scarta tutto, per i chiamanti che non mostrano avanzamento.
pub fn no_progress(_: Progress) {}

/// Utilità condivise dai test dei vari moduli.
#[cfg(test)]
pub(crate) mod testing {
    use std::path::{Path, PathBuf};

    /// Cartella temporanea che si cancella da sola a fine test.
    pub(crate) struct TempDir(PathBuf);

    impl TempDir {
        pub(crate) fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "open-takeout-hub-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("creazione cartella temporanea");
            Self(path)
        }

        pub(crate) fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Scrive un file creando le cartelle intermedie.
    pub(crate) fn write_file(path: &Path, content: &str) {
        write_bytes(path, content.as_bytes());
    }

    /// Variante binaria, per i fixture di immagini reali.
    pub(crate) fn write_bytes(path: &Path, content: &[u8]) {
        std::fs::create_dir_all(path.parent().expect("percorso con genitore"))
            .expect("creazione cartelle");
        std::fs::write(path, content).expect("scrittura file");
    }

    /// JPEG valido di 8x8 pixel, per esercitare la scrittura EXIF reale.
    ///
    /// Serve un contenitore autentico: `little_exif` rifiuta giustamente un
    /// file con la firma sbagliata, quindi un finto JPEG di testo verificherebbe
    /// solo la gestione dell'errore.
    pub(crate) const MINIMAL_JPEG: &[u8] = include_bytes!("../fixtures/minimal.jpg");
}

/// Tipologia di sorgente selezionata dall'utente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceKind {
    /// Cartella `Takeout/` già estratta.
    Folder,
    /// Archivio `takeout-*.zip` non estratto.
    Archive,
}

/// Sezione di Google Takeout riconosciuta all'interno della sorgente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TakeoutSection {
    GooglePhotos,
    Contacts,
    Drive,
    Mail,
    Calendar,
    YouTube,
    Other,
}

impl TakeoutSection {
    /// Deduce la sezione dal nome della cartella di primo livello dentro `Takeout/`.
    ///
    /// I nomi sono localizzati nella lingua dell'account, quindi il match copre
    /// le varianti italiane e inglesi più diffuse.
    pub fn from_dir_name(name: &str) -> Self {
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "google foto" | "google photos" => Self::GooglePhotos,
            "contatti" | "contacts" => Self::Contacts,
            "drive" | "google drive" => Self::Drive,
            "mail" | "posta" => Self::Mail,
            "calendar" | "calendario" => Self::Calendar,
            "youtube e youtube music" | "youtube and youtube music" | "youtube" => Self::YouTube,
            _ => Self::Other,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::GooglePhotos => "Google Foto",
            Self::Contacts => "Contatti",
            Self::Drive => "Drive",
            Self::Mail => "Mail",
            Self::Calendar => "Calendario",
            Self::YouTube => "YouTube",
            Self::Other => "Altro",
        }
    }
}

/// Riepilogo di una singola sezione trovata nella sorgente.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionSummary {
    pub section: TakeoutSection,
    pub label: String,
    pub dir_name: String,
    pub path: PathBuf,
    pub file_count: usize,
    pub total_bytes: u64,
}

/// Riepilogo della sorgente caricata, restituito al frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSummary {
    pub root: PathBuf,
    pub display_name: String,
    pub kind: SourceKind,
    pub sections: Vec<SectionSummary>,
    pub file_count: usize,
    pub total_bytes: u64,
    /// Avvisi non bloccanti emersi durante la scansione.
    pub warnings: Vec<String>,
}

/// Sorgente attualmente caricata in sessione.
///
/// `root` è ridondante rispetto a `summary.root` ma resta il campo autorevole
/// per i comandi che lavorano sul percorso senza toccare il riepilogo.
#[derive(Debug, Clone)]
pub struct LoadedSource {
    pub root: PathBuf,
    pub summary: SourceSummary,
}

#[derive(Debug, Default)]
struct StateInner {
    source: Option<LoadedSource>,
}

/// Stato condiviso registrato in `tauri::Builder::manage`.
#[derive(Debug, Default)]
pub struct AppState {
    inner: Mutex<StateInner>,
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra la sorgente caricata, sostituendo l'eventuale precedente.
    pub fn set_source(&self, source: LoadedSource) -> Result<()> {
        let mut guard = self.inner.lock().map_err(|_| TakeoutError::Poisoned)?;
        guard.source = Some(source);
        Ok(())
    }

    /// Restituisce il riepilogo della sorgente corrente.
    pub fn summary(&self) -> Result<SourceSummary> {
        let guard = self.inner.lock().map_err(|_| TakeoutError::Poisoned)?;
        guard
            .source
            .as_ref()
            .map(|s| s.summary.clone())
            .ok_or(TakeoutError::NoSource)
    }

    /// Radice della sorgente corrente.
    pub fn root(&self) -> Result<PathBuf> {
        let guard = self.inner.lock().map_err(|_| TakeoutError::Poisoned)?;
        guard
            .source
            .as_ref()
            .map(|s| s.root.clone())
            .ok_or(TakeoutError::NoSource)
    }

    /// Svuota lo stato: usata dal comando "Chiudi sorgente".
    pub fn clear(&self) -> Result<()> {
        let mut guard = self.inner.lock().map_err(|_| TakeoutError::Poisoned)?;
        guard.source = None;
        Ok(())
    }
}

/// Dichiarazione esplicita del profilo privacy, esposta alla UI.
///
/// I valori sono costanti compilate: se un giorno qualcuno introducesse una
/// dipendenza di rete, questo blocco andrebbe aggiornato a mano e la modifica
/// resterebbe visibile in diff.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyReport {
    pub network_calls: bool,
    pub telemetry: bool,
    pub crash_reporting: bool,
    pub auto_updater: bool,
    pub external_links: bool,
    pub notes: Vec<String>,
}

impl Default for PrivacyReport {
    fn default() -> Self {
        Self {
            network_calls: false,
            telemetry: false,
            crash_reporting: false,
            auto_updater: false,
            external_links: false,
            notes: vec![
                "Nessuna crate HTTP nel grafo delle dipendenze.".to_string(),
                "CSP restrittiva: connect-src limitato al canale IPC locale.".to_string(),
                "Nessun updater e nessun plugin di apertura URL registrato.".to_string(),
                "I dati restano nei percorsi scelti dall'utente e in memoria.".to_string(),
            ],
        }
    }
}

/// Dimensione leggibile, per i messaggi di errore.
pub(crate) fn formatta_byte(bytes: u64) -> String {
    const UNITA: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut valore = bytes as f64;
    let mut unita = 0;
    while valore >= 1000.0 && unita < UNITA.len() - 1 {
        valore /= 1000.0;
        unita += 1;
    }
    if unita == 0 {
        format!("{bytes} B")
    } else {
        format!("{valore:.1} {}", UNITA[unita])
    }
}

/// Una sottocartella della sorgente, con il suo peso.
///
/// Serve a lavorare a tranche quando la libreria intera non ci sta: si ripara
/// una cartella per volta, si sposta il risultato altrove, si passa alla
/// successiva.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderSize {
    pub name: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub file_count: usize,
    /// Vero se la copia di questa sola cartella ci sta nello spazio rimasto.
    pub fits: bool,
}

/// Conti sullo spazio, per decidere prima di cominciare.
///
/// Serve a rispondere alla domanda che si pone chiunque abbia una libreria
/// grande: ci sta? E se non ci sta, che cosa posso fare?
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceEstimate {
    /// Quanto pesa la libreria di origine.
    pub source_bytes: u64,
    /// Quanto spazio resta sul volume di destinazione.
    pub available_bytes: u64,
    /// Quanto ne servirebbe per la copia, margine compreso.
    pub needed_for_copy: u64,
    /// Vero se la copia riparata ci sta.
    pub copy_fits: bool,
    /// Spazio extra richiesto dalla riscrittura sul posto.
    ///
    /// È un temporaneo per thread, quindi resta nell'ordine delle decine di
    /// megabyte qualunque sia la dimensione della libreria: è la via
    /// praticabile quando la copia non ci sta.
    pub needed_in_place: u64,
    /// Sottocartelle di primo livello, dalla più pesante alla più leggera.
    ///
    /// Quando l'intera libreria non entra, sono le tranche in cui dividere il
    /// lavoro.
    pub subfolders: Vec<FolderSize>,
}

/// Margine di sicurezza richiesto oltre ai byte da scrivere.
///
/// Riempire un disco fino all'ultimo byte non è mai una buona idea: il sistema
/// ha bisogno di spazio per i propri file temporanei, e su APFS le istantanee
/// possono trattenere blocchi che sembrano liberi.
const MARGINE_DISCO: f64 = 1.10;

/// Rifiuta l'operazione se sulla destinazione non c'è spazio sufficiente.
///
/// Serve perché la copia riparata duplica l'intera libreria: su un export da
/// sessanta gigabyte ne servono altrettanti. Senza questo controllo il disco si
/// riempirebbe a metà lavoro, lasciando un albero di uscita che sembra
/// completo e non lo è, e l'utente lo scoprirebbe solo contando i file.
pub fn require_free_space(destination: &Path, needed: u64) -> Result<()> {
    // Lo spazio si misura sulla cartella esistente più vicina: la destinazione
    // potrebbe non essere ancora stata creata.
    let mut probe = destination;
    while !probe.exists() {
        match probe.parent() {
            Some(parent) => probe = parent,
            None => return Ok(()),
        }
    }

    let available = match fs4::available_space(probe) {
        Ok(bytes) => bytes,
        // Se il filesystem non sa rispondere non blocchiamo il lavoro: meglio
        // provare e fallire sul singolo file che rifiutare senza motivo.
        Err(_) => return Ok(()),
    };

    let required = (needed as f64 * MARGINE_DISCO) as u64;
    if available >= required {
        return Ok(());
    }

    Err(TakeoutError::NotEnoughSpace {
        needed: required,
        available,
    })
}

/// Calcola i conti sullo spazio tra una sorgente e una destinazione.
pub fn estimate_space(source: &Path, destination: &Path, largest: u64) -> Result<SpaceEstimate> {
    require_existing(source)?;

    let source_bytes: u64 = walkdir::WalkDir::new(source)
        .follow_links(false)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum();

    let mut probe = destination;
    while !probe.exists() {
        match probe.parent() {
            Some(parent) => probe = parent,
            None => break,
        }
    }
    let available_bytes = fs4::available_space(probe).unwrap_or(0);
    let needed_for_copy = (source_bytes as f64 * MARGINE_DISCO) as u64;

    // Sul posto si lavora su un file per volta e per thread: quattro copie del
    // file più grande bastano ad avere margine.
    let needed_in_place = largest.saturating_mul(4).max(64 * 1024 * 1024);

    // Le sottocartelle di primo livello sono le tranche naturali: in un export
    // Google Foto corrispondono agli anni e agli album.
    let mut subfolders: Vec<FolderSize> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(source) {
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let name = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            if name.starts_with('.') {
                continue;
            }

            let mut bytes = 0u64;
            let mut file_count = 0usize;
            for file in walkdir::WalkDir::new(&dir)
                .follow_links(false)
                .into_iter()
                .flatten()
                .filter(|e| e.file_type().is_file())
            {
                file_count += 1;
                bytes += file.metadata().map(|m| m.len()).unwrap_or(0);
            }

            subfolders.push(FolderSize {
                name,
                path: dir,
                bytes,
                file_count,
                fits: available_bytes >= (bytes as f64 * MARGINE_DISCO) as u64,
            });
        }
    }
    subfolders.sort_by_key(|f| std::cmp::Reverse(f.bytes));

    Ok(SpaceEstimate {
        source_bytes,
        available_bytes,
        needed_for_copy,
        copy_fits: available_bytes >= needed_for_copy,
        needed_in_place,
        subfolders,
    })
}

/// Verifica che un percorso ricevuto dal frontend esista davvero.
pub fn require_existing(path: &Path) -> Result<()> {
    if path.exists() {
        Ok(())
    } else {
        Err(TakeoutError::NotFound(path.to_path_buf()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn riconosce_le_sezioni_localizzate() {
        assert_eq!(
            TakeoutSection::from_dir_name("Google Foto"),
            TakeoutSection::GooglePhotos
        );
        assert_eq!(
            TakeoutSection::from_dir_name("google photos"),
            TakeoutSection::GooglePhotos
        );
        assert_eq!(TakeoutSection::from_dir_name("Keep"), TakeoutSection::Other);
    }

    #[test]
    fn lo_stato_parte_vuoto_e_si_svuota() {
        let state = AppState::new();
        assert!(matches!(state.summary(), Err(TakeoutError::NoSource)));
        state
            .clear()
            .expect("clear su stato vuoto non deve fallire");
    }
}
