// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Lettura e pulizia dell'export Calendario di Google (iCalendar, RFC 5545).
//!
//! Google esporta un `.ics` per ogni calendario dell'account. I file sono
//! utilizzabili così come sono, ma portano due fastidi per chi migra altrove:
//! proprietà proprietarie `X-GOOGLE-*` che nessun altro servizio interpreta, e
//! lo stesso evento ripetuto quando due calendari condividono un invito.
//!
//! Il formato di content line è lo stesso di vCard, quindi unfolding, escaping
//! e divisione delle proprietà arrivano da [`crate::contacts`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::app_state::{ExportReport, Notice, Result, TakeoutError};
use crate::contacts::{split_property, unescape, unfold};

/// Lunghezza massima di una riga prima del folding, in ottetti (RFC 5545).
const FOLD_WIDTH: usize = 75;

/// Proprietà che vengono rimosse durante la pulizia.
///
/// Sono estensioni di Google senza significato fuori dai suoi servizi: le
/// lasciamo fuori dal file esportato invece di trascinarle in Proton o
/// Nextcloud, dove diventano rumore.
const DROPPED_PREFIXES: &[&str] = &["X-GOOGLE-", "X-MICROSOFT-", "X-EVOLUTION-"];

/// Proprietà di un evento conservate nell'export pulito, in ordine di scrittura.
const KEPT_PROPERTIES: &[&str] = &[
    "UID",
    "DTSTAMP",
    "DTSTART",
    "DTEND",
    "DURATION",
    "RRULE",
    "EXDATE",
    "RDATE",
    "SUMMARY",
    "DESCRIPTION",
    "LOCATION",
    "STATUS",
    "TRANSP",
    "CLASS",
    "CATEGORIES",
    "ORGANIZER",
    "ATTENDEE",
    "URL",
    "SEQUENCE",
    "CREATED",
    "LAST-MODIFIED",
    "RECURRENCE-ID",
];

/// Un evento, ridotto ai campi che sopravvivono alla pulizia.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEvent {
    pub uid: Option<String>,
    pub summary: Option<String>,
    pub location: Option<String>,
    pub description: Option<String>,
    /// Inizio, nella forma grezza dell'iCalendar (`20200101T120000Z`).
    pub start: Option<String>,
    pub end: Option<String>,
    pub is_recurring: bool,
    pub is_all_day: bool,
    /// Righe conservate, già normalizzate, pronte per la riscrittura.
    #[serde(skip)]
    lines: Vec<(String, String)>,
}

impl CalendarEvent {
    fn is_empty(&self) -> bool {
        self.uid.is_none() && self.summary.is_none() && self.start.is_none()
    }

    /// Chiave di deduplica: l'UID identifica l'evento, ma un'occorrenza singola
    /// di una serie ricorrente ha lo stesso UID e un `RECURRENCE-ID` diverso.
    fn dedup_key(&self) -> Option<String> {
        let uid = self.uid.as_ref()?;
        let recurrence = self
            .lines
            .iter()
            .find(|(name, _)| name.starts_with("RECURRENCE-ID"))
            .map(|(_, value)| value.as_str())
            .unwrap_or("");
        Some(format!("{uid}|{recurrence}"))
    }
}

/// Esito della lettura di uno o più file iCalendar.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarReport {
    pub sources: Vec<PathBuf>,
    pub total: usize,
    pub unique: usize,
    pub duplicates: usize,
    pub recurring: usize,
    pub all_day: usize,
    /// Proprietà proprietarie rimosse durante la pulizia.
    pub dropped_properties: usize,
    pub warnings: Vec<Notice>,
    pub sample: Vec<CalendarEvent>,
}

/// Vero se la proprietà va scartata perché specifica di un fornitore.
fn is_dropped(name: &str) -> bool {
    DROPPED_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// Vero se la proprietà va conservata nell'export pulito.
///
/// Il confronto ignora i parametri: `DTSTART;VALUE=DATE` resta `DTSTART`.
fn is_kept(name: &str) -> bool {
    let base = name.split(';').next().unwrap_or(name);
    KEPT_PROPERTIES.contains(&base)
}

/// Interpreta il contenuto di un file iCalendar.
pub fn parse_ics(content: &str) -> (Vec<CalendarEvent>, usize) {
    let mut events = Vec::new();
    let mut current: Option<CalendarEvent> = None;
    let mut dropped = 0usize;
    // Gli allarmi sono blocchi annidati dentro l'evento: le loro proprietà non
    // vanno confuse con quelle dell'evento che le contiene.
    let mut inside_alarm = false;

    for line in unfold(content) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed.eq_ignore_ascii_case("BEGIN:VEVENT") {
            current = Some(CalendarEvent::default());
            continue;
        }
        if trimmed.eq_ignore_ascii_case("END:VEVENT") {
            if let Some(event) = current.take() {
                if !event.is_empty() {
                    events.push(event);
                }
            }
            continue;
        }
        if trimmed.eq_ignore_ascii_case("BEGIN:VALARM") {
            inside_alarm = true;
            continue;
        }
        if trimmed.eq_ignore_ascii_case("END:VALARM") {
            inside_alarm = false;
            continue;
        }

        let Some(event) = current.as_mut() else {
            continue;
        };
        if inside_alarm {
            continue;
        }

        let Some((name, _, raw_value)) = split_property(trimmed) else {
            continue;
        };

        if is_dropped(&name) {
            dropped += 1;
            continue;
        }

        // La riga completa (con parametri) va conservata per la riscrittura: in
        // `DTSTART;TZID=Europe/Rome` il fuso orario sta nei parametri, e
        // buttarlo sposterebbe l'evento di ore.
        let full_name = trimmed
            .split_once(':')
            .map(|(head, _)| head.to_string())
            .unwrap_or_else(|| name.clone());

        if is_kept(&full_name.to_ascii_uppercase()) {
            event.lines.push((full_name.clone(), raw_value.clone()));
        }

        let value = unescape(&raw_value).trim().to_string();
        let base = name.as_str();

        match base {
            "UID" => event.uid = Some(value),
            "SUMMARY" => event.summary = Some(value),
            "LOCATION" => event.location = Some(value),
            "DESCRIPTION" => event.description = Some(value),
            "DTSTART" => {
                // `VALUE=DATE` senza orario significa evento di giornata intera.
                event.is_all_day =
                    full_name.to_ascii_uppercase().contains("VALUE=DATE") && !value.contains('T');
                event.start = Some(value);
            }
            "DTEND" => event.end = Some(value),
            "RRULE" => event.is_recurring = true,
            _ => {}
        }
    }

    (events, dropped)
}

/// Legge un singolo file `.ics`.
pub fn parse_file(path: &Path) -> Result<(Vec<CalendarEvent>, usize)> {
    let content = std::fs::read_to_string(path).map_err(|e| TakeoutError::io(path, e))?;
    Ok(parse_ics(&content))
}

/// Raccoglie i file `.ics` sotto `root`, oppure il singolo file indicato.
fn collect_ics(root: &Path) -> Vec<PathBuf> {
    if root.is_file() {
        return vec![root.to_path_buf()];
    }

    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("ics"))
                .unwrap_or(false)
        })
        .collect()
}

/// Legge tutti i calendari sotto `root` e restituisce report ed eventi unici.
fn collect_events(root: &Path) -> Result<(CalendarReport, Vec<CalendarEvent>)> {
    crate::app_state::require_existing(root)?;

    let mut report = CalendarReport::default();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut unique: Vec<CalendarEvent> = Vec::new();

    for file in collect_ics(root) {
        match parse_file(&file) {
            Ok((events, dropped)) => {
                report.sources.push(file);
                report.dropped_properties += dropped;

                for event in events {
                    report.total += 1;
                    if event.is_recurring {
                        report.recurring += 1;
                    }
                    if event.is_all_day {
                        report.all_day += 1;
                    }

                    match event.dedup_key() {
                        Some(key) if seen.contains_key(&key) => report.duplicates += 1,
                        Some(key) => {
                            seen.insert(key, unique.len());
                            unique.push(event);
                        }
                        None => unique.push(event),
                    }
                }
            }
            Err(err) => report
                .warnings
                .push(Notice::read_failed(file.display(), err)),
        }
    }

    report.unique = unique.len();
    Ok((report, unique))
}

/// Analizza i calendari sotto `root`.
pub fn scan_directory(root: &Path, sample_size: usize) -> Result<CalendarReport> {
    let (mut report, unique) = collect_events(root)?;
    report.sample = unique.into_iter().take(sample_size).collect();
    Ok(report)
}

/// Spezza una riga secondo la regola di folding di RFC 5545.
///
/// Il limite è in ottetti, non in caratteri: tagliare a metà di una sequenza
/// UTF-8 produrrebbe un file illeggibile, quindi il taglio avanza per confini
/// di carattere.
fn fold_line(line: &str) -> String {
    if line.len() <= FOLD_WIDTH {
        return line.to_string();
    }

    let mut out = String::with_capacity(line.len() + line.len() / FOLD_WIDTH * 3);
    let mut budget = FOLD_WIDTH;
    let mut used = 0usize;

    for ch in line.chars() {
        let width = ch.len_utf8();
        if used + width > budget {
            out.push_str("\r\n ");
            used = 1; // lo spazio iniziale della continuazione conta
            budget = FOLD_WIDTH;
        }
        out.push(ch);
        used += width;
    }

    out
}

/// Scrive un calendario iCalendar 2.0 pulito.
///
/// Il file prodotto contiene solo proprietà standard, un evento per UID e le
/// righe ripiegate come prescritto: è importabile su Proton, Tuta e Nextcloud
/// senza passaggi intermedi.
pub fn export_ics(root: &Path, destination: &Path) -> Result<ExportReport> {
    let (_, events) = collect_events(root)?;

    let mut out = String::new();
    out.push_str("BEGIN:VCALENDAR\r\n");
    out.push_str("VERSION:2.0\r\n");
    out.push_str("PRODID:-//Open Takeout Hub//EN\r\n");
    out.push_str("CALSCALE:GREGORIAN\r\n");

    // I valori vengono riscritti come sono stati letti, cioè ancora protetti
    // dall'escaping del file di origine. Ri-applicarlo qui produrrebbe un
    // doppio escaping e trasformerebbe "Milano\, Italia" in "Milano\\, Italia".
    for event in &events {
        out.push_str("BEGIN:VEVENT\r\n");
        // L'ordine segue KEPT_PROPERTIES, così file diversi prodotti dallo
        // stesso input risultano identici e confrontabili con un diff.
        for wanted in KEPT_PROPERTIES {
            for (name, value) in &event.lines {
                let base = name.split(';').next().unwrap_or(name).to_ascii_uppercase();
                if base == *wanted {
                    out.push_str(&fold_line(&format!("{name}:{value}")));
                    out.push_str("\r\n");
                }
            }
        }
        out.push_str("END:VEVENT\r\n");
    }

    out.push_str("END:VCALENDAR\r\n");

    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|e| TakeoutError::io(parent, e))?;
    }
    std::fs::write(destination, &out).map_err(|e| TakeoutError::io(destination, e))?;

    Ok(ExportReport {
        path: destination.to_path_buf(),
        written: events.len(),
        bytes: out.len() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Google Inc//Google Calendar 70.9054//EN\r\n\
         BEGIN:VEVENT\r\nUID:evento-1@google.com\r\nDTSTART:20200101T120000Z\r\nDTEND:20200101T130000Z\r\n\
         SUMMARY:Riunione\r\nLOCATION:Milano\r\nX-GOOGLE-CONFERENCE:https://meet.google.com/abc\r\n\
         BEGIN:VALARM\r\nACTION:DISPLAY\r\nSUMMARY:Promemoria allarme\r\nEND:VALARM\r\nEND:VEVENT\r\n\
         BEGIN:VEVENT\r\nUID:evento-2@google.com\r\nDTSTART;VALUE=DATE:20200315\r\nSUMMARY:Compleanno\r\n\
         RRULE:FREQ=YEARLY\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn legge_gli_eventi_di_base() {
        let (events, dropped) = parse_ics(SAMPLE);

        assert_eq!(events.len(), 2);
        assert_eq!(dropped, 1, "la proprietà X-GOOGLE- va scartata");

        assert_eq!(events[0].uid.as_deref(), Some("evento-1@google.com"));
        assert_eq!(events[0].summary.as_deref(), Some("Riunione"));
        assert_eq!(events[0].location.as_deref(), Some("Milano"));
        assert!(!events[0].is_recurring);
    }

    #[test]
    fn non_confonde_gli_allarmi_con_levento() {
        let (events, _) = parse_ics(SAMPLE);
        // Il VALARM ha un proprio SUMMARY: non deve sovrascrivere quello
        // dell'evento che lo contiene.
        assert_eq!(events[0].summary.as_deref(), Some("Riunione"));
    }

    #[test]
    fn riconosce_ricorrenze_e_giornate_intere() {
        let (events, _) = parse_ics(SAMPLE);
        assert!(events[1].is_recurring, "RRULE indica una ricorrenza");
        assert!(
            events[1].is_all_day,
            "VALUE=DATE indica una giornata intera"
        );
    }

    #[test]
    fn deduplica_per_uid() {
        let doppio = format!("{SAMPLE}{SAMPLE}");
        let temp = crate::app_state::testing::TempDir::new("cal-dedup");
        let file = temp.path().join("calendario.ics");
        crate::app_state::testing::write_file(&file, &doppio);

        let report = scan_directory(temp.path(), 10).expect("scansione");
        assert_eq!(report.total, 4);
        assert_eq!(report.duplicates, 2);
        assert_eq!(report.unique, 2);
    }

    #[test]
    fn esporta_un_ics_pulito_e_rileggibile() {
        let temp = crate::app_state::testing::TempDir::new("cal-export");
        let file = temp.path().join("calendario.ics");
        crate::app_state::testing::write_file(&file, SAMPLE);

        let destination = temp.path().join("uscita").join("calendar_cleaned.ics");
        let report = export_ics(temp.path(), &destination).expect("export");
        assert_eq!(report.written, 2);

        let content = std::fs::read_to_string(&destination).expect("lettura export");
        assert!(content.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(content.ends_with("END:VCALENDAR\r\n"));
        assert!(
            !content.contains("X-GOOGLE-"),
            "le proprietà proprietarie non devono sopravvivere"
        );
        assert!(
            !content.contains("Promemoria allarme"),
            "i VALARM non vengono riportati"
        );
        // Il fuso orario nei parametri va conservato.
        assert!(content.contains("DTSTART;VALUE=DATE:20200315"));

        // Prova del nove: il file prodotto deve essere rileggibile da noi stessi.
        let (rilette, _) = parse_ics(&content);
        assert_eq!(rilette.len(), 2);
        assert_eq!(rilette[0].summary.as_deref(), Some("Riunione"));
    }

    #[test]
    fn ripiega_le_righe_lunghe_senza_spezzare_i_caratteri() {
        let lunga = format!("DESCRIPTION:{}", "à".repeat(100));
        let folded = fold_line(&lunga);

        assert!(folded.contains("\r\n "), "la riga va ripiegata");
        for segment in folded.split("\r\n") {
            assert!(
                segment.len() <= FOLD_WIDTH + 1,
                "segmento troppo lungo: {} ottetti",
                segment.len()
            );
        }
        // Nessun carattere deve essere stato troncato a metà.
        let ricomposta = folded.replace("\r\n ", "");
        assert_eq!(ricomposta, lunga);
    }
}
