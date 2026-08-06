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

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use exif::{In, Tag, Value};
use little_exif::exif_tag::ExifTag;
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
}

/// Origine scelta per il dato finale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataSource {
    Exif,
    Sidecar,
    Missing,
}

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
    pub total_bytes: u64,
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

/// Parametri della riparazione.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteOptions {
    pub mode: WriteMode,
    /// Radice di destinazione, obbligatoria con [`WriteMode::CopyToOutput`].
    pub output_root: Option<PathBuf>,
    /// Scrive data e coordinate nei tag EXIF del file.
    pub write_exif: bool,
    /// Allinea la data di modifica del file alla data di scatto.
    pub write_file_times: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            mode: WriteMode::DryRun,
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

    let taken_at = [Tag::DateTimeOriginal, Tag::DateTimeDigitized, Tag::DateTime]
        .iter()
        .find_map(|tag| exif.get_field(*tag, In::PRIMARY))
        .and_then(|field| ascii_value(&field.value))
        .and_then(|raw| parse_exif_datetime(&raw));

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

/// L'EXIF usa `YYYY:MM:DD HH:MM:SS` senza fuso orario: interpretiamo come UTC e
/// lo dichiariamo, invece di indovinare il fuso locale dello scatto.
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

/// Individua e legge il sidecar JSON associato al media.
pub fn read_sidecar(media: &Path) -> Result<Option<SidecarData>> {
    let Some(sidecar_path) = find_sidecar(media) else {
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
    }))
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

fn find_sidecar(media: &Path) -> Option<PathBuf> {
    let candidates = sidecar_candidates(media);
    let found = candidates.iter().find(|c| c.is_file()).cloned();

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
pub fn inspect_media(path: &Path) -> Result<MediaRecord> {
    let metadata = std::fs::metadata(path).map_err(|e| TakeoutError::io(path, e))?;
    let exif = read_exif(path)?;
    let sidecar = read_sidecar(path)?;

    let sidecar_taken = sidecar.as_ref().and_then(|s| s.taken_at.or(s.created_at));

    // L'EXIF, quando c'è, resta la fonte autorevole: descrive il momento dello
    // scatto, mentre il sidecar riflette quanto sa il servizio.
    let (resolved_taken_at, taken_at_source) = match (exif.taken_at, sidecar_taken) {
        (Some(date), _) => (Some(date), MetadataSource::Exif),
        (None, Some(date)) => (Some(date), MetadataSource::Sidecar),
        (None, None) => (None, MetadataSource::Missing),
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
        needs_repair: exif.taken_at.is_none() && sidecar_taken.is_some(),
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

    for entry in WalkDir::new(root).follow_links(false).into_iter().flatten() {
        if !entry.file_type().is_file() || !is_media_file(entry.path()) {
            continue;
        }

        match inspect_media(entry.path()) {
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
            Err(err) => report.unreadable.push(format!(
                "{}: {err}",
                entry
                    .path()
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            )),
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

/// Applica i tag EXIF risolti al file indicato.
///
/// Il file passato qui è sempre una copia di lavoro, mai l'originale: chi
/// chiama si occupa di crearla e di sostituirla in modo atomico.
fn write_exif_tags(target: &Path, record: &MediaRecord) -> Result<()> {
    // Un file senza blocco EXIF non è un errore: si parte da metadati vuoti e
    // `little_exif` inserisce il segmento mancante.
    let mut writer = ExifWriter::new_from_path(target).unwrap_or_else(|_| ExifWriter::new());

    if let Some(taken_at) = record.resolved_taken_at {
        let formatted = taken_at.format("%Y:%m:%d %H:%M:%S").to_string();
        writer.set_tag(ExifTag::DateTimeOriginal(formatted.clone()));
        writer.set_tag(ExifTag::CreateDate(formatted));
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

    writer
        .write_to_file(target)
        .map_err(|e| TakeoutError::io(target, e))
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
fn repair_one(record: &MediaRecord, root: &Path, options: &WriteOptions) -> Result<WriteOutcome> {
    let source = record.path.as_path();

    // Destinazione finale: l'originale, oppure il suo omologo nell'albero di
    // uscita, conservando la struttura di cartelle.
    let destination = match options.mode {
        WriteMode::InPlace => source.to_path_buf(),
        WriteMode::CopyToOutput => {
            let output_root = options.output_root.as_ref().ok_or_else(|| {
                TakeoutError::Metadata("manca la cartella di destinazione".to_string())
            })?;
            let relative = source.strip_prefix(root).unwrap_or(source);
            output_root.join(relative)
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
    }

    let media: Vec<PathBuf> = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file() && is_media_file(e.path()))
        .map(|e| e.into_path())
        .collect();

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
    use std::sync::atomic::Ordering::Relaxed;

    let outcomes: Vec<PerFile> = pool.install(|| {
        media
            .par_iter()
            .map(|path| {
                let outcome = process_media(path, root, options);
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
fn process_media(path: &Path, root: &Path, options: &WriteOptions) -> PerFile {
    let record = match inspect_media(path) {
        Ok(record) => record,
        Err(err) => {
            return PerFile {
                failure: Some(format!("{}: {err}", path.display())),
                ..Default::default()
            }
        }
    };

    // Vale la pena scrivere solo se abbiamo qualcosa da scrivere.
    let is_candidate = record.resolved_taken_at.is_some() || record.resolved_geo.is_some();
    if !is_candidate {
        return PerFile::default();
    }

    let unsupported = options.write_exif && !is_exif_writable(path);

    match repair_one(&record, root, options) {
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
