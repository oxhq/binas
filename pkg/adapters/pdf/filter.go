package pdf

import (
	"bytes"
	"compress/zlib"
	"encoding/ascii85"
	"fmt"
	"io"
	"strconv"
	"strings"
)

const (
	pdfFilterASCII85Decode   = "ASCII85Decode"
	pdfFilterASCIIHexDecode  = "ASCIIHexDecode"
	pdfFilterCrypt           = "Crypt"
	pdfFilterFlateDecode     = "FlateDecode"
	pdfFilterLZWDecode       = "LZWDecode"
	pdfFilterRunLengthDecode = "RunLengthDecode"
)

func decodeFlateDecode(input []byte) ([]byte, error) {
	reader, err := zlib.NewReader(bytes.NewReader(input))
	if err != nil {
		return nil, fmt.Errorf("decode FlateDecode stream: %w", err)
	}
	defer reader.Close()

	output, err := io.ReadAll(reader)
	if err != nil {
		return nil, fmt.Errorf("read FlateDecode stream: %w", err)
	}
	return output, nil
}

func encodeFlateDecode(input []byte) ([]byte, error) {
	var output bytes.Buffer
	writer := zlib.NewWriter(&output)
	if _, err := writer.Write(input); err != nil {
		writer.Close()
		return nil, fmt.Errorf("encode FlateDecode stream: %w", err)
	}
	if err := writer.Close(); err != nil {
		return nil, fmt.Errorf("finish FlateDecode stream: %w", err)
	}
	return output.Bytes(), nil
}

func decodeASCII85Decode(input []byte) ([]byte, error) {
	if eod := bytes.Index(input, []byte("~>")); eod != -1 {
		input = input[:eod]
	}
	output := make([]byte, len(input))
	n, _, err := ascii85.Decode(output, input, true)
	if err != nil {
		return nil, fmt.Errorf("decode ASCII85Decode stream: %w", err)
	}
	return output[:n], nil
}

func encodeASCII85Decode(input []byte) ([]byte, error) {
	output := make([]byte, ascii85.MaxEncodedLen(len(input)))
	n := ascii85.Encode(output, input)
	return output[:n], nil
}

func decodeASCIIHexDecode(input []byte) ([]byte, error) {
	output := make([]byte, 0, len(input)/2)
	highNibble := -1
	for _, b := range input {
		if b == '>' {
			if highNibble != -1 {
				output = append(output, byte(highNibble<<4))
			}
			return output, nil
		}
		if isPDFSpace(b) {
			continue
		}
		nibble, ok := asciiHexValue(b)
		if !ok {
			return nil, fmt.Errorf("decode ASCIIHexDecode stream: invalid hex byte %q", b)
		}
		if highNibble == -1 {
			highNibble = nibble
			continue
		}
		output = append(output, byte(highNibble<<4|nibble))
		highNibble = -1
	}
	return nil, fmt.Errorf("decode ASCIIHexDecode stream: missing end-of-data marker")
}

func encodeASCIIHexDecode(input []byte) ([]byte, error) {
	const hex = "0123456789ABCDEF"
	output := make([]byte, 0, len(input)*2+1)
	for _, b := range input {
		output = append(output, hex[b>>4], hex[b&0x0f])
	}
	output = append(output, '>')
	return output, nil
}

func decodeRunLengthDecode(input []byte) ([]byte, error) {
	output := make([]byte, 0, len(input))
	for i := 0; i < len(input); {
		header := input[i]
		i++
		switch {
		case header <= 127:
			runLength := int(header) + 1
			if i+runLength > len(input) {
				return nil, fmt.Errorf("decode RunLengthDecode stream: literal run exceeds input")
			}
			output = append(output, input[i:i+runLength]...)
			i += runLength
		case header == 128:
			return output, nil
		default:
			runLength := 257 - int(header)
			if i >= len(input) {
				return nil, fmt.Errorf("decode RunLengthDecode stream: repeated run is missing byte")
			}
			for range runLength {
				output = append(output, input[i])
			}
			i++
		}
	}
	return nil, fmt.Errorf("decode RunLengthDecode stream: missing end-of-data marker")
}

func encodeRunLengthDecode(input []byte) ([]byte, error) {
	output := make([]byte, 0, len(input)+1)
	for i := 0; i < len(input); {
		repeatLength := runLengthRepeat(input, i)
		if repeatLength >= 2 {
			output = append(output, byte(257-repeatLength), input[i])
			i += repeatLength
			continue
		}
		literalStart := i
		i++
		for i < len(input) && i-literalStart < 128 {
			if runLengthRepeat(input, i) >= 2 {
				break
			}
			i++
		}
		output = append(output, byte(i-literalStart-1))
		output = append(output, input[literalStart:i]...)
	}
	output = append(output, 128)
	return output, nil
}

func runLengthRepeat(input []byte, start int) int {
	if start >= len(input) {
		return 0
	}
	length := 1
	for start+length < len(input) && length < 128 && input[start+length] == input[start] {
		length++
	}
	return length
}

func decodeLZWDecode(input []byte, params pdfLZWDecodeParms) ([]byte, error) {
	reader := lzwBitReader{input: input}
	dict := initialLZWDecodeDictionary()
	nextCode := 258
	codeWidth := 9
	var previous []byte
	var output []byte

	for {
		code, err := reader.read(codeWidth)
		if err != nil {
			if err == io.EOF {
				return nil, fmt.Errorf("decode LZWDecode stream: missing end-of-data marker")
			}
			return nil, fmt.Errorf("decode LZWDecode stream: %w", err)
		}
		switch code {
		case 256:
			dict = initialLZWDecodeDictionary()
			nextCode = 258
			codeWidth = 9
			previous = nil
			continue
		case 257:
			return output, nil
		}

		var entry []byte
		switch {
		case code < 256:
			entry = []byte{byte(code)}
		case code < nextCode:
			value, ok := dict[code]
			if !ok {
				return nil, fmt.Errorf("decode LZWDecode stream: invalid code %d", code)
			}
			entry = value
		case code == nextCode && len(previous) > 0:
			entry = append(bytes.Clone(previous), previous[0])
		default:
			return nil, fmt.Errorf("decode LZWDecode stream: invalid code %d", code)
		}

		output = append(output, entry...)
		if len(previous) > 0 && nextCode <= 4095 {
			dict[nextCode] = append(bytes.Clone(previous), entry[0])
			nextCode++
			codeWidth = lzwNextCodeWidth(codeWidth, nextCode, params.earlyChange)
		}
		previous = entry
	}
}

func encodeLZWDecode(input []byte, params pdfLZWDecodeParms) ([]byte, error) {
	writer := lzwBitWriter{}
	dict := initialLZWEncodeDictionary()
	nextCode := 258
	codeWidth := 9
	w := ""

	for _, b := range input {
		k := string([]byte{b})
		wk := w + k
		if _, ok := dict[wk]; ok {
			w = wk
			continue
		}
		if err := writer.write(dict[w], codeWidth); err != nil {
			return nil, fmt.Errorf("encode LZWDecode stream: %w", err)
		}
		if nextCode <= 4095 {
			dict[wk] = nextCode
			nextCode++
			codeWidth = lzwNextCodeWidth(codeWidth, nextCode, params.earlyChange)
		}
		w = k
	}
	if w != "" {
		if err := writer.write(dict[w], codeWidth); err != nil {
			return nil, fmt.Errorf("encode LZWDecode stream: %w", err)
		}
	}
	if err := writer.write(257, codeWidth); err != nil {
		return nil, fmt.Errorf("encode LZWDecode stream: %w", err)
	}
	return writer.bytes(), nil
}

func initialLZWDecodeDictionary() map[int][]byte {
	dict := make(map[int][]byte, 256)
	for i := 0; i < 256; i++ {
		dict[i] = []byte{byte(i)}
	}
	return dict
}

func initialLZWEncodeDictionary() map[string]int {
	dict := make(map[string]int, 256)
	for i := 0; i < 256; i++ {
		dict[string([]byte{byte(i)})] = i
	}
	return dict
}

func lzwNextCodeWidth(width, nextCode, earlyChange int) int {
	if width < 12 && nextCode >= (1<<width)-earlyChange {
		return width + 1
	}
	return width
}

type lzwBitReader struct {
	input []byte
	bit   int
}

func (r *lzwBitReader) read(width int) (int, error) {
	if width < 1 || width > 12 {
		return 0, fmt.Errorf("invalid code width %d", width)
	}
	if len(r.input)*8-r.bit < width {
		return 0, io.EOF
	}
	code := 0
	for range width {
		b := r.input[r.bit/8]
		shift := 7 - r.bit%8
		code = code<<1 | int((b>>shift)&1)
		r.bit++
	}
	return code, nil
}

type lzwBitWriter struct {
	output []byte
	bits   uint32
	count  int
}

func (w *lzwBitWriter) write(code, width int) error {
	if width < 1 || width > 12 {
		return fmt.Errorf("invalid code width %d", width)
	}
	if code < 0 || code >= 1<<width {
		return fmt.Errorf("code %d does not fit in %d bits", code, width)
	}
	w.bits = (w.bits << width) | uint32(code)
	w.count += width
	for w.count >= 8 {
		shift := w.count - 8
		w.output = append(w.output, byte(w.bits>>shift))
		w.bits &= (1 << shift) - 1
		w.count -= 8
	}
	return nil
}

func (w *lzwBitWriter) bytes() []byte {
	if w.count > 0 {
		w.output = append(w.output, byte(w.bits<<(8-w.count)))
		w.bits = 0
		w.count = 0
	}
	return w.output
}

func asciiHexValue(b byte) (int, bool) {
	switch {
	case b >= '0' && b <= '9':
		return int(b - '0'), true
	case b >= 'A' && b <= 'F':
		return int(b-'A') + 10, true
	case b >= 'a' && b <= 'f':
		return int(b-'a') + 10, true
	default:
		return 0, false
	}
}

func decodeStreamFilter(filter string, input []byte) ([]byte, error) {
	return decodeStreamFilterWithDecodeParms(filter, "", input)
}

func decodeStreamFilterWithDecodeParms(filter, decodeParms string, input []byte) ([]byte, error) {
	return decodeStreamFilterWithDecodeParmsAndCrypt(filter, decodeParms, input, nil)
}

type pdfStreamCryptHandler struct {
	Decrypt func(name string, input []byte) ([]byte, error)
	Encrypt func(name string, input []byte) ([]byte, error)
}

func decodeStreamFilterWithDecodeParmsAndCrypt(filter, decodeParms string, input []byte, crypt *pdfStreamCryptHandler) ([]byte, error) {
	filters := parsePDFStreamFilterChain(filter)
	if len(filters) == 0 {
		if strings.TrimSpace(decodeParms) != "" {
			return nil, fmt.Errorf("unsupported stream: /DecodeParms requires /Filter")
		}
		return bytes.Clone(input), nil
	}
	if !isSupportedPDFStreamFilterChain(filters) {
		return nil, fmt.Errorf("unsupported PDF stream filter %q", strings.Join(filters, " "))
	}
	params, err := parsePDFStreamDecodeParms(filters, decodeParms)
	if err != nil {
		return nil, err
	}
	output := bytes.Clone(input)
	for i, filter := range filters {
		var err error
		switch filter {
		case pdfFilterASCII85Decode:
			output, err = decodeASCII85Decode(output)
		case pdfFilterASCIIHexDecode:
			output, err = decodeASCIIHexDecode(output)
		case pdfFilterCrypt:
			name := pdfCryptFilterName(params[i].crypt)
			if name == "" || name == "Identity" {
				continue
			}
			if crypt == nil || crypt.Decrypt == nil {
				return nil, fmt.Errorf("unsupported PDF stream crypt filter /%s requires encryption context", name)
			}
			output, err = crypt.Decrypt(name, output)
		case pdfFilterFlateDecode:
			output, err = decodeFlateDecode(output)
			if err == nil && params[i].flate != nil {
				output, err = decodeFlateDecodeParms(output, *params[i].flate)
			}
		case pdfFilterLZWDecode:
			lzwParams := lzwDecodeParmsOrDefault(params[i].lzw)
			output, err = decodeLZWDecode(output, lzwParams)
			if err == nil && lzwParams.predictor != nil {
				output, err = decodeFlateDecodeParms(output, *lzwParams.predictor)
			}
		case pdfFilterRunLengthDecode:
			output, err = decodeRunLengthDecode(output)
		default:
			return nil, fmt.Errorf("unsupported PDF stream filter %q", filter)
		}
		if err != nil {
			return nil, err
		}
	}
	return output, nil
}

func encodeStreamFilter(filter string, input []byte) ([]byte, error) {
	return encodeStreamFilterWithDecodeParms(filter, "", input)
}

func encodeStreamFilterWithDecodeParms(filter, decodeParms string, input []byte) ([]byte, error) {
	return encodeStreamFilterWithDecodeParmsAndCrypt(filter, decodeParms, input, nil)
}

func encodeStreamFilterWithDecodeParmsAndCrypt(filter, decodeParms string, input []byte, crypt *pdfStreamCryptHandler) ([]byte, error) {
	filters := parsePDFStreamFilterChain(filter)
	if len(filters) == 0 {
		if strings.TrimSpace(decodeParms) != "" {
			return nil, fmt.Errorf("unsupported stream: /DecodeParms requires /Filter")
		}
		return bytes.Clone(input), nil
	}
	if !isSupportedPDFStreamFilterChain(filters) {
		return nil, fmt.Errorf("unsupported PDF stream filter %q", strings.Join(filters, " "))
	}
	params, err := parsePDFStreamDecodeParms(filters, decodeParms)
	if err != nil {
		return nil, err
	}
	output := bytes.Clone(input)
	for i := len(filters) - 1; i >= 0; i-- {
		var err error
		switch filters[i] {
		case pdfFilterASCII85Decode:
			output, err = encodeASCII85Decode(output)
		case pdfFilterASCIIHexDecode:
			output, err = encodeASCIIHexDecode(output)
		case pdfFilterCrypt:
			name := pdfCryptFilterName(params[i].crypt)
			if name == "" || name == "Identity" {
				continue
			}
			if crypt == nil || crypt.Encrypt == nil {
				return nil, fmt.Errorf("unsupported PDF stream crypt filter /%s requires encryption context", name)
			}
			output, err = crypt.Encrypt(name, output)
		case pdfFilterFlateDecode:
			if params[i].flate != nil {
				output, err = encodeFlateDecodeParms(output, *params[i].flate)
				if err != nil {
					return nil, err
				}
			}
			output, err = encodeFlateDecode(output)
		case pdfFilterLZWDecode:
			lzwParams := lzwDecodeParmsOrDefault(params[i].lzw)
			if lzwParams.predictor != nil {
				output, err = encodeFlateDecodeParms(output, *lzwParams.predictor)
				if err != nil {
					return nil, err
				}
			}
			output, err = encodeLZWDecode(output, lzwParams)
		case pdfFilterRunLengthDecode:
			output, err = encodeRunLengthDecode(output)
		default:
			return nil, fmt.Errorf("unsupported PDF stream filter %q", filters[i])
		}
		if err != nil {
			return nil, err
		}
	}
	return output, nil
}

func isPassthroughPDFStreamFilter(filter string) bool {
	switch normalizePDFStreamFilter(filter) {
	case "", "Identity":
		return true
	default:
		return false
	}
}

func normalizePDFStreamFilter(filter string) string {
	filter = strings.TrimSpace(filter)
	filter = strings.TrimPrefix(filter, "/")
	filter = strings.TrimSpace(filter)
	switch filter {
	case "A85":
		return pdfFilterASCII85Decode
	case "AHx":
		return pdfFilterASCIIHexDecode
	case "Fl":
		return pdfFilterFlateDecode
	case "LZW":
		return pdfFilterLZWDecode
	case "RL":
		return pdfFilterRunLengthDecode
	default:
		return filter
	}
}

func parsePDFStreamFilterChain(filter string) []string {
	filter = strings.TrimSpace(filter)
	if filter == "" {
		return nil
	}
	if strings.HasPrefix(filter, "[") && strings.HasSuffix(filter, "]") {
		inner := strings.TrimSpace(strings.TrimSuffix(strings.TrimPrefix(filter, "["), "]"))
		if inner == "" {
			return nil
		}
		parts := strings.Fields(inner)
		filters := make([]string, 0, len(parts))
		for _, part := range parts {
			filters = append(filters, normalizePDFStreamFilter(part))
		}
		return filters
	}
	filter = normalizePDFStreamFilter(filter)
	if isPassthroughPDFStreamFilter(filter) {
		return nil
	}
	return []string{filter}
}

func isSupportedPDFStreamFilterChain(filters []string) bool {
	if len(filters) == 0 {
		return false
	}
	for _, filter := range filters {
		if !isSupportedReversiblePDFStreamFilter(filter) {
			return false
		}
	}
	return true
}

func isSupportedReversiblePDFStreamFilter(filter string) bool {
	switch filter {
	case pdfFilterFlateDecode, pdfFilterASCII85Decode, pdfFilterASCIIHexDecode, pdfFilterLZWDecode, pdfFilterRunLengthDecode, pdfFilterCrypt:
		return true
	default:
		return false
	}
}

type pdfStreamDecodeParms struct {
	crypt *pdfCryptDecodeParms
	flate *pdfFlateDecodeParms
	lzw   *pdfLZWDecodeParms
}

type pdfCryptDecodeParms struct {
	name string
}

type pdfFlateDecodeParms struct {
	predictor        int
	columns          int
	colors           int
	bitsPerComponent int
}

type pdfLZWDecodeParms struct {
	earlyChange int
	predictor   *pdfFlateDecodeParms
}

func parsePDFStreamDecodeParms(filters []string, decodeParms string) ([]pdfStreamDecodeParms, error) {
	params := make([]pdfStreamDecodeParms, len(filters))
	decodeParms = strings.TrimSpace(decodeParms)
	if decodeParms == "" {
		return params, nil
	}
	if decodeParms == "null" {
		return params, nil
	}
	if strings.HasPrefix(decodeParms, "<<") {
		if len(filters) != 1 {
			return nil, fmt.Errorf("unsupported stream: direct /DecodeParms dictionary requires a single /Filter")
		}
		switch filters[0] {
		case pdfFilterCrypt:
			crypt, err := parseCryptDecodeParmsDictionary(decodeParms)
			if err != nil {
				return nil, err
			}
			params[0].crypt = &crypt
		case pdfFilterFlateDecode:
			flate, err := parseFlateDecodeParmsDictionary(decodeParms)
			if err != nil {
				return nil, err
			}
			params[0].flate = &flate
		case pdfFilterLZWDecode:
			lzw, err := parseLZWDecodeParmsDictionary(decodeParms)
			if err != nil {
				return nil, err
			}
			params[0].lzw = &lzw
		default:
			return nil, fmt.Errorf("unsupported stream: /DecodeParms for /%s must be null", filters[0])
		}
		return params, nil
	}
	if strings.HasPrefix(decodeParms, "[") {
		values, err := parsePDFDecodeParmsArray(decodeParms)
		if err != nil {
			return nil, err
		}
		if len(values) != len(filters) {
			return nil, fmt.Errorf("unsupported stream: /DecodeParms array length must match /Filter array length")
		}
		if pdfDecodeParmsArrayIsAllNull(values) {
			return params, nil
		}
		for i, value := range values {
			if value == "null" {
				continue
			}
			switch filters[i] {
			case pdfFilterCrypt:
				crypt, err := parseCryptDecodeParmsDictionary(value)
				if err != nil {
					return nil, err
				}
				params[i].crypt = &crypt
			case pdfFilterFlateDecode:
				flate, err := parseFlateDecodeParmsDictionary(value)
				if err != nil {
					return nil, err
				}
				params[i].flate = &flate
			case pdfFilterLZWDecode:
				lzw, err := parseLZWDecodeParmsDictionary(value)
				if err != nil {
					return nil, err
				}
				params[i].lzw = &lzw
			default:
				return nil, fmt.Errorf("unsupported stream: /DecodeParms for /%s must be null", filters[i])
			}
		}
		return params, nil
	}
	return nil, fmt.Errorf("unsupported stream: /DecodeParms must be a dictionary, array, or null")
}

func pdfCryptFilterName(params *pdfCryptDecodeParms) string {
	if params == nil {
		return "Identity"
	}
	return params.name
}

func parseCryptDecodeParmsDictionary(value string) (pdfCryptDecodeParms, error) {
	input := []byte(strings.TrimSpace(value))
	parser := pdfValueParser{input: input}
	parsed, err := parser.parseValue()
	if err != nil {
		return pdfCryptDecodeParms{}, fmt.Errorf("unsupported stream: /DecodeParms /Crypt dictionary is malformed: %w", err)
	}
	parser.skipSpaceAndComments()
	if parser.i != len(input) {
		return pdfCryptDecodeParms{}, fmt.Errorf("unsupported stream: /DecodeParms /Crypt dictionary has trailing data")
	}
	dict, ok := parsed.(pdfDict)
	if !ok {
		return pdfCryptDecodeParms{}, fmt.Errorf("unsupported stream: /DecodeParms /Crypt must be a direct dictionary")
	}
	for name := range dict {
		if name != "Name" {
			return pdfCryptDecodeParms{}, fmt.Errorf("unsupported stream: /DecodeParms /Crypt key /%s is not supported", name)
		}
	}
	name, ok := dictPDFName(dict, "Name")
	if !ok || name == "" {
		name = "Identity"
	}
	return pdfCryptDecodeParms{name: name}, nil
}

func pdfDecodeParmsArrayIsAllNull(values []string) bool {
	for _, value := range values {
		if value != "null" {
			return false
		}
	}
	return true
}

func parsePDFDecodeParmsArray(value string) ([]string, error) {
	input := []byte(strings.TrimSpace(value))
	if len(input) < 2 || input[0] != '[' || input[len(input)-1] != ']' {
		return nil, fmt.Errorf("unsupported stream: /DecodeParms array is not closed")
	}
	var values []string
	for i := 1; i < len(input)-1; {
		for i < len(input)-1 && isPDFSpace(input[i]) {
			i++
		}
		if i >= len(input)-1 {
			break
		}
		if bytes.HasPrefix(input[i:], []byte("null")) && isPDFTokenEnd(input, i+len("null")) {
			values = append(values, "null")
			i += len("null")
			continue
		}
		if i+1 < len(input) && input[i] == '<' && input[i+1] == '<' {
			end, ok := findDictionaryEnd(input, i)
			if !ok || end > len(input)-1 {
				return nil, fmt.Errorf("unsupported stream: /DecodeParms dictionary is not closed")
			}
			values = append(values, string(input[i:end]))
			i = end
			continue
		}
		return nil, fmt.Errorf("unsupported stream: /DecodeParms array entries must be null or direct dictionaries")
	}
	return values, nil
}

func parseFlateDecodeParmsDictionary(value string) (pdfFlateDecodeParms, error) {
	dict := []byte(strings.TrimSpace(value))
	if len(dict) < 4 || dict[0] != '<' || dict[1] != '<' || dict[len(dict)-2] != '>' || dict[len(dict)-1] != '>' {
		return pdfFlateDecodeParms{}, fmt.Errorf("unsupported stream: /DecodeParms must be a direct dictionary")
	}
	names := decodeParmsDictionaryNames(dict)
	for _, name := range names {
		switch name {
		case "Predictor", "Columns", "Colors", "BitsPerComponent":
		default:
			return pdfFlateDecodeParms{}, fmt.Errorf("unsupported stream: /DecodeParms key /%s is not supported", name)
		}
	}
	params, ok, err := parsePredictorDecodeParmsDictionary(dict)
	if err != nil {
		return pdfFlateDecodeParms{}, err
	}
	if !ok {
		return pdfFlateDecodeParms{}, fmt.Errorf("unsupported stream: /DecodeParms /Predictor is missing")
	}
	return params, nil
}

func parsePredictorDecodeParmsDictionary(dict []byte) (pdfFlateDecodeParms, bool, error) {
	predictor, ok, err := decodeParmsInteger(dict, "Predictor")
	if err != nil {
		return pdfFlateDecodeParms{}, false, err
	}
	if !ok {
		if err := validateFlateDecodeParmsPredictor1Defaults(dict); err != nil {
			return pdfFlateDecodeParms{}, true, err
		}
		return pdfFlateDecodeParms{predictor: 1}, true, nil
	}
	if predictor == 1 {
		if err := validateFlateDecodeParmsPredictor1Defaults(dict); err != nil {
			return pdfFlateDecodeParms{}, true, err
		}
		return pdfFlateDecodeParms{predictor: predictor}, true, nil
	}
	if predictor == 2 {
		params, err := parseFlateDecodeParmsPredictorGeometry(dict, predictor, "TIFF", true)
		if err != nil {
			return pdfFlateDecodeParms{}, true, err
		}
		if _, _, err := tiffPredictorGeometry(params); err != nil {
			return pdfFlateDecodeParms{}, true, err
		}
		return params, true, nil
	}
	if predictor < 10 || predictor > 15 {
		return pdfFlateDecodeParms{}, true, fmt.Errorf("unsupported stream: /DecodeParms is not implemented")
	}
	params, err := parseFlateDecodeParmsPredictorGeometry(dict, predictor, "PNG", true)
	if err != nil {
		return pdfFlateDecodeParms{}, true, err
	}
	return params, true, nil
}

func parseLZWDecodeParmsDictionary(value string) (pdfLZWDecodeParms, error) {
	dict := []byte(strings.TrimSpace(value))
	if len(dict) < 4 || dict[0] != '<' || dict[1] != '<' || dict[len(dict)-2] != '>' || dict[len(dict)-1] != '>' {
		return pdfLZWDecodeParms{}, fmt.Errorf("unsupported stream: /DecodeParms must be a direct dictionary")
	}
	names := decodeParmsDictionaryNames(dict)
	hasPredictorDecodeParms := false
	for _, name := range names {
		switch name {
		case "EarlyChange":
		case "Predictor", "Columns", "Colors", "BitsPerComponent":
			hasPredictorDecodeParms = true
		default:
			return pdfLZWDecodeParms{}, fmt.Errorf("unsupported stream: /DecodeParms key /%s is not supported", name)
		}
	}
	earlyChange, ok, err := decodeParmsInteger(dict, "EarlyChange")
	if err != nil {
		return pdfLZWDecodeParms{}, err
	}
	if !ok {
		earlyChange = 1
	}
	if earlyChange != 0 && earlyChange != 1 {
		return pdfLZWDecodeParms{}, fmt.Errorf("unsupported stream: /DecodeParms /EarlyChange must be 0 or 1")
	}
	params := pdfLZWDecodeParms{earlyChange: earlyChange}
	if !hasPredictorDecodeParms {
		return params, nil
	}
	predictor, ok, err := parsePredictorDecodeParmsDictionary(dict)
	if err != nil {
		return pdfLZWDecodeParms{}, err
	}
	if !ok {
		return pdfLZWDecodeParms{}, fmt.Errorf("unsupported stream: /DecodeParms /Predictor is missing")
	}
	params.predictor = &predictor
	return params, nil
}

func lzwDecodeParmsOrDefault(params *pdfLZWDecodeParms) pdfLZWDecodeParms {
	if params == nil {
		return pdfLZWDecodeParms{earlyChange: 1}
	}
	return *params
}

func validateFlateDecodeParmsPredictor1Defaults(dict []byte) error {
	for _, name := range []string{"Columns", "Colors", "BitsPerComponent"} {
		_, _, err := decodeParmsInteger(dict, name)
		if err != nil {
			return err
		}
	}
	return nil
}

func parseFlateDecodeParmsPredictorGeometry(dict []byte, predictor int, label string, defaultColumns bool) (pdfFlateDecodeParms, error) {
	colors, ok, err := decodeParmsInteger(dict, "Colors")
	if err != nil {
		return pdfFlateDecodeParms{}, err
	}
	if !ok {
		colors = 1
	}
	if colors < 1 {
		if label == "PNG" {
			return pdfFlateDecodeParms{}, fmt.Errorf("unsupported stream: /DecodeParms PNG predictors require /Colors >= 1")
		}
		return pdfFlateDecodeParms{}, fmt.Errorf("unsupported stream: /DecodeParms %s predictor requires /Colors >= 1", label)
	}
	bitsPerComponent, ok, err := decodeParmsInteger(dict, "BitsPerComponent")
	if err != nil {
		return pdfFlateDecodeParms{}, err
	}
	if !ok {
		bitsPerComponent = 8
	}
	if bitsPerComponent < 1 {
		if label == "PNG" {
			return pdfFlateDecodeParms{}, fmt.Errorf("unsupported stream: /DecodeParms PNG predictors require /BitsPerComponent >= 1")
		}
		return pdfFlateDecodeParms{}, fmt.Errorf("unsupported stream: /DecodeParms %s predictor requires /BitsPerComponent >= 1", label)
	}
	columns, ok, err := decodeParmsInteger(dict, "Columns")
	if err != nil {
		return pdfFlateDecodeParms{}, err
	}
	if !ok {
		if !defaultColumns {
			return pdfFlateDecodeParms{}, fmt.Errorf("unsupported stream: /DecodeParms is not implemented")
		}
		columns = 1
	}
	if columns < 1 {
		if label == "PNG" {
			return pdfFlateDecodeParms{}, fmt.Errorf("unsupported stream: /DecodeParms PNG predictors require /Columns >= 1")
		}
		return pdfFlateDecodeParms{}, fmt.Errorf("unsupported stream: /DecodeParms %s predictor requires /Columns >= 1", label)
	}
	return pdfFlateDecodeParms{
		predictor:        predictor,
		columns:          columns,
		colors:           colors,
		bitsPerComponent: bitsPerComponent,
	}, nil
}

func decodeParmsDictionaryNames(dict []byte) []string {
	var names []string
	for i := 2; i < len(dict)-2; i++ {
		if dict[i] != '/' {
			continue
		}
		j := i + 1
		for j < len(dict)-2 && !isPDFSpace(dict[j]) && !isPDFDelimiter(dict[j]) {
			j++
		}
		if j > i+1 {
			names = append(names, string(dict[i+1:j]))
		}
		i = j
	}
	return names
}

func decodeParmsInteger(dict []byte, name string) (int, bool, error) {
	token := []byte("/" + name)
	searchStart := 0
	for {
		at := bytes.Index(dict[searchStart:], token)
		if at == -1 {
			return 0, false, nil
		}
		at += searchStart
		i := at + len(token)
		if i < len(dict) && !isPDFSpace(dict[i]) && !isPDFDelimiter(dict[i]) {
			searchStart = i
			continue
		}
		for i < len(dict) && isPDFSpace(dict[i]) {
			i++
		}
		if i >= len(dict) || (!isPDFDigit(dict[i]) && dict[i] != '-' && dict[i] != '+') {
			return 0, true, fmt.Errorf("unsupported stream: /DecodeParms /%s must be a direct integer", name)
		}
		start := i
		if dict[i] == '-' || dict[i] == '+' {
			i++
		}
		if i >= len(dict) || !isPDFDigit(dict[i]) {
			return 0, true, fmt.Errorf("unsupported stream: /DecodeParms /%s must be a direct integer", name)
		}
		for i < len(dict) && isPDFDigit(dict[i]) {
			i++
		}
		value, err := strconv.Atoi(string(dict[start:i]))
		if err != nil {
			return 0, true, err
		}
		return value, true, nil
	}
}

func decodeFlateDecodeParms(input []byte, params pdfFlateDecodeParms) ([]byte, error) {
	if params.predictor == 1 {
		return input, nil
	}
	if params.predictor == 2 {
		rowBytes, sampleCount, err := tiffPredictorGeometry(params)
		if err != nil {
			return nil, err
		}
		return decodeTIFFRows(input, rowBytes, sampleCount, params.colors, params.bitsPerComponent)
	}
	rowBytes, err := pngPredictorRowBytes(params)
	if err != nil {
		return nil, err
	}
	bytesPerPixel, err := pngPredictorBytesPerPixel(params)
	if err != nil {
		return nil, err
	}
	return decodePNGRows(input, rowBytes, bytesPerPixel)
}

func encodeFlateDecodeParms(input []byte, params pdfFlateDecodeParms) ([]byte, error) {
	if params.predictor == 1 {
		return input, nil
	}
	if params.predictor == 2 {
		rowBytes, sampleCount, err := tiffPredictorGeometry(params)
		if err != nil {
			return nil, err
		}
		return encodeTIFFRows(input, rowBytes, sampleCount, params.colors, params.bitsPerComponent)
	}
	rowBytes, err := pngPredictorRowBytes(params)
	if err != nil {
		return nil, err
	}
	return encodePNGRows(input, rowBytes)
}

func pngPredictorRowBytes(params pdfFlateDecodeParms) (int, error) {
	rowBits, err := predictorRowBits(params, "PNG")
	if err != nil {
		return 0, err
	}
	return (rowBits + 7) / 8, nil
}

func tiffPredictorRowBytes(params pdfFlateDecodeParms) (int, error) {
	rowBits, err := predictorRowBits(params, "TIFF")
	if err != nil {
		return 0, err
	}
	return (rowBits + 7) / 8, nil
}

func tiffPredictorGeometry(params pdfFlateDecodeParms) (int, int, error) {
	rowBytes, err := tiffPredictorRowBytes(params)
	if err != nil {
		return 0, 0, err
	}
	if params.bitsPerComponent > 32 {
		return 0, 0, fmt.Errorf("unsupported stream: /DecodeParms TIFF predictor requires /BitsPerComponent <= 32")
	}
	if params.columns > int(^uint(0)>>1)/params.colors {
		return 0, 0, fmt.Errorf("TIFF predictor stream: sample count overflows")
	}
	return rowBytes, params.columns * params.colors, nil
}

func predictorRowBits(params pdfFlateDecodeParms, label string) (int, error) {
	if params.columns < 1 {
		return 0, fmt.Errorf("%s predictor stream: invalid columns", label)
	}
	if params.colors < 1 {
		return 0, fmt.Errorf("%s predictor stream: invalid colors", label)
	}
	if params.bitsPerComponent < 1 {
		return 0, fmt.Errorf("%s predictor stream: invalid bits per component", label)
	}
	if params.colors > int(^uint(0)>>1)/params.bitsPerComponent {
		return 0, fmt.Errorf("%s predictor stream: row width overflows", label)
	}
	bitsPerColumn := params.colors * params.bitsPerComponent
	if params.columns > int(^uint(0)>>1)/bitsPerColumn {
		return 0, fmt.Errorf("%s predictor stream: row width overflows", label)
	}
	return params.columns * bitsPerColumn, nil
}

func pngPredictorBytesPerPixel(params pdfFlateDecodeParms) (int, error) {
	return predictorBytesPerPixel(params, "PNG")
}

func predictorBytesPerPixel(params pdfFlateDecodeParms, label string) (int, error) {
	if params.colors < 1 {
		return 0, fmt.Errorf("%s predictor stream: invalid colors", label)
	}
	if params.bitsPerComponent < 1 {
		return 0, fmt.Errorf("%s predictor stream: invalid bits per component", label)
	}
	if params.colors > int(^uint(0)>>1)/params.bitsPerComponent {
		return 0, fmt.Errorf("%s predictor stream: bytes per pixel overflows", label)
	}
	sampleBits := params.colors * params.bitsPerComponent
	return (sampleBits + 7) / 8, nil
}

func decodePNGOneByteRows(input []byte, columns int) ([]byte, error) {
	return decodePNGRows(input, columns, 1)
}

func decodePNGRows(input []byte, rowBytes, bytesPerPixel int) ([]byte, error) {
	if rowBytes < 1 {
		return nil, fmt.Errorf("decode PNG predictor stream: invalid row width")
	}
	if bytesPerPixel < 1 {
		return nil, fmt.Errorf("decode PNG predictor stream: invalid bytes per pixel")
	}
	rowLength := rowBytes + 1
	if len(input)%rowLength != 0 {
		return nil, fmt.Errorf("decode PNG predictor stream: partial row")
	}
	output := make([]byte, 0, len(input)/rowLength*rowBytes)
	previousRow := make([]byte, rowBytes)
	for rowStart := 0; rowStart < len(input); rowStart += rowLength {
		filter := input[rowStart]
		row := make([]byte, rowBytes)
		for col := 0; col < rowBytes; col++ {
			x := input[rowStart+1+col]
			left := byte(0)
			if col >= bytesPerPixel {
				left = row[col-bytesPerPixel]
			}
			up := previousRow[col]
			upLeft := byte(0)
			if col >= bytesPerPixel {
				upLeft = previousRow[col-bytesPerPixel]
			}
			switch filter {
			case 0:
				row[col] = x
			case 1:
				row[col] = x + left
			case 2:
				row[col] = x + up
			case 3:
				row[col] = x + byte((int(left)+int(up))/2)
			case 4:
				row[col] = x + paethPredictor(left, up, upLeft)
			default:
				return nil, fmt.Errorf("decode PNG predictor stream: unsupported row filter %d", filter)
			}
		}
		output = append(output, row...)
		copy(previousRow, row)
	}
	return output, nil
}

func encodePNGOneByteRows(input []byte, columns int) ([]byte, error) {
	return encodePNGRows(input, columns)
}

func decodeTIFFRows(input []byte, rowBytes, sampleCount, colors, bitsPerComponent int) ([]byte, error) {
	if rowBytes < 1 {
		return nil, fmt.Errorf("decode TIFF predictor stream: invalid row width")
	}
	if sampleCount < 1 {
		return nil, fmt.Errorf("decode TIFF predictor stream: invalid sample count")
	}
	if colors < 1 {
		return nil, fmt.Errorf("decode TIFF predictor stream: invalid colors")
	}
	if bitsPerComponent < 1 || bitsPerComponent > 32 {
		return nil, fmt.Errorf("decode TIFF predictor stream: invalid bits per component")
	}
	if len(input)%rowBytes != 0 {
		return nil, fmt.Errorf("decode TIFF predictor stream: partial row")
	}
	output := make([]byte, 0, len(input))
	mask := tiffPredictorSampleMask(bitsPerComponent)
	for rowStart := 0; rowStart < len(input); rowStart += rowBytes {
		row := bytes.Clone(input[rowStart : rowStart+rowBytes])
		for sample := 0; sample < sampleCount; sample++ {
			value := readPackedTIFFSample(row, sample, bitsPerComponent)
			if sample >= colors {
				left := readPackedTIFFSample(row, sample-colors, bitsPerComponent)
				value = (value + left) & mask
			}
			writePackedTIFFSample(row, sample, bitsPerComponent, value)
		}
		output = append(output, row...)
	}
	return output, nil
}

func encodeTIFFRows(input []byte, rowBytes, sampleCount, colors, bitsPerComponent int) ([]byte, error) {
	if rowBytes < 1 {
		return nil, fmt.Errorf("encode TIFF predictor stream: invalid row width")
	}
	if sampleCount < 1 {
		return nil, fmt.Errorf("encode TIFF predictor stream: invalid sample count")
	}
	if colors < 1 {
		return nil, fmt.Errorf("encode TIFF predictor stream: invalid colors")
	}
	if bitsPerComponent < 1 || bitsPerComponent > 32 {
		return nil, fmt.Errorf("encode TIFF predictor stream: invalid bits per component")
	}
	if len(input)%rowBytes != 0 {
		return nil, fmt.Errorf("encode TIFF predictor stream: partial row")
	}
	output := make([]byte, 0, len(input))
	mask := tiffPredictorSampleMask(bitsPerComponent)
	for rowStart := 0; rowStart < len(input); rowStart += rowBytes {
		decodedRow := input[rowStart : rowStart+rowBytes]
		encodedRow := bytes.Clone(decodedRow)
		for sample := 0; sample < sampleCount; sample++ {
			value := readPackedTIFFSample(decodedRow, sample, bitsPerComponent)
			if sample >= colors {
				left := readPackedTIFFSample(decodedRow, sample-colors, bitsPerComponent)
				value = (value - left) & mask
			}
			writePackedTIFFSample(encodedRow, sample, bitsPerComponent, value)
		}
		output = append(output, encodedRow...)
	}
	return output, nil
}

func tiffPredictorSampleMask(bitsPerComponent int) uint64 {
	return (uint64(1) << bitsPerComponent) - 1
}

func readPackedTIFFSample(row []byte, sample, bitsPerComponent int) uint64 {
	bitOffset := sample * bitsPerComponent
	var value uint64
	for bit := 0; bit < bitsPerComponent; bit++ {
		position := bitOffset + bit
		b := row[position/8]
		shift := 7 - position%8
		value = value<<1 | uint64((b>>shift)&1)
	}
	return value
}

func writePackedTIFFSample(row []byte, sample, bitsPerComponent int, value uint64) {
	bitOffset := sample * bitsPerComponent
	for bit := 0; bit < bitsPerComponent; bit++ {
		position := bitOffset + bit
		shift := 7 - position%8
		mask := byte(1 << shift)
		if (value>>(bitsPerComponent-1-bit))&1 == 1 {
			row[position/8] |= mask
			continue
		}
		row[position/8] &^= mask
	}
}

func encodePNGRows(input []byte, rowBytes int) ([]byte, error) {
	if rowBytes < 1 {
		return nil, fmt.Errorf("encode PNG predictor stream: invalid row width")
	}
	if len(input)%rowBytes != 0 {
		return nil, fmt.Errorf("encode PNG predictor stream: partial row")
	}
	rowCount := len(input) / rowBytes
	output := make([]byte, 0, rowCount*(rowBytes+1))
	for rowStart := 0; rowStart < len(input); rowStart += rowBytes {
		output = append(output, 0)
		output = append(output, input[rowStart:rowStart+rowBytes]...)
	}
	return output, nil
}

func paethPredictor(left, up, upLeft byte) byte {
	p := int(left) + int(up) - int(upLeft)
	pa := abs(p - int(left))
	pb := abs(p - int(up))
	pc := abs(p - int(upLeft))
	if pa <= pb && pa <= pc {
		return left
	}
	if pb <= pc {
		return up
	}
	return upLeft
}

func abs(value int) int {
	if value < 0 {
		return -value
	}
	return value
}
