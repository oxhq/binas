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
	pdfFilterFlateDecode     = "FlateDecode"
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
		case pdfFilterFlateDecode:
			output, err = decodeFlateDecode(output)
			if err == nil && params[i].flate != nil {
				output, err = decodeFlateDecodeParms(output, *params[i].flate)
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
		case pdfFilterFlateDecode:
			if params[i].flate != nil {
				output, err = encodeFlateDecodeParms(output, *params[i].flate)
				if err != nil {
					return nil, err
				}
			}
			output, err = encodeFlateDecode(output)
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
	return strings.TrimSpace(filter)
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
	if len(filters) == 1 {
		return filters[0] == pdfFilterFlateDecode ||
			filters[0] == pdfFilterASCII85Decode ||
			filters[0] == pdfFilterASCIIHexDecode ||
			filters[0] == pdfFilterRunLengthDecode
	}
	if len(filters) == 2 {
		return (filters[0] == pdfFilterASCII85Decode ||
			filters[0] == pdfFilterASCIIHexDecode ||
			filters[0] == pdfFilterRunLengthDecode) &&
			filters[1] == pdfFilterFlateDecode
	}
	return len(filters) == 3 &&
		(((filters[0] == pdfFilterASCII85Decode ||
			filters[0] == pdfFilterASCIIHexDecode) &&
			filters[1] == pdfFilterRunLengthDecode) ||
			(filters[0] == pdfFilterRunLengthDecode &&
				(filters[1] == pdfFilterASCII85Decode ||
					filters[1] == pdfFilterASCIIHexDecode))) &&
		filters[2] == pdfFilterFlateDecode
}

type pdfStreamDecodeParms struct {
	flate *pdfFlateDecodeParms
}

type pdfFlateDecodeParms struct {
	predictor        int
	columns          int
	colors           int
	bitsPerComponent int
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
		if len(filters) != 1 || filters[0] != pdfFilterFlateDecode {
			return nil, fmt.Errorf("unsupported stream: direct /DecodeParms dictionary only supports /FlateDecode")
		}
		flate, err := parseFlateDecodeParmsDictionary(decodeParms)
		if err != nil {
			return nil, err
		}
		params[0].flate = &flate
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
		if len(filters) == 1 && filters[0] == pdfFilterFlateDecode && values[0] != "null" {
			flate, err := parseFlateDecodeParmsDictionary(values[0])
			if err != nil {
				return nil, err
			}
			params[0].flate = &flate
			return params, nil
		}
		if len(filters) == 2 &&
			(filters[0] == pdfFilterASCII85Decode || filters[0] == pdfFilterASCIIHexDecode || filters[0] == pdfFilterRunLengthDecode) &&
			filters[1] == pdfFilterFlateDecode {
			if values[0] != "null" {
				return nil, fmt.Errorf("unsupported stream: /DecodeParms for /%s must be null", filters[0])
			}
			if values[1] != "null" {
				flate, err := parseFlateDecodeParmsDictionary(values[1])
				if err != nil {
					return nil, err
				}
				params[1].flate = &flate
			}
			return params, nil
		}
		if len(filters) == 3 &&
			(((filters[0] == pdfFilterASCII85Decode || filters[0] == pdfFilterASCIIHexDecode) &&
				filters[1] == pdfFilterRunLengthDecode) ||
				(filters[0] == pdfFilterRunLengthDecode &&
					(filters[1] == pdfFilterASCII85Decode || filters[1] == pdfFilterASCIIHexDecode))) &&
			filters[2] == pdfFilterFlateDecode {
			if values[0] != "null" {
				return nil, fmt.Errorf("unsupported stream: /DecodeParms for /%s must be null", filters[0])
			}
			if values[1] != "null" {
				return nil, fmt.Errorf("unsupported stream: /DecodeParms for /%s must be null", filters[1])
			}
			if values[2] != "null" {
				flate, err := parseFlateDecodeParmsDictionary(values[2])
				if err != nil {
					return nil, err
				}
				params[2].flate = &flate
			}
			return params, nil
		}
		return nil, fmt.Errorf("unsupported stream: /DecodeParms array shape is not supported")
	}
	return nil, fmt.Errorf("unsupported stream: /DecodeParms must be a dictionary, array, or null")
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
	predictor, ok, err := decodeParmsInteger(dict, "Predictor")
	if err != nil {
		return pdfFlateDecodeParms{}, err
	}
	if !ok {
		return pdfFlateDecodeParms{}, fmt.Errorf("unsupported stream: /DecodeParms /Predictor is missing")
	}
	if predictor == 1 {
		if err := validateFlateDecodeParmsPredictor1Defaults(dict); err != nil {
			return pdfFlateDecodeParms{}, err
		}
		return pdfFlateDecodeParms{predictor: predictor}, nil
	}
	if predictor == 2 {
		params, err := parseFlateDecodeParmsPredictorGeometry(dict, predictor, "TIFF", true)
		if err != nil {
			return pdfFlateDecodeParms{}, err
		}
		if _, err := tiffPredictorRowBytes(params); err != nil {
			return pdfFlateDecodeParms{}, err
		}
		return params, nil
	}
	if predictor < 10 || predictor > 15 {
		return pdfFlateDecodeParms{}, fmt.Errorf("unsupported stream: /DecodeParms is not implemented")
	}
	return parseFlateDecodeParmsPredictorGeometry(dict, predictor, "PNG", true)
}

func validateFlateDecodeParmsPredictor1Defaults(dict []byte) error {
	defaults := map[string]int{
		"Columns":          1,
		"Colors":           1,
		"BitsPerComponent": 8,
	}
	for name, want := range defaults {
		value, ok, err := decodeParmsInteger(dict, name)
		if err != nil {
			return err
		}
		if ok && value != want {
			return fmt.Errorf("unsupported stream: /DecodeParms is not implemented")
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
	if label == "PNG" && !decodeParmsHasInteger(dict, "Columns") && colors != 1 {
		return pdfFlateDecodeParms{}, fmt.Errorf("unsupported stream: /DecodeParms PNG predictors require /Colors 1")
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
		if i >= len(dict) || !isPDFDigit(dict[i]) {
			return 0, true, fmt.Errorf("unsupported stream: /DecodeParms /%s must be a direct integer", name)
		}
		start := i
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

func decodeParmsHasInteger(dict []byte, name string) bool {
	_, ok, err := decodeParmsInteger(dict, name)
	return ok && err == nil
}

func decodeFlateDecodeParms(input []byte, params pdfFlateDecodeParms) ([]byte, error) {
	if params.predictor == 1 {
		return input, nil
	}
	if params.predictor == 2 {
		rowBytes, err := tiffPredictorRowBytes(params)
		if err != nil {
			return nil, err
		}
		bytesPerPixel, err := tiffPredictorBytesPerPixel(params)
		if err != nil {
			return nil, err
		}
		return decodeTIFFRows(input, rowBytes, bytesPerPixel)
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
		rowBytes, err := tiffPredictorRowBytes(params)
		if err != nil {
			return nil, err
		}
		bytesPerPixel, err := tiffPredictorBytesPerPixel(params)
		if err != nil {
			return nil, err
		}
		return encodeTIFFRows(input, rowBytes, bytesPerPixel)
	}
	rowBytes, err := pngPredictorRowBytes(params)
	if err != nil {
		return nil, err
	}
	return encodePNGRows(input, rowBytes)
}

func pngPredictorRowBytes(params pdfFlateDecodeParms) (int, error) {
	return predictorRowBytes(params, "PNG")
}

func tiffPredictorRowBytes(params pdfFlateDecodeParms) (int, error) {
	return predictorRowBytes(params, "TIFF")
}

func predictorRowBytes(params pdfFlateDecodeParms, label string) (int, error) {
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
	rowBits := params.columns * bitsPerColumn
	if rowBits%8 != 0 {
		return 0, fmt.Errorf("unsupported stream: /DecodeParms %s predictor row width must be byte-aligned", label)
	}
	return rowBits / 8, nil
}

func pngPredictorBytesPerPixel(params pdfFlateDecodeParms) (int, error) {
	return predictorBytesPerPixel(params, "PNG")
}

func tiffPredictorBytesPerPixel(params pdfFlateDecodeParms) (int, error) {
	return predictorBytesPerPixel(params, "TIFF")
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

func decodeTIFFRows(input []byte, rowBytes, bytesPerPixel int) ([]byte, error) {
	if rowBytes < 1 {
		return nil, fmt.Errorf("decode TIFF predictor stream: invalid row width")
	}
	if bytesPerPixel < 1 {
		return nil, fmt.Errorf("decode TIFF predictor stream: invalid bytes per pixel")
	}
	if len(input)%rowBytes != 0 {
		return nil, fmt.Errorf("decode TIFF predictor stream: partial row")
	}
	output := make([]byte, 0, len(input))
	for rowStart := 0; rowStart < len(input); rowStart += rowBytes {
		row := bytes.Clone(input[rowStart : rowStart+rowBytes])
		for col := bytesPerPixel; col < rowBytes; col++ {
			row[col] += row[col-bytesPerPixel]
		}
		output = append(output, row...)
	}
	return output, nil
}

func encodeTIFFRows(input []byte, rowBytes, bytesPerPixel int) ([]byte, error) {
	if rowBytes < 1 {
		return nil, fmt.Errorf("encode TIFF predictor stream: invalid row width")
	}
	if bytesPerPixel < 1 {
		return nil, fmt.Errorf("encode TIFF predictor stream: invalid bytes per pixel")
	}
	if len(input)%rowBytes != 0 {
		return nil, fmt.Errorf("encode TIFF predictor stream: partial row")
	}
	output := make([]byte, 0, len(input))
	for rowStart := 0; rowStart < len(input); rowStart += rowBytes {
		row := input[rowStart : rowStart+rowBytes]
		for col, decoded := range row {
			if col < bytesPerPixel {
				output = append(output, decoded)
				continue
			}
			output = append(output, decoded-row[col-bytesPerPixel])
		}
	}
	return output, nil
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
