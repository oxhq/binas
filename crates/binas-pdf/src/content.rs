use binas_core::Span;

use crate::{
    error::PdfError,
    filters::{PdfFilter, decode_filter_chain},
    limits::Limits,
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
    pub text: String,
    pub span: Span,
    pub raw: Vec<u8>,
    pub decoded_span: Span,
    pub font: Option<Vec<u8>>,
    pub operator: Vec<u8>,
    pub geometry: Option<Box<TextItemGeometry>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TextItemGeometry {
    pub user_matrix: [f64; 6],
    pub text_matrix: Option<[f64; 6]>,
    pub font_size: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct XObjectInvocation {
    pub name: Vec<u8>,
    pub user_matrix: [f64; 6],
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
) -> Result<Vec<TextItem>, PdfError> {
    Ok(extract_text_show_with_xobjects(input, base_offset, limits)?
        .events
        .into_iter()
        .filter_map(|event| match event {
            TextShowEvent::Text(item) => Some(item),
            TextShowEvent::XObject(_) => None,
        })
        .collect())
}

pub(crate) fn extract_text_show_with_xobjects(
    input: &[u8],
    base_offset: usize,
    limits: &Limits,
) -> Result<TextShowExtraction, PdfError> {
    let mut lexer = Lexer::new(input, base_offset, limits);
    let mut operands = Vec::new();
    let mut events = Vec::new();
    let mut in_text = false;
    let mut graphics_depth = 0usize;
    let mut operations = 0usize;
    let mut active_font = None;
    let identity = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mut user_matrix = identity;
    let mut graphics_stack = Vec::new();
    let mut text_matrix = Some(identity);
    let mut text_line_matrix = identity;
    let mut font_size = None;
    let mut leading = 0.0;

    while let Some(token) = lexer.next(0)? {
        let Token::Operator(operator) = token else {
            operands.push(token);
            continue;
        };
        operations += 1;
        if operations > limits.max_container_items {
            return Err(PdfError::limit("content operation count exceeds limit"));
        }
        match operator.as_slice() {
            b"BT" => {
                require_operands(&operands, 0, "BT")?;
                if in_text {
                    return Err(PdfError::syntax("nested BT text object", lexer.pos));
                }
                in_text = true;
                text_matrix = Some(identity);
                text_line_matrix = identity;
            }
            b"ET" => {
                require_operands(&operands, 0, "ET")?;
                if !in_text {
                    return Err(PdfError::syntax("ET without BT", lexer.pos));
                }
                in_text = false;
            }
            b"q" => {
                require_operands(&operands, 0, "q")?;
                graphics_depth = graphics_depth
                    .checked_add(1)
                    .ok_or_else(|| PdfError::limit("graphics state depth overflows"))?;
                if graphics_depth > limits.max_parser_depth {
                    return Err(PdfError::limit("graphics state depth exceeds limit"));
                }
                graphics_stack.push(user_matrix);
            }
            b"Q" => {
                require_operands(&operands, 0, "Q")?;
                graphics_depth = graphics_depth
                    .checked_sub(1)
                    .ok_or_else(|| PdfError::syntax("Q without matching q", lexer.pos))?;
                user_matrix = graphics_stack
                    .pop()
                    .ok_or_else(|| PdfError::syntax("Q without saved graphics state", lexer.pos))?;
            }
            b"cm" => {
                require_operands(&operands, 6, "cm")?;
                user_matrix = multiply_matrix(number_matrix(&operands, "cm")?, user_matrix);
            }
            b"Do" => {
                require_operands(&operands, 1, "Do")?;
                let Token::Name(name) = &operands[0] else {
                    return Err(PdfError::syntax(
                        "Do resource operand is not a name",
                        lexer.pos,
                    ));
                };
                events.push(TextShowEvent::XObject(XObjectInvocation {
                    name: name.clone(),
                    user_matrix,
                }));
            }
            b"Tj" | b"'" => {
                require_text_object(in_text, &operator, lexer.pos)?;
                require_operands(&operands, 1, operator_name(&operator))?;
                if operator == b"'" {
                    text_line_matrix = translate_matrix(text_line_matrix, 0.0, -leading);
                    text_matrix = Some(text_line_matrix);
                }
                events.push(TextShowEvent::Text(
                    text_operand(&operands[0], lexer.pos, active_font.as_deref(), &operator)?
                        .with_geometry(user_matrix, text_matrix, font_size),
                ));
                text_matrix = None;
            }
            b"\"" => {
                require_text_object(in_text, &operator, lexer.pos)?;
                require_operands(&operands, 3, "\"")?;
                if !matches!(operands[0], Token::Number(_))
                    || !matches!(operands[1], Token::Number(_))
                {
                    return Err(PdfError::syntax(
                        "double-quote text operator requires two numeric operands",
                        lexer.pos,
                    ));
                }
                text_line_matrix = translate_matrix(text_line_matrix, 0.0, -leading);
                text_matrix = Some(text_line_matrix);
                events.push(TextShowEvent::Text(
                    text_operand(&operands[2], lexer.pos, active_font.as_deref(), &operator)?
                        .with_geometry(user_matrix, text_matrix, font_size),
                ));
                text_matrix = None;
            }
            b"TJ" => {
                require_text_object(in_text, &operator, lexer.pos)?;
                require_operands(&operands, 1, "TJ")?;
                let Token::Array(items) = &operands[0] else {
                    return Err(PdfError::syntax("TJ operand is not an array", lexer.pos));
                };
                for item in items {
                    match item {
                        Token::String(item) => {
                            events.push(TextShowEvent::Text(
                                item.clone()
                                    .with_context(active_font.as_deref(), &operator)
                                    .with_geometry(user_matrix, text_matrix, font_size),
                            ));
                            text_matrix = None;
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
                require_text_object(in_text, &operator, lexer.pos)?;
                require_operands(&operands, 2, "Tf")?;
                let Token::Name(name) = &operands[0] else {
                    return Err(PdfError::syntax(
                        "Tf font resource operand is not a name",
                        lexer.pos,
                    ));
                };
                if !matches!(operands[1], Token::Number(_)) {
                    return Err(PdfError::syntax(
                        "Tf font size operand is not a number",
                        lexer.pos,
                    ));
                }
                active_font = Some(name.clone());
                let Token::Number(size) = &operands[1] else {
                    unreachable!()
                };
                font_size = Some(*size);
            }
            b"Tm" => {
                require_text_object(in_text, &operator, lexer.pos)?;
                require_operands(&operands, 6, "Tm")?;
                let matrix = number_matrix(&operands, "Tm")?;
                text_line_matrix = matrix;
                text_matrix = Some(matrix);
            }
            b"Td" | b"TD" => {
                require_text_object(in_text, &operator, lexer.pos)?;
                require_operands(&operands, 2, operator_name(&operator))?;
                let [tx, ty] = two_numbers(&operands, operator_name(&operator))?;
                if operator == b"TD" {
                    leading = -ty;
                }
                text_line_matrix = translate_matrix(text_line_matrix, tx, ty);
                text_matrix = Some(text_line_matrix);
            }
            b"T*" => {
                require_text_object(in_text, &operator, lexer.pos)?;
                require_operands(&operands, 0, "T*")?;
                text_line_matrix = translate_matrix(text_line_matrix, 0.0, -leading);
                text_matrix = Some(text_line_matrix);
            }
            b"TL" => {
                require_text_object(in_text, &operator, lexer.pos)?;
                require_operands(&operands, 1, "TL")?;
                leading = one_number(&operands, "TL")?;
            }
            b"BI" => {
                require_operands(&operands, 0, "BI")?;
                let image = parse_inline_image(input, lexer.pos - 2, lexer.pos, lexer.limits)?;
                lexer.pos = image.end;
            }
            _ => {}
        }
        operands.clear();
    }
    if in_text {
        return Err(PdfError::syntax("unterminated BT text object", input.len()));
    }
    if graphics_depth != 0 {
        return Err(PdfError::syntax(
            "unbalanced q/Q graphics state",
            input.len(),
        ));
    }
    if !operands.is_empty() {
        return Err(PdfError::syntax(
            "content stream ends with unused operands",
            input.len(),
        ));
    }
    Ok(TextShowExtraction { events })
}

pub(crate) fn inline_images(input: &[u8], limits: &Limits) -> Result<Vec<InlineImage>, PdfError> {
    let mut lexer = Lexer::new(input, 0, limits);
    let mut images = Vec::new();
    while let Some(token) = lexer.next(0)? {
        if matches!(token, Token::Operator(ref operator) if operator == b"BI") {
            let image = parse_inline_image(input, lexer.pos - 2, lexer.pos, limits)?;
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
) -> Result<InlineImage, PdfError> {
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
    let data_end = inline_data_end(input, data_start, expected, filter, limits)?;
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
        InlineFilter::AsciiHex => input[start..]
            .iter()
            .position(|byte| *byte == b'>')
            .map(|offset| start + offset + 1)
            .ok_or_else(|| PdfError::syntax("inline ASCIIHex image has no terminator", start))
            .and_then(|end| validate_inline_decode(input, start, end, expected, filter, limits)),
        InlineFilter::Ascii85 => input[start..]
            .windows(2)
            .position(|bytes| bytes == b"~>")
            .map(|offset| start + offset + 2)
            .ok_or_else(|| PdfError::syntax("inline ASCII85 image has no terminator", start))
            .and_then(|end| validate_inline_decode(input, start, end, expected, filter, limits)),
        InlineFilter::RunLength => {
            let mut position = start;
            loop {
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
            validate_inline_decode(input, start, position, expected, filter, limits)
        }
        InlineFilter::Flate => {
            let mut candidates = Vec::new();
            for position in start..input.len().saturating_sub(2) {
                if is_ws(input[position])
                    && input.get(position + 1..position + 3) == Some(b"EI")
                    && input
                        .get(position + 3)
                        .is_none_or(|byte| is_delimiter(*byte))
                    && validate_inline_decode(input, start, position, expected, filter, limits)
                        .is_ok()
                {
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

fn validate_inline_decode(
    input: &[u8],
    start: usize,
    end: usize,
    expected: usize,
    filter: InlineFilter,
    limits: &Limits,
) -> Result<usize, PdfError> {
    let pdf_filter = match filter {
        InlineFilter::Flate => PdfFilter::FlateDecode,
        InlineFilter::AsciiHex => PdfFilter::ASCIIHexDecode,
        InlineFilter::Ascii85 => PdfFilter::ASCII85Decode,
        InlineFilter::RunLength => PdfFilter::RunLengthDecode,
        InlineFilter::Raw => return Ok(end),
    };
    let decoded = decode_filter_chain(&input[start..end], &[pdf_filter], &[None], expected)?;
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
            text: String::from_utf8_lossy(&value).into_owned(),
            span: Span::new(start as u64, end as u64)
                .map_err(|_| PdfError::limit("content source span is invalid"))?,
            raw: value,
            decoded_span,
            font: None,
            operator: Vec::new(),
            geometry: None,
        })
    }
}

impl TextItem {
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
    ) -> Self {
        self.geometry = Some(Box::new(TextItemGeometry {
            user_matrix,
            text_matrix,
            font_size,
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
    use crate::PdfErrorCode;

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
