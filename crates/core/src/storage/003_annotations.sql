-- View screen: highlighted passages with an optional comment.
-- `start`/`end` are UTF-16 offsets into the section's stored text;
-- `quote` is the highlighted text, used to find the passage again if the
-- stored text changes (for example after full text is retrieved again).
CREATE TABLE annotations (
  id INTEGER PRIMARY KEY,
  doc_id INTEGER NOT NULL REFERENCES documents (id),
  section TEXT NOT NULL,               -- title | abstract | description | claims
  start INTEGER NOT NULL,
  "end" INTEGER NOT NULL,
  quote TEXT NOT NULL,
  comment TEXT,
  color TEXT NOT NULL,                 -- yellow | green | blue | pink
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE INDEX annotations_doc_id ON annotations(doc_id);
