use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct EngineConfig {
    pub limits: Limits,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Limits {
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    pub max_objects: usize,
    pub max_xref_entries: usize,
    pub max_xref_revisions: usize,
    pub max_parser_depth: usize,
    pub max_container_items: usize,
    pub max_token_bytes: usize,
    pub max_stream_bytes: usize,
    pub max_total_decoded_bytes: usize,
    /// Retained decoded payload bytes; eviction does not refund decode work.
    #[serde(default = "default_decode_cache_bytes")]
    pub max_decode_cache_bytes: usize,
    /// Logical decoded bytes alive in cache and shared caller handles.
    #[serde(default = "default_decode_live_bytes")]
    pub max_decode_live_bytes: usize,
    /// Cumulative decoder/input-fingerprint and content/CMap/text work across
    /// reads and document clones. Logical lookup units are not exact CPU time.
    #[serde(default = "default_decode_work_bytes")]
    pub max_decode_work_bytes: usize,
    pub max_pages: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: 64 * 1024 * 1024,
            max_output_bytes: 128 * 1024 * 1024,
            max_objects: 250_000,
            max_xref_entries: 500_000,
            max_xref_revisions: 32,
            max_parser_depth: 128,
            max_container_items: 1_000_000,
            max_token_bytes: 8 * 1024 * 1024,
            max_stream_bytes: 32 * 1024 * 1024,
            max_total_decoded_bytes: 128 * 1024 * 1024,
            max_decode_cache_bytes: default_decode_cache_bytes(),
            max_decode_live_bytes: default_decode_live_bytes(),
            max_decode_work_bytes: default_decode_work_bytes(),
            max_pages: 100_000,
        }
    }
}

fn default_decode_cache_bytes() -> usize {
    16 * 1024 * 1024
}
fn default_decode_live_bytes() -> usize {
    64 * 1024 * 1024
}
fn default_decode_work_bytes() -> usize {
    512 * 1024 * 1024
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct OpenOptions {
    #[serde(default)]
    pub repair: bool,
}
