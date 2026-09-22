//! Tauri-managed state wrapping the loaded embedding model (SPEC section 6).
//! Loaded once at startup; re-embedding on a model change is a later
//! milestone (section 6's "swappability").

use embed_lib::BgeSmallEmbedder;

pub struct Model(pub BgeSmallEmbedder);
