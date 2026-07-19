use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
};

use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};

use crate::{PdfError, limits::Limits, parser::Value};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfFilter {
    FlateDecode,
    ASCIIHexDecode,
    ASCII85Decode,
    RunLengthDecode,
    LzwDecode,
    Unsupported(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeParams {
    pub predictor: u8,
    pub colors: usize,
    pub bits_per_component: u8,
    pub columns: usize,
    pub early_change: u8,
}

impl Default for DecodeParams {
    fn default() -> Self {
        Self {
            predictor: 1,
            colors: 1,
            bits_per_component: 8,
            columns: 1,
            early_change: 1,
        }
    }
}

pub fn decode_filter_chain(
    input: &[u8],
    filters: &[PdfFilter],
    decode_params: &[Option<DecodeParams>],
    max_output: usize,
) -> Result<Vec<u8>, PdfError> {
    if filters.len() != decode_params.len() {
        return Err(PdfError::syntax(
            "Filter and DecodeParms arrays must have equal lengths",
            0,
        ));
    }

    if filters.is_empty() {
        return bounded_copy(input, max_output);
    }
    let mut output = input.to_vec();
    for (filter, params) in filters.iter().zip(decode_params) {
        let has_params = params.is_some();
        let params = params.unwrap_or_default();
        validate_params(filter, params, has_params)?;
        output = match filter {
            PdfFilter::FlateDecode => decode_flate(&output, max_output)?,
            PdfFilter::ASCIIHexDecode => decode_ascii_hex(&output, max_output)?,
            PdfFilter::ASCII85Decode => decode_ascii85(&output, max_output)?,
            PdfFilter::RunLengthDecode => decode_run_length(&output, max_output)?,
            PdfFilter::LzwDecode => decode_lzw(&output, params.early_change, max_output)?,
            PdfFilter::Unsupported(name) => {
                return Err(PdfError::unsupported(format!(
                    "unsupported PDF stream filter /{name}"
                )));
            }
        };
        if matches!(filter, PdfFilter::FlateDecode | PdfFilter::LzwDecode) {
            output = decode_predictor(output, params, max_output)?;
        }
    }
    Ok(output)
}

pub(crate) fn encode_pdf_stream(
    value: &Value,
    decoded: &[u8],
    limits: &Limits,
) -> Result<Vec<u8>, PdfError> {
    if decoded.len() > limits.max_stream_bytes {
        return Err(PdfError::limit("decoded stream exceeds max_stream_bytes"));
    }
    let filters = parse_filters(value, limits.max_container_items)?;
    let decode_params = parse_decode_params_list(value, filters.len(), limits.max_container_items)?;
    if filters.len() != decode_params.len() {
        return Err(PdfError::syntax(
            "Filter and DecodeParms arrays must have equal lengths",
            0,
        ));
    }
    let mut output = decoded.to_vec();
    for (filter, params) in filters.iter().zip(&decode_params).rev() {
        let has_params = params.is_some();
        let params = params.unwrap_or_default();
        validate_params(filter, params, has_params)?;
        if matches!(filter, PdfFilter::FlateDecode | PdfFilter::LzwDecode) {
            output = encode_predictor(output, params, limits.max_stream_bytes)?;
        }
        output = match filter {
            PdfFilter::FlateDecode => encode_flate(&output, limits.max_stream_bytes)?,
            PdfFilter::ASCIIHexDecode => encode_ascii_hex(&output, limits.max_stream_bytes)?,
            PdfFilter::ASCII85Decode => encode_ascii85(&output, limits.max_stream_bytes)?,
            PdfFilter::RunLengthDecode => encode_run_length(&output, limits.max_stream_bytes)?,
            PdfFilter::LzwDecode => {
                encode_lzw(&output, params.early_change, limits.max_stream_bytes)?
            }
            PdfFilter::Unsupported(name) => {
                return Err(PdfError::unsupported(format!(
                    "stream encoding for /{name} is not implemented"
                )));
            }
        };
    }
    Ok(output)
}

pub(crate) fn encode_flate(input: &[u8], max_output: usize) -> Result<Vec<u8>, PdfError> {
    let mut encoder = ZlibEncoder::new(BoundedWriter::new(max_output), Compression::default());
    encoder
        .write_all(input)
        .map_err(|error| PdfError::limit(format!("Flate encode failed: {error}")))?;
    let output = encoder
        .finish()
        .map_err(|error| PdfError::limit(format!("Flate encode failed: {error}")))?;
    Ok(output.bytes)
}

fn parse_filters(value: &Value, max_items: usize) -> Result<Vec<PdfFilter>, PdfError> {
    match dict_get(value, b"Filter") {
        None => Ok(Vec::new()),
        Some(Value::Name(name)) => Ok(vec![pdf_filter(name)]),
        Some(Value::Array(values)) => {
            if values.len() > max_items {
                return Err(PdfError::limit("stream filter count exceeds limit"));
            }
            values
                .iter()
                .map(|value| match value {
                    Value::Name(name) => Ok(pdf_filter(name)),
                    _ => Err(PdfError::syntax(
                        "stream /Filter array contains a non-name",
                        0,
                    )),
                })
                .collect()
        }
        Some(_) => Err(PdfError::syntax(
            "stream /Filter must be a name or array",
            0,
        )),
    }
}

fn parse_decode_params_list(
    value: &Value,
    filter_count: usize,
    max_items: usize,
) -> Result<Vec<Option<DecodeParams>>, PdfError> {
    match dict_get(value, b"DecodeParms") {
        None | Some(Value::Null) => Ok(vec![None; filter_count]),
        Some(params @ Value::Dict(_)) => Ok(vec![Some(parse_decode_params(params)?)]),
        Some(Value::Array(values)) => {
            if values.len() > max_items {
                return Err(PdfError::limit("stream DecodeParms count exceeds limit"));
            }
            values
                .iter()
                .map(|value| match value {
                    Value::Null => Ok(None),
                    Value::Dict(_) => parse_decode_params(value).map(Some),
                    _ => Err(PdfError::syntax(
                        "stream /DecodeParms array contains an invalid value",
                        0,
                    )),
                })
                .collect()
        }
        Some(_) => Err(PdfError::syntax(
            "stream /DecodeParms must be a dictionary or array",
            0,
        )),
    }
}

fn parse_decode_params(value: &Value) -> Result<DecodeParams, PdfError> {
    let mut params = DecodeParams::default();
    if let Some(value) = dict_integer(value, b"Predictor") {
        params.predictor =
            u8::try_from(value).map_err(|_| PdfError::syntax("/Predictor exceeds u8", 0))?;
    }
    if let Some(value) = dict_integer(value, b"Colors") {
        params.colors = usize::try_from(value)
            .map_err(|_| PdfError::syntax("/Colors must be non-negative", 0))?;
    }
    if let Some(value) = dict_integer(value, b"BitsPerComponent") {
        params.bits_per_component =
            u8::try_from(value).map_err(|_| PdfError::syntax("/BitsPerComponent exceeds u8", 0))?;
    }
    if let Some(value) = dict_integer(value, b"Columns") {
        params.columns = usize::try_from(value)
            .map_err(|_| PdfError::syntax("/Columns must be non-negative", 0))?;
    }
    if let Some(value) = dict_integer(value, b"EarlyChange") {
        params.early_change =
            u8::try_from(value).map_err(|_| PdfError::syntax("/EarlyChange exceeds u8", 0))?;
    }
    Ok(params)
}

fn pdf_filter(name: &[u8]) -> PdfFilter {
    match name {
        b"FlateDecode" => PdfFilter::FlateDecode,
        b"ASCIIHexDecode" => PdfFilter::ASCIIHexDecode,
        b"ASCII85Decode" => PdfFilter::ASCII85Decode,
        b"RunLengthDecode" => PdfFilter::RunLengthDecode,
        b"LZWDecode" => PdfFilter::LzwDecode,
        name => PdfFilter::Unsupported(String::from_utf8_lossy(name).into_owned()),
    }
}

fn dict_get<'a>(value: &'a Value, key: &[u8]) -> Option<&'a Value> {
    let Value::Dict(dictionary) = value else {
        return None;
    };
    dictionary.get(key)
}

fn dict_integer(value: &Value, key: &[u8]) -> Option<i64> {
    match dict_get(value, key)? {
        Value::Integer(value) => Some(*value),
        _ => None,
    }
}

struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
}

impl BoundedWriter {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
        }
    }
}

impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self
            .bytes
            .len()
            .checked_add(bytes.len())
            .is_none_or(|length| length > self.limit)
        {
            return Err(io::Error::other("encoded stream exceeds max_stream_bytes"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn validate_params(
    filter: &PdfFilter,
    params: DecodeParams,
    has_params: bool,
) -> Result<(), PdfError> {
    if has_params && !matches!(filter, PdfFilter::FlateDecode | PdfFilter::LzwDecode) {
        return Err(PdfError::unsupported(
            "DecodeParms are only supported for FlateDecode and LZWDecode",
        ));
    }
    if params.early_change > 1 {
        return Err(PdfError::syntax("EarlyChange must be 0 or 1", 0));
    }
    if matches!(filter, PdfFilter::FlateDecode) && params.early_change != 1 {
        return Err(PdfError::unsupported(
            "EarlyChange is only supported for LZWDecode",
        ));
    }
    if !matches!(params.predictor, 1 | 2 | 10..=15) {
        return Err(PdfError::unsupported(format!(
            "unsupported predictor {}",
            params.predictor
        )));
    }
    if params.colors == 0 || params.columns == 0 {
        return Err(PdfError::syntax(
            "predictor Colors and Columns must be positive",
            0,
        ));
    }
    if !(1..=32).contains(&params.bits_per_component) {
        return Err(PdfError::syntax(
            "predictor BitsPerComponent must be between 1 and 32",
            0,
        ));
    }
    predictor_geometry(params)?;
    Ok(())
}

fn decode_flate(input: &[u8], max_output: usize) -> Result<Vec<u8>, PdfError> {
    let mut reader = ZlibDecoder::new(input);
    let mut output = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let read = reader
            .read(&mut chunk)
            .map_err(|error| PdfError::syntax(format!("invalid FlateDecode stream: {error}"), 0))?;
        if read == 0 {
            return Ok(output);
        }
        extend_bounded(&mut output, &chunk[..read], max_output)?;
    }
}

fn encode_ascii_hex(input: &[u8], max_output: usize) -> Result<Vec<u8>, PdfError> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let length = input
        .len()
        .checked_mul(2)
        .and_then(|length| length.checked_add(1))
        .ok_or_else(|| PdfError::limit("ASCIIHex encoded length overflows"))?;
    if length > max_output {
        return Err(PdfError::limit("encoded stream exceeds max_stream_bytes"));
    }
    let mut output = Vec::with_capacity(length);
    for &byte in input {
        output.push(HEX[usize::from(byte >> 4)]);
        output.push(HEX[usize::from(byte & 15)]);
    }
    output.push(b'>');
    Ok(output)
}

fn encode_ascii85(input: &[u8], max_output: usize) -> Result<Vec<u8>, PdfError> {
    let mut output = Vec::new();
    let mut chunks = input.chunks_exact(4);
    for chunk in &mut chunks {
        let value = u32::from_be_bytes(chunk.try_into().unwrap());
        if value == 0 {
            push_bounded(&mut output, b'z', max_output)?;
        } else {
            append_ascii85_value(&mut output, value, 5, max_output)?;
        }
    }
    let remainder = chunks.remainder();
    if !remainder.is_empty() {
        let mut padded = [0_u8; 4];
        padded[..remainder.len()].copy_from_slice(remainder);
        append_ascii85_value(
            &mut output,
            u32::from_be_bytes(padded),
            remainder.len() + 1,
            max_output,
        )?;
    }
    extend_bounded(&mut output, b"~>", max_output)?;
    Ok(output)
}

fn append_ascii85_value(
    output: &mut Vec<u8>,
    mut value: u32,
    take: usize,
    max_output: usize,
) -> Result<(), PdfError> {
    let mut encoded = [0_u8; 5];
    for byte in encoded.iter_mut().rev() {
        *byte = (value % 85) as u8 + b'!';
        value /= 85;
    }
    extend_bounded(output, &encoded[..take], max_output)
}

fn encode_run_length(input: &[u8], max_output: usize) -> Result<Vec<u8>, PdfError> {
    let mut output = Vec::new();
    for chunk in input.chunks(128) {
        push_bounded(&mut output, (chunk.len() - 1) as u8, max_output)?;
        extend_bounded(&mut output, chunk, max_output)?;
    }
    push_bounded(&mut output, 128, max_output)?;
    Ok(output)
}

fn encode_lzw(input: &[u8], early_change: u8, max_output: usize) -> Result<Vec<u8>, PdfError> {
    let mut writer = BitWriter::new(max_output);
    let mut dictionary = initial_lzw_encoder_dictionary();
    let mut next_code = 258_u16;
    let mut state = LzwWriteState::new(early_change);
    writer.write(256, state.width)?;
    if input.is_empty() {
        writer.write(257, state.width)?;
        return writer.finish();
    }

    let mut current = vec![input[0]];
    for &byte in &input[1..] {
        let mut candidate = current.clone();
        candidate.push(byte);
        if dictionary.contains_key(&candidate) {
            current = candidate;
            continue;
        }
        writer.write(dictionary[&current], state.width)?;
        state.after_data_code();
        if next_code < 4096 {
            dictionary.insert(candidate, next_code);
            next_code += 1;
        } else {
            writer.write(256, state.width)?;
            state.reset();
            dictionary = initial_lzw_encoder_dictionary();
            next_code = 258;
        }
        current.clear();
        current.push(byte);
    }
    writer.write(dictionary[&current], state.width)?;
    state.after_data_code();
    writer.write(257, state.width)?;
    writer.finish()
}

fn initial_lzw_encoder_dictionary() -> BTreeMap<Vec<u8>, u16> {
    (0_u16..=255)
        .map(|value| (vec![value as u8], value))
        .collect()
}

struct LzwWriteState {
    width: u8,
    next_code: usize,
    has_previous: bool,
    early_change: u8,
}

impl LzwWriteState {
    fn new(early_change: u8) -> Self {
        Self {
            width: 9,
            next_code: 258,
            has_previous: false,
            early_change,
        }
    }

    fn after_data_code(&mut self) {
        if self.has_previous && self.next_code < 4096 {
            self.next_code += 1;
            if self.width < 12
                && self.next_code >= (1_usize << self.width) - usize::from(self.early_change)
            {
                self.width += 1;
            }
        }
        self.has_previous = true;
    }

    fn reset(&mut self) {
        self.width = 9;
        self.next_code = 258;
        self.has_previous = false;
    }
}

struct BitWriter {
    output: Vec<u8>,
    pending: u32,
    pending_bits: u8,
    limit: usize,
}

impl BitWriter {
    fn new(limit: usize) -> Self {
        Self {
            output: Vec::new(),
            pending: 0,
            pending_bits: 0,
            limit,
        }
    }

    fn write(&mut self, code: u16, width: u8) -> Result<(), PdfError> {
        self.pending = (self.pending << width) | u32::from(code);
        self.pending_bits += width;
        while self.pending_bits >= 8 {
            self.pending_bits -= 8;
            push_bounded(
                &mut self.output,
                (self.pending >> self.pending_bits) as u8,
                self.limit,
            )?;
            self.pending &= (1_u32 << self.pending_bits) - 1;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Vec<u8>, PdfError> {
        if self.pending_bits != 0 {
            let byte = (self.pending << (8 - self.pending_bits)) as u8;
            push_bounded(&mut self.output, byte, self.limit)?;
        }
        Ok(self.output)
    }
}

fn decode_ascii_hex(input: &[u8], max_output: usize) -> Result<Vec<u8>, PdfError> {
    let mut output = Vec::new();
    let mut high = None;
    for &byte in input {
        if byte == b'>' {
            if let Some(nibble) = high {
                push_bounded(&mut output, nibble << 4, max_output)?;
            }
            return Ok(output);
        }
        if byte.is_ascii_whitespace() {
            continue;
        }
        let nibble = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return Err(PdfError::syntax("invalid ASCIIHexDecode byte", 0)),
        };
        if let Some(previous) = high.take() {
            push_bounded(&mut output, previous << 4 | nibble, max_output)?;
        } else {
            high = Some(nibble);
        }
    }
    Err(PdfError::syntax(
        "ASCIIHexDecode stream is missing its end marker",
        0,
    ))
}

fn decode_ascii85(input: &[u8], max_output: usize) -> Result<Vec<u8>, PdfError> {
    let mut output = Vec::new();
    let mut group = [0_u8; 5];
    let mut count = 0;
    let mut index = 0;
    let mut terminated = false;
    while index < input.len() {
        let byte = input[index];
        index += 1;
        if byte.is_ascii_whitespace() {
            continue;
        }
        if byte == b'<' && count == 0 && input.get(index) == Some(&b'~') {
            index += 1;
            continue;
        }
        if byte == b'~' {
            if input.get(index) != Some(&b'>') {
                return Err(PdfError::syntax("invalid ASCII85Decode end marker", 0));
            }
            index += 1;
            if input[index..]
                .iter()
                .any(|byte| !byte.is_ascii_whitespace())
            {
                return Err(PdfError::syntax("data follows ASCII85Decode end marker", 0));
            }
            terminated = true;
            break;
        }
        if byte == b'z' {
            if count != 0 {
                return Err(PdfError::syntax("ASCII85Decode z inside a group", 0));
            }
            extend_bounded(&mut output, &[0; 4], max_output)?;
            continue;
        }
        if !(b'!'..=b'u').contains(&byte) {
            return Err(PdfError::syntax("invalid ASCII85Decode byte", 0));
        }
        group[count] = byte - b'!';
        count += 1;
        if count == 5 {
            append_ascii85_group(&mut output, &group, 4, max_output)?;
            count = 0;
        }
    }
    if !terminated {
        return Err(PdfError::syntax(
            "ASCII85Decode stream is missing its end marker",
            0,
        ));
    }
    if count == 1 {
        return Err(PdfError::syntax("incomplete ASCII85Decode group", 0));
    }
    if count > 1 {
        group[count..].fill(84);
        append_ascii85_group(&mut output, &group, count - 1, max_output)?;
    }
    Ok(output)
}

fn append_ascii85_group(
    output: &mut Vec<u8>,
    group: &[u8; 5],
    take: usize,
    max_output: usize,
) -> Result<(), PdfError> {
    let mut value = 0_u32;
    for digit in group {
        value = value
            .checked_mul(85)
            .and_then(|value| value.checked_add(u32::from(*digit)))
            .ok_or_else(|| PdfError::syntax("ASCII85Decode group overflows", 0))?;
    }
    extend_bounded(output, &value.to_be_bytes()[..take], max_output)
}

fn decode_run_length(input: &[u8], max_output: usize) -> Result<Vec<u8>, PdfError> {
    let mut output = Vec::new();
    let mut index = 0;
    while let Some(&header) = input.get(index) {
        index += 1;
        match header {
            0..=127 => {
                let length = usize::from(header) + 1;
                let end = index
                    .checked_add(length)
                    .filter(|end| *end <= input.len())
                    .ok_or_else(|| PdfError::syntax("RunLengthDecode literal exceeds input", 0))?;
                extend_bounded(&mut output, &input[index..end], max_output)?;
                index = end;
            }
            128 => return Ok(output),
            129..=255 => {
                let byte = *input.get(index).ok_or_else(|| {
                    PdfError::syntax("RunLengthDecode repeat is missing a byte", 0)
                })?;
                index += 1;
                let length = 257 - usize::from(header);
                ensure_capacity(&output, length, max_output)?;
                output.resize(output.len() + length, byte);
            }
        }
    }
    Err(PdfError::syntax(
        "RunLengthDecode stream is missing its end marker",
        0,
    ))
}

fn decode_lzw(input: &[u8], early_change: u8, max_output: usize) -> Result<Vec<u8>, PdfError> {
    let mut reader = BitReader::new(input);
    let mut dictionary = initial_lzw_dictionary();
    let mut next_code = 258_usize;
    let mut width = 9_u8;
    let mut previous: Option<Vec<u8>> = None;
    let mut output = Vec::new();

    loop {
        let code = reader
            .read(width)
            .ok_or_else(|| PdfError::syntax("LZWDecode stream is missing its end marker", 0))?
            as usize;
        match code {
            256 => {
                dictionary = initial_lzw_dictionary();
                next_code = 258;
                width = 9;
                previous = None;
                continue;
            }
            257 => return Ok(output),
            _ => {}
        }

        let entry = if let Some(Some(entry)) = dictionary.get(code) {
            entry.clone()
        } else if code == next_code {
            let previous = previous
                .as_ref()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| PdfError::syntax("invalid LZWDecode code", 0))?;
            let mut entry = previous.clone();
            ensure_capacity(&entry, 1, max_output)?;
            entry.push(previous[0]);
            entry
        } else {
            return Err(PdfError::syntax("invalid LZWDecode code", 0));
        };

        extend_bounded(&mut output, &entry, max_output)?;
        if let Some(previous) = previous.as_ref()
            && next_code < 4096
        {
            let mut value = previous.clone();
            ensure_capacity(&value, 1, max_output)?;
            value.push(entry[0]);
            dictionary[next_code] = Some(value);
            next_code += 1;
            if width < 12 && next_code >= (1_usize << width) - usize::from(early_change) {
                width += 1;
            }
        }
        previous = Some(entry);
    }
}

fn initial_lzw_dictionary() -> Vec<Option<Vec<u8>>> {
    let mut dictionary = vec![None; 4096];
    for (value, entry) in dictionary.iter_mut().take(256).enumerate() {
        *entry = Some(vec![value as u8]);
    }
    dictionary
}

struct BitReader<'a> {
    input: &'a [u8],
    bit: usize,
}

impl<'a> BitReader<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, bit: 0 }
    }

    fn read(&mut self, width: u8) -> Option<u16> {
        let width = usize::from(width);
        if self.input.len().checked_mul(8)?.checked_sub(self.bit)? < width {
            return None;
        }
        let mut code = 0_u16;
        for _ in 0..width {
            code = (code << 1) | u16::from((self.input[self.bit / 8] >> (7 - self.bit % 8)) & 1);
            self.bit += 1;
        }
        Some(code)
    }
}

fn decode_predictor(
    input: Vec<u8>,
    params: DecodeParams,
    max_output: usize,
) -> Result<Vec<u8>, PdfError> {
    match params.predictor {
        1 => Ok(input),
        2 => decode_tiff_predictor(input, params, max_output),
        10..=15 => decode_png_predictor(input, params, max_output),
        _ => Err(PdfError::unsupported(format!(
            "unsupported predictor {}",
            params.predictor
        ))),
    }
}

fn encode_predictor(
    input: Vec<u8>,
    params: DecodeParams,
    max_output: usize,
) -> Result<Vec<u8>, PdfError> {
    match params.predictor {
        1 => bounded_copy(&input, max_output),
        2 => encode_tiff_predictor(input, params, max_output),
        10..=15 => encode_png_predictor(input, params, max_output),
        _ => Err(PdfError::unsupported(format!(
            "unsupported predictor {}",
            params.predictor
        ))),
    }
}

fn predictor_geometry(params: DecodeParams) -> Result<(usize, usize, usize), PdfError> {
    let samples = params
        .columns
        .checked_mul(params.colors)
        .ok_or_else(|| PdfError::limit("predictor sample count overflows"))?;
    let row_bits = samples
        .checked_mul(usize::from(params.bits_per_component))
        .ok_or_else(|| PdfError::limit("predictor row width overflows"))?;
    let row_bytes = row_bits
        .checked_add(7)
        .ok_or_else(|| PdfError::limit("predictor row width overflows"))?
        / 8;
    let bytes_per_pixel = params
        .colors
        .checked_mul(usize::from(params.bits_per_component))
        .and_then(|bits| bits.checked_add(7))
        .ok_or_else(|| PdfError::limit("predictor pixel width overflows"))?
        / 8;
    Ok((samples, row_bytes, bytes_per_pixel))
}

fn decode_tiff_predictor(
    mut input: Vec<u8>,
    params: DecodeParams,
    max_output: usize,
) -> Result<Vec<u8>, PdfError> {
    let (samples, row_bytes, _) = predictor_geometry(params)?;
    if !input.len().is_multiple_of(row_bytes) {
        return Err(PdfError::syntax("partial TIFF predictor row", 0));
    }
    if input.len() > max_output {
        return Err(PdfError::limit("decoded stream exceeds output limit"));
    }
    let bits = usize::from(params.bits_per_component);
    let mask = (1_u64 << bits) - 1;
    for row in input.chunks_exact_mut(row_bytes) {
        for sample in 0..samples {
            let mut value = read_packed_sample(row, sample, bits);
            if sample >= params.colors {
                value = (value + read_packed_sample(row, sample - params.colors, bits)) & mask;
            }
            write_packed_sample(row, sample, bits, value);
        }
    }
    Ok(input)
}

fn encode_tiff_predictor(
    mut input: Vec<u8>,
    params: DecodeParams,
    max_output: usize,
) -> Result<Vec<u8>, PdfError> {
    let (samples, row_bytes, _) = predictor_geometry(params)?;
    if row_bytes == 0 || !input.len().is_multiple_of(row_bytes) {
        return Err(PdfError::syntax("partial TIFF predictor row", 0));
    }
    if input.len() > max_output {
        return Err(PdfError::limit("encoded stream exceeds max_stream_bytes"));
    }
    let bits = usize::from(params.bits_per_component);
    let mask = (1_u64 << bits) - 1;
    for row in input.chunks_exact_mut(row_bytes) {
        for sample in (params.colors..samples).rev() {
            let value = read_packed_sample(row, sample, bits);
            let left = read_packed_sample(row, sample - params.colors, bits);
            write_packed_sample(row, sample, bits, value.wrapping_sub(left) & mask);
        }
    }
    Ok(input)
}

fn read_packed_sample(row: &[u8], sample: usize, bits: usize) -> u64 {
    let mut value = 0_u64;
    let start = sample * bits;
    for offset in 0..bits {
        let position = start + offset;
        value = (value << 1) | u64::from((row[position / 8] >> (7 - position % 8)) & 1);
    }
    value
}

fn write_packed_sample(row: &mut [u8], sample: usize, bits: usize, value: u64) {
    let start = sample * bits;
    for offset in 0..bits {
        let position = start + offset;
        let mask = 1 << (7 - position % 8);
        if (value >> (bits - 1 - offset)) & 1 == 1 {
            row[position / 8] |= mask;
        } else {
            row[position / 8] &= !mask;
        }
    }
}

fn decode_png_predictor(
    input: Vec<u8>,
    params: DecodeParams,
    max_output: usize,
) -> Result<Vec<u8>, PdfError> {
    let (_, row_bytes, bytes_per_pixel) = predictor_geometry(params)?;
    let encoded_row = row_bytes
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("PNG predictor row width overflows"))?;
    if !input.len().is_multiple_of(encoded_row) {
        return Err(PdfError::syntax("partial PNG predictor row", 0));
    }
    let row_count = input.len() / encoded_row;
    let output_len = row_count
        .checked_mul(row_bytes)
        .ok_or_else(|| PdfError::limit("PNG predictor output length overflows"))?;
    if output_len > max_output {
        return Err(PdfError::limit("decoded stream exceeds output limit"));
    }

    let mut output = Vec::with_capacity(output_len);
    let mut previous = vec![0_u8; row_bytes];
    for encoded in input.chunks_exact(encoded_row) {
        let filter = encoded[0];
        if filter > 4 {
            return Err(PdfError::syntax("invalid PNG predictor row filter", 0));
        }
        let mut row = vec![0_u8; row_bytes];
        for column in 0..row_bytes {
            let left = column
                .checked_sub(bytes_per_pixel)
                .map_or(0, |index| row[index]);
            let up = previous[column];
            let up_left = column
                .checked_sub(bytes_per_pixel)
                .map_or(0, |index| previous[index]);
            let predicted = match filter {
                0 => 0,
                1 => left,
                2 => up,
                3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
                4 => paeth(left, up, up_left),
                _ => return Err(PdfError::syntax("invalid PNG predictor row filter", 0)),
            };
            row[column] = encoded[column + 1].wrapping_add(predicted);
        }
        output.extend_from_slice(&row);
        previous = row;
    }
    Ok(output)
}

fn encode_png_predictor(
    input: Vec<u8>,
    params: DecodeParams,
    max_output: usize,
) -> Result<Vec<u8>, PdfError> {
    let (_, row_bytes, bytes_per_pixel) = predictor_geometry(params)?;
    if row_bytes == 0 || !input.len().is_multiple_of(row_bytes) {
        return Err(PdfError::syntax("partial PNG predictor row", 0));
    }
    let row_count = input.len() / row_bytes;
    let output_len = row_count
        .checked_mul(
            row_bytes
                .checked_add(1)
                .ok_or_else(|| PdfError::limit("PNG predictor row width overflows"))?,
        )
        .ok_or_else(|| PdfError::limit("PNG predictor output length overflows"))?;
    if output_len > max_output {
        return Err(PdfError::limit("encoded stream exceeds max_stream_bytes"));
    }
    let filter = match params.predictor {
        10 | 15 => 0,
        11 => 1,
        12 => 2,
        13 => 3,
        14 => 4,
        _ => unreachable!(),
    };
    let mut output = Vec::with_capacity(output_len);
    let mut previous = vec![0_u8; row_bytes];
    for row in input.chunks_exact(row_bytes) {
        output.push(filter);
        for column in 0..row_bytes {
            let left = column
                .checked_sub(bytes_per_pixel)
                .map_or(0, |index| row[index]);
            let up = previous[column];
            let up_left = column
                .checked_sub(bytes_per_pixel)
                .map_or(0, |index| previous[index]);
            let predicted = match filter {
                0 => 0,
                1 => left,
                2 => up,
                3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
                4 => paeth(left, up, up_left),
                _ => unreachable!(),
            };
            output.push(row[column].wrapping_sub(predicted));
        }
        previous.copy_from_slice(row);
    }
    Ok(output)
}

fn paeth(left: u8, up: u8, up_left: u8) -> u8 {
    let left = i32::from(left);
    let up = i32::from(up);
    let up_left = i32::from(up_left);
    let estimate = left + up - up_left;
    let left_distance = (estimate - left).abs();
    let up_distance = (estimate - up).abs();
    let diagonal_distance = (estimate - up_left).abs();
    if left_distance <= up_distance && left_distance <= diagonal_distance {
        left as u8
    } else if up_distance <= diagonal_distance {
        up as u8
    } else {
        up_left as u8
    }
}

fn bounded_copy(input: &[u8], max_output: usize) -> Result<Vec<u8>, PdfError> {
    if input.len() > max_output {
        return Err(PdfError::limit("decoded stream exceeds output limit"));
    }
    Ok(input.to_vec())
}

fn push_bounded(output: &mut Vec<u8>, byte: u8, max_output: usize) -> Result<(), PdfError> {
    ensure_capacity(output, 1, max_output)?;
    output.push(byte);
    Ok(())
}

fn extend_bounded(output: &mut Vec<u8>, bytes: &[u8], max_output: usize) -> Result<(), PdfError> {
    ensure_capacity(output, bytes.len(), max_output)?;
    output.extend_from_slice(bytes);
    Ok(())
}

fn ensure_capacity(output: &[u8], additional: usize, max_output: usize) -> Result<(), PdfError> {
    if output
        .len()
        .checked_add(additional)
        .is_none_or(|length| length > max_output)
    {
        return Err(PdfError::limit("decoded stream exceeds output limit"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream_value(filters: Vec<&[u8]>, params: Vec<Option<DecodeParams>>) -> Value {
        let mut dictionary = BTreeMap::new();
        dictionary.insert(
            b"Filter".to_vec(),
            if filters.len() == 1 {
                Value::Name(filters[0].to_vec())
            } else {
                Value::Array(
                    filters
                        .into_iter()
                        .map(|name| Value::Name(name.to_vec()))
                        .collect(),
                )
            },
        );
        if params.iter().any(Option::is_some) {
            let values: Vec<Value> = params
                .into_iter()
                .map(|params| match params {
                    None => Value::Null,
                    Some(params) => {
                        let mut value = BTreeMap::new();
                        value.insert(
                            b"Predictor".to_vec(),
                            Value::Integer(i64::from(params.predictor)),
                        );
                        value.insert(b"Colors".to_vec(), Value::Integer(params.colors as i64));
                        value.insert(
                            b"BitsPerComponent".to_vec(),
                            Value::Integer(i64::from(params.bits_per_component)),
                        );
                        value.insert(b"Columns".to_vec(), Value::Integer(params.columns as i64));
                        value.insert(
                            b"EarlyChange".to_vec(),
                            Value::Integer(i64::from(params.early_change)),
                        );
                        Value::Dict(value)
                    }
                })
                .collect();
            dictionary.insert(
                b"DecodeParms".to_vec(),
                if values.len() == 1 {
                    values.into_iter().next().unwrap()
                } else {
                    Value::Array(values)
                },
            );
        }
        Value::Dict(dictionary)
    }

    fn roundtrip(value: &Value, decoded: &[u8]) {
        let encoded = encode_pdf_stream(value, decoded, &Limits::default()).unwrap();
        let encoded_again = encode_pdf_stream(value, decoded, &Limits::default()).unwrap();
        assert_eq!(encoded, encoded_again);
        let filters = parse_filters(value, usize::MAX).unwrap();
        let params = parse_decode_params_list(value, filters.len(), usize::MAX).unwrap();
        assert_eq!(
            decode_filter_chain(
                &encoded,
                &filters,
                &params,
                Limits::default().max_stream_bytes
            )
            .unwrap(),
            decoded
        );
    }

    #[test]
    fn every_editable_filter_and_chain_encodes_deterministically() {
        let data = b"ABABABABABABABABABABABABABABABAB";
        for name in [
            b"FlateDecode".as_slice(),
            b"ASCIIHexDecode",
            b"ASCII85Decode",
            b"RunLengthDecode",
        ] {
            roundtrip(&stream_value(vec![name], vec![None]), data);
        }
        for early_change in [0, 1] {
            roundtrip(
                &stream_value(
                    vec![b"LZWDecode"],
                    vec![Some(DecodeParams {
                        early_change,
                        ..DecodeParams::default()
                    })],
                ),
                data,
            );
            let long: Vec<u8> = (0..10_000).map(|index| (index * 31) as u8).collect();
            roundtrip(
                &stream_value(
                    vec![b"LZWDecode"],
                    vec![Some(DecodeParams {
                        early_change,
                        ..DecodeParams::default()
                    })],
                ),
                &long,
            );
        }
        roundtrip(
            &stream_value(vec![b"ASCIIHexDecode", b"FlateDecode"], vec![None, None]),
            data,
        );
    }

    #[test]
    fn tiff_and_png_predictors_encode_roundtrip() {
        roundtrip(
            &stream_value(
                vec![b"FlateDecode"],
                vec![Some(DecodeParams {
                    predictor: 2,
                    columns: 4,
                    bits_per_component: 4,
                    ..DecodeParams::default()
                })],
            ),
            &[0x12, 0x34, 0x56, 0x78],
        );
        for predictor in 10..=15 {
            roundtrip(
                &stream_value(
                    vec![b"LZWDecode"],
                    vec![Some(DecodeParams {
                        predictor,
                        columns: 3,
                        ..DecodeParams::default()
                    })],
                ),
                &[1, 2, 3, 4, 5, 6],
            );
        }
        roundtrip(
            &stream_value(
                vec![b"ASCII85Decode", b"FlateDecode"],
                vec![
                    None,
                    Some(DecodeParams {
                        predictor: 12,
                        columns: 3,
                        ..DecodeParams::default()
                    }),
                ],
            ),
            &[1, 2, 3, 4, 5, 6],
        );
    }

    #[test]
    fn encoding_rejects_unsupported_malformed_and_over_budget_streams() {
        for name in [b"DCTDecode".as_slice(), b"Crypt", b"JPXDecode"] {
            assert_eq!(
                encode_pdf_stream(
                    &stream_value(vec![name], vec![None]),
                    b"data",
                    &Limits::default()
                )
                .unwrap_err()
                .code,
                crate::PdfErrorCode::UnsupportedFeature
            );
        }
        let mismatch = stream_value(
            vec![b"FlateDecode", b"ASCIIHexDecode"],
            vec![Some(DecodeParams::default())],
        );
        assert_eq!(
            encode_pdf_stream(&mismatch, b"data", &Limits::default())
                .unwrap_err()
                .code,
            crate::PdfErrorCode::InvalidSyntax
        );
        let limits = Limits {
            max_stream_bytes: 4,
            ..Limits::default()
        };
        assert_eq!(
            encode_pdf_stream(
                &stream_value(vec![b"ASCIIHexDecode"], vec![None]),
                b"data",
                &limits
            )
            .unwrap_err()
            .code,
            crate::PdfErrorCode::ResourceLimit
        );
        assert_eq!(
            encode_pdf_stream(
                &stream_value(vec![b"FlateDecode"], vec![None]),
                b"data",
                &limits
            )
            .unwrap_err()
            .code,
            crate::PdfErrorCode::ResourceLimit
        );
        let partial_predictor = stream_value(
            vec![b"FlateDecode"],
            vec![Some(DecodeParams {
                predictor: 2,
                columns: 3,
                ..DecodeParams::default()
            })],
        );
        assert_eq!(
            encode_pdf_stream(&partial_predictor, b"data", &Limits::default())
                .unwrap_err()
                .code,
            crate::PdfErrorCode::InvalidSyntax
        );
    }

    #[test]
    fn packed_tiff_predictor_decodes() {
        let params = DecodeParams {
            predictor: 2,
            columns: 4,
            colors: 1,
            bits_per_component: 4,
            ..DecodeParams::default()
        };
        assert_eq!(
            decode_tiff_predictor(vec![0x11, 0x11], params, 2).unwrap(),
            vec![0x12, 0x34]
        );
    }

    #[test]
    fn paeth_vector() {
        assert_eq!(paeth(10, 20, 15), 15);
        assert_eq!(paeth(30, 20, 10), 30);
    }

    #[test]
    fn png_predictors_ten_through_fifteen_decode() {
        for predictor in 10..=15 {
            let params = DecodeParams {
                predictor,
                columns: 2,
                ..DecodeParams::default()
            };
            assert_eq!(
                decode_png_predictor(vec![0, 1, 2], params, 2).unwrap(),
                [1, 2]
            );
        }
    }
}
