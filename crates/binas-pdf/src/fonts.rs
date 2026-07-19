use std::collections::BTreeMap;

use crate::{cmap::ToUnicodeCMap, error::PdfError, limits::Limits};

#[derive(Clone, Debug)]
pub struct FontDecoder {
    to_unicode: ToUnicodeCMap,
}

impl FontDecoder {
    pub fn new(to_unicode: ToUnicodeCMap) -> Self {
        Self { to_unicode }
    }

    pub fn decode(&self, encoded: &[u8]) -> Result<String, PdfError> {
        let mut output = String::new();
        let mut offset = 0;
        while offset < encoded.len() {
            let width = self
                .to_unicode
                .longest_code_len(&encoded[offset..])
                .ok_or_else(|| {
                    PdfError::syntax(
                        format!("font code at byte {offset} is outside the ToUnicode codespaces"),
                        offset,
                    )
                })?;
            let code = &encoded[offset..offset + width];
            let mapped = self.to_unicode.mapping(code).ok_or_else(|| {
                PdfError::unsupported(format!(
                    "ToUnicode CMap has no mapping for source code <{}>",
                    hex(code)
                ))
            })?;
            output.push_str(mapped);
            offset += width;
        }
        Ok(output)
    }

    pub fn encode(&self, text: &str, limits: &Limits) -> Result<Vec<u8>, PdfError> {
        if text.len() > limits.max_token_bytes {
            return Err(PdfError::limit("replacement text exceeds max_token_bytes"));
        }
        let mut reverse = BTreeMap::<&str, &[u8]>::new();
        for (source, destination) in self.to_unicode.mappings() {
            reverse
                .entry(destination)
                .and_modify(|selected| {
                    if (source.len(), source) < (selected.len(), *selected) {
                        *selected = source;
                    }
                })
                .or_insert(source);
        }

        let mut states = BTreeMap::new();
        states.insert(
            0,
            ReverseState {
                ways: 1,
                previous: 0,
                code: [0; 4],
                width: 0,
            },
        );
        let mut work = 0usize;
        for position in text.char_indices().map(|(position, _)| position) {
            let Some(current) = states.get(&position).copied() else {
                continue;
            };
            for (&destination, &source) in &reverse {
                work = work
                    .checked_add(1)
                    .ok_or_else(|| PdfError::limit("reverse ToUnicode work overflows"))?;
                if work > limits.max_container_items {
                    return Err(PdfError::limit(
                        "reverse ToUnicode work exceeds max_container_items",
                    ));
                }
                if !text[position..].starts_with(destination) {
                    continue;
                }
                let end = position + destination.len();
                let mut code = [0; 4];
                code[..source.len()].copy_from_slice(source);
                states
                    .entry(end)
                    .and_modify(|state: &mut ReverseState| state.ways = 2)
                    .or_insert(ReverseState {
                        ways: current.ways,
                        previous: position,
                        code,
                        width: source.len() as u8,
                    });
            }
        }

        let Some(final_state) = states.get(&text.len()) else {
            return Err(PdfError::unsupported(
                "replacement text has no exact ToUnicode reverse mapping",
            ));
        };
        if final_state.ways != 1 {
            return Err(PdfError::unsafe_rewrite(
                "replacement text has ambiguous ToUnicode segmentation",
            ));
        }
        let mut chunks = Vec::new();
        let mut position = text.len();
        while position != 0 {
            let state = states
                .get(&position)
                .ok_or_else(|| PdfError::unsafe_rewrite("reverse ToUnicode path is incomplete"))?;
            chunks.push((state.code, state.width));
            position = state.previous;
        }
        let encoded_len = chunks.iter().try_fold(0usize, |length, (_, width)| {
            length
                .checked_add(usize::from(*width))
                .ok_or_else(|| PdfError::limit("encoded replacement length overflows"))
        })?;
        if encoded_len > limits.max_token_bytes {
            return Err(PdfError::limit(
                "encoded replacement exceeds max_token_bytes",
            ));
        }
        let mut output = Vec::with_capacity(encoded_len);
        for (code, width) in chunks.into_iter().rev() {
            output.extend_from_slice(&code[..usize::from(width)]);
        }
        if self.decode(&output).as_deref() != Ok(text) {
            return Err(PdfError::unsafe_rewrite(
                "replacement text has ambiguous encoded code boundaries",
            ));
        }
        Ok(output)
    }
}

#[derive(Clone, Copy)]
struct ReverseState {
    ways: u8,
    previous: usize,
    code: [u8; 4],
    width: u8,
}

fn hex(input: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let mut output = String::with_capacity(input.len() * 2);
    for byte in input {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{error::PdfErrorCode, limits::Limits};

    #[test]
    fn decodes_using_the_longest_valid_source_code() {
        let cmap = ToUnicodeCMap::parse(
            b"2 begincodespacerange <00> <ff> <0100> <01ff> endcodespacerange\n\
              2 beginbfchar <01> <0041> <0102> <0042> endbfchar",
            &Limits::default(),
        )
        .unwrap();

        assert_eq!(FontDecoder::new(cmap).decode(&[1, 2]).unwrap(), "B");
    }

    #[test]
    fn explicitly_reports_missing_mappings() {
        let cmap = ToUnicodeCMap::parse(
            b"1 begincodespacerange <00> <ff> endcodespacerange",
            &Limits::default(),
        )
        .unwrap();
        let error = FontDecoder::new(cmap).decode(&[0x2a]).unwrap_err();

        assert_eq!(error.code, PdfErrorCode::UnsupportedFeature);
        assert!(error.message.contains("<2A>"));
    }

    #[test]
    fn reverse_encoding_is_deterministic_and_exact() {
        let cmap = ToUnicodeCMap::parse(
            b"2 begincodespacerange <00> <ff> <0000> <00ff> endcodespacerange 4 beginbfchar <02> <0041> <01> <0041> <0001> <0041> <03> <03a9> endbfchar",
            &Limits::default(),
        )
        .unwrap();
        let decoder = FontDecoder::new(cmap);

        assert_eq!(
            decoder.encode("A\u{3a9}", &Limits::default()).unwrap(),
            [1, 3]
        );
    }

    #[test]
    fn reverse_encoding_rejects_ambiguous_missing_and_over_budget_text() {
        let cmap = ToUnicodeCMap::parse(
            b"1 begincodespacerange <00> <ff> endcodespacerange 3 beginbfchar <01> <0041> <02> <0042> <03> <00410042> endbfchar",
            &Limits::default(),
        )
        .unwrap();
        let decoder = FontDecoder::new(cmap);

        assert_eq!(
            decoder.encode("AB", &Limits::default()).unwrap_err().code,
            PdfErrorCode::UnsafeRewrite
        );
        assert_eq!(
            decoder.encode("C", &Limits::default()).unwrap_err().code,
            PdfErrorCode::UnsupportedFeature
        );
        assert_eq!(
            decoder
                .encode(
                    "A",
                    &Limits {
                        max_container_items: 1,
                        ..Limits::default()
                    }
                )
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );

        let overlapping = ToUnicodeCMap::parse(
            b"2 begincodespacerange <00> <ff> <0000> <ffff> endcodespacerange 2 beginbfchar <03> <0041> <04> <0042> endbfchar",
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(
            FontDecoder::new(overlapping)
                .encode("AB", &Limits::default())
                .unwrap_err()
                .code,
            PdfErrorCode::UnsafeRewrite
        );
    }
}
