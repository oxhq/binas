use std::collections::BTreeMap;

use crate::{error::PdfError, limits::Limits};

#[derive(Clone, Debug)]
struct CodeSpaceRange {
    start: Vec<u8>,
    end: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct ToUnicodeCMap {
    codespaces: Vec<CodeSpaceRange>,
    mappings: BTreeMap<Vec<u8>, String>,
}

impl ToUnicodeCMap {
    pub fn parse(input: &[u8], limits: &Limits) -> Result<Self, PdfError> {
        if input.len() > limits.max_input_bytes {
            return Err(PdfError::limit("CMap input exceeds max_input_bytes"));
        }
        let tokens = Lexer::new(input, limits).all()?;
        let mut cmap = Self {
            codespaces: Vec::new(),
            mappings: BTreeMap::new(),
        };
        let mut index = 0;

        while index < tokens.len() {
            let TokenKind::Word(operator) = &tokens[index].kind else {
                index += 1;
                continue;
            };
            let (end, count) = match operator.as_slice() {
                b"begincodespacerange" => (
                    b"endcodespacerange".as_slice(),
                    block_count(&tokens, index)?,
                ),
                b"beginbfchar" => (b"endbfchar".as_slice(), block_count(&tokens, index)?),
                b"beginbfrange" => (b"endbfrange".as_slice(), block_count(&tokens, index)?),
                b"endcodespacerange" | b"endbfchar" | b"endbfrange" => {
                    return Err(PdfError::syntax(
                        "unexpected CMap block terminator",
                        tokens[index].offset,
                    ));
                }
                _ => {
                    index += 1;
                    continue;
                }
            };
            if count > limits.max_container_items {
                return Err(PdfError::limit("CMap block item count exceeds limit"));
            }
            index += 1;
            match operator.as_slice() {
                b"begincodespacerange" => {
                    for _ in 0..count {
                        let start = hex_at(&tokens, index, "codespace start")?.to_vec();
                        let end_value = hex_at(&tokens, index + 1, "codespace end")?.to_vec();
                        cmap.add_codespace(start, end_value, tokens[index].offset)?;
                        index += 2;
                    }
                }
                b"beginbfchar" => {
                    for _ in 0..count {
                        let source = source_at(&tokens, index)?.to_vec();
                        let destination = decode_utf16be(
                            string_at(&tokens, index + 1, "bfchar destination")?,
                            tokens[index + 1].offset,
                        )?;
                        cmap.add_mapping(source, destination, tokens[index].offset, limits)?;
                        index += 2;
                    }
                }
                b"beginbfrange" => {
                    for _ in 0..count {
                        let start = source_at(&tokens, index)?.to_vec();
                        let end_value = source_at(&tokens, index + 1)?.to_vec();
                        let destination = tokens.get(index + 2).ok_or_else(|| {
                            PdfError::syntax("missing bfrange destination", input.len())
                        })?;
                        cmap.add_range(
                            &start,
                            &end_value,
                            destination,
                            tokens[index].offset,
                            limits,
                        )?;
                        index += 3;
                    }
                }
                _ => unreachable!(),
            }
            let terminator = tokens
                .get(index)
                .ok_or_else(|| PdfError::syntax("unterminated CMap block", input.len()))?;
            if !terminator.is_word(end) {
                return Err(PdfError::syntax(
                    "CMap block count does not match its contents",
                    terminator.offset,
                ));
            }
            index += 1;
        }

        for source in cmap.mappings.keys() {
            if !cmap.in_codespace(source) {
                return Err(PdfError::syntax(
                    "CMap source mapping is outside declared codespace ranges",
                    0,
                ));
            }
        }
        Ok(cmap)
    }

    pub fn mapping(&self, source: &[u8]) -> Option<&str> {
        self.mappings.get(source).map(String::as_str)
    }

    pub(crate) fn mappings(&self) -> impl Iterator<Item = (&[u8], &str)> {
        self.mappings
            .iter()
            .map(|(source, destination)| (source.as_slice(), destination.as_str()))
    }

    pub(crate) fn longest_code_len(&self, input: &[u8]) -> Option<usize> {
        (1..=4.min(input.len()))
            .rev()
            .find(|&width| self.in_codespace(&input[..width]))
    }

    fn in_codespace(&self, source: &[u8]) -> bool {
        self.codespaces.iter().any(|range| {
            range.start.len() == source.len()
                && range.start.as_slice() <= source
                && source <= range.end.as_slice()
        })
    }

    fn add_codespace(
        &mut self,
        start: Vec<u8>,
        end: Vec<u8>,
        offset: usize,
    ) -> Result<(), PdfError> {
        validate_source_range(&start, &end, offset)?;
        if self.codespaces.iter().any(|range| {
            range.start.len() == start.len()
                && start.as_slice() <= range.end.as_slice()
                && range.start.as_slice() <= end.as_slice()
        }) {
            return Err(PdfError::syntax(
                "duplicate or overlapping CMap codespace range",
                offset,
            ));
        }
        self.codespaces.push(CodeSpaceRange { start, end });
        Ok(())
    }

    fn add_mapping(
        &mut self,
        source: Vec<u8>,
        destination: String,
        offset: usize,
        limits: &Limits,
    ) -> Result<(), PdfError> {
        validate_source(&source, offset)?;
        if self.mappings.len() >= limits.max_container_items {
            return Err(PdfError::limit("CMap mapping count exceeds limit"));
        }
        if self.mappings.insert(source, destination).is_some() {
            return Err(PdfError::syntax("duplicate CMap source mapping", offset));
        }
        Ok(())
    }

    fn add_range(
        &mut self,
        start: &[u8],
        end: &[u8],
        destination: &Token,
        offset: usize,
        limits: &Limits,
    ) -> Result<(), PdfError> {
        validate_source_range(start, end, offset)?;
        let first = bytes_to_u32(start);
        let last = bytes_to_u32(end);
        let count = usize::try_from(u64::from(last) - u64::from(first) + 1)
            .map_err(|_| PdfError::limit("CMap source range size overflows"))?;
        let total_mappings = self
            .mappings
            .len()
            .checked_add(count)
            .ok_or_else(|| PdfError::limit("CMap expanded mapping count overflows"))?;
        if count > limits.max_container_items || total_mappings > limits.max_container_items {
            return Err(PdfError::limit("CMap expanded mapping count exceeds limit"));
        }

        match &destination.kind {
            TokenKind::Hex(initial) | TokenKind::Literal(initial) => {
                let mut value = initial.clone();
                for delta in 0..count {
                    if delta != 0 && !increment_be(&mut value) {
                        return Err(PdfError::syntax(
                            "CMap sequential destination range overflows",
                            destination.offset,
                        ));
                    }
                    let source = range_source(first, delta, start.len())?;
                    let decoded = decode_utf16be(&value, destination.offset)?;
                    self.add_mapping(source, decoded, offset, limits)?;
                }
            }
            TokenKind::Array(values) => {
                if values.len() != count {
                    return Err(PdfError::syntax(
                        "CMap destination array length does not match source range",
                        destination.offset,
                    ));
                }
                for (delta, value) in values.iter().enumerate() {
                    let encoded = match &value.kind {
                        TokenKind::Hex(encoded) | TokenKind::Literal(encoded) => encoded,
                        _ => {
                            return Err(PdfError::syntax(
                                "CMap destination array contains a non-string value",
                                value.offset,
                            ));
                        }
                    };
                    let source = range_source(first, delta, start.len())?;
                    let decoded = decode_utf16be(encoded, value.offset)?;
                    self.add_mapping(source, decoded, offset, limits)?;
                }
            }
            _ => {
                return Err(PdfError::syntax(
                    "bfrange destination is not a string or array",
                    destination.offset,
                ));
            }
        }
        Ok(())
    }
}

fn validate_source(source: &[u8], offset: usize) -> Result<(), PdfError> {
    if (1..=4).contains(&source.len()) {
        Ok(())
    } else {
        Err(PdfError::syntax(
            "CMap source code width must be between 1 and 4 bytes",
            offset,
        ))
    }
}

fn validate_source_range(start: &[u8], end: &[u8], offset: usize) -> Result<(), PdfError> {
    validate_source(start, offset)?;
    if start.len() != end.len() {
        return Err(PdfError::syntax(
            "CMap source range widths do not match",
            offset,
        ));
    }
    if start > end {
        return Err(PdfError::syntax("CMap source range is reversed", offset));
    }
    Ok(())
}

fn decode_utf16be(input: &[u8], offset: usize) -> Result<String, PdfError> {
    if input.is_empty() || !input.len().is_multiple_of(2) {
        return Err(PdfError::syntax(
            "CMap destination is not valid UTF-16BE",
            offset,
        ));
    }
    char::decode_utf16(
        input
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]])),
    )
    .collect::<Result<String, _>>()
    .map_err(|_| PdfError::syntax("CMap destination has an unpaired UTF-16 surrogate", offset))
}

fn bytes_to_u32(value: &[u8]) -> u32 {
    value
        .iter()
        .fold(0, |result, byte| (result << 8) | u32::from(*byte))
}

fn u32_to_bytes(value: u32, width: usize) -> Vec<u8> {
    value.to_be_bytes()[4 - width..].to_vec()
}

fn range_source(first: u32, delta: usize, width: usize) -> Result<Vec<u8>, PdfError> {
    let delta = u32::try_from(delta).map_err(|_| PdfError::limit("CMap range index overflows"))?;
    let value = first
        .checked_add(delta)
        .ok_or_else(|| PdfError::limit("CMap source range overflows"))?;
    Ok(u32_to_bytes(value, width))
}

fn increment_be(value: &mut [u8]) -> bool {
    for byte in value.iter_mut().rev() {
        let (next, overflow) = byte.overflowing_add(1);
        *byte = next;
        if !overflow {
            return true;
        }
    }
    false
}

fn block_count(tokens: &[Token], operator: usize) -> Result<usize, PdfError> {
    let count = operator
        .checked_sub(1)
        .and_then(|index| tokens.get(index))
        .ok_or_else(|| {
            PdfError::syntax("CMap block is missing its count", tokens[operator].offset)
        })?;
    let TokenKind::Word(value) = &count.kind else {
        return Err(PdfError::syntax(
            "CMap block count is not an integer",
            count.offset,
        ));
    };
    if value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
        return Err(PdfError::syntax(
            "CMap block count is not a non-negative integer",
            count.offset,
        ));
    }
    value.iter().try_fold(0usize, |result, digit| {
        result
            .checked_mul(10)
            .and_then(|result| result.checked_add(usize::from(digit - b'0')))
            .ok_or_else(|| PdfError::limit("CMap block count overflows"))
    })
}

fn source_at(tokens: &[Token], index: usize) -> Result<&[u8], PdfError> {
    let source = hex_at(tokens, index, "CMap source")?;
    validate_source(source, tokens[index].offset)?;
    Ok(source)
}

fn hex_at<'a>(tokens: &'a [Token], index: usize, label: &str) -> Result<&'a [u8], PdfError> {
    let token = tokens
        .get(index)
        .ok_or_else(|| PdfError::syntax(format!("missing {label}"), 0))?;
    match &token.kind {
        TokenKind::Hex(value) => Ok(value),
        _ => Err(PdfError::syntax(
            format!("{label} is not a hex string"),
            token.offset,
        )),
    }
}

fn string_at<'a>(tokens: &'a [Token], index: usize, label: &str) -> Result<&'a [u8], PdfError> {
    let token = tokens
        .get(index)
        .ok_or_else(|| PdfError::syntax(format!("missing {label}"), 0))?;
    match &token.kind {
        TokenKind::Hex(value) | TokenKind::Literal(value) => Ok(value),
        _ => Err(PdfError::syntax(
            format!("{label} is not a string"),
            token.offset,
        )),
    }
}

#[derive(Clone, Debug)]
struct Token {
    kind: TokenKind,
    offset: usize,
}

impl Token {
    fn is_word(&self, expected: &[u8]) -> bool {
        matches!(&self.kind, TokenKind::Word(value) if value == expected)
    }
}

#[derive(Clone, Debug)]
enum TokenKind {
    Hex(Vec<u8>),
    Literal(Vec<u8>),
    Array(Vec<Token>),
    Word(Vec<u8>),
}

struct Lexer<'a> {
    input: &'a [u8],
    limits: &'a Limits,
    pos: usize,
    tokens: usize,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a [u8], limits: &'a Limits) -> Self {
        Self {
            input,
            limits,
            pos: 0,
            tokens: 0,
        }
    }

    fn all(mut self) -> Result<Vec<Token>, PdfError> {
        let mut output = Vec::new();
        while let Some(token) = self.next(0)? {
            output.push(token);
        }
        Ok(output)
    }

    fn next(&mut self, depth: usize) -> Result<Option<Token>, PdfError> {
        self.skip_ws_and_comments();
        if self.pos == self.input.len() {
            return Ok(None);
        }
        if depth > self.limits.max_parser_depth {
            return Err(PdfError::limit("CMap nesting depth exceeds limit"));
        }
        self.tokens = self
            .tokens
            .checked_add(1)
            .ok_or_else(|| PdfError::limit("CMap token count overflows"))?;
        if self.tokens > self.limits.max_container_items {
            return Err(PdfError::limit("CMap token count exceeds limit"));
        }
        let offset = self.pos;
        let kind = match self.input[self.pos] {
            b'<' if self.input.get(self.pos + 1) != Some(&b'<') => {
                TokenKind::Hex(self.hex_string()?)
            }
            b'[' => TokenKind::Array(self.array(depth + 1)?),
            b']' => {
                return Err(PdfError::syntax(
                    "unexpected CMap array terminator",
                    self.pos,
                ));
            }
            b'(' => TokenKind::Literal(self.literal_string(depth)?),
            _ => TokenKind::Word(self.word()?),
        };
        Ok(Some(Token { kind, offset }))
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

    fn word(&mut self) -> Result<Vec<u8>, PdfError> {
        let start = self.pos;
        if matches!(self.input.get(self.pos..self.pos + 2), Some(b"<<" | b">>")) {
            self.pos += 2;
        } else {
            while self.input.get(self.pos).is_some_and(|byte| {
                !is_ws(*byte) && !matches!(*byte, b'<' | b'[' | b']' | b'(' | b'%')
            }) {
                self.pos += 1;
            }
        }
        if self.pos == start {
            return Err(PdfError::syntax("empty CMap token", self.pos));
        }
        if self.pos - start > self.limits.max_token_bytes {
            return Err(PdfError::limit("CMap token exceeds max_token_bytes"));
        }
        Ok(self.input[start..self.pos].to_vec())
    }

    fn hex_string(&mut self) -> Result<Vec<u8>, PdfError> {
        let start = self.pos;
        self.pos += 1;
        let mut digits = Vec::new();
        loop {
            let byte = *self
                .input
                .get(self.pos)
                .ok_or_else(|| PdfError::syntax("unterminated CMap hex string", start))?;
            self.pos += 1;
            if byte == b'>' {
                break;
            }
            if is_ws(byte) {
                continue;
            }
            if !byte.is_ascii_hexdigit() {
                return Err(PdfError::syntax(
                    "invalid digit in CMap hex string",
                    self.pos - 1,
                ));
            }
            digits.push(byte);
            if digits.len() > self.limits.max_token_bytes.saturating_mul(2) {
                return Err(PdfError::limit("CMap hex string exceeds max_token_bytes"));
            }
        }
        if digits.len() % 2 != 0 {
            digits.push(b'0');
        }
        digits
            .chunks_exact(2)
            .map(|pair| {
                let high = hex_value(pair[0]);
                let low = hex_value(pair[1]);
                Ok((high << 4) | low)
            })
            .collect()
    }

    fn array(&mut self, depth: usize) -> Result<Vec<Token>, PdfError> {
        if depth > self.limits.max_parser_depth {
            return Err(PdfError::limit("CMap nesting depth exceeds limit"));
        }
        self.pos += 1;
        let mut values = Vec::new();
        loop {
            self.skip_ws_and_comments();
            if self.input.get(self.pos) == Some(&b']') {
                self.pos += 1;
                return Ok(values);
            }
            if self.pos == self.input.len() {
                return Err(PdfError::syntax("unterminated CMap array", self.pos));
            }
            let token = self
                .next(depth)?
                .ok_or_else(|| PdfError::syntax("unterminated CMap array", self.pos))?;
            values.push(token);
        }
    }

    fn literal_string(&mut self, base_depth: usize) -> Result<Vec<u8>, PdfError> {
        if base_depth > self.limits.max_parser_depth {
            return Err(PdfError::limit("CMap nesting depth exceeds limit"));
        }
        let start = self.pos;
        self.pos += 1;
        let mut output = Vec::new();
        let mut literal_depth = 1usize;
        while let Some(&byte) = self.input.get(self.pos) {
            self.pos += 1;
            match byte {
                b'\\' => {
                    let escaped = *self.input.get(self.pos).ok_or_else(|| {
                        PdfError::syntax("unterminated CMap literal escape", self.pos)
                    })?;
                    self.pos += 1;
                    match escaped {
                        b'n' => output.push(b'\n'),
                        b'r' => output.push(b'\r'),
                        b't' => output.push(b'\t'),
                        b'b' => output.push(0x08),
                        b'f' => output.push(0x0c),
                        b'\r' => {
                            if self.input.get(self.pos) == Some(&b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        b'0'..=b'7' => {
                            let mut value = u16::from(escaped - b'0');
                            for _ in 0..2 {
                                let Some(next @ b'0'..=b'7') = self.input.get(self.pos).copied()
                                else {
                                    break;
                                };
                                self.pos += 1;
                                value = (value << 3) | u16::from(next - b'0');
                            }
                            output.push(value as u8);
                        }
                        value => output.push(value),
                    }
                }
                b'(' => {
                    literal_depth = literal_depth
                        .checked_add(1)
                        .ok_or_else(|| PdfError::limit("CMap nesting depth overflows"))?;
                    let total_depth = base_depth
                        .checked_add(literal_depth - 1)
                        .ok_or_else(|| PdfError::limit("CMap nesting depth overflows"))?;
                    if total_depth > self.limits.max_parser_depth {
                        return Err(PdfError::limit("CMap nesting depth exceeds limit"));
                    }
                    output.push(byte);
                }
                b')' => {
                    literal_depth -= 1;
                    if literal_depth == 0 {
                        return Ok(output);
                    }
                    output.push(byte);
                }
                value => output.push(value),
            }
            if output.len() > self.limits.max_token_bytes {
                return Err(PdfError::limit("CMap token exceeds max_token_bytes"));
            }
        }
        Err(PdfError::syntax("unterminated CMap literal string", start))
    }
}

fn is_ws(byte: u8) -> bool {
    matches!(byte, 0 | b'\t' | b'\n' | 0x0c | b'\r' | b' ')
}

fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::PdfErrorCode;

    fn parse(body: &str) -> Result<ToUnicodeCMap, PdfError> {
        ToUnicodeCMap::parse(body.as_bytes(), &Limits::default())
    }

    #[test]
    fn parses_one_to_four_byte_codes_and_utf16_surrogates() {
        let cmap = parse(
            "4 begincodespacerange\n<00> <7f>\n<8100> <81ff>\n<820000> <82ffff>\n<83000000> <83ffffff>\nendcodespacerange\n\
             4 beginbfchar\n<41> <0041>\n<8101> <03a9>\n<820102> <d83dde00>\n<83010203> <4e2d>\nendbfchar",
        )
        .unwrap();

        assert_eq!(cmap.mapping(&[0x41]), Some("A"));
        assert_eq!(cmap.mapping(&[0x81, 1]), Some("Ω"));
        assert_eq!(cmap.mapping(&[0x82, 1, 2]), Some("😀"));
        assert_eq!(cmap.mapping(&[0x83, 1, 2, 3]), Some("中"));
    }

    #[test]
    fn expands_sequential_and_array_ranges() {
        let cmap = parse(
            "2 begincodespacerange <20> <22> <30> <32> endcodespacerange\n\
             2 beginbfrange <20> <22> <0041> <30> <32> [<0061> <0062> <d83dde00>] endbfrange",
        )
        .unwrap();

        assert_eq!(cmap.mapping(&[0x20]), Some("A"));
        assert_eq!(cmap.mapping(&[0x22]), Some("C"));
        assert_eq!(cmap.mapping(&[0x30]), Some("a"));
        assert_eq!(cmap.mapping(&[0x32]), Some("😀"));
    }

    #[test]
    fn decodes_literal_string_destinations() {
        let cmap = parse(
            r"1 begincodespacerange <0001> <0003> endcodespacerange
              1 beginbfchar <0001> (\000f\000i) endbfchar
              1 beginbfrange <0002> <0003> [(\000X) <0059>] endbfrange",
        )
        .unwrap();

        assert_eq!(cmap.mapping(&[0, 1]), Some("fi"));
        assert_eq!(cmap.mapping(&[0, 2]), Some("X"));
        assert_eq!(cmap.mapping(&[0, 3]), Some("Y"));
    }

    #[test]
    fn rejects_invalid_ranges_duplicates_and_utf16() {
        assert!(parse("1 begincodespacerange <00> <ffff> endcodespacerange").is_err());
        assert!(parse("1 begincodespacerange <ff> <00> endcodespacerange").is_err());
        assert!(parse(
            "1 begincodespacerange <00> <ff> endcodespacerange 2 beginbfchar <01> <0041> <01> <0042> endbfchar"
        )
        .is_err());
        assert!(parse(
            "1 begincodespacerange <00> <ff> endcodespacerange 1 beginbfchar <01> <d800> endbfchar"
        )
        .is_err());
        assert!(parse(
            "1 begincodespacerange <00> <ff> endcodespacerange 1 beginbfchar <01> <00> endbfchar"
        )
        .is_err());
    }

    #[test]
    fn enforces_token_container_and_depth_budgets() {
        let limits = Limits {
            max_token_bytes: 4,
            ..Limits::default()
        };
        let error = ToUnicodeCMap::parse(
            b"1 begincodespacerange <00> <ff> endcodespacerange",
            &limits,
        )
        .unwrap_err();
        assert_eq!(error.code, PdfErrorCode::ResourceLimit);

        let limits = Limits {
            max_container_items: 3,
            ..Limits::default()
        };
        let error =
            ToUnicodeCMap::parse(b"1 beginbfchar <01> <0041> endbfchar", &limits).unwrap_err();
        assert_eq!(error.code, PdfErrorCode::ResourceLimit);

        let limits = Limits {
            max_parser_depth: 1,
            ..Limits::default()
        };
        let error = ToUnicodeCMap::parse(b"[[<0041>]]", &limits).unwrap_err();
        assert_eq!(error.code, PdfErrorCode::ResourceLimit);
    }
}
