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
            max_pages: 100_000,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct OpenOptions {
    #[serde(default)]
    pub repair: bool,
}
