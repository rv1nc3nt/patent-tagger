-- SPEC 5.6: saved OPS searches by applicant, imported in batches.
CREATE TABLE saved_searches (
  id INTEGER PRIMARY KEY,
  name TEXT UNIQUE NOT NULL,
  applicant TEXT NOT NULL,
  country TEXT,
  year_from INTEGER,
  year_to INTEGER,
  query TEXT NOT NULL,
  total_results INTEGER,
  next_start INTEGER NOT NULL DEFAULT 1,
  imported INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  last_run_at TEXT
);

-- Search batches skip families already in the database.
CREATE INDEX documents_family_id ON documents(family_id);
