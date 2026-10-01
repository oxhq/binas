use binas_core::Span;
pub(crate) mod operations;
use std::collections::BTreeMap;

use crate::{
    PdfErrorCode,
    error::PdfError,
    limits::Limits,
    parser::{ParseBudget, Value, decode_stream_bounded},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InlineColorSpace {
    Gray,
    Rgb,
    Cmyk,
}

impl InlineColorSpace {
    pub(crate) fn components(self) -> usize {
        match self {
            Self::Gray => 1,
            Self::Rgb => 3,
            Self::Cmyk => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InlineFilter {
    Raw,
    Flate,
    AsciiHex,
    Ascii85,
    RunLength,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InlineImage {
    pub start: usize,
    pub data_start: usize,
    pub data_end: usize,
    pub end: usize,
    pub width: u32,
    pub height: u32,
    pub bits_per_component: u8,
    pub color_space: InlineColorSpace,
    pub filter: InlineFilter,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TextItem {
    /// Physical content part containing the complete string operand.
    pub part_index: usize,
    pub text: String,
    pub span: Span,
    pub raw: Vec<u8>,
    pub decoded_span: Span,
    pub font: Option<Vec<u8>>,
    pub font_part_index: Option<usize>,
    pub operator: Vec<u8>,
    pub geometry: Option<Box<TextItemGeometry>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TextItemGeometry {
    pub user_matrix: [f64; 6],
    pub text_matrix: Option<[f64; 6]>,
    pub font_size: Option<f64>,
    pub text_rise: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct XObjectInvocation {
    pub name: Vec<u8>,
    pub user_matrix: [f64; 6],
    pub inherited_font: Option<Vec<u8>>,
    pub inherited_font_part_index: Option<usize>,
    pub inherited_font_size: Option<f64>,
    pub inherited_leading: f64,
    pub inherited_text_rise: f64,
}

const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

#[derive(Debug)]
struct SavedGraphicsState {
    user_matrix: [f64; 6],
    active_font: Option<Vec<u8>>,
    font_part_index: Option<usize>,
    font_size: Option<f64>,
    leading: f64,
    text_rise: f64,
}

/// Supported text interpretation across logically concatenated Contents parts.
/// This is not a complete graphics-state interpreter: font metrics, gs effects,
/// clipping and other rendering semantics remain separate coverage work.
#[derive(Debug)]
pub(crate) struct ContentTextState {
    operands: Vec<Token>,
    in_text: bool,
    graphics_depth: usize,
    graphics_stack: Vec<SavedGraphicsState>,
    operations: usize,
    tokens: usize,
    active_font: Option<Vec<u8>>,
    font_part_index: Option<usize>,
    user_matrix: [f64; 6],
    text_matrix: Option<[f64; 6]>,
    text_line_matrix: [f64; 6],
    font_size: Option<f64>,
    leading: f64,
    text_rise: f64,
    failed: bool,
}

impl ContentTextState {
    pub(crate) fn new() -> Self {
        Self {
            operands: Vec::new(),
            in_text: false,
            graphics_depth: 0,
            graphics_stack: Vec::new(),
            operations: 0,
            tokens: 0,
            active_font: None,
            font_part_index: None,
            user_matrix: IDENTITY,
            text_matrix: Some(IDENTITY),
            text_line_matrix: IDENTITY,
            font_size: None,
            leading: 0.0,
            text_rise: 0.0,
            failed: false,
        }
    }

    /// A Form inherits supported text parameters but has an independent parser
    /// balance and identity local CTM; the document walker composes invocation
    /// and Form matrices externally. The parent's state is never modified.
    pub(crate) fn for_form(invocation: &XObjectInvocation) -> Self {
        Self {
            active_font: invocation.inherited_font.clone(),
            font_part_index: invocation.inherited_font_part_index,
            font_size: invocation.inherited_font_size,
            leading: invocation.inherited_leading,
            text_rise: invocation.inherited_text_rise,
            ..Self::new()
        }
    }

    pub(crate) fn finish(&self) -> Result<(), PdfError> {
        if self.failed {
            return Err(PdfError::syntax(
                "content state failed during a previous part",
                0,
            ));
        }
        if self.in_text {
            return Err(PdfError::syntax("unterminated BT text object", 0));
        }
        if self.graphics_depth != 0 {
            return Err(PdfError::syntax("unbalanced q/Q graphics state", 0));
        }
        if !self.operands.is_empty() {
            return Err(PdfError::syntax(
                "content stream ends with unused operands",
                0,
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TextShowExtraction {
    pub events: Vec<TextShowEvent>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TextShowEvent {
    Text(TextItem),
    XObject(XObjectInvocation),
}

pub(crate) fn extract_text_show(
    input: &[u8],
    base_offset: usize,
    limits: &Limits,
    budget: &mut ParseBudget,
) -> Result<Vec<TextItem>, PdfError> {
    Ok(
        extract_text_show_with_xobjects(input, base_offset, limits, budget)?
            .events
            .into_iter()
            .filter_map(|event| match event {
                TextShowEvent::Text(item) => Some(item),
                TextShowEvent::XObject(_) => None,
            })
            .collect(),
    )
}

pub(crate) fn extract_text_show_with_xobjects(
    input: &[u8],
    base_offset: usize,
    limits: &Limits,
    budget: &mut ParseBudget,
) -> Result<TextShowExtraction, PdfError> {
    let mut state = ContentTextState::new();
    let extraction = extract_text_show_part(input, base_offset, 0, &mut state, limits, budget)?;
    state.finish()?;
    Ok(extraction)
}

pub(crate) fn extract_text_show_part(
    input: &[u8],
    base_offset: usize,
    part_index: usize,
    state: &mut ContentTextState,
    limits: &Limits,
    budget: &mut ParseBudget,
) -> Result<TextShowExtraction, PdfError> {
    if state.failed {
        return Err(PdfError::syntax(
            "content state failed during a previous part",
            0,
        ));
    }
    let result = extract_part(input, base_offset, part_index, state, limits, budget);
    if result.is_err() {
        state.failed = true;
    }
    result
}

fn extract_part(
    input: &[u8],
    base_offset: usize,
    part_index: usize,
    state: &mut ContentTextState,
    limits: &Limits,
    budget: &mut ParseBudget,
) -> Result<TextShowExtraction, PdfError> {
    budget.charge_work(input.len())?;
    let mut lexer = Lexer::new(input, base_offset, limits);
    lexer.tokens = state.tokens;
    let mut events = Vec::new();
    while let Some(mut token) = lexer.next(0)? {
        assign_part(&mut token, part_index);
        let Token::Operator(operator) = token else {
            if state.operands.len() >= limits.max_container_items {
                return Err(PdfError::limit("content operands exceed limit"));
            }
            state.operands.push(token);
            continue;
        };
        budget.charge_work(1)?;
        state.operations = state
            .operations
            .checked_add(1)
            .ok_or_else(|| PdfError::limit("content operation count overflows"))?;
        if state.operations > limits.max_container_items {
            return Err(PdfError::limit("content operation count exceeds limit"));
        }
        match operator.as_slice() {
            b"BT" => {
                require_operands(&state.operands, 0, "BT")?;
                if state.in_text {
                    return Err(PdfError::syntax("nested BT text object", lexer.pos));
                }
                state.in_text = true;
                state.text_matrix = Some(IDENTITY);
                state.text_line_matrix = IDENTITY;
            }
            b"ET" => {
                require_operands(&state.operands, 0, "ET")?;
                if !state.in_text {
                    return Err(PdfError::syntax("ET without BT", lexer.pos));
                }
                state.in_text = false;
            }
            b"q" => {
                require_operands(&state.operands, 0, "q")?;
                state.graphics_depth = state
                    .graphics_depth
                    .checked_add(1)
                    .ok_or_else(|| PdfError::limit("graphics state depth overflows"))?;
                if state.graphics_depth > limits.max_parser_depth {
                    return Err(PdfError::limit("graphics state depth exceeds limit"));
                }
                state.graphics_stack.push(SavedGraphicsState {
                    user_matrix: state.user_matrix,
                    active_font: state.active_font.clone(),
                    font_part_index: state.font_part_index,
                    font_size: state.font_size,
                    leading: state.leading,
                    text_rise: state.text_rise,
                });
            }
            b"Q" => {
                require_operands(&state.operands, 0, "Q")?;
                state.graphics_depth = state
                    .graphics_depth
                    .checked_sub(1)
                    .ok_or_else(|| PdfError::syntax("Q without matching q", lexer.pos))?;
                let saved = state
                    .graphics_stack
                    .pop()
                    .ok_or_else(|| PdfError::syntax("Q without saved graphics state", lexer.pos))?;
                state.user_matrix = saved.user_matrix;
                state.active_font = saved.active_font;
                state.font_part_index = saved.font_part_index;
                state.font_size = saved.font_size;
                state.leading = saved.leading;
                state.text_rise = saved.text_rise;
            }
            b"cm" => {
                require_operands(&state.operands, 6, "cm")?;
                // PDF 1.5 §4.2.3 premultiplies row-vector matrices. Our
                // column-vector storage transposes that order: current * new.
                state.user_matrix =
                    multiply_matrix(state.user_matrix, number_matrix(&state.operands, "cm")?);
            }
            b"Do" => {
                require_operands(&state.operands, 1, "Do")?;
                let Token::Name(name) = &state.operands[0] else {
                    return Err(PdfError::syntax(
                        "Do resource operand is not a name",
                        lexer.pos,
                    ));
                };
                events.push(TextShowEvent::XObject(XObjectInvocation {
                    name: name.clone(),
                    user_matrix: state.user_matrix,
                    inherited_font: state.active_font.clone(),
                    inherited_font_part_index: state.font_part_index,
                    inherited_font_size: state.font_size,
                    inherited_leading: state.leading,
                    inherited_text_rise: state.text_rise,
                }));
            }
            b"Tj" | b"'" => {
                require_text_object(state.in_text, &operator, lexer.pos)?;
                require_operands(&state.operands, 1, operator_name(&operator))?;
                if operator == b"'" {
                    state.text_line_matrix =
                        translate_matrix(state.text_line_matrix, 0.0, -state.leading);
                    state.text_matrix = Some(state.text_line_matrix);
                }
                events.push(TextShowEvent::Text(
                    text_operand(
                        &state.operands[0],
                        lexer.pos,
                        state.active_font.as_deref(),
                        &operator,
                    )?
                    .with_font_part(state.font_part_index)
                    .with_geometry(
                        state.user_matrix,
                        state.text_matrix,
                        state.font_size,
                        state.text_rise,
                    ),
                ));
                state.text_matrix = None;
            }
            b"\"" => {
                require_text_object(state.in_text, &operator, lexer.pos)?;
                require_operands(&state.operands, 3, "\"")?;
                if !matches!(state.operands[0], Token::Number(_))
                    || !matches!(state.operands[1], Token::Number(_))
                {
                    return Err(PdfError::syntax(
                        "double-quote text operator requires two numeric operands",
                        lexer.pos,
                    ));
                }
                state.text_line_matrix =
                    translate_matrix(state.text_line_matrix, 0.0, -state.leading);
                state.text_matrix = Some(state.text_line_matrix);
                events.push(TextShowEvent::Text(
                    text_operand(
                        &state.operands[2],
                        lexer.pos,
                        state.active_font.as_deref(),
                        &operator,
                    )?
                    .with_font_part(state.font_part_index)
                    .with_geometry(
                        state.user_matrix,
                        state.text_matrix,
                        state.font_size,
                        state.text_rise,
                    ),
                ));
                state.text_matrix = None;
            }
            b"TJ" => {
                require_text_object(state.in_text, &operator, lexer.pos)?;
                require_operands(&state.operands, 1, "TJ")?;
                let Token::Array(items) = &state.operands[0] else {
                    return Err(PdfError::syntax("TJ operand is not an array", lexer.pos));
                };
                for item in items {
                    match item {
                        Token::String(item) => {
                            events.push(TextShowEvent::Text(
                                item.clone()
                                    .with_context(state.active_font.as_deref(), &operator)
                                    .with_font_part(state.font_part_index)
                                    .with_geometry(
                                        state.user_matrix,
                                        state.text_matrix,
                                        state.font_size,
                                        state.text_rise,
                                    ),
                            ));
                            state.text_matrix = None;
                        }
                        Token::Number(_) => {}
                        _ => {
                            return Err(PdfError::syntax(
                                "TJ array contains a non-string, non-number value",
                                lexer.pos,
                            ));
                        }
                    }
                }
            }
            b"Tf" => {
                require_operands(&state.operands, 2, "Tf")?;
                let Token::Name(name) = &state.operands[0] else {
                    return Err(PdfError::syntax(
                        "Tf font resource operand is not a name",
                        lexer.pos,
                    ));
                };
                if !matches!(state.operands[1], Token::Number(_)) {
                    return Err(PdfError::syntax(
                        "Tf font size operand is not a number",
                        lexer.pos,
                    ));
                }
                state.active_font = Some(name.clone());
                state.font_part_index = Some(part_index);
                let Token::Number(size) = &state.operands[1] else {
                    unreachable!()
                };
                state.font_size = Some(*size);
            }
            b"Tm" => {
                require_text_object(state.in_text, &operator, lexer.pos)?;
                require_operands(&state.operands, 6, "Tm")?;
                let matrix = number_matrix(&state.operands, "Tm")?;
                state.text_line_matrix = matrix;
                state.text_matrix = Some(matrix);
            }
            b"Td" | b"TD" => {
                require_text_object(state.in_text, &operator, lexer.pos)?;
                require_operands(&state.operands, 2, operator_name(&operator))?;
                let [tx, ty] = two_numbers(&state.operands, operator_name(&operator))?;
                if operator == b"TD" {
                    state.leading = -ty;
                }
                state.text_line_matrix = translate_matrix(state.text_line_matrix, tx, ty);
                state.text_matrix = Some(state.text_line_matrix);
            }
            b"T*" => {
                require_text_object(state.in_text, &operator, lexer.pos)?;
                require_operands(&state.operands, 0, "T*")?;
                state.text_line_matrix =
                    translate_matrix(state.text_line_matrix, 0.0, -state.leading);
                state.text_matrix = Some(state.text_line_matrix);
            }
            b"TL" => {
                require_operands(&state.operands, 1, "TL")?;
                state.leading = one_number(&state.operands, "TL")?;
            }
            b"Ts" => {
                require_operands(&state.operands, 1, "Ts")?;
                state.text_rise = one_number(&state.operands, "Ts")?;
            }
            b"BI" => {
                require_operands(&state.operands, 0, "BI")?;
                let image =
                    parse_inline_image(input, lexer.pos - 2, lexer.pos, lexer.limits, budget)?;
                lexer.pos = image.end;
            }
            b"gs" => {
                return Err(PdfError::unsupported(
                    "ExtGState resource effects require unsupported graphics-state interpretation",
                ));
            }
            _ => {}
        }
        if !state.user_matrix.iter().all(|number| number.is_finite())
            || !state
                .text_line_matrix
                .iter()
                .all(|number| number.is_finite())
            || state
                .text_matrix
                .is_some_and(|matrix| !matrix.iter().all(|number| number.is_finite()))
        {
            return Err(PdfError::limit("content matrix arithmetic is non-finite"));
        }
        state.operands.clear();
    }
    state.tokens = lexer.tokens;
    Ok(TextShowExtraction { events })
}
pub(crate) fn inline_images(
    input: &[u8],
    limits: &Limits,
    budget: &mut ParseBudget,
) -> Result<Vec<InlineImage>, PdfError> {
    budget.charge_work(0)?;
    let mut lexer = Lexer::new(input, 0, limits);
    let mut images = Vec::new();
    while let Some(token) = lexer.next(0)? {
        if matches!(token, Token::Operator(ref operator) if operator == b"BI") {
            let image = parse_inline_image(input, lexer.pos - 2, lexer.pos, limits, budget)?;
            lexer.pos = image.end;
            images.push(image);
            if images.len() > limits.max_container_items {
                return Err(PdfError::limit("inline image count exceeds limit"));
            }
        }
    }
    Ok(images)
}

fn parse_inline_image(
    input: &[u8],
    start: usize,
    mut position: usize,
    limits: &Limits,
    budget: &mut ParseBudget,
) -> Result<InlineImage, PdfError> {
    budget.charge_work(1)?;
    let mut width = None;
    let mut height = None;
    let mut bits = None;
    let mut color_space = None;
    let mut filter = InlineFilter::Raw;
    let mut entries = 0_usize;
    let data_start = loop {
        skip_inline_ws(input, &mut position);
        if input.get(position..position + 2) == Some(b"ID")
            && input.get(position + 2).is_some_and(|byte| is_ws(*byte))
        {
            position += 2;
            if input.get(position) == Some(&b'\r') && input.get(position + 1) == Some(&b'\n') {
                position += 2;
            } else {
                position += 1;
            }
            break position;
        }
        let key = inline_name(input, &mut position, limits)?;
        skip_inline_ws(input, &mut position);
        entries += 1;
        if entries > limits.max_container_items {
            return Err(PdfError::limit("inline image dictionary exceeds limit"));
        }
        match key.as_slice() {
            b"W" | b"Width" => width = Some(inline_u32(input, &mut position, limits)?),
            b"H" | b"Height" => height = Some(inline_u32(input, &mut position, limits)?),
            b"BPC" | b"BitsPerComponent" => {
                bits = Some(
                    u8::try_from(inline_u32(input, &mut position, limits)?)
                        .map_err(|_| PdfError::unsupported("inline image BPC exceeds u8"))?,
                );
            }
            b"CS" | b"ColorSpace" => {
                let value = inline_name(input, &mut position, limits)?;
                color_space = Some(match value.as_slice() {
                    b"G" | b"DeviceGray" => InlineColorSpace::Gray,
                    b"RGB" | b"DeviceRGB" => InlineColorSpace::Rgb,
                    b"CMYK" | b"DeviceCMYK" => InlineColorSpace::Cmyk,
                    _ => {
                        return Err(PdfError::unsupported(
                            "inline image supports only device color spaces",
                        ));
                    }
                });
            }
            b"F" | b"Filter" => {
                let value = inline_name(input, &mut position, limits)?;
                filter = match value.as_slice() {
                    b"Fl" | b"FlateDecode" => InlineFilter::Flate,
                    b"AHx" | b"ASCIIHexDecode" => InlineFilter::AsciiHex,
                    b"A85" | b"ASCII85Decode" => InlineFilter::Ascii85,
                    b"RL" | b"RunLengthDecode" => InlineFilter::RunLength,
                    _ => {
                        return Err(PdfError::unsupported("unsupported inline image filter"));
                    }
                };
            }
            b"DP" | b"DecodeParms" | b"D" | b"Decode" | b"IM" | b"ImageMask" => {
                return Err(PdfError::unsupported(
                    "inline image Decode, DecodeParms, and ImageMask are not supported",
                ));
            }
            _ => {
                return Err(PdfError::unsupported(format!(
                    "unsupported inline image dictionary key /{}",
                    String::from_utf8_lossy(&key)
                )));
            }
        }
    };
    let width = width.ok_or_else(|| PdfError::syntax("inline image has no width", start))?;
    let height = height.ok_or_else(|| PdfError::syntax("inline image has no height", start))?;
    let bits_per_component =
        bits.ok_or_else(|| PdfError::syntax("inline image has no bits per component", start))?;
    let color_space =
        color_space.ok_or_else(|| PdfError::syntax("inline image has no color space", start))?;
    if width == 0 || height == 0 || !matches!(bits_per_component, 1 | 2 | 4 | 8 | 16) {
        return Err(PdfError::unsupported(
            "inline image dimensions and bits per component are invalid",
        ));
    }
    let expected = inline_sample_bytes(width, height, bits_per_component, color_space, limits)?;
    let data_end = inline_data_end(input, data_start, expected, filter, limits, budget)?;
    let end = inline_ei_end(input, data_end)?;
    Ok(InlineImage {
        start,
        data_start,
        data_end,
        end,
        width,
        height,
        bits_per_component,
        color_space,
        filter,
    })
}

fn inline_data_end(
    input: &[u8],
    start: usize,
    expected: usize,
    filter: InlineFilter,
    limits: &Limits,
    budget: &mut ParseBudget,
) -> Result<usize, PdfError> {
    match filter {
        InlineFilter::Raw => {
            let end = start
                .checked_add(expected)
                .ok_or_else(|| PdfError::limit("inline image length overflows"))?;
            if end > input.len() {
                return Err(PdfError::syntax("inline image data is truncated", start));
            }
            Ok(end)
        }
        InlineFilter::AsciiHex => inline_terminator(input, start, b">", budget).and_then(|end| {
            validate_inline_decode(input, start, end, expected, filter, limits, budget)
        }),
        InlineFilter::Ascii85 => inline_terminator(input, start, b"~>", budget).and_then(|end| {
            validate_inline_decode(input, start, end, expected, filter, limits, budget)
        }),
        InlineFilter::RunLength => {
            let mut position = start;
            loop {
                budget.charge_work(1)?;
                let control = *input.get(position).ok_or_else(|| {
                    PdfError::syntax("inline RunLength image is truncated", start)
                })?;
                position += 1;
                match control {
                    128 => break,
                    0..=127 => position = position.saturating_add(usize::from(control) + 1),
                    129..=255 => position = position.saturating_add(1),
                }
                if position > input.len() || position - start > limits.max_stream_bytes {
                    return Err(PdfError::limit("inline RunLength image exceeds limit"));
                }
            }
            validate_inline_decode(input, start, position, expected, filter, limits, budget)
        }
        InlineFilter::Flate => {
            let mut candidates = Vec::new();
            let scan_end = input.len().saturating_sub(2);
            for position in start..scan_end {
                // Reserve bounded scan chunks before inspecting candidate bytes.
                // These units are conservative when an error stops a chunk early.
                if (position - start).is_multiple_of(1024) {
                    budget.charge_work((scan_end - position).min(1024))?;
                }
                if is_ws(input[position])
                    && input.get(position + 1..position + 3) == Some(b"EI")
                    && input
                        .get(position + 3)
                        .is_none_or(|byte| is_delimiter(*byte))
                {
                    match validate_inline_decode(
                        input, start, position, expected, filter, limits, budget,
                    ) {
                        Ok(_) => {}
                        Err(error) if error.code == PdfErrorCode::InvalidSyntax => continue,
                        Err(error) => return Err(error),
                    }
                    candidates.push(position);
                    if candidates.len() > 1 {
                        return Err(PdfError::unsupported(
                            "inline Flate image has ambiguous EI delimiters",
                        ));
                    }
                }
            }
            candidates.pop().ok_or_else(|| {
                PdfError::syntax("inline Flate image has no valid EI delimiter", start)
            })
        }
    }
}

fn inline_terminator(
    input: &[u8],
    start: usize,
    marker: &[u8],
    budget: &ParseBudget,
) -> Result<usize, PdfError> {
    let input = input
        .get(start..)
        .ok_or_else(|| PdfError::syntax("inline image data starts past input", start))?;
    let windows = input.len().saturating_sub(marker.len()).saturating_add(1);
    for (offset, bytes) in input.windows(marker.len()).enumerate() {
        if offset.is_multiple_of(1024) {
            // Conservative precharged scan units, including a partial final chunk.
            budget.charge_work((windows - offset).min(1024))?;
        }
        if bytes == marker {
            return Ok(start + offset + marker.len());
        }
    }
    Err(PdfError::syntax(
        "inline image has no encoded-data terminator",
        start,
    ))
}

fn validate_inline_decode(
    input: &[u8],
    start: usize,
    end: usize,
    expected: usize,
    filter: InlineFilter,
    limits: &Limits,
    budget: &mut ParseBudget,
) -> Result<usize, PdfError> {
    let pdf_filter: &[u8] = match filter {
        InlineFilter::Flate => b"FlateDecode",
        InlineFilter::AsciiHex => b"ASCIIHexDecode",
        InlineFilter::Ascii85 => b"ASCII85Decode",
        InlineFilter::RunLength => b"RunLengthDecode",
        InlineFilter::Raw => return Ok(end),
    };
    let value = Value::Dict(BTreeMap::from([(
        b"Filter".to_vec(),
        Value::Name(pdf_filter.to_vec()),
    )]));
    let encoded = input
        .get(start..end)
        .ok_or_else(|| PdfError::syntax("inline image data range is invalid", start))?;
    let decoded = decode_stream_bounded(&value, encoded, limits, budget, expected)?;
    if decoded.len() != expected {
        return Err(PdfError::syntax(
            "inline image decoded sample length does not match dimensions",
            start,
        ));
    }
    if end - start > limits.max_stream_bytes {
        return Err(PdfError::limit("inline image encoded data exceeds limit"));
    }
    Ok(end)
}

fn inline_ei_end(input: &[u8], data_end: usize) -> Result<usize, PdfError> {
    let mut position = data_end;
    if !input.get(position).is_some_and(|byte| is_ws(*byte)) {
        return Err(PdfError::unsupported(
            "inline image EI is not safely whitespace-delimited",
        ));
    }
    while input.get(position).is_some_and(|byte| is_ws(*byte)) {
        position += 1;
    }
    if input.get(position..position + 2) != Some(b"EI")
        || input
            .get(position + 2)
            .is_some_and(|byte| !is_delimiter(*byte))
    {
        return Err(PdfError::unsupported(
            "inline image EI is not safely delimited",
        ));
    }
    Ok(position + 2)
}

fn inline_sample_bytes(
    width: u32,
    height: u32,
    bits: u8,
    color_space: InlineColorSpace,
    limits: &Limits,
) -> Result<usize, PdfError> {
    let row_bits = usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(color_space.components()))
        .and_then(|samples| samples.checked_mul(usize::from(bits)))
        .ok_or_else(|| PdfError::limit("inline image row size overflows"))?;
    let row = row_bits
        .checked_add(7)
        .map(|bits| bits / 8)
        .ok_or_else(|| PdfError::limit("inline image row size overflows"))?;
    let length = row
        .checked_mul(
            usize::try_from(height)
                .map_err(|_| PdfError::limit("inline image height exceeds usize"))?,
        )
        .ok_or_else(|| PdfError::limit("inline image sample size overflows"))?;
    if length > limits.max_stream_bytes {
        return Err(PdfError::limit("inline image samples exceed stream limit"));
    }
    Ok(length)
}

fn skip_inline_ws(input: &[u8], position: &mut usize) {
    loop {
        while input.get(*position).is_some_and(|byte| is_ws(*byte)) {
            *position += 1;
        }
        if input.get(*position) != Some(&b'%') {
            return;
        }
        while input
            .get(*position)
            .is_some_and(|byte| !matches!(*byte, b'\r' | b'\n'))
        {
            *position += 1;
        }
    }
}

fn inline_name(input: &[u8], position: &mut usize, limits: &Limits) -> Result<Vec<u8>, PdfError> {
    if input.get(*position) != Some(&b'/') {
        return Err(PdfError::syntax(
            "inline image dictionary key/value is not a name",
            *position,
        ));
    }
    let mut lexer = Lexer::new(input, 0, limits);
    lexer.pos = *position;
    let Token::Name(name) = lexer
        .next(0)?
        .ok_or_else(|| PdfError::syntax("truncated inline image name", *position))?
    else {
        unreachable!();
    };
    *position = lexer.pos;
    Ok(name)
}

fn inline_u32(input: &[u8], position: &mut usize, limits: &Limits) -> Result<u32, PdfError> {
    let start = *position;
    while input
        .get(*position)
        .is_some_and(|byte| !is_delimiter(*byte))
    {
        *position += 1;
    }
    if *position == start || *position - start > limits.max_token_bytes {
        return Err(PdfError::syntax("invalid inline image integer", start));
    }
    std::str::from_utf8(&input[start..*position])
        .ok()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| PdfError::syntax("invalid inline image integer", start))
}

fn require_text_object(in_text: bool, operator: &[u8], offset: usize) -> Result<(), PdfError> {
    if in_text {
        Ok(())
    } else {
        Err(PdfError::syntax(
            format!("{} outside BT/ET", operator_name(operator)),
            offset,
        ))
    }
}

fn require_operands(operands: &[Token], count: usize, operator: &str) -> Result<(), PdfError> {
    if operands.len() == count {
        Ok(())
    } else {
        Err(PdfError::syntax(
            format!(
                "{operator} requires {count} operands, got {}",
                operands.len()
            ),
            0,
        ))
    }
}

fn one_number(operands: &[Token], operator: &str) -> Result<f64, PdfError> {
    match operands {
        [Token::Number(value)] => Ok(*value),
        _ => Err(PdfError::syntax(
            format!("{operator} requires numeric operands"),
            0,
        )),
    }
}

fn two_numbers(operands: &[Token], operator: &str) -> Result<[f64; 2], PdfError> {
    match operands {
        [Token::Number(first), Token::Number(second)] => Ok([*first, *second]),
        _ => Err(PdfError::syntax(
            format!("{operator} requires numeric operands"),
            0,
        )),
    }
}

fn number_matrix(operands: &[Token], operator: &str) -> Result<[f64; 6], PdfError> {
    match operands {
        [
            Token::Number(a),
            Token::Number(b),
            Token::Number(c),
            Token::Number(d),
            Token::Number(e),
            Token::Number(f),
        ] => Ok([*a, *b, *c, *d, *e, *f]),
        _ => Err(PdfError::syntax(
            format!("{operator} requires numeric operands"),
            0,
        )),
    }
}

fn multiply_matrix(left: [f64; 6], right: [f64; 6]) -> [f64; 6] {
    [
        left[0] * right[0] + left[2] * right[1],
        left[1] * right[0] + left[3] * right[1],
        left[0] * right[2] + left[2] * right[3],
        left[1] * right[2] + left[3] * right[3],
        left[0] * right[4] + left[2] * right[5] + left[4],
        left[1] * right[4] + left[3] * right[5] + left[5],
    ]
}

fn translate_matrix(matrix: [f64; 6], tx: f64, ty: f64) -> [f64; 6] {
    multiply_matrix(matrix, [1.0, 0.0, 0.0, 1.0, tx, ty])
}

fn text_operand(
    token: &Token,
    offset: usize,
    font: Option<&[u8]>,
    operator: &[u8],
) -> Result<TextItem, PdfError> {
    match token {
        Token::String(item) => Ok(item.clone().with_context(font, operator)),
        _ => Err(PdfError::syntax(
            "text-show operand is not a string",
            offset,
        )),
    }
}

fn operator_name(operator: &[u8]) -> &str {
    std::str::from_utf8(operator).unwrap_or("non-ASCII operator")
}

#[derive(Debug)]
enum Token {
    String(TextItem),
    Number(f64),
    Name(Vec<u8>),
    Array(Vec<Token>),
    Other,
    Operator(Vec<u8>),
}

fn assign_part(token: &mut Token, part_index: usize) {
    match token {
        Token::String(item) => item.part_index = part_index,
        Token::Array(items) => {
            for item in items {
                assign_part(item, part_index);
            }
        }
        _ => {}
    }
}

struct Lexer<'a> {
    input: &'a [u8],
    base_offset: usize,
    limits: &'a Limits,
    pos: usize,
    tokens: usize,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a [u8], base_offset: usize, limits: &'a Limits) -> Self {
        Self {
            input,
            base_offset,
            limits,
            pos: 0,
            tokens: 0,
        }
    }

    fn next(&mut self, depth: usize) -> Result<Option<Token>, PdfError> {
        self.skip_ws_and_comments();
        if self.pos == self.input.len() {
            return Ok(None);
        }
        if depth > self.limits.max_parser_depth {
            return Err(PdfError::limit("content nesting depth exceeds limit"));
        }
        self.tokens += 1;
        if self.tokens > self.limits.max_container_items {
            return Err(PdfError::limit("content token count exceeds limit"));
        }
        match self.input[self.pos] {
            b'(' => self.literal_string(depth).map(Token::String).map(Some),
            b'<' if self.input.get(self.pos + 1) == Some(&b'<') => {
                self.dictionary(depth + 1)?;
                Ok(Some(Token::Other))
            }
            b'<' => self.hex_string().map(Token::String).map(Some),
            b'[' => self.array(depth + 1).map(Token::Array).map(Some),
            b']' | b')' | b'>' => Err(PdfError::syntax("unexpected content delimiter", self.pos)),
            b'/' => self.name().map(Token::Name).map(Some),
            _ => {
                let word = self.word()?.to_vec();
                if let Some(number) = parse_number(&word) {
                    Ok(Some(Token::Number(number)))
                } else {
                    Ok(Some(Token::Operator(word)))
                }
            }
        }
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            while self.input.get(self.pos).is_some_and(|byte| is_ws(*byte)) {
                self.pos += 1;
            }
            if self.input.get(self.pos) != Some(&b'%') {
                return;
            }
            while self
                .input
                .get(self.pos)
                .is_some_and(|byte| !matches!(*byte, b'\r' | b'\n'))
            {
                self.pos += 1;
            }
        }
    }

    fn word(&mut self) -> Result<&'a [u8], PdfError> {
        let start = self.pos;
        while self
            .input
            .get(self.pos)
            .is_some_and(|byte| !is_delimiter(*byte))
        {
            self.pos += 1;
        }
        if self.pos == start {
            return Err(PdfError::syntax("empty content token", self.pos));
        }
        if self.pos - start > self.limits.max_token_bytes {
            return Err(PdfError::limit("content token exceeds max_token_bytes"));
        }
        Ok(&self.input[start..self.pos])
    }

    fn name(&mut self) -> Result<Vec<u8>, PdfError> {
        let start = self.pos;
        self.pos += 1;
        let mut value = Vec::new();
        while self
            .input
            .get(self.pos)
            .is_some_and(|byte| !is_delimiter(*byte))
        {
            let byte = self.input[self.pos];
            if byte == b'#' {
                let hi = *self
                    .input
                    .get(self.pos + 1)
                    .ok_or_else(|| PdfError::syntax("truncated content name escape", self.pos))?;
                let lo = *self
                    .input
                    .get(self.pos + 2)
                    .ok_or_else(|| PdfError::syntax("truncated content name escape", self.pos))?;
                value
                    .push(hex_pair(hi, lo).ok_or_else(|| {
                        PdfError::syntax("invalid content name escape", self.pos)
                    })?);
                self.pos += 3;
            } else {
                value.push(byte);
                self.pos += 1;
            }
        }
        if self.pos - start > self.limits.max_token_bytes {
            return Err(PdfError::limit("content name exceeds max_token_bytes"));
        }
        Ok(value)
    }

    fn literal_string(&mut self, initial_depth: usize) -> Result<TextItem, PdfError> {
        let start = self.pos;
        self.pos += 1;
        let mut depth = initial_depth + 1;
        if depth > self.limits.max_parser_depth {
            return Err(PdfError::limit("literal string depth exceeds limit"));
        }
        let mut value = Vec::new();
        while let Some(byte) = self.input.get(self.pos).copied() {
            self.pos += 1;
            match byte {
                b'(' => {
                    depth += 1;
                    if depth > self.limits.max_parser_depth {
                        return Err(PdfError::limit("literal string depth exceeds limit"));
                    }
                    value.push(byte);
                }
                b')' => {
                    depth -= 1;
                    if depth == initial_depth {
                        return self.text_item(start, value);
                    }
                    value.push(byte);
                }
                b'\\' => self.literal_escape(&mut value)?,
                _ => value.push(byte),
            }
            if value.len() > self.limits.max_token_bytes {
                return Err(PdfError::limit("content string exceeds max_token_bytes"));
            }
        }
        Err(PdfError::syntax("unterminated literal string", start))
    }

    fn literal_escape(&mut self, value: &mut Vec<u8>) -> Result<(), PdfError> {
        let byte = *self
            .input
            .get(self.pos)
            .ok_or_else(|| PdfError::syntax("truncated literal string escape", self.pos))?;
        self.pos += 1;
        match byte {
            b'n' => value.push(b'\n'),
            b'r' => value.push(b'\r'),
            b't' => value.push(b'\t'),
            b'b' => value.push(8),
            b'f' => value.push(12),
            b'(' | b')' | b'\\' => value.push(byte),
            b'\r' => {
                if self.input.get(self.pos) == Some(&b'\n') {
                    self.pos += 1;
                }
            }
            b'\n' => {}
            b'0'..=b'7' => {
                let mut octal = u16::from(byte - b'0');
                for _ in 0..2 {
                    let Some(next @ b'0'..=b'7') = self.input.get(self.pos).copied() else {
                        break;
                    };
                    self.pos += 1;
                    octal = octal * 8 + u16::from(next - b'0');
                }
                value.push((octal & 0xff) as u8);
            }
            _ => value.push(byte),
        }
        Ok(())
    }

    fn hex_string(&mut self) -> Result<TextItem, PdfError> {
        let start = self.pos;
        self.pos += 1;
        let mut digits = Vec::new();
        loop {
            let byte = *self
                .input
                .get(self.pos)
                .ok_or_else(|| PdfError::syntax("unterminated hex string", start))?;
            self.pos += 1;
            if byte == b'>' {
                break;
            }
            if is_ws(byte) {
                continue;
            }
            if !byte.is_ascii_hexdigit() {
                return Err(PdfError::syntax("invalid hex string digit", self.pos - 1));
            }
            digits.push(byte);
            if digits.len() > self.limits.max_token_bytes.saturating_mul(2) {
                return Err(PdfError::limit("hex string exceeds max_token_bytes"));
            }
        }
        if digits.len() % 2 == 1 {
            digits.push(b'0');
        }
        let mut value = Vec::with_capacity(digits.len() / 2);
        for pair in digits.chunks_exact(2) {
            value.push(
                hex_pair(pair[0], pair[1])
                    .ok_or_else(|| PdfError::syntax("invalid hex string digit", start))?,
            );
        }
        self.text_item(start, value)
    }

    fn array(&mut self, depth: usize) -> Result<Vec<Token>, PdfError> {
        self.pos += 1;
        let mut values = Vec::new();
        loop {
            self.skip_ws_and_comments();
            if self.input.get(self.pos) == Some(&b']') {
                self.pos += 1;
                return Ok(values);
            }
            let token = self
                .next(depth)?
                .ok_or_else(|| PdfError::syntax("unterminated content array", self.pos))?;
            values.push(token);
        }
    }

    fn dictionary(&mut self, depth: usize) -> Result<(), PdfError> {
        self.pos += 2;
        loop {
            self.skip_ws_and_comments();
            if self.input.get(self.pos..self.pos.saturating_add(2)) == Some(b">>") {
                self.pos += 2;
                return Ok(());
            }
            self.next(depth)?
                .ok_or_else(|| PdfError::syntax("unterminated content dictionary", self.pos))?;
        }
    }

    fn text_item(&self, start: usize, value: Vec<u8>) -> Result<TextItem, PdfError> {
        let decoded_span = Span::new(start as u64, self.pos as u64)
            .map_err(|_| PdfError::limit("content decoded span is invalid"))?;
        let start = self
            .base_offset
            .checked_add(start)
            .ok_or_else(|| PdfError::limit("content source span overflows"))?;
        let end = self
            .base_offset
            .checked_add(self.pos)
            .ok_or_else(|| PdfError::limit("content source span overflows"))?;
        Ok(TextItem {
            part_index: 0,
            text: String::from_utf8_lossy(&value).into_owned(),
            span: Span::new(start as u64, end as u64)
                .map_err(|_| PdfError::limit("content source span is invalid"))?,
            raw: value,
            decoded_span,
            font: None,
            font_part_index: None,
            operator: Vec::new(),
            geometry: None,
        })
    }
}

impl TextItem {
    fn with_font_part(mut self, part_index: Option<usize>) -> Self {
        self.font_part_index = part_index;
        self
    }
    fn with_context(mut self, font: Option<&[u8]>, operator: &[u8]) -> Self {
        self.font = font.map(<[u8]>::to_vec);
        self.operator = operator.to_vec();
        self
    }

    fn with_geometry(
        mut self,
        user_matrix: [f64; 6],
        text_matrix: Option<[f64; 6]>,
        font_size: Option<f64>,
        text_rise: f64,
    ) -> Self {
        self.geometry = Some(Box::new(TextItemGeometry {
            user_matrix,
            text_matrix,
            font_size,
            text_rise,
        }));
        self
    }
}

fn parse_number(word: &[u8]) -> Option<f64> {
    std::str::from_utf8(word)
        .ok()
        .and_then(|word| word.parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

fn is_ws(byte: u8) -> bool {
    matches!(byte, 0 | b'\t' | b'\n' | 12 | b'\r' | b' ')
}

fn is_delimiter(byte: u8) -> bool {
    is_ws(byte)
        || matches!(
            byte,
            b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
        )
}

fn hex_pair(hi: u8, lo: u8) -> Option<u8> {
    Some(hex(hi)? * 16 + hex(lo)?)
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Standalone lexer fixtures have no document session; production callers
    // always provide their existing document's retained decode budget.
    fn extract_text_show(
        input: &[u8],
        offset: usize,
        limits: &Limits,
    ) -> Result<Vec<TextItem>, PdfError> {
        super::extract_text_show(
            input,
            offset,
            limits,
            &mut ParseBudget::new(limits, binas_core::CancellationToken::new()),
        )
    }

    fn inline_images(input: &[u8], limits: &Limits) -> Result<Vec<InlineImage>, PdfError> {
        super::inline_images(
            input,
            limits,
            &mut ParseBudget::new(limits, binas_core::CancellationToken::new()),
        )
    }
    use crate::PdfErrorCode;

    fn part(
        input: &[u8],
        index: usize,
        state: &mut ContentTextState,
        limits: &Limits,
        budget: &mut ParseBudget,
    ) -> TextShowExtraction {
        extract_text_show_part(input, 1000 * index, index, state, limits, budget).unwrap()
    }

    fn text_events(extraction: TextShowExtraction) -> Vec<TextItem> {
        extraction
            .events
            .into_iter()
            .filter_map(|event| match event {
                TextShowEvent::Text(item) => Some(item),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn text_and_operands_cross_parts_with_physical_operand_provenance() {
        let limits = Limits::default();
        let mut budget = ParseBudget::new(&limits, binas_core::CancellationToken::new());
        let mut state = ContentTextState::new();
        let first = b"q 2 0";
        assert!(
            part(first, 3, &mut state, &limits, &mut budget)
                .events
                .is_empty()
        );
        let second = b"0 2 0 0 cm BT /F1 12 Tf (before)";
        assert!(
            part(second, 4, &mut state, &limits, &mut budget)
                .events
                .is_empty()
        );
        let events = text_events(part(
            b"Tj [(array) -4 <686578>]",
            5,
            &mut state,
            &limits,
            &mut budget,
        ));
        assert_eq!(events[0].text, "before");
        assert_eq!(events[0].part_index, 4);
        assert_eq!(events[0].font_part_index, Some(4));
        assert_eq!(
            events[0].decoded_span.slice(second),
            Some(b"(before)".as_slice())
        );
        assert_eq!(
            events[0].span.start(),
            4000 + events[0].decoded_span.start()
        );
        assert_eq!(
            events[0].geometry.as_ref().unwrap().user_matrix,
            [2.0, 0.0, 0.0, 2.0, 0.0, 0.0]
        );
        let events = text_events(part(b"TJ ET Q", 6, &mut state, &limits, &mut budget));
        assert_eq!(
            events
                .iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>(),
            ["array", "hex"]
        );
        assert!(
            events
                .iter()
                .all(|item| item.part_index == 5 && item.font_part_index == Some(4))
        );
        state.finish().unwrap();
    }

    #[test]
    fn graphics_restore_supported_text_parameters_but_not_text_matrices() {
        let limits = Limits::default();
        let mut budget = ParseBudget::new(&limits, binas_core::CancellationToken::new());
        let mut state = ContentTextState::new();
        part(
            b"/F1 10 Tf 7 TL BT 1 0 0 1 1 2 Tm q",
            10,
            &mut state,
            &limits,
            &mut budget,
        );
        let items = text_events(part(
            b"/F2 20 Tf 30 TL 1 0 0 1 9 8 Tm Q (a) Tj T* (b) Tj ET BT (c) Tj ET",
            11,
            &mut state,
            &limits,
            &mut budget,
        ));
        assert!(
            items
                .iter()
                .all(|item| item.font.as_deref() == Some(b"F1".as_slice())
                    && item.font_part_index == Some(10))
        );
        assert!(
            items
                .iter()
                .all(|item| item.geometry.as_ref().unwrap().font_size == Some(10.0))
        );
        assert_eq!(
            items[0].geometry.as_ref().unwrap().text_matrix.unwrap()[4..],
            [9.0, 8.0]
        );
        assert_eq!(
            items[1].geometry.as_ref().unwrap().text_matrix.unwrap()[4..],
            [9.0, 1.0]
        );
        assert_eq!(
            items[2].geometry.as_ref().unwrap().text_matrix,
            Some(IDENTITY)
        );
        state.finish().unwrap();
    }

    #[test]
    fn cm_concatenation_preserves_existing_translation_across_parts_and_forms() {
        // Adobe PDF 1.5 §4.2.3, printed pp177–179: each new coordinate
        // system maps into the previous one. Translate then scale therefore
        // keeps the translated origin, unlike scale then translate.
        let limits = Limits::default();
        let mut budget = ParseBudget::new(&limits, binas_core::CancellationToken::new());
        let mut state = ContentTextState::new();
        part(b"1 0 0 1 10 20 cm", 0, &mut state, &limits, &mut budget);
        let extraction = part(
            b"2 0 0 3 0 0 cm /Fm Do BT (a) Tj ET",
            1,
            &mut state,
            &limits,
            &mut budget,
        );
        let TextShowEvent::XObject(invocation) = &extraction.events[0] else {
            panic!()
        };
        assert_eq!(invocation.user_matrix, [2.0, 0.0, 0.0, 3.0, 10.0, 20.0]);
        let TextShowEvent::Text(item) = &extraction.events[1] else {
            panic!()
        };
        assert_eq!(
            item.geometry.as_ref().unwrap().user_matrix,
            invocation.user_matrix
        );
        let mut child = ContentTextState::for_form(invocation);
        let child_item = text_events(part(
            b"1 0 0 1 4 5 cm 2 0 0 2 0 0 cm BT (b) Tj ET",
            2,
            &mut child,
            &limits,
            &mut budget,
        ))
        .remove(0);
        let local = child_item.geometry.as_ref().unwrap().user_matrix;
        assert_eq!(local, [2.0, 0.0, 0.0, 2.0, 4.0, 5.0]);
        // Caller composition remains parent * FormMatrix * child-local.
        let form_matrix = [1.0, 0.0, 0.0, 1.0, 7.0, 8.0];
        assert_eq!(
            multiply_matrix(multiply_matrix(invocation.user_matrix, form_matrix), local),
            [4.0, 0.0, 0.0, 6.0, 32.0, 59.0]
        );
        state.finish().unwrap();
        child.finish().unwrap();
        let mut reversed = ContentTextState::new();
        let item = text_events(part(
            b"2 0 0 3 0 0 cm 1 0 0 1 10 20 cm BT (c) Tj ET",
            0,
            &mut reversed,
            &limits,
            &mut budget,
        ))
        .remove(0);
        assert_eq!(
            item.geometry.as_ref().unwrap().user_matrix,
            [2.0, 0.0, 0.0, 3.0, 20.0, 60.0]
        );
    }

    #[test]
    fn forms_inherit_font_identity_and_restore_parent_state() {
        let limits = Limits::default();
        let mut budget = ParseBudget::new(&limits, binas_core::CancellationToken::new());
        let mut parent = ContentTextState::new();
        let extraction = part(
            b"2 0 0 2 50 60 cm /F1 12 Tf 9 TL /Fm Do",
            1,
            &mut parent,
            &limits,
            &mut budget,
        );
        let TextShowEvent::XObject(invocation) = &extraction.events[0] else {
            panic!()
        };
        assert_eq!(invocation.inherited_font_part_index, Some(1));
        let mut child = ContentTextState::for_form(invocation);
        let items = text_events(part(
            b"BT (inherited) Tj /F2 20 Tf (local) Tj ET",
            8,
            &mut child,
            &limits,
            &mut budget,
        ));
        child.finish().unwrap();
        assert_eq!(items[0].font_part_index, Some(1));
        assert_eq!(items[1].font_part_index, Some(8));
        assert_eq!(items[0].geometry.as_ref().unwrap().user_matrix, IDENTITY);
        let item = text_events(part(
            b"BT (parent) Tj ET",
            2,
            &mut parent,
            &limits,
            &mut budget,
        ))
        .remove(0);
        assert_eq!(item.font.as_deref(), Some(b"F1".as_slice()));
        assert_eq!(item.font_part_index, Some(1));
        assert_eq!(
            item.geometry.as_ref().unwrap().user_matrix,
            invocation.user_matrix
        );
        parent.finish().unwrap();
    }

    #[test]
    fn text_rise_persists_across_parts_graphics_restore_and_forms() {
        let limits = Limits::default();
        let mut budget = ParseBudget::new(&limits, binas_core::CancellationToken::new());
        let mut parent = ContentTextState::new();
        part(b"5 Ts q", 0, &mut parent, &limits, &mut budget);
        let extraction = part(
            b"12 Ts BT (raised) Tj ET Q /Fm Do BT (restored) Tj ET",
            1,
            &mut parent,
            &limits,
            &mut budget,
        );
        let TextShowEvent::Text(raised) = &extraction.events[0] else {
            panic!()
        };
        assert_eq!(raised.geometry.as_ref().unwrap().text_rise, 12.0);
        let TextShowEvent::XObject(invocation) = &extraction.events[1] else {
            panic!()
        };
        assert_eq!(invocation.inherited_text_rise, 5.0);
        let mut child = ContentTextState::for_form(invocation);
        let inherited = text_events(part(
            b"BT (form) Tj ET",
            2,
            &mut child,
            &limits,
            &mut budget,
        ));
        assert_eq!(inherited[0].geometry.as_ref().unwrap().text_rise, 5.0);
        let TextShowEvent::Text(restored) = &extraction.events[2] else {
            panic!()
        };
        assert_eq!(restored.geometry.as_ref().unwrap().text_rise, 5.0);
        parent.finish().unwrap();
        child.finish().unwrap();
    }

    #[test]
    fn parts_share_limits_and_fail_closed_on_split_tokens_unknown_state_or_overflow() {
        let limits = Limits {
            max_container_items: 3,
            ..Limits::default()
        };
        let mut budget = ParseBudget::new(&limits, binas_core::CancellationToken::new());
        let mut state = ContentTextState::new();
        part(b"q Q", 0, &mut state, &limits, &mut budget);
        assert_eq!(
            extract_text_show_part(b"q Q", 0, 1, &mut state, &limits, &mut budget)
                .err()
                .unwrap()
                .code,
            PdfErrorCode::ResourceLimit
        );
        let limits = Limits::default();
        for input in [
            b"BT (incomplete".as_slice(),
            b"/GS1 gs",
            b"1e308 0 0 1e308 0 0 cm 1e308 0 0 1e308 0 0 cm",
        ] {
            let mut state = ContentTextState::new();
            let mut budget = ParseBudget::new(&limits, binas_core::CancellationToken::new());
            let error = extract_text_show_part(input, 0, 0, &mut state, &limits, &mut budget)
                .err()
                .unwrap();
            if input == b"/GS1 gs" {
                assert_eq!(error.code, PdfErrorCode::UnsupportedFeature);
            }
            assert!(state.finish().is_err());
            assert!(
                extract_text_show_part(b"BT (later) Tj ET", 0, 1, &mut state, &limits, &mut budget)
                    .is_err()
            );
        }
        let token = binas_core::CancellationToken::new();
        let mut budget = ParseBudget::new(&limits, token.clone());
        let mut state = ContentTextState::new();
        part(b"BT", 0, &mut state, &limits, &mut budget);
        assert!(state.finish().is_err());
        token.cancel();
        assert_eq!(
            extract_text_show_part(b"ET", 0, 1, &mut state, &limits, &mut budget)
                .err()
                .unwrap()
                .code,
            PdfErrorCode::Cancelled
        );
    }

    #[test]
    fn extracts_all_text_show_operators_with_exact_string_spans() {
        let input =
            b"q BT /F#31 12 Tf (one) Tj [(two) -120 <7468726565>] TJ (four) ' 10 20 (five) \" ET Q";
        let items = extract_text_show(input, 100, &Limits::default()).unwrap();
        assert_eq!(
            items
                .iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>(),
            ["one", "two", "three", "four", "five"]
        );
        for item in &items {
            assert_eq!(item.font.as_deref(), Some(b"F1".as_slice()));
            let local = Span::new(item.span.start() - 100, item.span.end() - 100).unwrap();
            let encoded = local.slice(input).unwrap();
            assert_eq!(item.decoded_span.slice(input), Some(encoded));
            assert!(encoded.starts_with(b"(") || encoded.starts_with(b"<"));
        }
        assert_eq!(items[2].raw, b"three");
        assert_eq!(
            items
                .iter()
                .map(|item| item.operator.as_slice())
                .collect::<Vec<_>>(),
            [b"Tj".as_slice(), b"TJ", b"TJ", b"'", b"\""]
        );
    }

    #[test]
    fn decodes_literal_escapes_nesting_and_odd_hex() {
        let input = b"BT (a\\040\\(b\\)\\n(c)) Tj <4142F> Tj ET";
        let items = extract_text_show(input, 0, &Limits::default()).unwrap();
        assert_eq!(items[0].text, "a (b)\n(c)");
        assert_eq!(items[1].text, "AB�");
        assert_eq!(items[1].span.slice(input).unwrap(), b"<4142F>");
    }

    #[test]
    fn rejects_malformed_text_and_graphics_structure() {
        for input in [
            b"(x) Tj".as_slice(),
            b"BT BT ET".as_slice(),
            b"ET".as_slice(),
            b"Q".as_slice(),
            b"q".as_slice(),
            b"BT [(x) /bad] TJ ET".as_slice(),
        ] {
            assert_eq!(
                extract_text_show(input, 0, &Limits::default())
                    .unwrap_err()
                    .code,
                PdfErrorCode::InvalidSyntax
            );
        }
    }

    #[test]
    fn enforces_token_string_and_depth_budgets() {
        let tiny_tokens = Limits {
            max_container_items: 2,
            ..Limits::default()
        };
        assert_eq!(
            extract_text_show(b"BT (x) Tj ET", 0, &tiny_tokens)
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );

        let tiny_string = Limits {
            max_token_bytes: 2,
            ..Limits::default()
        };
        assert_eq!(
            extract_text_show(b"BT (long) Tj ET", 0, &tiny_string)
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );

        let tiny_depth = Limits {
            max_parser_depth: 1,
            ..Limits::default()
        };
        assert_eq!(
            extract_text_show(b"q q Q Q", 0, &tiny_depth)
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
    }

    #[test]
    fn parses_bounded_inline_images_and_continues_text_extraction() {
        let input = b"BI /W 1 /H 1 /BPC 8 /CS /G ID x EI BT (after) Tj ET";
        let images = inline_images(input, &Limits::default()).unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(&input[images[0].data_start..images[0].data_end], b"x");
        assert_eq!(
            extract_text_show(input, 0, &Limits::default()).unwrap()[0].text,
            "after"
        );

        assert_eq!(
            inline_images(
                b"BI /W 1 /H 1 /BPC 8 /CS /G /DP << >> ID x EI",
                &Limits::default()
            )
            .unwrap_err()
            .code,
            PdfErrorCode::UnsupportedFeature
        );
    }
}
