-- SPEC section 4.2. One comprehensive initial migration; unused tables sit
-- empty until their milestone lands (see docs/DECISIONS.md).

CREATE TABLE documents (
  id INTEGER PRIMARY KEY,
  pub_key TEXT UNIQUE NOT NULL,
  input_raw TEXT NOT NULL,
  kind_codes TEXT,
  application_number TEXT,
  family_id TEXT,
  title TEXT,
  "abstract" TEXT,
  abstract_source TEXT,
  applicants TEXT,
  publication_date TEXT,
  cpc TEXT,
  ipc TEXT,
  fetch_status TEXT NOT NULL,
  fetch_error TEXT,
  review_state TEXT NOT NULL,
  imported_at TEXT NOT NULL,
  validated_at TEXT
);

CREATE VIRTUAL TABLE documents_fts USING fts5(
  title,
  "abstract",
  content = 'documents',
  content_rowid = 'id'
);

CREATE TRIGGER documents_ai AFTER INSERT ON documents BEGIN
  INSERT INTO documents_fts (rowid, title, "abstract")
  VALUES (new.id, new.title, new."abstract");
END;

CREATE TRIGGER documents_ad AFTER DELETE ON documents BEGIN
  INSERT INTO documents_fts (documents_fts, rowid, title, "abstract")
  VALUES ('delete', old.id, old.title, old."abstract");
END;

CREATE TRIGGER documents_au AFTER UPDATE ON documents BEGIN
  INSERT INTO documents_fts (documents_fts, rowid, title, "abstract")
  VALUES ('delete', old.id, old.title, old."abstract");
  INSERT INTO documents_fts (rowid, title, "abstract")
  VALUES (new.id, new.title, new."abstract");
END;

CREATE TABLE fulltext (
  doc_id INTEGER PRIMARY KEY REFERENCES documents (id),
  description TEXT,
  claims TEXT,
  lang TEXT,
  source TEXT,
  status TEXT NOT NULL,
  fetched_at TEXT
);

CREATE VIRTUAL TABLE fulltext_fts USING fts5(
  description,
  claims,
  content = 'fulltext',
  content_rowid = 'doc_id'
);

CREATE TRIGGER fulltext_ai AFTER INSERT ON fulltext BEGIN
  INSERT INTO fulltext_fts (rowid, description, claims)
  VALUES (new.doc_id, new.description, new.claims);
END;

CREATE TRIGGER fulltext_ad AFTER DELETE ON fulltext BEGIN
  INSERT INTO fulltext_fts (fulltext_fts, rowid, description, claims)
  VALUES ('delete', old.doc_id, old.description, old.claims);
END;

CREATE TRIGGER fulltext_au AFTER UPDATE ON fulltext BEGIN
  INSERT INTO fulltext_fts (fulltext_fts, rowid, description, claims)
  VALUES ('delete', old.doc_id, old.description, old.claims);
  INSERT INTO fulltext_fts (rowid, description, claims)
  VALUES (new.doc_id, new.description, new.claims);
END;

CREATE TABLE drawings (
  doc_id INTEGER NOT NULL REFERENCES documents (id),
  page INTEGER NOT NULL,
  source TEXT,
  path TEXT NOT NULL,
  width INTEGER,
  height INTEGER,
  fetched_at TEXT,
  PRIMARY KEY (doc_id, page)
);

CREATE TABLE drawings_status (
  doc_id INTEGER PRIMARY KEY REFERENCES documents (id),
  status TEXT NOT NULL,
  page_count INTEGER,
  source TEXT,
  updated_at TEXT
);

CREATE TABLE ops_raw (
  doc_id INTEGER NOT NULL REFERENCES documents (id),
  endpoint TEXT NOT NULL,
  fetched_at TEXT NOT NULL,
  body BLOB,
  PRIMARY KEY (doc_id, endpoint)
);

CREATE TABLE tags (
  id INTEGER PRIMARY KEY,
  name TEXT UNIQUE NOT NULL,
  definition TEXT NOT NULL,
  parent_id INTEGER REFERENCES tags (id),
  color TEXT,
  hotkey TEXT,
  version INTEGER NOT NULL DEFAULT 1,
  archived INTEGER NOT NULL DEFAULT 0,
  auto_enabled INTEGER NOT NULL DEFAULT 0,
  threshold REAL,
  neg_threshold REAL,
  created_at TEXT NOT NULL
);

CREATE TABLE labels (
  doc_id INTEGER NOT NULL REFERENCES documents (id),
  tag_id INTEGER NOT NULL REFERENCES tags (id),
  state TEXT NOT NULL,
  source TEXT NOT NULL,
  confidence REAL,
  model_version TEXT,
  tag_version INTEGER NOT NULL,
  created_at TEXT NOT NULL,
  PRIMARY KEY (doc_id, tag_id)
);

CREATE TABLE label_history (
  id INTEGER PRIMARY KEY,
  doc_id INTEGER NOT NULL REFERENCES documents (id),
  tag_id INTEGER NOT NULL REFERENCES tags (id),
  state TEXT NOT NULL,
  source TEXT NOT NULL,
  confidence REAL,
  model_version TEXT,
  tag_version INTEGER NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE embeddings (
  doc_id INTEGER NOT NULL REFERENCES documents (id),
  model_id TEXT NOT NULL,
  dim INTEGER NOT NULL,
  vector BLOB NOT NULL,
  PRIMARY KEY (doc_id, model_id)
);

CREATE TABLE tag_embeddings (
  tag_id INTEGER NOT NULL REFERENCES tags (id),
  model_id TEXT NOT NULL,
  tag_version INTEGER NOT NULL,
  vector BLOB NOT NULL,
  PRIMARY KEY (tag_id, model_id, tag_version)
);

CREATE TABLE predictions (
  id INTEGER PRIMARY KEY,
  doc_id INTEGER NOT NULL REFERENCES documents (id),
  tag_id INTEGER NOT NULL REFERENCES tags (id),
  model_version TEXT NOT NULL,
  score REAL NOT NULL,
  suggested INTEGER NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE classifiers (
  tag_id INTEGER PRIMARY KEY REFERENCES tags (id),
  model_id TEXT NOT NULL,
  weights BLOB NOT NULL,
  bias REAL NOT NULL,
  n_pos INTEGER NOT NULL,
  n_neg INTEGER NOT NULL,
  trained_at TEXT NOT NULL
);

CREATE TABLE jobs (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL,
  payload TEXT,
  state TEXT NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0,
  last_error TEXT,
  updated_at TEXT NOT NULL
);

CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT
);
