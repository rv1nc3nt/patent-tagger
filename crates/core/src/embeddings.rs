//! Persistence for `embeddings` and `tag_embeddings` (SPEC section 4.2).
//! Vectors are little-endian f32 BLOBs, L2-normalised (the embedder itself
//! guarantees normalisation; this module just stores/loads bytes).

use crate::storage::StorageError;
use rusqlite::{params, Connection, OptionalExtension};

fn to_bytes(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn from_bytes(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().expect("chunks_exact(4)")))
        .collect()
}

pub fn store_document_embedding(
    conn: &Connection,
    doc_id: i64,
    model_id: &str,
    vector: &[f32],
) -> Result<(), StorageError> {
    conn.execute(
        "INSERT INTO embeddings (doc_id, model_id, dim, vector) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (doc_id, model_id) DO UPDATE SET dim = excluded.dim, vector = excluded.vector",
        params![doc_id, model_id, vector.len() as i64, to_bytes(vector)],
    )?;
    Ok(())
}

pub fn get_document_embedding(
    conn: &Connection,
    doc_id: i64,
    model_id: &str,
) -> Result<Option<Vec<f32>>, StorageError> {
    conn.query_row(
        "SELECT vector FROM embeddings WHERE doc_id = ?1 AND model_id = ?2",
        params![doc_id, model_id],
        |row| row.get::<_, Vec<u8>>(0),
    )
    .optional()
    .map(|opt| opt.map(|bytes| from_bytes(&bytes)))
    .map_err(StorageError::from)
}

/// Every `(doc_id, vector)` for validated documents with an embedding under
/// `model_id` - the candidate pool for k-NN (SPEC section 7.2).
pub fn list_validated_document_embeddings(
    conn: &Connection,
    model_id: &str,
) -> Result<Vec<(i64, Vec<f32>)>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT e.doc_id, e.vector FROM embeddings e
         JOIN documents d ON d.id = e.doc_id
         WHERE e.model_id = ?1 AND d.review_state = 'validated'",
    )?;
    let rows = stmt
        .query_map(params![model_id], |row| {
            let doc_id: i64 = row.get(0)?;
            let bytes: Vec<u8> = row.get(1)?;
            Ok((doc_id, bytes))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .map(|(id, bytes)| (id, from_bytes(&bytes)))
        .collect())
}

/// `(embedding, is_positive)` for every document with a human label for
/// `tag_id` and an embedding under `model_id` - training data for
/// [`crate::classifier::train`] (SPEC 7.1: "only source = human labels are
/// used").
pub fn training_samples_for_tag(
    conn: &Connection,
    tag_id: i64,
    model_id: &str,
) -> Result<Vec<(Vec<f32>, bool)>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT e.vector, l.state FROM labels l
         JOIN embeddings e ON e.doc_id = l.doc_id AND e.model_id = ?2
         WHERE l.tag_id = ?1 AND l.source = 'human'",
    )?;
    let rows = stmt
        .query_map(params![tag_id, model_id], |row| {
            let bytes: Vec<u8> = row.get(0)?;
            let state: String = row.get(1)?;
            Ok((bytes, state == "pos"))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .map(|(bytes, is_pos)| (from_bytes(&bytes), is_pos))
        .collect())
}

pub fn store_tag_embedding(
    conn: &Connection,
    tag_id: i64,
    model_id: &str,
    tag_version: i64,
    vector: &[f32],
) -> Result<(), StorageError> {
    conn.execute(
        "INSERT INTO tag_embeddings (tag_id, model_id, tag_version, vector) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (tag_id, model_id, tag_version) DO UPDATE SET vector = excluded.vector",
        params![tag_id, model_id, tag_version, to_bytes(vector)],
    )?;
    Ok(())
}

pub fn get_tag_embedding(
    conn: &Connection,
    tag_id: i64,
    model_id: &str,
    tag_version: i64,
) -> Result<Option<Vec<f32>>, StorageError> {
    conn.query_row(
        "SELECT vector FROM tag_embeddings WHERE tag_id = ?1 AND model_id = ?2 AND tag_version = ?3",
        params![tag_id, model_id, tag_version],
        |row| row.get::<_, Vec<u8>>(0),
    )
    .optional()
    .map(|opt| opt.map(|bytes| from_bytes(&bytes)))
    .map_err(StorageError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{documents, storage};

    const MODEL: &str = "test-model@abc";

    #[test]
    fn document_embedding_round_trips() {
        let conn = storage::open_in_memory().expect("in-memory db");
        documents::insert_pending(&conn, "EP1234567", "EP1234567", "2026-01-01T00:00:00Z").unwrap();
        let doc = documents::find_by_pub_key(&conn, "EP1234567")
            .unwrap()
            .unwrap();

        let vector = vec![0.1, 0.2, 0.3];
        store_document_embedding(&conn, doc.id, MODEL, &vector).unwrap();
        let loaded = get_document_embedding(&conn, doc.id, MODEL)
            .unwrap()
            .unwrap();
        assert_eq!(loaded, vector);
    }

    #[test]
    fn training_samples_pairs_embeddings_with_human_labels() {
        use crate::labels;
        use std::collections::HashSet;

        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = crate::tags::create(
            &conn,
            "Battery",
            "About batteries",
            None,
            None,
            "2026-01-01T00:00:00Z",
        )
        .unwrap();
        let tag = crate::tags::get(&conn, tag_id).unwrap().unwrap();

        documents::insert_pending(&conn, "EP1111111", "EP1111111", "2026-01-01T00:00:00Z").unwrap();
        let pos_doc = documents::find_by_pub_key(&conn, "EP1111111")
            .unwrap()
            .unwrap();
        store_document_embedding(&conn, pos_doc.id, MODEL, &[1.0, 0.0]).unwrap();
        labels::validate_document(
            &conn,
            pos_doc.id,
            std::slice::from_ref(&tag),
            &[tag_id].into_iter().collect::<HashSet<_>>(),
            "2026-01-01T00:00:00Z",
        )
        .unwrap();

        documents::insert_pending(&conn, "EP2222222", "EP2222222", "2026-01-01T00:00:00Z").unwrap();
        let neg_doc = documents::find_by_pub_key(&conn, "EP2222222")
            .unwrap()
            .unwrap();
        store_document_embedding(&conn, neg_doc.id, MODEL, &[0.0, 1.0]).unwrap();
        labels::validate_document(
            &conn,
            neg_doc.id,
            std::slice::from_ref(&tag),
            &HashSet::new(),
            "2026-01-01T00:00:00Z",
        )
        .unwrap();

        let samples = training_samples_for_tag(&conn, tag_id, MODEL).unwrap();
        assert_eq!(samples.len(), 2);
        assert!(samples.contains(&(vec![1.0, 0.0], true)));
        assert!(samples.contains(&(vec![0.0, 1.0], false)));
    }

    #[test]
    fn storing_twice_overwrites_rather_than_erroring() {
        let conn = storage::open_in_memory().expect("in-memory db");
        documents::insert_pending(&conn, "EP1234567", "EP1234567", "2026-01-01T00:00:00Z").unwrap();
        let doc = documents::find_by_pub_key(&conn, "EP1234567")
            .unwrap()
            .unwrap();

        store_document_embedding(&conn, doc.id, MODEL, &[1.0, 0.0]).unwrap();
        store_document_embedding(&conn, doc.id, MODEL, &[0.0, 1.0]).unwrap();

        assert_eq!(
            get_document_embedding(&conn, doc.id, MODEL)
                .unwrap()
                .unwrap(),
            vec![0.0, 1.0]
        );
    }

    #[test]
    fn list_validated_only_includes_validated_documents() {
        let conn = storage::open_in_memory().expect("in-memory db");
        documents::insert_pending(&conn, "EP1111111", "EP1111111", "2026-01-01T00:00:00Z").unwrap();
        documents::insert_pending(&conn, "EP2222222", "EP2222222", "2026-01-01T00:00:00Z").unwrap();
        let queued = documents::find_by_pub_key(&conn, "EP1111111")
            .unwrap()
            .unwrap();
        let validated = documents::find_by_pub_key(&conn, "EP2222222")
            .unwrap()
            .unwrap();
        conn.execute(
            "UPDATE documents SET review_state = 'validated' WHERE id = ?1",
            params![validated.id],
        )
        .unwrap();

        store_document_embedding(&conn, queued.id, MODEL, &[1.0]).unwrap();
        store_document_embedding(&conn, validated.id, MODEL, &[2.0]).unwrap();

        let list = list_validated_document_embeddings(&conn, MODEL).unwrap();
        assert_eq!(list, vec![(validated.id, vec![2.0])]);
    }

    #[test]
    fn tag_embedding_round_trips() {
        let conn = storage::open_in_memory().expect("in-memory db");
        conn.execute(
            "INSERT INTO tags (name, definition, created_at) VALUES ('Foo', 'A foo tag', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        let tag_id = conn.last_insert_rowid();

        store_tag_embedding(&conn, tag_id, MODEL, 1, &[0.5, 0.5]).unwrap();
        assert_eq!(
            get_tag_embedding(&conn, tag_id, MODEL, 1).unwrap().unwrap(),
            vec![0.5, 0.5]
        );
        assert_eq!(get_tag_embedding(&conn, tag_id, MODEL, 2).unwrap(), None);
    }
}
