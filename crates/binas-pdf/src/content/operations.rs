//! Operation boundaries over the existing content lexer. Complete operands may
//! cross parts; token-internal splits are rejected. Dictionary contents are not
//! interpreted, and are surfaced explicitly so callers cannot infer coverage.
//! Input scans are prepaid; cancellation is checked between tokens/operators.
//! Temporary token/event allocations obey PDF limits, not live reservations.
use super::{InlineImage, Lexer, Token, assign_part, parse_inline_image};
use crate::{Limits, PdfError, parser::ParseBudget};
use binas_core::Span;

#[derive(Debug, PartialEq)]
pub(crate) enum ContentOperand {
    Number(f64),
    Name(Vec<u8>),
    Bytes {
        bytes: Vec<u8>,
        part_index: usize,
        decoded_span: Span,
    },
    Array(Vec<ContentOperand>),
    UnsupportedDictionary,
    Boolean(bool),
    Null,
}

#[derive(Debug)]
pub(crate) struct ContentOperation {
    pub operator: Vec<u8>,
    pub operands: Vec<ContentOperand>,
    pub part_index: usize,
    pub decoded_offset: usize,
    pub inline_image: Option<InlineImage>,
}

#[derive(Debug, Default)]
pub(crate) struct OperationState {
    operands: Vec<ContentOperand>,
    tokens: usize,
    operations: usize,
    failed: bool,
}
impl OperationState {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn finish(&self) -> Result<(), PdfError> {
        if self.failed {
            return Err(PdfError::unsupported(
                "content operation sequence previously failed",
            ));
        }
        if !self.operands.is_empty() {
            return Err(PdfError::syntax(
                "content sequence ends with unused operands",
                0,
            ));
        }
        Ok(())
    }
}

pub(crate) fn read_operations_part(
    input: &[u8],
    part_index: usize,
    state: &mut OperationState,
    limits: &Limits,
    budget: &mut ParseBudget,
) -> Result<Vec<ContentOperation>, PdfError> {
    if state.failed {
        return Err(PdfError::unsupported(
            "content operation sequence previously failed",
        ));
    }
    let result = read_part(input, part_index, state, limits, budget);
    if result.is_err() {
        state.failed = true;
    }
    result
}

fn read_part(
    input: &[u8],
    part_index: usize,
    state: &mut OperationState,
    limits: &Limits,
    budget: &mut ParseBudget,
) -> Result<Vec<ContentOperation>, PdfError> {
    budget.charge_work(input.len().saturating_add(1))?;
    let mut lexer = Lexer::new(input, 0, limits);
    lexer.tokens = state.tokens;
    let mut output = Vec::new();
    loop {
        budget.charge_work(1)?;
        lexer.skip_ws_and_comments();
        let decoded_offset = lexer.pos;
        let Some(mut token) = lexer.next(0)? else {
            break;
        };
        assign_part(&mut token, part_index);
        let token = match token {
            Token::Operator(ref word)
                if matches!(word.as_slice(), b"true" | b"false" | b"null") =>
            {
                token
            }
            Token::Operator(operator) => {
                state.operations = state
                    .operations
                    .checked_add(1)
                    .ok_or_else(|| PdfError::limit("content operation count overflows"))?;
                if state.operations > limits.max_container_items {
                    return Err(PdfError::limit("content operation count exceeds limit"));
                }
                let inline_image = if operator == b"BI" {
                    if !state.operands.is_empty() {
                        return Err(PdfError::syntax(
                            "BI has preceding operands",
                            decoded_offset,
                        ));
                    }
                    let image =
                        parse_inline_image(input, decoded_offset, lexer.pos, limits, budget)?;
                    lexer.pos = image.end;
                    Some(image)
                } else {
                    None
                };
                output.push(ContentOperation {
                    operator,
                    operands: std::mem::take(&mut state.operands),
                    part_index,
                    decoded_offset,
                    inline_image,
                });
                continue;
            }
            token => token,
        };
        if state.operands.len() >= limits.max_container_items {
            return Err(PdfError::limit("content operands exceed limit"));
        }
        state.operands.push(operand(token, budget)?);
    }
    state.tokens = lexer.tokens;
    Ok(output)
}

fn operand(token: Token, budget: &ParseBudget) -> Result<ContentOperand, PdfError> {
    budget.charge_work(1)?;
    Ok(match token {
        Token::Number(value) if value.is_finite() => ContentOperand::Number(value),
        Token::Number(_) => return Err(PdfError::unsupported("nonfinite content number")),
        Token::Name(name) => ContentOperand::Name(name),
        Token::String(item) => ContentOperand::Bytes {
            bytes: item.raw,
            part_index: item.part_index,
            decoded_span: item.decoded_span,
        },
        Token::Array(items) => ContentOperand::Array(
            items
                .into_iter()
                .map(|item| operand(item, budget))
                .collect::<Result<_, _>>()?,
        ),
        Token::Other => ContentOperand::UnsupportedDictionary,
        Token::Operator(word) if word == b"true" => ContentOperand::Boolean(true),
        Token::Operator(word) if word == b"false" => ContentOperand::Boolean(false),
        Token::Operator(word) if word == b"null" => ContentOperand::Null,
        Token::Operator(_) => {
            return Err(PdfError::syntax("operator inside content operand array", 0));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PdfErrorCode;
    use binas_core::CancellationToken;
    #[test]
    fn complete_operands_cross_parts_with_string_provenance() {
        let limits = Limits::default();
        let mut budget = ParseBudget::new(&limits, CancellationToken::new());
        let mut state = OperationState::new();
        assert!(
            read_operations_part(
                b"/F1 12 [(hidden /Fake Do)]",
                3,
                &mut state,
                &limits,
                &mut budget
            )
            .unwrap()
            .is_empty()
        );
        let result =
            read_operations_part(b"  custom /Real Do", 4, &mut state, &limits, &mut budget)
                .unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].operator, b"custom");
        assert_eq!(result[0].decoded_offset, 2);
        assert_eq!(result[0].part_index, 4);
        let ContentOperand::Array(items) = &result[0].operands[2] else {
            panic!()
        };
        let ContentOperand::Bytes {
            bytes,
            part_index,
            decoded_span,
        } = &items[0]
        else {
            panic!()
        };
        assert_eq!(*part_index, 3);
        assert_eq!(bytes, b"hidden /Fake Do");
        assert_eq!(decoded_span.start(), 8);
        assert_eq!(
            result[1].operands,
            vec![ContentOperand::Name(b"Real".to_vec())]
        );
        state.finish().unwrap();
    }
    #[test]
    fn inline_payload_is_one_event_and_dictionary_is_explicit() {
        let limits = Limits::default();
        let mut budget = ParseBudget::new(&limits, CancellationToken::new());
        let mut state = OperationState::new();
        let result = read_operations_part(b"BI /W 8 /H 1 /BPC 8 /CS /G ID /Fake Do EI /Tag << /Hidden /Font >> BDC /GS gs /C cs /P scn", 0, &mut state, &limits, &mut budget).unwrap();
        assert_eq!(
            result
                .iter()
                .map(|op| op.operator.as_slice())
                .collect::<Vec<_>>(),
            vec![b"BI".as_slice(), b"BDC", b"gs", b"cs", b"scn"]
        );
        assert_eq!(result[0].inline_image.as_ref().unwrap().width, 8);
        assert_eq!(result[1].operands[1], ContentOperand::UnsupportedDictionary);
        state.finish().unwrap();
    }
    #[test]
    fn cumulative_limits_cancel_and_partial_tokens_fail() {
        let limits = Limits {
            max_container_items: 3,
            ..Limits::default()
        };
        let token = CancellationToken::new();
        let mut budget = ParseBudget::new(&limits, token.clone());
        let mut state = OperationState::new();
        read_operations_part(b"q Q", 0, &mut state, &limits, &mut budget).unwrap();
        assert_eq!(
            read_operations_part(b"q Q", 1, &mut state, &limits, &mut budget)
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        assert!(state.finish().is_err());
        let mut state = OperationState::new();
        token.cancel();
        assert_eq!(
            read_operations_part(b"", 0, &mut state, &limits, &mut budget)
                .unwrap_err()
                .code,
            PdfErrorCode::Cancelled
        );
        for bytes in [b"(split".as_slice(), b"[1", b"<< /Key"] {
            let mut state = OperationState::new();
            let mut budget = ParseBudget::new(&limits, CancellationToken::new());
            assert!(read_operations_part(bytes, 0, &mut state, &limits, &mut budget).is_err());
        }
    }

    #[test]
    fn work_is_cumulative_even_for_fresh_sequences() {
        let limits = Limits {
            max_decode_work_bytes: 24,
            ..Limits::default()
        };
        let mut budget = ParseBudget::new(&limits, CancellationToken::new());
        let mut failed = false;
        for part in 0..10 {
            let mut state = OperationState::new();
            match read_operations_part(b"q", part, &mut state, &limits, &mut budget) {
                Ok(_) => state.finish().unwrap(),
                Err(error) => {
                    assert_eq!(error.code, PdfErrorCode::ResourceLimit);
                    assert!(part > 0);
                    failed = true;
                    break;
                }
            }
        }
        assert!(failed);
    }
}
