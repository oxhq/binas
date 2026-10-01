use binas_core::OperationContext;
use std::collections::BTreeMap;
mod retained;
pub(crate) use retained::{
    EncodedFontText, SimpleFontTextMetrics, decode_existing_cmap_resource_text,
    encode_existing_cmap_resource_text, encode_existing_cmap_text, simple_font_text_metrics,
};

use crate::{cmap::ToUnicodeCMap, error::PdfError, limits::Limits};

#[derive(Clone, Debug)]
pub struct FontDecoder {
    to_unicode: ToUnicodeCMap,
}

impl FontDecoder {
    pub fn new(to_unicode: ToUnicodeCMap) -> Self {
        Self { to_unicode }
    }

    /// Legacy unbudgeted convenience decoder. Document extraction uses the
    /// internal bounded entrypoint to enforce token expansion and cumulative work.
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

    /// Limits encoded and expanded UTF-8 token lengths before allocation/append.
    /// Work is cumulative in the document decode budget, with cancellation per
    /// code. Returned String retention and allocator overhead are not live-memory
    /// accounting. Codespace charging covers all range attempts; mapping-tree
    /// lookup uses four logical work units per logarithmic level. This is a work
    /// policy model, not an exact CPU comparison or allocator accounting claim.
    pub(crate) fn decode_bounded(
        &self,
        encoded: &[u8],
        limits: &Limits,
        budget: &crate::parser::ParseBudget,
    ) -> Result<String, PdfError> {
        budget.charge_work(0)?;
        if encoded.len() > limits.max_token_bytes {
            return Err(PdfError::limit(
                "encoded font token exceeds max_token_bytes",
            ));
        }
        budget.charge_work(encoded.len())?;
        let (codespaces, mappings) = self.to_unicode.lookup_counts();
        let mapping_work = ((usize::BITS - mappings.leading_zeros()) as usize + 1)
            .checked_mul(4)
            .ok_or_else(|| PdfError::limit("font mapping work overflows"))?;
        let mut output = String::new();
        let mut offset = 0;
        while offset < encoded.len() {
            // longest_code_len tests up to four widths; each range test can
            // compare up to four bytes against each of two endpoints.
            let lookup = 4
                .min(encoded.len() - offset)
                .checked_mul(codespaces)
                .and_then(|n| n.checked_mul(9))
                .and_then(|n| n.checked_add(mapping_work))
                .and_then(|n| n.checked_add(1))
                .ok_or_else(|| PdfError::limit("font lookup work overflows"))?;
            budget.charge_work(lookup)?;
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
            let length = output
                .len()
                .checked_add(mapped.len())
                .ok_or_else(|| PdfError::limit("expanded Unicode token length overflows"))?;
            if length > limits.max_token_bytes {
                return Err(PdfError::limit(
                    "expanded Unicode token exceeds max_token_bytes",
                ));
            }
            budget.charge_work(mapped.len())?;
            if length > output.capacity() {
                let capacity = length
                    .max(output.capacity().saturating_mul(2))
                    .min(limits.max_token_bytes);
                output
                    .try_reserve_exact(capacity - output.len())
                    .map_err(|_| PdfError::limit("expanded Unicode allocation failed"))?;
            }
            output.push_str(mapped);
            offset += width;
        }
        Ok(output)
    }

    pub fn encode(&self, text: &str, limits: &Limits) -> Result<Vec<u8>, PdfError> {
        self.encode_inner(text, limits, None)
    }

    /// Same deterministic codec as `encode`, with cumulative work and temporary
    /// logical record reservations. Excludes allocator/BTree node overhead and
    /// caller-retained output memory after return. This proves Unicode mapping,
    /// not font-program glyphs, metrics, shaping, or visual equivalence.
    pub fn encode_with_context(
        &self,
        text: &str,
        limits: &Limits,
        context: &OperationContext,
    ) -> Result<Vec<u8>, PdfError> {
        self.encode_inner(text, limits, Some(context))
    }

    fn encode_inner(
        &self,
        text: &str,
        limits: &Limits,
        context: Option<&OperationContext>,
    ) -> Result<Vec<u8>, PdfError> {
        charge(context, 1)?;
        if text.len() > limits.max_token_bytes {
            return Err(PdfError::limit("replacement text exceeds max_token_bytes"));
        }
        let mut count = 0usize;
        for _ in self.to_unicode.mappings() {
            charge(context, 1)?;
            count += 1;
        }
        // Conservative upper bound on logical borrowed reverse records, one
        // state/path record per UTF-8 byte, and up to four output bytes per byte.
        let logical = count
            .checked_mul(std::mem::size_of::<(&str, &[u8])>())
            .and_then(|n| {
                text.len()
                    .checked_add(1)
                    .and_then(|len| {
                        len.checked_mul(std::mem::size_of::<(usize, ReverseState)>() + 9)
                    })
                    .and_then(|extra| n.checked_add(extra))
            })
            .ok_or_else(|| PdfError::limit("reverse encoding reservation overflows"))?;
        let _reservation = context
            .map(|ctx| ctx.reserve(logical as u64).map_err(PdfError::from_context))
            .transpose()?;
        let mut reverse = BTreeMap::<&str, &[u8]>::new();
        for (source, destination) in self.to_unicode.mappings() {
            charge(
                context,
                source
                    .len()
                    .saturating_add(destination.len())
                    .saturating_add(1),
            )?;
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
            charge(context, 1)?;
            let Some(current) = states.get(&position).copied() else {
                continue;
            };
            for (&destination, &source) in &reverse {
                charge(context, destination.len().saturating_add(1))?;
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
            charge(context, 1)?;
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
        charge(context, encoded_len)?;
        let mut output = Vec::with_capacity(encoded_len);
        for (code, width) in chunks.into_iter().rev() {
            output.extend_from_slice(&code[..usize::from(width)]);
        }
        let mut offset = 0;
        let mut remaining = text;
        let mut matches = true;
        while offset < output.len() {
            charge(context, 1)?;
            let Some(width) = self.to_unicode.longest_code_len(&output[offset..]) else {
                matches = false;
                break;
            };
            let Some(mapped) = self.to_unicode.mapping(&output[offset..offset + width]) else {
                matches = false;
                break;
            };
            charge(context, mapped.len())?;
            let Some(rest) = remaining.strip_prefix(mapped) else {
                matches = false;
                break;
            };
            remaining = rest;
            offset += width;
        }
        if !matches || !remaining.is_empty() {
            return Err(PdfError::unsafe_rewrite(
                "replacement text has ambiguous encoded code boundaries",
            ));
        }
        Ok(output)
    }
}

fn charge(context: Option<&OperationContext>, units: usize) -> Result<(), PdfError> {
    if let Some(context) = context {
        context
            .charge_work(units as u64)
            .map_err(PdfError::from_context)?;
    }
    Ok(())
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
    fn bounded_decode_caps_input_and_utf8_expansion_before_growth() {
        use crate::parser::ParseBudget;
        use binas_core::CancellationToken;
        let cmap = ToUnicodeCMap::parse(b"1 begincodespacerange <00> <ff> endcodespacerange 1 beginbfchar <41> <03A903A903A903A9> endbfchar", &Limits::default()).unwrap();
        let decoder = FontDecoder::new(cmap);
        let limits = Limits {
            max_token_bytes: 16,
            ..Limits::default()
        };
        let budget = ParseBudget::new(&limits, CancellationToken::new());
        assert_eq!(
            decoder.decode_bounded(b"AA", &limits, &budget).unwrap(),
            "Ω".repeat(8)
        );
        assert_eq!(decoder.decode(b"AAA").unwrap().len(), 24);
        assert_eq!(
            decoder
                .decode_bounded(b"AAA", &limits, &budget)
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(
            decoder
                .decode_bounded(&[b'A'; 17], &limits, &budget)
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
    }

    #[test]
    fn bounded_decode_charges_codespace_scan_and_repeated_work_with_cancellation() {
        use crate::parser::ParseBudget;
        use binas_core::CancellationToken;
        let mut source = "256 begincodespacerange ".to_owned();
        for code in 0..256 {
            source.push_str(&format!("<{code:02X}> <{code:02X}> "));
        }
        source.push_str("endcodespacerange 1 beginbfchar <FF> <03A9> endbfchar");
        let decoder =
            FontDecoder::new(ToUnicodeCMap::parse(source.as_bytes(), &Limits::default()).unwrap());
        let mut limits = Limits {
            max_decode_work_bytes: 100,
            ..Limits::default()
        };
        let limited = ParseBudget::new(&limits, CancellationToken::new());
        assert_eq!(
            decoder
                .decode_bounded(&[255], &limits, &limited)
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        // One code spends input1 + ranges256*9 + mapping8 + checkpoint1 + UTF8two.
        limits.max_decode_work_bytes = 2316;
        let cumulative = ParseBudget::new(&limits, CancellationToken::new());
        assert_eq!(
            decoder
                .decode_bounded(&[255], &limits, &cumulative)
                .unwrap(),
            "Ω"
        );
        assert_eq!(
            decoder
                .decode_bounded(&[255], &limits, &cumulative)
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        let token = CancellationToken::new();
        let cancelled = ParseBudget::new(&limits, token.clone());
        token.cancel();
        assert_eq!(
            decoder
                .decode_bounded(&[], &limits, &cancelled)
                .unwrap_err()
                .code,
            PdfErrorCode::Cancelled
        );
    }

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
