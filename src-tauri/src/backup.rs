//! Backup/restore (SPEC section 8): the database and `drawings/` folder as
//! one `.zip` archive, using SQLite's online backup API for a consistent
//! database snapshot (SPEC section 4.2: "Backup...therefore covers the
//! database and the drawings/ folder together, as one .zip archive").

use core_lib::rusqlite::{Connection, MAIN_DB};
use std::io::{Read, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const DB_ENTRY_NAME: &str = "patent-tagger.sqlite3";
const DRAWINGS_PREFIX: &str = "drawings/";

pub fn create_backup(data_dir: &Path, conn: &Connection, out_path: &Path) -> anyhow::Result<()> {
    let snapshot = tempfile::NamedTempFile::new()?;
    conn.backup(MAIN_DB, snapshot.path(), None)?;

    let file = std::fs::File::create(out_path)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    zip.start_file(DB_ENTRY_NAME, options)?;
    let mut db_bytes = Vec::new();
    std::fs::File::open(snapshot.path())?.read_to_end(&mut db_bytes)?;
    zip.write_all(&db_bytes)?;

    let drawings_dir = data_dir.join("drawings");
    if drawings_dir.is_dir() {
        add_dir_to_zip(&mut zip, &drawings_dir, &drawings_dir, options)?;
    }

    zip.finish()?;
    Ok(())
}

fn add_dir_to_zip(
    zip: &mut ZipWriter<std::fs::File>,
    root: &Path,
    dir: &Path,
    options: SimpleFileOptions,
) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        if path.is_dir() {
            add_dir_to_zip(zip, root, &path, options)?;
        } else {
            zip.start_file(format!("{DRAWINGS_PREFIX}{relative}"), options)?;
            let mut bytes = Vec::new();
            std::fs::File::open(&path)?.read_to_end(&mut bytes)?;
            zip.write_all(&bytes)?;
        }
    }
    Ok(())
}

/// Restores the database (into the live connection, via SQLite's online
/// restore) and `drawings/` folder (overwritten in place) from a backup
/// created by [`create_backup`].
pub fn restore_backup(
    data_dir: &Path,
    conn: &mut Connection,
    zip_path: &Path,
) -> anyhow::Result<()> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = ZipArchive::new(file)?;

    let snapshot = tempfile::NamedTempFile::new()?;
    {
        let mut db_entry = archive.by_name(DB_ENTRY_NAME)?;
        let mut out = std::fs::File::create(snapshot.path())?;
        std::io::copy(&mut db_entry, &mut out)?;
    }
    conn.restore(
        MAIN_DB,
        snapshot.path(),
        None::<fn(core_lib::rusqlite::backup::Progress)>,
    )?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_string();
        let Some(relative) = name.strip_prefix(DRAWINGS_PREFIX) else {
            continue;
        };
        if relative.is_empty() {
            continue;
        }
        let dest = data_dir.join("drawings").join(relative);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&dest)?;
        std::io::copy(&mut entry, &mut out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_lib::{documents, storage};

    #[test]
    fn backup_and_restore_round_trips_the_database_and_drawings() {
        let source_dir = tempfile::tempdir().expect("tempdir");
        let conn = storage::open(&source_dir.path().join("db.sqlite3")).expect("open db");
        documents::insert_pending(&conn, "EP1234567", "EP1234567", "2026-01-01T00:00:00Z").unwrap();

        std::fs::create_dir_all(source_dir.path().join("drawings/EP1234567")).unwrap();
        std::fs::write(
            source_dir.path().join("drawings/EP1234567/001.png"),
            b"fake png bytes",
        )
        .unwrap();

        let backup_path = source_dir.path().join("backup.zip");
        create_backup(source_dir.path(), &conn, &backup_path).expect("backup should succeed");
        assert!(backup_path.exists());

        // Restore into a *different*, freshly created database and data
        // directory, to prove the backup is self-contained.
        let dest_dir = tempfile::tempdir().expect("tempdir");
        let mut restored_conn =
            storage::open(&dest_dir.path().join("db.sqlite3")).expect("open db");
        restore_backup(dest_dir.path(), &mut restored_conn, &backup_path)
            .expect("restore should succeed");

        let doc = documents::find_by_pub_key(&restored_conn, "EP1234567")
            .unwrap()
            .expect("the document should have survived the round trip");
        assert_eq!(doc.pub_key, "EP1234567");

        let restored_png = std::fs::read(dest_dir.path().join("drawings/EP1234567/001.png"))
            .expect("drawing should exist");
        assert_eq!(restored_png, b"fake png bytes");
    }
}
