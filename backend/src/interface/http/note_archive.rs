//! The zip container of a notes export or import.
//!
//! Reading treats the archive as hostile: the entry count is checked before any
//! entry is opened, and every size is measured from the bytes actually
//! decompressed, never from the sizes the archive declares. Entry names are
//! labels in memory and never reach a file system.

use std::io::{Cursor, Read, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::application::knowledge::{ExportFile, ImportFile, SkippedFile};

/// What an import may ask of the server.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_entries: usize,
    pub max_entry_bytes: u64,
    pub max_total_bytes: u64,
}

impl Limits {
    /// The caps in `docs/api.md`. The 20 MB cap on the zip itself is the
    /// request body limit on the route.
    pub const IMPORT: Self = Self {
        max_entries: 5_000,
        max_entry_bytes: 1024 * 1024,
        max_total_bytes: 100 * 1024 * 1024,
    };
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ArchiveError {
    #[error("the file is not a readable zip archive")]
    NotAZip,
    #[error("the archive has too many entries (limit {max})")]
    TooManyEntries { max: usize },
    #[error("'{name}' is larger than the {max} byte limit for one note")]
    EntryTooLarge { name: String, max: u64 },
    #[error("the archive is larger than {max} bytes once unpacked")]
    TotalTooLarge { max: u64 },
    #[error("could not build the archive")]
    Build,
}

/// The notes found in an archive, and the entries that could not be read.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReadArchive {
    pub files: Vec<ImportFile>,
    pub skipped: Vec<SkippedFile>,
}

/// Longest entry name kept in a report.
const MAX_REPORTED_NAME_CHARS: usize = 200;

pub fn read_archive(bytes: &[u8], limits: &Limits) -> Result<ReadArchive, ArchiveError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|_| ArchiveError::NotAZip)?;
    if archive.len() > limits.max_entries {
        return Err(ArchiveError::TooManyEntries {
            max: limits.max_entries,
        });
    }
    let mut out = ReadArchive::default();
    let mut total: u64 = 0;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|_| ArchiveError::NotAZip)?;
        let Ok(raw_name) = entry.name() else {
            out.skipped
                .push(skip(format!("entry {index}"), "has an unreadable name"));
            continue;
        };
        let name: String = raw_name.chars().take(MAX_REPORTED_NAME_CHARS).collect();
        if entry.is_dir() || !name.to_ascii_lowercase().ends_with(".md") {
            continue;
        }
        let mut raw = Vec::new();
        let read = (&mut entry)
            .take(limits.max_entry_bytes + 1)
            .read_to_end(&mut raw);
        if read.is_err() {
            out.skipped.push(skip(name, "could not be unpacked"));
            continue;
        }
        if raw.len() as u64 > limits.max_entry_bytes {
            return Err(ArchiveError::EntryTooLarge {
                name,
                max: limits.max_entry_bytes,
            });
        }
        total += raw.len() as u64;
        if total > limits.max_total_bytes {
            return Err(ArchiveError::TotalTooLarge {
                max: limits.max_total_bytes,
            });
        }
        match String::from_utf8(raw) {
            Ok(text) => out.files.push(ImportFile { name, text }),
            Err(_) => out.skipped.push(skip(name, "is not UTF-8 text")),
        }
    }
    Ok(out)
}

fn skip(file: String, reason: &str) -> SkippedFile {
    SkippedFile {
        file,
        reason: reason.to_string(),
    }
}

pub fn build_archive(files: &[ExportFile]) -> Result<Vec<u8>, ArchiveError> {
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for file in files {
        writer
            .start_file(file.path.as_str(), options)
            .map_err(|_| ArchiveError::Build)?;
        writer
            .write_all(file.text.as_bytes())
            .map_err(|_| ArchiveError::Build)?;
    }
    let cursor = writer.finish().map_err(|_| ArchiveError::Build)?;
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn export_file(path: &str, text: &str) -> ExportFile {
        ExportFile {
            path: path.to_string(),
            text: text.to_string(),
        }
    }

    fn zip_of(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, data) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    const SMALL: Limits = Limits {
        max_entries: 3,
        max_entry_bytes: 100,
        max_total_bytes: 150,
    };

    #[test]
    fn a_built_archive_reads_back_the_same_files() {
        let files = [
            export_file("notes/a-1.md", "alpha 猫"),
            export_file("notes/b-2.md", "beta"),
        ];
        let read = read_archive(&build_archive(&files).unwrap(), &Limits::IMPORT).unwrap();
        let names: Vec<_> = read.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["notes/a-1.md", "notes/b-2.md"]);
        assert_eq!(read.files[0].text, "alpha 猫");
        assert!(read.skipped.is_empty());
    }

    #[test]
    fn only_markdown_entries_are_read() {
        let bytes = zip_of(&[
            ("notes/a.md", b"keep".to_vec()),
            ("notes/B.MD", b"keep too".to_vec()),
            ("image.png", vec![0; 10_000]),
            ("notes/readme.txt", b"no".to_vec()),
            ("../../etc/passwd", b"no".to_vec()),
        ]);
        let limits = Limits {
            max_entries: 10,
            ..SMALL
        };
        let read = read_archive(&bytes, &limits).unwrap();
        let names: Vec<_> = read.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["notes/a.md", "notes/B.MD"]);
    }

    #[test]
    fn too_many_entries_are_refused_before_any_is_read() {
        let entries: Vec<_> = (0..4).map(|n| (format!("{n}.md"), b"x".to_vec())).collect();
        let refs: Vec<_> = entries
            .iter()
            .map(|(n, d)| (n.as_str(), d.clone()))
            .collect();
        let err = read_archive(&zip_of(&refs), &SMALL).unwrap_err();
        assert_eq!(err, ArchiveError::TooManyEntries { max: 3 });
        assert!(err.to_string().contains('3'));
    }

    #[test]
    fn an_entry_over_the_cap_is_refused_by_its_real_size() {
        // 10 KB of zeros deflates to a few bytes, so only a decompressed
        // count catches it.
        let bytes = zip_of(&[("big.md", vec![b'a'; 10_000])]);
        let err = read_archive(&bytes, &SMALL).unwrap_err();
        assert!(
            matches!(err, ArchiveError::EntryTooLarge { max: 100, .. }),
            "{err}"
        );
    }

    #[test]
    fn many_small_entries_cannot_add_up_past_the_total_cap() {
        let bytes = zip_of(&[("a.md", vec![b'a'; 80]), ("b.md", vec![b'b'; 80])]);
        let err = read_archive(&bytes, &SMALL).unwrap_err();
        assert_eq!(err, ArchiveError::TotalTooLarge { max: 150 });
    }

    #[test]
    fn non_text_markdown_is_skipped_and_garbage_is_not_a_zip() {
        let bytes = zip_of(&[
            ("bad.md", vec![0xff, 0xfe, 0x00]),
            ("ok.md", b"fine".to_vec()),
        ]);
        let read = read_archive(&bytes, &SMALL).unwrap();
        assert_eq!(read.files.len(), 1);
        assert_eq!(read.skipped[0].file, "bad.md");

        assert_eq!(
            read_archive(b"definitely not a zip", &SMALL).unwrap_err(),
            ArchiveError::NotAZip
        );
    }
}
