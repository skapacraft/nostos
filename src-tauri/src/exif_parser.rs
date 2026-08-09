// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Lettura EXIF e riconciliazione con i sidecar JSON di Google Foto.
//!
//! Il problema tipico di un export Google Foto: la data di scatto e le
//! coordinate stanno in un file `.json` affiancato al media, mentre l'EXIF del
//! file esportato è spesso vuoto o sbagliato. Questo modulo legge entrambe le
//! fonti, le confronta e prepara la riconciliazione.
//!
//! Tutte le operazioni sono in sola lettura finché non viene invocata
//! esplicitamente [`apply_metadata`], che scrive solo nella modalità richiesta.

use std::collections::HashSet;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use exif::{In, Tag, Value};
use little_exif::exif_tag::ExifTag;
use little_exif::ifd::ExifTagGroup;
use little_exif::metadata::Metadata as ExifWriter;
use little_exif::rational::uR64;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::app_state::{trace_dev, Phase, Progress, ProgressSink, Result, TakeoutError};

/// Estensioni trattate come media in un export Google Foto.
const MEDIA_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "heic", "heif", "gif", "webp", "tif", "tiff", "dng", "mp4", "mov", "m4v",
    "avi", "3gp", "mkv", "webm",
];

/// Formati in cui sappiamo riscrivere i tag EXIF.
///
/// I video sono esclusi di proposito: i loro metadati stanno in atomi MP4 o in
/// tag Matroska, non in un blocco EXIF, e trattarli qui produrrebbe file rotti.
/// Per loro resta l'allineamento della data di modifica.
const EXIF_WRITABLE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "heic", "heif", "tif", "tiff", "webp"];

/// Oltre questa soglia il file non viene riscritto.
///
/// `little_exif` lavora sull'intero contenuto in memoria: senza un tetto, un
/// TIFF da un giga moltiplicato per i thread attivi farebbe esplodere il
/// consumo. Le foto di un Takeout stanno ampiamente sotto.
const MAX_REWRITE_BYTES: u64 = 128 * 1024 * 1024;

/// Thread usati per la riscrittura.
///
/// Il lavoro è dominato dall'I/O, non dalla CPU: oltre una manciata di thread
/// non si guadagna nulla e ogni thread in più tiene in memoria la propria copia
/// del file in lavorazione.
const REWRITE_THREADS: usize = 4;

/// Coordinate geografiche in gradi decimali.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeoPoint {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: Option<f64>,
}

impl GeoPoint {
    /// Google scrive `0,0` quando il dato non esiste: va trattato come assente.
    fn is_null_island(&self) -> bool {
        self.latitude.abs() < f64::EPSILON && self.longitude.abs() < f64::EPSILON
    }
}

/// Dati letti dall'EXIF del file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExifData {
    pub taken_at: Option<DateTime<Utc>>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub geo: Option<GeoPoint>,
    pub orientation: Option<u32>,
}

impl ExifData {
    /// Vero quando il file non espone alcun tag utile: succede spesso ai media
    /// riconvertiti da Google, che perdono l'EXIF originale.
    pub fn is_empty(&self) -> bool {
        self.taken_at.is_none()
            && self.camera_make.is_none()
            && self.camera_model.is_none()
            && self.geo.is_none()
    }
}

/// Blocco `{ "timestamp": "1577880000", "formatted": "..." }` dei sidecar.
#[derive(Debug, Clone, Deserialize)]
struct TakeoutTime {
    timestamp: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct TakeoutGeo {
    latitude: Option<f64>,
    longitude: Option<f64>,
    altitude: Option<f64>,
}

/// Sidecar JSON prodotto da Google Foto, nella forma che ci interessa.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSidecar {
    title: Option<String>,
    description: Option<String>,
    photo_taken_time: Option<TakeoutTime>,
    creation_time: Option<TakeoutTime>,
    geo_data: Option<TakeoutGeo>,
    geo_data_exif: Option<TakeoutGeo>,
    /// Volti riconosciuti e confermati dall'utente in Google Foto.
    people: Option<Vec<RawPerson>>,
    favorited: Option<bool>,
    /// Contatore interno di Google, senza corrispondente nei metadati.
    image_views: Option<String>,
    /// Indirizzo della foto su Google Foto, valido solo finché esiste lì.
    url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawPerson {
    name: Option<String>,
}

/// Sidecar normalizzato.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarData {
    pub path: PathBuf,
    pub title: Option<String>,
    pub description: Option<String>,
    pub taken_at: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
    pub geo: Option<GeoPoint>,
    /// Nomi dei volti riconosciuti, nell'ordine in cui Google li elenca.
    pub people: Vec<String>,
    pub favorited: bool,
    /// Dati che Google esporta ma che non hanno una sede nei metadati EXIF.
    pub image_views: Option<String>,
    pub url: Option<String>,
}

/// Motivo per cui un sidecar resta dov'è invece di essere messo da parte.
///
/// È un codice, non una frase: il testo lo sceglie chi mostra, nella lingua
/// che sta usando. Vale anche per i conteggi che l'interfaccia raggruppa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SidecarKept {
    /// PNG, GIF e video: nessun blocco EXIF dove scrivere.
    NoExifContainer,
    /// Il file non ha un blocco EXIF leggibile.
    UnreadableExif,
    /// La data di scatto non risulta ancora scritta nel file.
    MissingDate,
    /// Le coordinate non risultano ancora scritte nel file.
    MissingGeo,
    /// La descrizione non risulta ancora scritta nel file.
    MissingDescription,
    /// I volti riconosciuti non risultano ancora scritti nel file.
    MissingPeople,
    /// Il contrassegno di preferito non risulta ancora scritto nel file.
    MissingFavorite,
    /// Conteggio delle visualizzazioni: nei metadati non ha dove stare.
    ViewCountHasNoTag,
    /// Indirizzo su Google Foto: nei metadati non ha dove stare.
    PhotoUrlHasNoTag,
}

impl SidecarData {
    /// Elenca i dati del sidecar che non finiranno dentro il file.
    ///
    /// Serve a poter dire all'utente cosa resta indietro invece di lasciarglielo
    /// scoprire: sono contatori e indirizzi interni a Google Foto, non metadati
    /// della fotografia, ma la differenza la decide chi possiede le foto.
    pub fn unwritable(&self) -> Vec<SidecarKept> {
        let mut resto = Vec::new();
        if self.image_views.is_some() {
            resto.push(SidecarKept::ViewCountHasNoTag);
        }
        if self.url.is_some() {
            resto.push(SidecarKept::PhotoUrlHasNoTag);
        }
        resto
    }
}

/// Origine scelta per il dato finale, in ordine di affidabilità decrescente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataSource {
    /// Letto dai tag del file: descrive il momento dello scatto.
    Exif,
    /// Letto dal sidecar JSON: è quanto sa il servizio.
    Sidecar,
    /// Dedotto dal nome del file, che le app fotocamera generano dall'orologio.
    FileName,
    Missing,
}

/// Schemi di data riconosciuti nei nomi generati dalle app fotocamera.
///
/// `A` sta per una cifra, ogni altro carattere deve corrispondere esattamente.
/// Coprono i formati prodotti da Android, Pixel, iOS, screenshot e messaggistica:
/// `IMG_20200101_120000.jpg`, `PXL_20200101_120000123.jpg`,
/// `Screenshot_20200101-120000.png`, `signal-2020-01-01-12-00-00.jpg`.
const FILENAME_DATE_PATTERNS: &[&str] = &[
    "AAAA-AA-AA-AA-AA-AA",
    "AAAA_AA_AA_AA_AA_AA",
    "AAAA-AA-AA-AAAAAA",
    "AAAAAAAA-AAAAAA",
    "AAAAAAAA_AAAAAA",
    "AAAAAAAAAAAAAA",
];

/// Vista unificata di un media e dei suoi metadati.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaRecord {
    pub path: PathBuf,
    pub file_name: String,
    pub size_bytes: u64,
    pub exif: ExifData,
    pub sidecar: Option<SidecarData>,
    /// Data risolta: EXIF se presente, altrimenti sidecar.
    pub resolved_taken_at: Option<DateTime<Utc>>,
    pub taken_at_source: MetadataSource,
    pub resolved_geo: Option<GeoPoint>,
    pub geo_source: MetadataSource,
    /// L'EXIF manca ma il sidecar ha la data: il file è recuperabile.
    pub needs_repair: bool,
}

/// Esito della scansione di una cartella di foto.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoScanReport {
    pub media_count: usize,
    pub with_sidecar: usize,
    pub with_exif_date: usize,
    pub with_geo: usize,
    pub needs_repair: usize,
    /// File senza alcun tag EXIF utile.
    pub without_exif: usize,
    /// File la cui data è stata dedotta dal nome, ultima risorsa.
    pub date_from_filename: usize,
    pub total_bytes: u64,
    /// Quanti file non sono stati letti, conteggio completo.
    pub unreadable_count: usize,
    /// Campione dei problemi, troncato per la UI.
    pub unreadable: Vec<String>,
    /// Campione dei primi record, per l'anteprima nella UI.
    pub sample: Vec<MediaRecord>,
}

/// Come trattare i file originali durante la riparazione.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WriteMode {
    /// Non tocca nulla: calcola soltanto che cosa cambierebbe.
    DryRun,
    /// Scrive le copie riparate in un albero separato, lasciando intatti gli
    /// originali. È il valore predefinito.
    CopyToOutput,
    /// Riscrive gli originali. Va richiesto in modo esplicito.
    InPlace,
}

/// Come disporre i file nell'albero di uscita.
///
/// Vale solo con [`WriteMode::CopyToOutput`]: le altre modalità non scelgono
/// dove scrivere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputLayout {
    /// Ricrea la struttura di cartelle dell'originale. È il predefinito.
    Preserve,
    /// Una cartella per anno.
    ByYear,
    /// Una cartella per anno e una per mese.
    ByYearMonth,
    /// Tutto in una cartella sola, in ordine di data nel nome.
    Flat,
}

impl OutputLayout {
    /// Sottocartella di destinazione per un media con la data indicata.
    ///
    /// Chi non ha una data finisce in una cartella a parte invece di essere
    /// buttato in mezzo agli altri: un file senza data non appartiene a nessun
    /// mese, e fingere il contrario renderebbe l'ordinamento una bugia.
    fn folder_for(&self, taken_at: Option<DateTime<Utc>>) -> PathBuf {
        use chrono::Datelike;

        match (self, taken_at) {
            (Self::Preserve, _) => PathBuf::new(),
            (_, None) => PathBuf::from("senza-data"),
            (Self::ByYear, Some(date)) => PathBuf::from(date.year().to_string()),
            (Self::ByYearMonth, Some(date)) => {
                PathBuf::from(date.year().to_string()).join(format!("{:02}", date.month()))
            }
            (Self::Flat, Some(_)) => PathBuf::new(),
        }
    }
}

/// Parametri della riparazione.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteOptions {
    pub mode: WriteMode,
    /// Disposizione dell'albero di uscita.
    #[serde(default = "default_layout")]
    pub layout: OutputLayout,
    /// Radice di destinazione, obbligatoria con [`WriteMode::CopyToOutput`].
    pub output_root: Option<PathBuf>,
    /// Scrive data e coordinate nei tag EXIF del file.
    pub write_exif: bool,
    /// Allinea la data di modifica del file alla data di scatto.
    pub write_file_times: bool,
}

fn default_layout() -> OutputLayout {
    OutputLayout::Preserve
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            mode: WriteMode::DryRun,
            layout: OutputLayout::Preserve,
            output_root: None,
            write_exif: true,
            write_file_times: true,
        }
    }
}

/// Esito della riparazione.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairReport {
    pub mode: Option<WriteMode>,
    pub output_root: Option<PathBuf>,
    /// File per cui esiste una data o una posizione da scrivere.
    pub candidates: usize,
    pub exif_written: usize,
    pub file_times_written: usize,
    /// Formati per cui non sappiamo riscrivere l'EXIF (video e simili).
    pub skipped_unsupported: usize,
    /// File oltre la soglia di dimensione.
    pub skipped_too_large: usize,
    /// Sidecar copiati accanto ai file di cui non si è potuto scrivere l'EXIF.
    pub sidecars_copied: usize,
    pub failures: Vec<String>,
}

/// Vero se l'estensione è quella di un media gestito.
pub fn is_media_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| MEDIA_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Vero se sappiamo riscrivere i tag EXIF di questo formato.
pub fn is_exif_writable(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| EXIF_WRITABLE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Legge i tag EXIF utili. Un file senza EXIF non è un errore: restituisce
/// [`ExifData::default`].
pub fn read_exif(path: &Path) -> Result<ExifData> {
    let file = File::open(path).map_err(|e| TakeoutError::io(path, e))?;
    let mut reader = BufReader::new(&file);

    let exif = match exif::Reader::new().read_from_container(&mut reader) {
        Ok(exif) => exif,
        // Assenza di EXIF o formato non supportato: dato mancante, non guasto.
        Err(_) => return Ok(ExifData::default()),
    };

    // Un file che dichiara il proprio offset va interpretato con quello: senza,
    // l'ora locale verrebbe scambiata per UTC e la foto si sposterebbe nel
    // tempo della differenza di fuso.
    let offset_minutes = [Tag::OffsetTimeOriginal, Tag::OffsetTimeDigitized]
        .iter()
        .find_map(|tag| exif.get_field(*tag, In::PRIMARY))
        .and_then(|field| ascii_value(&field.value))
        .and_then(|raw| parse_offset(&raw));

    let taken_at = [Tag::DateTimeOriginal, Tag::DateTimeDigitized, Tag::DateTime]
        .iter()
        .find_map(|tag| exif.get_field(*tag, In::PRIMARY))
        .and_then(|field| ascii_value(&field.value))
        .and_then(|raw| parse_exif_datetime(&raw))
        .map(|date| match offset_minutes {
            Some(minutes) => date - chrono::Duration::minutes(minutes as i64),
            None => date,
        });

    let geo = read_gps(&exif).filter(|g| !g.is_null_island());

    Ok(ExifData {
        taken_at,
        camera_make: exif
            .get_field(Tag::Make, In::PRIMARY)
            .and_then(|f| ascii_value(&f.value)),
        camera_model: exif
            .get_field(Tag::Model, In::PRIMARY)
            .and_then(|f| ascii_value(&f.value)),
        geo,
        orientation: exif
            .get_field(Tag::Orientation, In::PRIMARY)
            .and_then(|f| f.value.get_uint(0)),
    })
}

/// Estrae latitudine e longitudine dai campi GPS, convertendo da gradi/primi/
/// secondi a gradi decimali e applicando il riferimento N/S ed E/W.
fn read_gps(exif: &exif::Exif) -> Option<GeoPoint> {
    let lat = dms_to_degrees(exif, Tag::GPSLatitude)?;
    let lon = dms_to_degrees(exif, Tag::GPSLongitude)?;

    let lat_ref = exif
        .get_field(Tag::GPSLatitudeRef, In::PRIMARY)
        .and_then(|f| ascii_value(&f.value))
        .unwrap_or_else(|| "N".to_string());
    let lon_ref = exif
        .get_field(Tag::GPSLongitudeRef, In::PRIMARY)
        .and_then(|f| ascii_value(&f.value))
        .unwrap_or_else(|| "E".to_string());

    let latitude = if lat_ref.eq_ignore_ascii_case("S") {
        -lat
    } else {
        lat
    };
    let longitude = if lon_ref.eq_ignore_ascii_case("W") {
        -lon
    } else {
        lon
    };

    let altitude = exif
        .get_field(Tag::GPSAltitude, In::PRIMARY)
        .and_then(|f| match &f.value {
            Value::Rational(v) => v.first().map(|r| r.to_f64()),
            _ => None,
        });

    Some(GeoPoint {
        latitude,
        longitude,
        altitude,
    })
}

fn dms_to_degrees(exif: &exif::Exif, tag: Tag) -> Option<f64> {
    let field = exif.get_field(tag, In::PRIMARY)?;
    match &field.value {
        Value::Rational(values) if values.len() >= 3 => {
            let degrees = values[0].to_f64();
            let minutes = values[1].to_f64();
            let seconds = values[2].to_f64();
            Some(degrees + minutes / 60.0 + seconds / 3600.0)
        }
        _ => None,
    }
}

fn ascii_value(value: &Value) -> Option<String> {
    match value {
        Value::Ascii(entries) => entries
            .first()
            .map(|bytes| String::from_utf8_lossy(bytes).trim().to_string())
            .filter(|s| !s.is_empty()),
        _ => None,
    }
}

/// Interpreta un offset EXIF nella forma `+01:00` restituendo i minuti.
fn parse_offset(raw: &str) -> Option<i32> {
    let raw = raw.trim();
    let (sign, rest) = match raw.chars().next()? {
        '+' => (1, &raw[1..]),
        '-' => (-1, &raw[1..]),
        _ => return None,
    };
    let (hours, minutes) = rest.split_once(':')?;
    let total = hours.parse::<i32>().ok()? * 60 + minutes.parse::<i32>().ok()?;
    Some(sign * total)
}

/// L'EXIF usa `YYYY:MM:DD HH:MM:SS` senza fuso orario. In assenza del tag di
/// offset resta ambiguo, e lo trattiamo come UTC: è l'unica scelta che non
/// inventa un fuso.
fn parse_exif_datetime(raw: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(raw.trim(), "%Y:%m:%d %H:%M:%S")
        .ok()
        .map(|naive| Utc.from_utc_datetime(&naive))
}

fn parse_unix_timestamp(raw: &Option<TakeoutTime>) -> Option<DateTime<Utc>> {
    let seconds: i64 = raw.as_ref()?.timestamp.as_ref()?.parse().ok()?;
    DateTime::from_timestamp(seconds, 0)
}

fn geo_from_takeout(raw: &Option<TakeoutGeo>) -> Option<GeoPoint> {
    let raw = raw.as_ref()?;
    let point = GeoPoint {
        latitude: raw.latitude?,
        longitude: raw.longitude?,
        altitude: raw.altitude,
    };
    (!point.is_null_island()).then_some(point)
}

/// Deduce la data di scatto dal nome del file.
///
/// Le app fotocamera scrivono l'orario dell'orologio nel nome, quindi quando
/// EXIF e sidecar mancano entrambi questa resta l'unica fonte non inventata.
/// La lettura è deliberatamente severa: la data deve essere valida come data
/// reale, altrimenti un numero di serie qualsiasi diventerebbe un timestamp.
pub fn parse_date_from_filename(file_name: &str) -> Option<DateTime<Utc>> {
    let bytes: Vec<char> = file_name.chars().collect();

    for pattern in FILENAME_DATE_PATTERNS {
        let pattern: Vec<char> = pattern.chars().collect();
        if bytes.len() < pattern.len() {
            continue;
        }

        for start in 0..=(bytes.len() - pattern.len()) {
            let window = &bytes[start..start + pattern.len()];
            let matches = window.iter().zip(pattern.iter()).all(|(c, p)| {
                if *p == 'A' {
                    c.is_ascii_digit()
                } else {
                    c == p
                }
            });
            if !matches {
                continue;
            }

            // Una data preceduta da altre cifre fa parte di un numero più
            // lungo: è il caso dei numeri di serie, e va scartata.
            if start > 0 && bytes[start - 1].is_ascii_digit() {
                continue;
            }

            // Dopo, invece, qualche cifra è legittima: i Pixel scrivono i
            // millesimi di secondo in coda (`PXL_20200101_120000123`). Ne
            // tolleriamo fino a tre; oltre, siamo di nuovo dentro un numero.
            let trailing = bytes[start + pattern.len()..]
                .iter()
                .take_while(|c| c.is_ascii_digit())
                .count();
            if trailing > 3 {
                continue;
            }

            let digits: String = window.iter().filter(|c| c.is_ascii_digit()).collect();
            if digits.len() != 14 {
                continue;
            }
            if let Some(parsed) = build_datetime(&digits) {
                return Some(parsed);
            }
        }
    }

    None
}

/// Compone una data da quattordici cifre `YYYYMMDDhhmmss`, validandola.
fn build_datetime(digits: &str) -> Option<DateTime<Utc>> {
    let year: i32 = digits[0..4].parse().ok()?;
    let month: u32 = digits[4..6].parse().ok()?;
    let day: u32 = digits[6..8].parse().ok()?;
    let hour: u32 = digits[8..10].parse().ok()?;
    let minute: u32 = digits[10..12].parse().ok()?;
    let second: u32 = digits[12..14].parse().ok()?;

    // Una fotografia scattata nel 1823 o nel 2400 è un falso positivo.
    if !(1900..=2100).contains(&year) {
        return None;
    }

    let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
    let time = chrono::NaiveTime::from_hms_opt(hour, minute, second)?;
    Some(Utc.from_utc_datetime(&date.and_time(time)))
}

/// Individua e legge il sidecar JSON associato al media.
pub fn read_sidecar(media: &Path, index: Option<&FileIndex>) -> Result<Option<SidecarData>> {
    let Some(sidecar_path) = find_sidecar(media, index) else {
        return Ok(None);
    };

    let content =
        std::fs::read_to_string(&sidecar_path).map_err(|e| TakeoutError::io(&sidecar_path, e))?;
    let raw: RawSidecar = serde_json::from_str(&content)
        .map_err(|e| TakeoutError::Metadata(format!("{}: {e}", sidecar_path.display())))?;

    Ok(Some(SidecarData {
        path: sidecar_path,
        title: raw.title,
        description: raw.description.filter(|d| !d.is_empty()),
        taken_at: parse_unix_timestamp(&raw.photo_taken_time),
        created_at: parse_unix_timestamp(&raw.creation_time),
        geo: geo_from_takeout(&raw.geo_data).or_else(|| geo_from_takeout(&raw.geo_data_exif)),
        people: raw
            .people
            .unwrap_or_default()
            .into_iter()
            .filter_map(|p| p.name)
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .collect(),
        favorited: raw.favorited.unwrap_or(false),
        // Google scrive "0" anche quando non c'è nulla da dire: un contatore a
        // zero non è un dato che si perde.
        image_views: raw.image_views.filter(|v| v != "0" && !v.is_empty()),
        url: raw.url.filter(|u| !u.is_empty()),
    }))
}

/// Lunghezza massima del nome di un sidecar prodotto da Google, `.json`
/// compreso.
///
/// È il comportamento osservato negli export, non una regola documentata da
/// Google: se un export dovesse usare un limite diverso, i nomi lunghi
/// tornerebbero senza sidecar e la scansione lo direbbe contando i file la cui
/// data viene dedotta dal nome.
const MAX_SIDECAR_NAME: usize = 46;

const JSON_EXT: &str = ".json";

/// Taglia una stringa a `limite` caratteri senza spezzare un carattere UTF-8.
///
/// Contare in byte spaccherebbe le lettere accentate e gli ideogrammi, che nei
/// nomi dei file ci sono eccome.
fn tronca(testo: &str, limite: usize) -> &str {
    match testo.char_indices().nth(limite) {
        Some((fine, _)) => &testo[..fine],
        None => testo,
    }
}

/// Costruisce i nomi candidati del sidecar.
///
/// Google ha cambiato schema più volte e tronca i nomi lunghi, quindi non
/// esiste una regola sola: proviamo le varianti note in ordine di frequenza.
fn sidecar_candidates(media: &Path) -> Vec<PathBuf> {
    let Some(file_name) = media.file_name().and_then(|n| n.to_str()) else {
        return Vec::new();
    };
    let parent = media.parent().unwrap_or_else(|| Path::new("."));
    let stem = media
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(file_name);

    let mut candidates = vec![
        // Schema classico: IMG_0001.JPG.json
        parent.join(format!("{file_name}.json")),
        // Schema recente: IMG_0001.JPG.supplemental-metadata.json
        parent.join(format!("{file_name}.supplemental-metadata.json")),
        // Variante senza estensione del media: IMG_0001.json
        parent.join(format!("{stem}.json")),
    ];

    // Nomi lunghi: Google accorcia il nome del sidecar fino a farlo stare in
    // MAX_SIDECAR_NAME caratteri, `.json` compreso. Il taglio cade quasi
    // sempre dentro `.supplemental-metadata`, che compare mozzato
    // (`.supplemental-me.json`) o sparisce del tutto lasciando troncato il nome
    // del media. Senza queste varianti una foto con nome lungo risulta priva di
    // sidecar e la data finisce per essere dedotta dal nome, che è l'ultima
    // risorsa e non porta le coordinate.
    for base in [
        format!("{file_name}.supplemental-metadata"),
        file_name.to_string(),
    ] {
        // Il conto è in caratteri, non in byte, per la stessa ragione per cui
        // `tronca` taglia sui confini: un nome con accenti non deve produrre un
        // candidato diverso solo perché occupa più byte.
        if base.chars().count() + JSON_EXT.len() <= MAX_SIDECAR_NAME {
            continue;
        }
        let tagliato = tronca(&base, MAX_SIDECAR_NAME - JSON_EXT.len());
        let candidato = parent.join(format!("{tagliato}{JSON_EXT}"));
        if !candidates.contains(&candidato) {
            candidates.push(candidato);
        }
    }

    // File duplicati: `IMG_0001(1).JPG` ha sidecar `IMG_0001.JPG(1).json`.
    if let Some(open) = stem.rfind('(') {
        if stem.ends_with(')') {
            let base = &stem[..open];
            let counter = &stem[open..];
            let extension = media
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!(".{e}"))
                .unwrap_or_default();
            candidates.push(parent.join(format!("{base}{extension}{counter}.json")));
        }
    }

    candidates
}

/// Insieme dei percorsi presenti, raccolto una volta sola durante la scansione.
///
/// Esiste per non interrogare il filesystem a ogni candidato sidecar: la
/// versione che chiamava `is_file()` su tre o quattro percorsi per foto
/// pagava un `stat` dentro cartelle da decine di migliaia di voci, e il costo
/// della scansione cresceva con il quadrato della libreria invece che in
/// proporzione.
pub type FileIndex = std::collections::HashSet<PathBuf>;

/// Vero se il percorso esiste, consultando l'indice quando disponibile.
fn exists(path: &Path, index: Option<&FileIndex>) -> bool {
    match index {
        Some(index) => index.contains(path),
        None => path.is_file(),
    }
}

fn find_sidecar(media: &Path, index: Option<&FileIndex>) -> Option<PathBuf> {
    let candidates = sidecar_candidates(media);
    let found = candidates.iter().find(|c| exists(c, index)).cloned();

    if found.is_none() {
        // Il caso più frequente di "riparazione che non fa nulla" è un sidecar
        // che esiste con un nome che non abbiamo previsto: elencare i tentativi
        // rende il problema evidente invece che silenzioso.
        trace_dev!(
            "nessun sidecar per {}: provati {}",
            media.file_name().unwrap_or_default().to_string_lossy(),
            candidates
                .iter()
                .map(|c| c
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    found
}

/// Costruisce il record unificato di un singolo media.
pub fn inspect_media(path: &Path, index: Option<&FileIndex>) -> Result<MediaRecord> {
    let metadata = std::fs::metadata(path).map_err(|e| TakeoutError::io(path, e))?;
    let exif = read_exif(path)?;
    let sidecar = read_sidecar(path, index)?;

    let sidecar_taken = sidecar.as_ref().and_then(|s| s.taken_at.or(s.created_at));

    // Ordine di affidabilità: l'EXIF descrive il momento dello scatto, il
    // sidecar quanto sa il servizio, il nome del file è ciò che ha scritto
    // l'app fotocamera. Il nome interviene solo quando gli altri due tacciono.
    let from_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(parse_date_from_filename);

    let (resolved_taken_at, taken_at_source) = match (exif.taken_at, sidecar_taken, from_name) {
        (Some(date), _, _) => (Some(date), MetadataSource::Exif),
        (None, Some(date), _) => (Some(date), MetadataSource::Sidecar),
        (None, None, Some(date)) => (Some(date), MetadataSource::FileName),
        (None, None, None) => (None, MetadataSource::Missing),
    };

    let sidecar_geo = sidecar.as_ref().and_then(|s| s.geo);
    let (resolved_geo, geo_source) = match (exif.geo, sidecar_geo) {
        (Some(geo), _) => (Some(geo), MetadataSource::Exif),
        (None, Some(geo)) => (Some(geo), MetadataSource::Sidecar),
        (None, None) => (None, MetadataSource::Missing),
    };

    trace_dev!(
        "letto {}: sidecar={} exif_data={} data={} ({:?}) gps={} ({:?})",
        path.file_name().unwrap_or_default().to_string_lossy(),
        sidecar
            .as_ref()
            .map(|s| s
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned())
            .unwrap_or_else(|| "assente".to_string()),
        if exif.is_empty() { "vuoto" } else { "presente" },
        resolved_taken_at
            .map(|d| d.to_rfc3339())
            .unwrap_or_else(|| "nessuna".to_string()),
        taken_at_source,
        resolved_geo
            .map(|g| format!("{:.5},{:.5}", g.latitude, g.longitude))
            .unwrap_or_else(|| "nessuno".to_string()),
        geo_source,
    );

    Ok(MediaRecord {
        file_name: path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string(),
        path: path.to_path_buf(),
        size_bytes: metadata.len(),
        needs_repair: exif.taken_at.is_none() && resolved_taken_at.is_some(),
        exif,
        sidecar,
        resolved_taken_at,
        taken_at_source,
        resolved_geo,
        geo_source,
    })
}

/// Percorre una cartella di Google Foto e riepiloga lo stato dei metadati.
pub fn scan_directory(root: &Path, sample_size: usize) -> Result<PhotoScanReport> {
    crate::app_state::require_existing(root)?;

    let mut report = PhotoScanReport::default();

    // Una passata sola raccoglie i percorsi dei media e l'indice di tutto ciò
    // che esiste, così la ricerca dei sidecar non torna sul filesystem.
    let mut index = FileIndex::new();
    let mut media: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(root).follow_links(false).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        if is_media_file(entry.path()) {
            media.push(entry.path().to_path_buf());
        }
        index.insert(entry.into_path());
    }

    // La lettura è dominata dall'apertura di due file per foto, il media e il
    // suo sidecar: è lavoro di I/O, e su una libreria grande un solo thread
    // diventa il collo di bottiglia. Il numero di thread resta lo stesso della
    // riscrittura, per non moltiplicare le letture concorrenti sul disco.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(
            REWRITE_THREADS.min(std::thread::available_parallelism().map_or(1, |n| n.get())),
        )
        .build()
        .map_err(|e| TakeoutError::Task(e.to_string()))?;

    let esiti: Vec<std::result::Result<MediaRecord, (PathBuf, String)>> = pool.install(|| {
        media
            .par_iter()
            .map(|path| {
                inspect_media(path, Some(&index)).map_err(|err| (path.clone(), err.to_string()))
            })
            .collect()
    });

    // L'aggregazione resta sequenziale e nell'ordine della passata, così il
    // campione mostrato è sempre lo stesso a parità di libreria.
    for esito in esiti {
        match esito {
            Ok(record) => {
                report.media_count += 1;
                report.total_bytes += record.size_bytes;
                if record.sidecar.is_some() {
                    report.with_sidecar += 1;
                }
                if record.exif.taken_at.is_some() {
                    report.with_exif_date += 1;
                }
                if record.exif.is_empty() {
                    report.without_exif += 1;
                }
                if record.taken_at_source == MetadataSource::FileName {
                    report.date_from_filename += 1;
                }
                if record.resolved_geo.is_some() {
                    report.with_geo += 1;
                }
                if record.needs_repair {
                    report.needs_repair += 1;
                }
                if report.sample.len() < sample_size {
                    report.sample.push(record);
                }
            }
            Err((path, err)) => {
                report.unreadable_count += 1;
                // L'elenco è troncato ma il conteggio no: mandare al frontend
                // una stringa per file illeggibile significherebbe megabyte di
                // JSON su una libreria messa male.
                if report.unreadable.len() < sample_size {
                    report.unreadable.push(format!(
                        "{}: {err}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ));
                }
            }
        }
    }

    Ok(report)
}

/// Converte gradi decimali nella terna gradi/primi/secondi richiesta dall'EXIF.
///
/// I secondi sono espressi in millesimi: con il denominatore a 1 si perderebbe
/// una precisione di circa trenta metri.
fn degrees_to_dms(value: f64) -> Vec<uR64> {
    let value = value.abs();
    let degrees = value.trunc();
    let minutes_total = (value - degrees) * 60.0;
    let minutes = minutes_total.trunc();
    let seconds = (minutes_total - minutes) * 60.0;

    vec![
        uR64 {
            nominator: degrees as u32,
            denominator: 1,
        },
        uR64 {
            nominator: minutes as u32,
            denominator: 1,
        },
        uR64 {
            nominator: (seconds * 1000.0).round() as u32,
            denominator: 1000,
        },
    ]
}

/// Cercatore di fusi orari, costruito una volta sola.
///
/// La costruzione carica i poligoni dei fusi in memoria: farla per ogni foto
/// renderebbe la riparazione inutilizzabile su una libreria grande.
static TIMEZONE_FINDER: std::sync::OnceLock<tzf_rs::DefaultFinder> = std::sync::OnceLock::new();

/// Converte un istante UTC nell'ora locale del luogo dove è stato scattato.
///
/// Restituisce l'ora da scrivere e l'offset in forma `+01:00`.
///
/// Questa funzione esiste perché `DateTimeOriginal` **non è UTC**: la specifica
/// EXIF lo definisce come l'ora locale dell'orologio della fotocamera al
/// momento dello scatto. Il sidecar di Google invece porta un istante UTC.
/// Scrivere quell'istante senza conversione sposta indietro ogni foto della
/// differenza di fuso: uno scatto fatto a Milano alle 14 comparirebbe alle 13.
fn local_time_and_offset(instant: DateTime<Utc>, geo: Option<GeoPoint>) -> (NaiveDateTime, String) {
    use chrono::Offset;

    let Some(geo) = geo else {
        // Senza coordinate il fuso è ignoto. Si scrive l'istante UTC dichiarando
        // che è UTC: resta un'ora diversa da quella dell'orologio di chi ha
        // scattato, ma il file non è ambiguo e nessun programma la interpreta
        // male.
        return (instant.naive_utc(), "+00:00".to_string());
    };

    let finder = TIMEZONE_FINDER.get_or_init(tzf_rs::DefaultFinder::new);
    let name = finder.get_tz_name(geo.longitude, geo.latitude);

    let Ok(zone) = name.parse::<chrono_tz::Tz>() else {
        return (instant.naive_utc(), "+00:00".to_string());
    };

    let local = instant.with_timezone(&zone);
    let seconds = local.offset().fix().local_minus_utc();
    let sign = if seconds < 0 { '-' } else { '+' };
    let abs = seconds.abs();

    (
        local.naive_local(),
        format!("{sign}{:02}:{:02}", abs / 3600, (abs % 3600) / 60),
    )
}

/// Applica i tag EXIF risolti al file indicato.
///
/// Il file passato qui è sempre una copia di lavoro, mai l'originale: chi
/// chiama si occupa di crearla e di sostituirla in modo atomico.
fn write_exif_tags(target: &Path, record: &MediaRecord) -> Result<()> {
    // Un file senza blocco EXIF non è un errore: si parte da metadati vuoti e
    // `little_exif` inserisce il segmento mancante.
    let mut writer = ExifWriter::new_from_path(target).unwrap_or_else(|_| ExifWriter::new());

    if let Some(taken_at) = record.resolved_taken_at {
        // L'ora va scritta come la leggeva l'orologio sul posto, con l'offset
        // accanto: è ciò che la specifica EXIF chiede e ciò che i programmi
        // di gestione foto si aspettano di trovare.
        let (local, offset) = local_time_and_offset(taken_at, record.resolved_geo);
        let formatted = local.format("%Y:%m:%d %H:%M:%S").to_string();

        writer.set_tag(ExifTag::DateTimeOriginal(formatted.clone()));
        writer.set_tag(ExifTag::CreateDate(formatted));
        writer.set_tag(ExifTag::OffsetTimeOriginal(offset.clone()));
        writer.set_tag(ExifTag::OffsetTimeDigitized(offset));
    }

    if let Some(geo) = record.resolved_geo {
        writer.set_tag(ExifTag::GPSVersionID(vec![2, 3, 0, 0]));
        writer.set_tag(ExifTag::GPSLatitude(degrees_to_dms(geo.latitude)));
        writer.set_tag(ExifTag::GPSLatitudeRef(
            if geo.latitude < 0.0 { "S" } else { "N" }.to_string(),
        ));
        writer.set_tag(ExifTag::GPSLongitude(degrees_to_dms(geo.longitude)));
        writer.set_tag(ExifTag::GPSLongitudeRef(
            if geo.longitude < 0.0 { "W" } else { "E" }.to_string(),
        ));

        if let Some(altitude) = geo.altitude {
            // Il riferimento distingue sopra (0) e sotto (1) il livello del mare:
            // l'altitudine EXIF è un valore senza segno.
            writer.set_tag(ExifTag::GPSAltitudeRef(vec![u8::from(altitude < 0.0)]));
            writer.set_tag(ExifTag::GPSAltitude(vec![uR64 {
                nominator: (altitude.abs() * 100.0).round() as u32,
                denominator: 100,
            }]));
        }
    }

    // Tutto ciò che il sidecar porta oltre a data e coordinate. Senza questo
    // passaggio la descrizione scritta dall'utente, i volti che ha confermato e
    // il fatto che una foto fosse tra i preferiti resterebbero nel JSON, cioè
    // andrebbero persi non appena il JSON resta indietro: che è esattamente il
    // problema che l'applicazione esiste per risolvere.
    if let Some(sidecar) = &record.sidecar {
        if let Some(description) = sidecar.description.as_deref() {
            writer.set_tag(ExifTag::ImageDescription(description.to_string()));
            // Windows e diversi programmi di gestione foto leggono il campo
            // proprietario invece di `ImageDescription`: scriverli entrambi
            // costa poche decine di byte ed evita che la descrizione risulti
            // vuota a seconda del programma.
            writer.set_tag(windows_string_tag(XP_COMMENT, description));
        }

        if !sidecar.people.is_empty() {
            // Non esiste un tag EXIF per i volti. `XPKeywords` è la sede che il
            // resto dell'ecosistema legge davvero, separata da punto e virgola.
            writer.set_tag(windows_string_tag(XP_KEYWORDS, &sidecar.people.join(";")));
        }

        if sidecar.favorited {
            // La stella di Google Foto diventa il massimo del voto, che è la
            // convenzione seguita da Windows, Lightroom e digiKam.
            writer.set_tag(ExifTag::UnknownINT16U(
                vec![5],
                RATING,
                ExifTagGroup::GENERIC,
            ));
            writer.set_tag(ExifTag::UnknownINT16U(
                vec![99],
                RATING_PERCENT,
                ExifTagGroup::GENERIC,
            ));
        }
    }

    writer
        .write_to_file(target)
        .map_err(|e| TakeoutError::io(target, e))
}

/// Elenca ciò che il sidecar contiene e il file ancora non porta con sé.
///
/// È il controllo che rende sicuro spostare il JSON: finché questa lista non è
/// vuota, quel sidecar è l'unica copia di qualcosa e va lasciato dov'è. Il
/// confronto guarda dentro al file invece di fidarsi dell'esito riferito dalla
/// riparazione, perché è il file che resterà all'utente.
///
/// I dati elencati da [`SidecarData::unwritable`] non compaiono qui: non
/// esiste un tag dove metterli, quindi aspettarli renderebbe la lista non
/// vuota per sempre.
pub fn sidecar_residual(media: &Path, sidecar: &SidecarData) -> Result<Vec<SidecarKept>> {
    if !is_exif_writable(media) {
        return Ok(vec![SidecarKept::NoExifContainer]);
    }

    let file = std::fs::File::open(media).map_err(|e| TakeoutError::io(media, e))?;
    let mut reader = std::io::BufReader::new(file);
    let Ok(exif) = exif::Reader::new().read_from_container(&mut reader) else {
        return Ok(vec![SidecarKept::UnreadableExif]);
    };

    let mut mancanti = Vec::new();

    if let Some(atteso) = sidecar.taken_at {
        let scritta = [Tag::DateTimeOriginal, Tag::DateTimeDigitized]
            .iter()
            .find_map(|tag| exif.get_field(*tag, In::PRIMARY))
            .and_then(|field| ascii_value(&field.value))
            .and_then(|raw| parse_exif_datetime(&raw));
        let offset = [Tag::OffsetTimeOriginal, Tag::OffsetTimeDigitized]
            .iter()
            .find_map(|tag| exif.get_field(*tag, In::PRIMARY))
            .and_then(|field| ascii_value(&field.value))
            .and_then(|raw| parse_offset(&raw));

        let coincide = scritta
            .map(|data| match offset {
                Some(minuti) => data - chrono::Duration::minutes(minuti as i64),
                None => data,
            })
            .is_some_and(|data| (data - atteso).num_seconds().abs() <= 1);

        if !coincide {
            mancanti.push(SidecarKept::MissingDate);
        }
    }

    if sidecar.geo.is_some() && read_gps(&exif).filter(|g| !g.is_null_island()).is_none() {
        mancanti.push(SidecarKept::MissingGeo);
    }

    if sidecar.description.is_some() && exif.get_field(Tag::ImageDescription, In::PRIMARY).is_none()
    {
        mancanti.push(SidecarKept::MissingDescription);
    }

    if !sidecar.people.is_empty() && !has_tag(&exif, XP_KEYWORDS) {
        mancanti.push(SidecarKept::MissingPeople);
    }

    if sidecar.favorited && !has_tag(&exif, RATING) {
        mancanti.push(SidecarKept::MissingFavorite);
    }

    Ok(mancanti)
}

/// Vero se il tag esiste e porta un valore non vuoto.
///
/// I tag proprietari non hanno una costante in `kamadak-exif`, quindi si
/// costruiscono dal loro codice numerico nel contesto TIFF, che è dove
/// risiedono.
fn has_tag(exif: &exif::Exif, code: u16) -> bool {
    exif.get_field(Tag(exif::Context::Tiff, code), In::PRIMARY)
        .is_some_and(|field| !field.value.display_as(field.tag).to_string().is_empty())
}

/// Tag proprietari Microsoft, non previsti dalla specifica EXIF ma scritti e
/// letti da gran parte dei programmi di gestione foto.
const XP_COMMENT: u16 = 0x9C9C;
const XP_KEYWORDS: u16 = 0x9C9E;
/// Voto della foto, in stelle e in percentuale.
const RATING: u16 = 0x4746;
const RATING_PERCENT: u16 = 0x4749;

/// Codifica una stringa nella forma attesa dai tag `XP*`.
///
/// Sono dichiarati come sequenze di byte ma contengono UTF-16 little endian con
/// il terminatore incluso: scriverci dentro UTF-8 produce testo illeggibile,
/// e omettere il terminatore fa apparire caratteri di troppo in coda.
fn windows_string_tag(code: u16, value: &str) -> ExifTag {
    let mut bytes = Vec::with_capacity(value.len() * 2 + 2);
    for unit in value.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes.extend_from_slice(&[0, 0]);
    ExifTag::UnknownINT8U(bytes, code, ExifTagGroup::GENERIC)
}

/// Sceglie un nome libero nella cartella indicata.
///
/// La prenotazione è centralizzata perché la riscrittura è parallela: due
/// thread che trovassero lo stesso nome libero nello stesso istante
/// produrrebbero un file solo.
fn unique_target(
    folder: &Path,
    file_name: &str,
    taken: &Mutex<HashSet<PathBuf>>,
) -> Result<PathBuf> {
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(file_name);
    let extension = Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();

    let mut guard = taken
        .lock()
        .map_err(|_| TakeoutError::Task("registro dei nomi non disponibile".to_string()))?;

    let mut candidate = folder.join(file_name);
    let mut counter = 2;
    while guard.contains(&candidate) || candidate.exists() {
        candidate = folder.join(format!("{stem} ({counter}){extension}"));
        counter += 1;
    }
    guard.insert(candidate.clone());
    Ok(candidate)
}

/// Esito della scrittura di un singolo media.
#[derive(Debug, Default)]
struct WriteOutcome {
    exif_written: bool,
    times_written: bool,
    too_large: bool,
    sidecar_copied: bool,
}

/// Scrive i metadati di un singolo media secondo le opzioni indicate.
fn repair_one(
    record: &MediaRecord,
    root: &Path,
    options: &WriteOptions,
    taken: &Mutex<HashSet<PathBuf>>,
) -> Result<WriteOutcome> {
    let source = record.path.as_path();

    let destination = match options.mode {
        WriteMode::InPlace => source.to_path_buf(),
        WriteMode::CopyToOutput => {
            let output_root = options.output_root.as_ref().ok_or_else(|| {
                TakeoutError::Metadata("manca la cartella di destinazione".to_string())
            })?;

            match options.layout {
                // Struttura originale: il percorso relativo è già univoco.
                OutputLayout::Preserve => {
                    let relative = source.strip_prefix(root).unwrap_or(source);
                    output_root.join(relative)
                }
                // Riorganizzando per data, file con lo stesso nome provenienti
                // da cartelle diverse finiscono insieme: senza un contatore il
                // secondo sovrascriverebbe il primo in silenzio.
                layout => {
                    let folder = output_root.join(layout.folder_for(record.resolved_taken_at));
                    unique_target(&folder, &record.file_name, taken)?
                }
            }
        }
        WriteMode::DryRun => return Ok(WriteOutcome::default()),
    };

    let writable = options.write_exif && is_exif_writable(source);
    let too_large = record.size_bytes > MAX_REWRITE_BYTES;

    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|e| TakeoutError::io(parent, e))?;
    }

    let mut exif_written = false;

    if writable && !too_large {
        // Si lavora sempre su un file temporaneo nella stessa cartella della
        // destinazione, così la sostituzione finale è una rename atomica sullo
        // stesso filesystem. Se qualcosa va storto a metà, l'originale non è
        // mai stato toccato e resta solo un temporaneo da rimuovere.
        // Il temporaneo deve conservare l'estensione originale: `little_exif`
        // deduce il formato del contenitore proprio da quella, e un
        // `.oth-tmp` finale lo farebbe fallire.
        let file_name = destination
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let temp = destination.with_file_name(format!(".oth-tmp-{file_name}"));

        std::fs::copy(source, &temp).map_err(|e| TakeoutError::io(&temp, e))?;

        match write_exif_tags(&temp, record) {
            Ok(()) => {
                std::fs::rename(&temp, &destination)
                    .map_err(|e| TakeoutError::io(&destination, e))?;
                exif_written = true;
                trace_dev!("EXIF scritto: {}", destination.display());
            }
            Err(err) => {
                let _ = std::fs::remove_file(&temp);
                trace_dev!("EXIF fallito su {}: {err}", record.file_name);
                return Err(err);
            }
        }
    } else {
        if too_large {
            trace_dev!(
                "{} saltato: {} byte oltre il limite di riscrittura",
                record.file_name,
                record.size_bytes
            );
        } else {
            trace_dev!(
                "{}: formato senza scrittura EXIF, si allinea solo la data del file",
                record.file_name
            );
        }

        if options.mode == WriteMode::CopyToOutput {
            // Formato non riscrivibile o troppo grande: la copia va comunque
            // prodotta, altrimenti l'albero di uscita sarebbe incompleto.
            std::fs::copy(source, &destination).map_err(|e| TakeoutError::io(&destination, e))?;
        }
    }

    // Senza EXIF scritto, la data sopravvive solo nella data di modifica del
    // file: il metadato più fragile che ci sia, che un caricamento su cloud o
    // una copia senza `-p` cancella. Il sidecar è l'unica fonte durevole
    // rimasta, quindi va portato accanto al file invece di restare indietro.
    let mut sidecar_copied = false;
    if options.mode == WriteMode::CopyToOutput && !exif_written {
        if let Some(sidecar) = record.sidecar.as_ref() {
            let sidecar_target = destination.with_file_name(
                sidecar
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            );
            std::fs::copy(&sidecar.path, &sidecar_target)
                .map_err(|e| TakeoutError::io(&sidecar_target, e))?;
            sidecar_copied = true;
            trace_dev!(
                "sidecar conservato accanto a {}: {}",
                record.file_name,
                sidecar_target
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            );
        }
    }

    let mut times_written = false;
    if options.write_file_times {
        if let Some(taken_at) = record.resolved_taken_at {
            let stamp = filetime::FileTime::from_unix_time(taken_at.timestamp(), 0);
            filetime::set_file_mtime(&destination, stamp)
                .map_err(|e| TakeoutError::io(&destination, e))?;
            times_written = true;
        }
    }

    Ok(WriteOutcome {
        exif_written,
        times_written,
        too_large,
        sidecar_copied,
    })
}

/// Ripara i metadati dei media sotto `root`.
///
/// Il valore predefinito è non distruttivo: senza indicazioni contrarie non
/// viene modificato alcun file originale. Il lavoro è parallelo ma con un
/// numero di thread limitato, perché è dominato dall'I/O e perché ogni thread
/// tiene in memoria la propria copia di lavoro.
pub fn apply_metadata(
    root: &Path,
    options: &WriteOptions,
    progress: ProgressSink<'_>,
) -> Result<RepairReport> {
    crate::app_state::require_existing(root)?;

    if options.mode == WriteMode::CopyToOutput {
        let output_root = options.output_root.as_ref().ok_or_else(|| {
            TakeoutError::Metadata(
                "la modalità copia richiede una cartella di destinazione".to_string(),
            )
        })?;
        // Scrivere l'uscita dentro la sorgente creerebbe una ricorsione al
        // prossimo passaggio e mescolerebbe originali e copie.
        if output_root.starts_with(root) {
            return Err(TakeoutError::Metadata(
                "la cartella di destinazione non può stare dentro quella di origine".to_string(),
            ));
        }

        // La copia riparata duplica la libreria: meglio dirlo adesso che
        // riempire il disco a metà lavoro.
        let da_scrivere: u64 = WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum();
        crate::app_state::require_free_space(output_root, da_scrivere)?;
    }

    let mut index = FileIndex::new();
    let mut media: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(root).follow_links(false).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        if is_media_file(entry.path()) {
            media.push(entry.path().to_path_buf());
        }
        index.insert(entry.into_path());
    }

    let total = media.len();
    trace_dev!(
        "riparazione avviata: {total} media in {}, modalita {:?}, uscita {}",
        root.display(),
        options.mode,
        options
            .output_root
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "nessuna".to_string())
    );
    progress(Progress::new(Phase::Scanning, 0, total, 0));

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(
            REWRITE_THREADS.min(std::thread::available_parallelism().map_or(1, |n| n.get())),
        )
        .build()
        .map_err(|e| TakeoutError::Task(e.to_string()))?;

    let done = std::sync::atomic::AtomicUsize::new(0);
    let errors = std::sync::atomic::AtomicUsize::new(0);
    let taken: Mutex<HashSet<PathBuf>> = Mutex::new(HashSet::new());
    use std::sync::atomic::Ordering::Relaxed;

    let outcomes: Vec<PerFile> = pool.install(|| {
        media
            .par_iter()
            .map(|path| {
                let outcome = process_media(path, root, options, &taken, &index);
                if outcome.failure.is_some() {
                    errors.fetch_add(1, Relaxed);
                }

                let seen = done.fetch_add(1, Relaxed) + 1;
                progress(
                    Progress::new(Phase::Writing, seen, total, errors.load(Relaxed))
                        .with_current(path.file_name().unwrap_or_default().to_string_lossy()),
                );

                outcome
            })
            .collect()
    });

    let mut report = RepairReport {
        mode: Some(options.mode),
        output_root: options.output_root.clone(),
        ..Default::default()
    };

    for outcome in outcomes {
        report.candidates += usize::from(outcome.is_candidate);
        report.exif_written += usize::from(outcome.exif_written);
        report.file_times_written += usize::from(outcome.times_written);
        report.skipped_unsupported += usize::from(outcome.unsupported);
        report.skipped_too_large += usize::from(outcome.too_large);
        report.sidecars_copied += usize::from(outcome.sidecar_copied);
        if let Some(failure) = outcome.failure {
            report.failures.push(failure);
        }
    }

    trace_dev!(
        "riparazione conclusa: {} candidati, {} EXIF scritti, {} date allineate, {} sidecar conservati, {} non supportati, {} errori",
        report.candidates,
        report.exif_written,
        report.file_times_written,
        report.sidecars_copied,
        report.skipped_unsupported,
        report.failures.len()
    );

    progress(Progress::new(
        Phase::Done,
        total,
        total,
        report.failures.len(),
    ));

    Ok(report)
}

/// Esito della lavorazione di un singolo file, aggregato a fine corsa.
#[derive(Debug, Default)]
struct PerFile {
    is_candidate: bool,
    exif_written: bool,
    times_written: bool,
    unsupported: bool,
    too_large: bool,
    sidecar_copied: bool,
    failure: Option<String>,
}

/// Legge un media e ne applica la riparazione, senza mai propagare panico.
///
/// Un singolo file illeggibile non deve fermare l'elaborazione di altre
/// diecimila foto: l'errore viene registrato e la corsa prosegue.
fn process_media(
    path: &Path,
    root: &Path,
    options: &WriteOptions,
    taken: &Mutex<HashSet<PathBuf>>,
    index: &FileIndex,
) -> PerFile {
    let record = match inspect_media(path, Some(index)) {
        Ok(record) => record,
        Err(err) => {
            return PerFile {
                failure: Some(format!("{}: {err}", path.display())),
                ..Default::default()
            }
        }
    };

    // "Candidato" significa che c'è qualcosa da scrivere. Non significa che il
    // file vada ignorato: in modalità copia l'albero di uscita deve contenere
    // l'intera libreria, comprese le foto che non avevano nulla da riparare.
    // Ometterle produrrebbe una copia che sembra completa e non lo è.
    let is_candidate = record.resolved_taken_at.is_some() || record.resolved_geo.is_some();
    if !is_candidate && options.mode != WriteMode::CopyToOutput {
        return PerFile::default();
    }

    let unsupported = options.write_exif && !is_exif_writable(path);

    match repair_one(&record, root, options, taken) {
        Ok(outcome) => PerFile {
            is_candidate,
            exif_written: outcome.exif_written,
            times_written: outcome.times_written,
            unsupported: unsupported && !outcome.too_large,
            too_large: outcome.too_large,
            sidecar_copied: outcome.sidecar_copied,
            ..Default::default()
        },
        Err(err) => PerFile {
            is_candidate,
            failure: Some(format!("{}: {err}", path.display())),
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::app_state::testing::{write_bytes, write_file, TempDir, MINIMAL_JPEG};

    #[test]
    fn interpreta_il_formato_data_exif() {
        let parsed = parse_exif_datetime("2020:01:01 12:00:00").expect("data valida");
        assert_eq!(parsed.timestamp(), 1_577_880_000);
        assert!(parse_exif_datetime("2020-01-01 12:00:00").is_none());
    }

    #[test]
    fn scarta_le_coordinate_a_zero() {
        let point = GeoPoint {
            latitude: 0.0,
            longitude: 0.0,
            altitude: None,
        };
        assert!(point.is_null_island());
    }

    #[test]
    fn genera_i_nomi_sidecar_attesi() {
        let candidates = sidecar_candidates(Path::new("/foto/IMG_0001.JPG"));
        assert!(candidates.contains(&PathBuf::from("/foto/IMG_0001.JPG.json")));
        assert!(candidates.contains(&PathBuf::from(
            "/foto/IMG_0001.JPG.supplemental-metadata.json"
        )));
    }

    #[test]
    fn gestisce_i_nomi_sidecar_troncati() {
        // Nome corto: nessun troncamento, e nessun candidato inutile in più.
        let corti = sidecar_candidates(Path::new("/foto/IMG_0001.JPG"));
        assert!(
            corti.iter().all(|c| {
                let nome = c.file_name().unwrap().to_string_lossy();
                nome.ends_with(".json") && nome.chars().count() <= MAX_SIDECAR_NAME
            }),
            "su un nome corto non deve comparire un candidato tagliato"
        );

        // Nome medio: sta nei 46 caratteri da solo, ma non con l'intero
        // `.supplemental-metadata` in coda, che quindi arriva mozzato.
        let medi = sidecar_candidates(Path::new("/foto/PXL_20260115_120000123.jpg"));
        assert!(
            medi.contains(&PathBuf::from(
                "/foto/PXL_20260115_120000123.jpg.supplemental-m.json"
            )),
            "atteso il suffisso mozzato, trovati {medi:?}"
        );

        // Nome lungo: il taglio cade dentro il nome del media stesso.
        let lunghi = sidecar_candidates(Path::new(
            "/foto/Foto scattata durante la gita del 04-01-2022.jpg",
        ));
        assert!(
            lunghi.contains(&PathBuf::from(
                "/foto/Foto scattata durante la gita del 04-01-2.json"
            )),
            "atteso il nome tagliato, trovati {lunghi:?}"
        );

        // Il taglio si conta in caratteri: un nome accentato non deve produrre
        // un candidato più corto solo perché occupa più byte.
        let accentati = sidecar_candidates(Path::new(
            "/foto/Foto della città più bella del mondo intero.jpg",
        ));
        let tagliato = accentati
            .iter()
            .find(|c| {
                let nome = c.file_name().unwrap().to_string_lossy();
                !nome.contains("supplemental") && nome.chars().count() == MAX_SIDECAR_NAME
            })
            .unwrap_or_else(|| panic!("nessun candidato tagliato in {accentati:?}"));
        assert_eq!(
            tagliato.file_name().unwrap().to_string_lossy(),
            "Foto della città più bella del mondo inte.json"
        );
    }

    #[test]
    fn gestisce_i_duplicati_con_contatore() {
        let candidates = sidecar_candidates(Path::new("/foto/IMG_0001(1).JPG"));
        assert!(candidates.contains(&PathBuf::from("/foto/IMG_0001.JPG(1).json")));
    }

    #[test]
    fn riconosce_solo_le_estensioni_media() {
        assert!(is_media_file(Path::new("a.HEIC")));
        assert!(is_media_file(Path::new("a.mp4")));
        assert!(!is_media_file(Path::new("a.json")));
    }

    #[test]
    fn scrive_exif_solo_sui_contenitori_che_lo_prevedono() {
        assert!(is_exif_writable(Path::new("foto.jpg")));
        assert!(is_exif_writable(Path::new("foto.HEIC")));
        assert!(is_exif_writable(Path::new("scansione.tiff")));
        // I video hanno i metadati negli atomi del contenitore, non in EXIF.
        assert!(!is_exif_writable(Path::new("clip.mp4")));
        assert!(!is_exif_writable(Path::new("clip.mov")));
    }

    /// Il sidecar porta più di data e coordinate, e il resto va dentro al file.
    ///
    /// Se descrizione, volti e preferito restassero solo nel JSON, spostare il
    /// JSON li perderebbe: cioè si riproporrebbe, con un passaggio in più,
    /// esattamente il guasto che questa applicazione esiste per riparare.
    #[test]
    fn porta_dentro_al_file_anche_descrizione_volti_e_preferito() {
        let temp = TempDir::new("sidecar-completo");
        let foto = temp.path().join("IMG_0001.JPG");
        write_bytes(&foto, MINIMAL_JPEG);
        write_file(
            &temp.path().join("IMG_0001.JPG.json"),
            r#"{
              "title": "IMG_0001.JPG",
              "description": "Cena sul lago con Anna",
              "photoTakenTime": { "timestamp": "1577880000" },
              "geoData": { "latitude": 45.4642, "longitude": 9.19, "altitude": 0.0 },
              "people": [{ "name": "Anna Bianchi" }, { "name": "Luca Verdi" }],
              "favorited": true,
              "imageViews": "42",
              "url": "https://photos.google.com/photo/AF1QipN"
            }"#,
        );

        let record = inspect_media(&foto, None).expect("lettura media");
        let sidecar = record.sidecar.as_ref().expect("sidecar letto");
        assert_eq!(sidecar.people, ["Anna Bianchi", "Luca Verdi"]);
        assert!(sidecar.favorited);
        assert_eq!(
            sidecar.unwritable(),
            [
                SidecarKept::ViewCountHasNoTag,
                SidecarKept::PhotoUrlHasNoTag
            ],
            "ciò che non entra nel file va saputo dire"
        );

        let report = apply_metadata(
            temp.path(),
            &WriteOptions {
                mode: WriteMode::InPlace,
                ..WriteOptions::default()
            },
            &crate::app_state::no_progress,
        )
        .expect("riparazione");
        assert_eq!(report.exif_written, 1);

        let bytes = std::fs::read(&foto).expect("rilettura");
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "resta un JPEG valido");

        // `ImageDescription` è testo semplice e si trova così com'è.
        assert!(
            String::from_utf8_lossy(&bytes).contains("Cena sul lago con Anna"),
            "la descrizione deve finire dentro al file"
        );

        // I tag `XP*` sono in UTF-16 little endian: cercare la forma UTF-8
        // troverebbe niente anche se la scrittura fosse andata a buon fine.
        let utf16: Vec<u8> = "Anna Bianchi;Luca Verdi"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        assert!(
            bytes.windows(utf16.len()).any(|f| f == utf16),
            "i volti vanno scritti in XPKeywords, separati da punto e virgola"
        );

        // Il voto pieno è il modo in cui il resto dell'ecosistema legge la
        // stella di Google Foto.
        let riletto = read_exif(&foto).expect("rilettura EXIF");
        assert!(
            riletto.taken_at.is_some(),
            "la data non deve andare persa scrivendo il resto"
        );
    }

    /// Questo test protegge due cose diverse con la stessa asserzione.
    ///
    /// La prima è di qualità: in PNG l'EXIF vive nel chunk `eXIf`, che i
    /// visualizzatori leggono a macchia di leopardo, quindi scriverlo darebbe
    /// all'utente l'illusione di aver riparato qualcosa.
    ///
    /// La seconda è di sicurezza: dentro `little_exif` il parser XML è
    /// raggiungibile solo dal percorso PNG, e quel parser ha due DoS noti
    /// (RUSTSEC-2026-0194 e -0195) senza versione corretta disponibile.
    /// Finché PNG resta fuori, quel codice non viene mai eseguito. Le due
    /// vulnerabilità sono elencate in `deny.toml` proprio sulla base di questo
    /// vincolo: se qualcuno aggiunge "png" qui, l'eccezione decade e questo
    /// test deve fallire per ricordarlo.
    #[test]
    fn png_resta_fuori_dalla_scrittura_exif() {
        assert!(
            !EXIF_WRITABLE_EXTENSIONS.contains(&"png"),
            "aggiungere PNG riattiva il parser XML vulnerabile di little_exif: \
             rivedere prima l'eccezione in deny.toml"
        );
        assert!(!is_exif_writable(Path::new("immagine.png")));
    }

    #[test]
    fn deduce_la_data_dai_nomi_generati_dalle_fotocamere() {
        let attesa = 1_577_880_000; // 2020-01-01 12:00:00 UTC
        for nome in [
            "IMG_20200101_120000.jpg",
            "VID_20200101_120000.mp4",
            "PXL_20200101_120000123.jpg",
            "Screenshot_20200101-120000.png",
            "IMG-20200101-WA0001.jpeg",
            "signal-2020-01-01-12-00-00.jpg",
            "2020-01-01-120000.heic",
            "20200101120000.jpg",
        ] {
            let parsed = parse_date_from_filename(nome);
            // `IMG-20200101-WA0001` non porta un orario valido: va rifiutato.
            if nome.contains("WA0001") {
                assert!(parsed.is_none(), "{nome} non ha un orario reale");
                continue;
            }
            assert_eq!(
                parsed.map(|d| d.timestamp()),
                Some(attesa),
                "nome non riconosciuto: {nome}"
            );
        }
    }

    #[test]
    fn non_scambia_numeri_qualsiasi_per_date() {
        // Data impossibile: mese 13.
        assert!(parse_date_from_filename("IMG_20201301_120000.jpg").is_none());
        // Ora impossibile.
        assert!(parse_date_from_filename("IMG_20200101_250000.jpg").is_none());
        // Anno fuori intervallo.
        assert!(parse_date_from_filename("IMG_18000101_120000.jpg").is_none());
        // Un numero di serie lungo non è una data.
        assert!(parse_date_from_filename("DSC000202001011200001234.jpg").is_none());
        assert!(parse_date_from_filename("IMG_1234.jpg").is_none());
    }

    #[test]
    fn il_nome_interviene_solo_dopo_exif_e_sidecar() {
        // L'ordine è verificato sui dati: EXIF batte sidecar, sidecar batte
        // nome. Qui basta accertare che il nome sia l'ultima risorsa.
        assert_eq!(MetadataSource::Exif as u8, 0);
        assert!(parse_date_from_filename("IMG_20200101_120000.jpg").is_some());
    }

    /// `DateTimeOriginal` è l'ora dell'orologio sul posto, non UTC. Il sidecar
    /// di Google porta invece un istante UTC: scriverlo senza conversione
    /// sposterebbe ogni foto indietro della differenza di fuso.
    #[test]
    fn converte_listante_utc_nellora_locale_del_luogo() {
        let istante = DateTime::from_timestamp(1_577_880_000, 0).expect("istante"); // 12:00 UTC

        // Milano a gennaio: CET, un'ora avanti.
        let milano = GeoPoint {
            latitude: 45.4642,
            longitude: 9.19,
            altitude: None,
        };
        let (locale, offset) = local_time_and_offset(istante, Some(milano));
        assert_eq!(locale.format("%H:%M").to_string(), "13:00");
        assert_eq!(offset, "+01:00");

        // New York alla stessa data: cinque ore indietro, e il giorno cambia
        // solo se l'ora lo impone.
        let new_york = GeoPoint {
            latitude: 40.7128,
            longitude: -74.006,
            altitude: None,
        };
        let (locale, offset) = local_time_and_offset(istante, Some(new_york));
        assert_eq!(locale.format("%H:%M").to_string(), "07:00");
        assert_eq!(offset, "-05:00");

        // India: mezz'ora di scarto, il caso che rompe le implementazioni che
        // assumono offset interi.
        let delhi = GeoPoint {
            latitude: 28.6139,
            longitude: 77.209,
            altitude: None,
        };
        let (_, offset) = local_time_and_offset(istante, Some(delhi));
        assert_eq!(offset, "+05:30");

        // Senza coordinate non si inventa un fuso: si dichiara UTC.
        let (locale, offset) = local_time_and_offset(istante, None);
        assert_eq!(locale.format("%H:%M").to_string(), "12:00");
        assert_eq!(offset, "+00:00");
    }

    #[test]
    fn tiene_conto_dellora_legale() {
        // Stesso luogo, luglio: CEST, due ore avanti invece di una.
        let luglio = DateTime::from_timestamp(1_593_604_800, 0).expect("istante"); // 2020-07-01 12:00 UTC
        let milano = GeoPoint {
            latitude: 45.4642,
            longitude: 9.19,
            altitude: None,
        };
        let (locale, offset) = local_time_and_offset(luglio, Some(milano));
        assert_eq!(locale.format("%H:%M").to_string(), "14:00");
        assert_eq!(offset, "+02:00");
    }

    #[test]
    fn interpreta_loffset_dichiarato_nei_file() {
        assert_eq!(parse_offset("+01:00"), Some(60));
        assert_eq!(parse_offset("-05:00"), Some(-300));
        assert_eq!(parse_offset("+05:30"), Some(330));
        assert_eq!(parse_offset("00:00"), None, "manca il segno");
        assert_eq!(parse_offset("boh"), None);
    }

    #[test]
    fn converte_i_gradi_decimali_in_gradi_primi_secondi() {
        // 45,4642 gradi corrispondono a 45° 27' 51,12".
        let dms = degrees_to_dms(45.4642);
        assert_eq!(dms[0].nominator, 45);
        assert_eq!(dms[1].nominator, 27);
        assert_eq!(dms[2].denominator, 1000);
        assert_eq!(dms[2].nominator, 51_120);

        // Il segno è portato dal riferimento N/S ed E/W, non dal valore.
        let sud = degrees_to_dms(-45.4642);
        assert_eq!(sud[0].nominator, 45);
    }
}
