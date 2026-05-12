package pdf

import (
	"bytes"
	"errors"
	"fmt"
	"sort"
	"strconv"
	"strings"

	"github.com/oxhq/binas/pkg/core"
)

type pdfObjectID struct {
	Number     int
	Generation int
}

type pdfRef struct {
	ID pdfObjectID
}

type pdfName string
type pdfLiteralString string
type pdfHexString string
type pdfDecryptedString []byte
type pdfArray []pdfValue
type pdfDict map[string]pdfValue
type pdfValue any

type pdfStreamObject struct {
	Dict        pdfDict
	Data        []byte
	SourceStart int
	SourceEnd   int
}

type pdfIndirectObject struct {
	ID             pdfObjectID
	Value          pdfValue
	Offset         int
	InObjectStream bool
}

type pdfGraph struct {
	Header     string
	Objects    map[pdfObjectID]*pdfIndirectObject
	Trailer    pdfDict
	Root       *pdfObjectID
	Boundaries residualBoundarySummary
	Xref       xrefSummary
	XrefStream []pdfXrefEntry
	Encryption *pdfGraphEncryption
}

type pdfXrefEntry struct {
	ObjectNumber int
	Type         int
	Offset       int
	Generation   int
	StreamNumber int
	StreamIndex  int
}

type pdfValueParser struct {
	input []byte
	i     int
}

type pdfGraphParseOptions struct {
	AllowEncryption bool
	AllowSignature  bool
	AllowXFA        bool
	Password        string
}

type pdfCanonicalWriteOptions struct {
	AllowSignatureInvalidation bool
	AllowEncryption            bool
}

func parsePDFGraph(input []byte) (*pdfGraph, error) {
	return parsePDFGraphWithOptions(input, pdfGraphParseOptions{})
}

func parsePDFGraphWithOptions(input []byte, opts pdfGraphParseOptions) (*pdfGraph, error) {
	if !bytes.HasPrefix(input, []byte("%PDF-")) {
		return nil, errors.New("not a PDF file")
	}
	boundaries := summarizeResidualBoundariesForInput(input)
	if boundaries.HasEncryption && (!opts.AllowEncryption || opts.Password == "") {
		return nil, ErrEncryptedPDFPasswordRequired
	}
	if boundaries.HasSignature && !opts.AllowSignature {
		return nil, ErrSignedPDFRequiresInvalidation
	}
	if boundaries.HasXFA && !opts.AllowXFA {
		return nil, errors.New("unsupported PDF: XFA forms are not implemented")
	}
	graph := &pdfGraph{
		Header:     header(input),
		Objects:    make(map[pdfObjectID]*pdfIndirectObject),
		Boundaries: boundaries,
		Xref:       summarizeXref(input),
	}
	if err := graph.parseIndirectObjects(input); err != nil {
		return nil, err
	}
	graph.Trailer = parseLastTrailerDictionary(input)
	if boundaries.HasEncryption {
		encryption, err := graph.prepareStandardSecurityEncryption([]byte(opts.Password))
		if err != nil {
			return nil, err
		}
		graph.Encryption = encryption
		if err := graph.decryptStandardSecurityObjects(); err != nil {
			return nil, err
		}
	}
	if err := graph.validateHybridXrefStream(); err != nil {
		return nil, err
	}
	for _, object := range sortedPDFObjects(graph.Objects) {
		stream, ok := object.Value.(pdfStreamObject)
		if !ok || !dictHasType(stream.Dict, "ObjStm") {
			continue
		}
		if err := graph.resolveObjectStream(object, stream); err != nil {
			return nil, err
		}
	}
	for _, object := range sortedPDFObjects(graph.Objects) {
		stream, ok := object.Value.(pdfStreamObject)
		if !ok || !dictHasType(stream.Dict, "XRef") {
			continue
		}
		entries, err := parsePDFXrefStream(stream.Dict, stream.Data)
		if err != nil {
			return nil, err
		}
		graph.XrefStream = append(graph.XrefStream, entries...)
		if graph.Trailer == nil {
			graph.Trailer = clonePDFDict(stream.Dict)
		}
	}
	if root, ok := dictRef(graph.Trailer, "Root"); ok {
		graph.Root = &root.ID
	} else if root, ok := graph.findCatalogRoot(); ok {
		graph.Root = &root
	}
	return graph, nil
}

func (g *pdfGraph) validateHybridXrefStream() error {
	if g.Trailer == nil {
		return nil
	}
	if _, exists := g.Trailer["XRefStm"]; !exists {
		return nil
	}
	offset, ok := dictInt(g.Trailer, "XRefStm")
	if !ok {
		return errors.New("hybrid xref /XRefStm must be an integer offset")
	}
	if offset < 0 {
		return fmt.Errorf("hybrid xref /XRefStm offset %d is invalid", offset)
	}
	var object *pdfIndirectObject
	for _, candidate := range g.Objects {
		if candidate.Offset == offset {
			object = candidate
			break
		}
	}
	if object == nil {
		return fmt.Errorf("hybrid xref /XRefStm offset %d does not point to an indirect object", offset)
	}
	stream, ok := object.Value.(pdfStreamObject)
	if !ok || !dictHasType(stream.Dict, "XRef") {
		return fmt.Errorf("hybrid xref /XRefStm offset %d does not point to an xref stream", offset)
	}
	if _, err := parsePDFXrefStream(stream.Dict, stream.Data); err != nil {
		return fmt.Errorf("hybrid xref /XRefStm stream: %w", err)
	}
	return nil
}

func parsePDFGraphTree(input []byte, opts core.ParseOptions) (*core.Tree, error) {
	if opts.Strict && !bytes.Contains(input, []byte("%%EOF")) {
		return nil, errors.New("malformed PDF: missing EOF marker")
	}
	graph, err := parsePDFGraph(input)
	if err != nil {
		return nil, err
	}
	return graph.toTree(input), nil
}

func (g *pdfGraph) parseIndirectObjects(input []byte) error {
	objects := findXrefObjectOffsets(input)
	sort.Slice(objects, func(i, j int) bool {
		return objects[i].Offset < objects[j].Offset
	})
	for i, object := range objects {
		end := len(input)
		if i+1 < len(objects) {
			end = objects[i+1].Offset
		}
		if object.Offset < 0 || object.Offset >= end || end > len(input) {
			continue
		}
		bodyStartRel := bytes.Index(input[object.Offset:end], []byte("obj"))
		if bodyStartRel == -1 {
			continue
		}
		bodyStart := object.Offset + bodyStartRel + len("obj")
		bodyEndRel := bytes.Index(input[bodyStart:end], []byte("endobj"))
		if bodyEndRel == -1 {
			return fmt.Errorf("malformed PDF: object %d %d missing endobj", object.Number, object.Generation)
		}
		bodyEnd := bodyStart + bodyEndRel
		value, err := parsePDFIndirectObjectValue(input, bodyStart, bodyEnd)
		if err != nil {
			return fmt.Errorf("parse object %d %d: %w", object.Number, object.Generation, err)
		}
		id := pdfObjectID{Number: object.Number, Generation: object.Generation}
		g.Objects[id] = &pdfIndirectObject{ID: id, Value: value, Offset: object.Offset}
	}
	return nil
}

func parsePDFIndirectObjectValue(input []byte, bodyStart, bodyEnd int) (pdfValue, error) {
	body := input[bodyStart:bodyEnd]
	parser := pdfValueParser{input: body}
	value, err := parser.parseValue()
	if err != nil {
		return nil, err
	}
	parser.skipSpaceAndComments()
	dict, ok := value.(pdfDict)
	if !ok || !parser.hasKeyword("stream") {
		return value, nil
	}
	streamKeywordStart := parser.i
	parser.i += len("stream")
	dataStart := parser.i
	if dataStart < len(body) && body[dataStart] == '\r' {
		dataStart++
	}
	if dataStart < len(body) && body[dataStart] == '\n' {
		dataStart++
	}
	dataEnd := -1
	if length, ok := dictInt(dict, "Length"); ok && length >= 0 && dataStart+length <= len(body) {
		dataEnd = dataStart + length
	} else {
		endstream := bytes.Index(body[dataStart:], []byte("endstream"))
		if endstream == -1 {
			return nil, errors.New("stream missing endstream")
		}
		dataEnd = dataStart + endstream
		for dataEnd > dataStart && (body[dataEnd-1] == '\r' || body[dataEnd-1] == '\n') {
			dataEnd--
		}
	}
	if dataEnd < dataStart || dataEnd > len(body) {
		return nil, errors.New("stream length extends past object body")
	}
	if !bytes.Contains(body[streamKeywordStart:], []byte("endstream")) {
		return nil, errors.New("stream missing endstream")
	}
	return pdfStreamObject{
		Dict:        dict,
		Data:        bytes.Clone(body[dataStart:dataEnd]),
		SourceStart: bodyStart + dataStart,
		SourceEnd:   bodyStart + dataEnd,
	}, nil
}

func (g *pdfGraph) resolveObjectStream(container *pdfIndirectObject, stream pdfStreamObject) error {
	decoded, err := decodePDFGraphStream(stream)
	if err != nil {
		return fmt.Errorf("decode object stream %d %d: %w", container.ID.Number, container.ID.Generation, err)
	}
	n, ok := dictInt(stream.Dict, "N")
	if !ok || n < 0 {
		return fmt.Errorf("object stream %d %d missing /N", container.ID.Number, container.ID.Generation)
	}
	first, ok := dictInt(stream.Dict, "First")
	if !ok || first < 0 || first > len(decoded) {
		return fmt.Errorf("object stream %d %d has invalid /First", container.ID.Number, container.ID.Generation)
	}
	headerFields := strings.Fields(string(decoded[:first]))
	if len(headerFields) < n*2 {
		return fmt.Errorf("object stream %d %d has malformed object header", container.ID.Number, container.ID.Generation)
	}
	type entry struct {
		number int
		offset int
	}
	entries := make([]entry, 0, n)
	for i := 0; i < n; i++ {
		number, err := strconv.Atoi(headerFields[i*2])
		if err != nil {
			return fmt.Errorf("object stream %d %d object number: %w", container.ID.Number, container.ID.Generation, err)
		}
		offset, err := strconv.Atoi(headerFields[i*2+1])
		if err != nil {
			return fmt.Errorf("object stream %d %d object offset: %w", container.ID.Number, container.ID.Generation, err)
		}
		if offset < 0 || first+offset > len(decoded) {
			return fmt.Errorf("object stream %d %d object offset out of range", container.ID.Number, container.ID.Generation)
		}
		entries = append(entries, entry{number: number, offset: offset})
	}
	for i, entry := range entries {
		valueStart := first + entry.offset
		valueEnd := len(decoded)
		if i+1 < len(entries) {
			valueEnd = first + entries[i+1].offset
		}
		if valueStart > valueEnd {
			return fmt.Errorf("object stream %d %d object offsets are not ordered", container.ID.Number, container.ID.Generation)
		}
		body := bytes.TrimSpace(decoded[valueStart:valueEnd])
		if len(body) == 0 {
			continue
		}
		parser := pdfValueParser{input: body}
		value, err := parser.parseValue()
		if err != nil {
			continue
		}
		parser.skipSpaceAndComments()
		if parser.i != len(parser.input) {
			continue
		}
		id := pdfObjectID{Number: entry.number, Generation: 0}
		if _, exists := g.Objects[id]; exists {
			continue
		}
		g.Objects[id] = &pdfIndirectObject{ID: id, Value: value, InObjectStream: true}
	}
	return nil
}

func parsePDFXrefStream(dict pdfDict, encoded []byte) ([]pdfXrefEntry, error) {
	decoded, err := decodePDFGraphStream(pdfStreamObject{Dict: dict, Data: encoded})
	if err != nil {
		return nil, fmt.Errorf("decode xref stream: %w", err)
	}
	widths, ok := dictIntArray(dict, "W")
	if !ok || len(widths) != 3 {
		return nil, errors.New("xref stream missing /W [type field2 field3]")
	}
	for _, width := range widths {
		if width < 0 || width > 8 {
			return nil, errors.New("xref stream has invalid /W entry")
		}
	}
	indexes, ok := dictIntArray(dict, "Index")
	if !ok {
		size, ok := dictInt(dict, "Size")
		if !ok || size < 0 {
			return nil, errors.New("xref stream missing /Size")
		}
		indexes = []int{0, size}
	}
	if len(indexes)%2 != 0 {
		return nil, errors.New("xref stream /Index must contain start/count pairs")
	}
	rowWidth := widths[0] + widths[1] + widths[2]
	if rowWidth <= 0 {
		return nil, errors.New("xref stream row width is zero")
	}
	entries := make([]pdfXrefEntry, 0)
	pos := 0
	for i := 0; i < len(indexes); i += 2 {
		start, count := indexes[i], indexes[i+1]
		if start < 0 || count < 0 {
			return nil, errors.New("xref stream /Index values must be non-negative")
		}
		for j := 0; j < count; j++ {
			if pos+rowWidth > len(decoded) {
				return nil, errors.New("xref stream data ended before all entries")
			}
			entryType := readPDFXrefField(decoded[pos:], widths[0])
			field2 := readPDFXrefField(decoded[pos+widths[0]:], widths[1])
			field3 := readPDFXrefField(decoded[pos+widths[0]+widths[1]:], widths[2])
			pos += rowWidth
			if widths[0] == 0 {
				entryType = 1
			}
			entry := pdfXrefEntry{ObjectNumber: start + j, Type: entryType}
			switch entryType {
			case 0:
				entry.Offset = field2
				entry.Generation = field3
			case 1:
				entry.Offset = field2
				entry.Generation = field3
			case 2:
				entry.StreamNumber = field2
				entry.StreamIndex = field3
			default:
				return nil, fmt.Errorf("xref stream entry %d has unsupported type %d", start+j, entryType)
			}
			entries = append(entries, entry)
		}
	}
	if trailing := bytes.TrimSpace(decoded[pos:]); len(trailing) > 0 {
		return nil, errors.New("xref stream has trailing data")
	}
	return entries, nil
}

func readPDFXrefField(input []byte, width int) int {
	value := 0
	for i := 0; i < width; i++ {
		value = value<<8 | int(input[i])
	}
	return value
}

func (p *pdfValueParser) parseValue() (pdfValue, error) {
	p.skipSpaceAndComments()
	if p.i >= len(p.input) {
		return nil, errors.New("unexpected EOF")
	}
	switch p.input[p.i] {
	case '/':
		return p.parseName()
	case '(':
		return p.parseLiteralString()
	case '<':
		if p.i+1 < len(p.input) && p.input[p.i+1] == '<' {
			return p.parseDict()
		}
		return p.parseHexString()
	case '[':
		return p.parseArray()
	case 't':
		if p.consumeKeyword("true") {
			return true, nil
		}
	case 'f':
		if p.consumeKeyword("false") {
			return false, nil
		}
	case 'n':
		if p.consumeKeyword("null") {
			return nil, nil
		}
	default:
		if p.input[p.i] == '-' || p.input[p.i] == '+' || p.input[p.i] == '.' || isPDFDigit(p.input[p.i]) {
			return p.parseNumberOrRef()
		}
	}
	return nil, fmt.Errorf("unexpected PDF token at byte %d", p.i)
}

func (p *pdfValueParser) parseDict() (pdfDict, error) {
	if p.i+1 >= len(p.input) || p.input[p.i] != '<' || p.input[p.i+1] != '<' {
		return nil, errors.New("dictionary must start with <<")
	}
	p.i += 2
	out := make(pdfDict)
	for {
		p.skipSpaceAndComments()
		if p.i+1 < len(p.input) && p.input[p.i] == '>' && p.input[p.i+1] == '>' {
			p.i += 2
			return out, nil
		}
		if p.i >= len(p.input) {
			return nil, errors.New("dictionary missing >>")
		}
		key, err := p.parseName()
		if err != nil {
			return nil, fmt.Errorf("dictionary key: %w", err)
		}
		value, err := p.parseValue()
		if err != nil {
			return nil, fmt.Errorf("dictionary /%s value: %w", key, err)
		}
		out[string(key)] = value
	}
}

func (p *pdfValueParser) parseArray() (pdfArray, error) {
	if p.input[p.i] != '[' {
		return nil, errors.New("array must start with [")
	}
	p.i++
	out := make(pdfArray, 0)
	for {
		p.skipSpaceAndComments()
		if p.i >= len(p.input) {
			return nil, errors.New("array missing ]")
		}
		if p.input[p.i] == ']' {
			p.i++
			return out, nil
		}
		value, err := p.parseValue()
		if err != nil {
			return nil, err
		}
		out = append(out, value)
	}
}

func (p *pdfValueParser) parseName() (pdfName, error) {
	if p.i >= len(p.input) || p.input[p.i] != '/' {
		return "", errors.New("name must start with /")
	}
	start := p.i + 1
	p.i = start
	for p.i < len(p.input) && !isPDFSpace(p.input[p.i]) && !isPDFDelimiter(p.input[p.i]) {
		p.i++
	}
	if p.i == start {
		return "", errors.New("empty name")
	}
	return pdfName(p.input[start:p.i]), nil
}

func (p *pdfValueParser) parseLiteralString() (pdfLiteralString, error) {
	if p.input[p.i] != '(' {
		return "", errors.New("literal string must start with (")
	}
	closeAt, ok := findLiteralEnd(p.input, p.i+1, len(p.input))
	if !ok {
		return "", errors.New("literal string missing )")
	}
	value := pdfLiteralString(p.input[p.i+1 : closeAt])
	p.i = closeAt + 1
	return value, nil
}

func (p *pdfValueParser) parseHexString() (pdfHexString, error) {
	if p.input[p.i] != '<' {
		return "", errors.New("hex string must start with <")
	}
	closeAt, ok := findHexStringEnd(p.input, p.i+1, len(p.input))
	if !ok {
		return "", errors.New("hex string missing >")
	}
	value := pdfHexString(p.input[p.i+1 : closeAt])
	p.i = closeAt + 1
	return value, nil
}

func (p *pdfValueParser) parseNumberOrRef() (pdfValue, error) {
	start := p.i
	first, firstIsInt, err := p.parseNumber()
	if err != nil {
		return nil, err
	}
	if firstIsInt {
		save := p.i
		p.skipSpaceAndComments()
		secondStart := p.i
		second, secondIsInt, secondErr := p.parseNumber()
		if secondErr == nil && secondIsInt {
			p.skipSpaceAndComments()
			if p.i < len(p.input) && p.input[p.i] == 'R' && isPDFTokenEnd(p.input, p.i+1) {
				p.i++
				return pdfRef{ID: pdfObjectID{Number: int(first), Generation: int(second)}}, nil
			}
		}
		p.i = save
		if secondStart == save {
			p.i = save
		}
	}
	if firstIsInt {
		return int(first), nil
	}
	p.i = start
	return p.parseReal()
}

func (p *pdfValueParser) parseNumber() (float64, bool, error) {
	start := p.i
	if p.i < len(p.input) && (p.input[p.i] == '-' || p.input[p.i] == '+') {
		p.i++
	}
	dot := false
	digits := 0
	for p.i < len(p.input) {
		c := p.input[p.i]
		if isPDFDigit(c) {
			digits++
			p.i++
			continue
		}
		if c == '.' && !dot {
			dot = true
			p.i++
			continue
		}
		break
	}
	if digits == 0 {
		p.i = start
		return 0, false, errors.New("number has no digits")
	}
	raw := string(p.input[start:p.i])
	if dot {
		value, err := strconv.ParseFloat(raw, 64)
		return value, false, err
	}
	value, err := strconv.Atoi(raw)
	return float64(value), true, err
}

func (p *pdfValueParser) parseReal() (float64, error) {
	value, _, err := p.parseNumber()
	return value, err
}

func (p *pdfValueParser) skipSpaceAndComments() {
	for p.i < len(p.input) {
		if isPDFSpace(p.input[p.i]) {
			p.i++
			continue
		}
		if p.input[p.i] == '%' {
			p.i = skipPDFComment(p.input, p.i+1)
			continue
		}
		return
	}
}

func (p *pdfValueParser) hasKeyword(keyword string) bool {
	return p.i+len(keyword) <= len(p.input) &&
		string(p.input[p.i:p.i+len(keyword)]) == keyword &&
		isPDFTokenBoundary(p.input, p.i-1) &&
		isPDFTokenBoundary(p.input, p.i+len(keyword))
}

func (p *pdfValueParser) consumeKeyword(keyword string) bool {
	if !p.hasKeyword(keyword) {
		return false
	}
	p.i += len(keyword)
	return true
}

func (g *pdfGraph) toTree(input []byte) *core.Tree {
	tree := documentTree(input, g.Boundaries, g.Xref)
	cmapContext := g.cmapContext()
	if root, ok := tree.Node(tree.Root); ok {
		value := root.Value.(map[string]any)
		value["pages"] = g.pageCount()
		value["object_graph"] = map[string]any{
			"parsed":                    true,
			"canonical_rewrite_capable": true,
			"object_count":              len(g.Objects),
			"xref_stream_entries":       len(g.XrefStream),
		}
		tree.Nodes[tree.Root] = root
	}
	for _, object := range sortedPDFObjects(g.Objects) {
		stream, ok := object.Value.(pdfStreamObject)
		if !ok || dictHasType(stream.Dict, "ObjStm") || dictHasType(stream.Dict, "XRef") {
			continue
		}
		decoded, err := g.decodePDFGraphObjectStream(object.ID, stream)
		streamMeta := map[string]any{
			"object_number":     object.ID.Number,
			"object_generation": object.ID.Generation,
			"raw":               isPassthroughPDFStreamFilter(pdfGraphStreamFilterString(stream.Dict)),
			"filter":            normalizePDFStreamFilter(pdfGraphStreamFilterString(stream.Dict)),
			"decode_parms":      pdfGraphDecodeParmsString(stream.Dict),
			"canonical_graph":   true,
		}
		if object.InObjectStream {
			streamMeta["in_object_stream"] = true
		}
		if err != nil {
			streamMeta["unsupported"] = err.Error()
		}
		streamSpan := core.Span{Start: int64(stream.SourceStart), End: int64(stream.SourceEnd)}
		streamID := tree.AddNode(core.Node{
			Kind: KindStream,
			Span: streamSpan,
			Meta: streamMeta,
		})
		tree.Nodes[tree.Root].Children = append(tree.Nodes[tree.Root].Children, streamID)
		if err != nil {
			continue
		}
		parseTextShow(decoded, 0, len(decoded), tree, streamID, textShowContext{
			sourceOffset:      stream.SourceStart,
			streamSpan:        streamSpan,
			streamFilter:      pdfGraphStreamFilterString(stream.Dict),
			streamDecodeParms: pdfGraphDecodeParmsString(stream.Dict),
			streamEncoded:     bytes.Clone(stream.Data),
			decodedContent:    bytes.Clone(decoded),
			toUnicode:         cmapContext.fallback,
			fontToUnicode:     cmapContext.fontCMapsForStream(stream.SourceStart),
			fontMetrics:       cmapContext.fontMetricsForStream(stream.SourceStart),
		})
	}
	return tree
}

func (g *pdfGraph) pageCount() int {
	count := 0
	for _, object := range g.Objects {
		dict, ok := object.Value.(pdfDict)
		if ok && dictHasType(dict, "Page") {
			count++
		}
		stream, ok := object.Value.(pdfStreamObject)
		if ok && dictHasType(stream.Dict, "Page") {
			count++
		}
	}
	return count
}

func (g *pdfGraph) findCatalogRoot() (pdfObjectID, bool) {
	for _, object := range sortedPDFObjects(g.Objects) {
		dict, ok := object.Value.(pdfDict)
		if ok && dictHasType(dict, "Catalog") {
			return object.ID, true
		}
	}
	return pdfObjectID{}, false
}

type canonicalTextCandidate struct {
	Object  *pdfIndirectObject
	Stream  pdfStreamObject
	Decoded []byte
	Show    parsedTextShow
}

type parsedTextShow struct {
	Text     string
	Encoded  string
	Encoding string
	CMap     *toUnicodeCMap
	Start    int
	End      int
}

func EditCanonical(input []byte, selector core.Match, mutation core.Mutation, invariants []core.Invariant) ([]byte, core.Report, core.Verification, error) {
	return editCanonicalWithOptions(input, selector, mutation, invariants, pdfGraphParseOptions{}, pdfCanonicalWriteOptions{})
}

func EditCanonicalInvalidatingSignatures(input []byte, selector core.Match, mutation core.Mutation, invariants []core.Invariant) ([]byte, core.Report, core.Verification, error) {
	return editCanonicalWithOptions(
		input,
		selector,
		mutation,
		invariants,
		pdfGraphParseOptions{AllowSignature: true},
		pdfCanonicalWriteOptions{AllowSignatureInvalidation: true},
	)
}

func EditCanonicalWithPassword(input []byte, password string, selector core.Match, mutation core.Mutation, invariants []core.Invariant) ([]byte, core.Report, core.Verification, error) {
	return editCanonicalWithOptions(
		input,
		selector,
		mutation,
		invariants,
		pdfGraphParseOptions{AllowEncryption: true, Password: password},
		pdfCanonicalWriteOptions{AllowEncryption: true},
	)
}

func EditCanonicalWithPasswordInvalidatingSignatures(input []byte, password string, selector core.Match, mutation core.Mutation, invariants []core.Invariant) ([]byte, core.Report, core.Verification, error) {
	return editCanonicalWithOptions(
		input,
		selector,
		mutation,
		invariants,
		pdfGraphParseOptions{AllowEncryption: true, AllowSignature: true, Password: password},
		pdfCanonicalWriteOptions{AllowEncryption: true, AllowSignatureInvalidation: true},
	)
}

func editCanonicalWithOptions(input []byte, selector core.Match, mutation core.Mutation, invariants []core.Invariant, parseOpts pdfGraphParseOptions, writeOpts pdfCanonicalWriteOptions) ([]byte, core.Report, core.Verification, error) {
	if selector.Kind == "" {
		selector.Kind = KindTextShow
	}
	if selector.Kind != KindTextShow {
		return nil, core.Report{}, core.Verification{}, fmt.Errorf("canonical PDF edit supports kind=%q only", KindTextShow)
	}
	graph, err := parsePDFGraphWithOptions(input, parseOpts)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	candidates, err := graph.textShowCandidatesWithCMapContext(selector.Text, graph.cmapContext())
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	if len(candidates) == 0 {
		return nil, core.Report{}, core.Verification{}, fmt.Errorf("no nodes match kind=%q text=%q", selector.Kind, selector.Text)
	}
	index := 0
	var matchIndex *int
	if selector.MatchIndex != nil {
		if *selector.MatchIndex < 0 || *selector.MatchIndex >= len(candidates) {
			return nil, core.Report{}, core.Verification{}, fmt.Errorf("match index %d out of range for %d matches (zero-based)", *selector.MatchIndex, len(candidates))
		}
		index = *selector.MatchIndex
		selected := index
		matchIndex = &selected
	} else if len(candidates) > 1 {
		return nil, core.Report{}, core.Verification{}, fmt.Errorf("selector matched %d nodes; pass --match-index N (zero-based, 0..%d) to choose one", len(candidates), len(candidates)-1)
	}
	candidate := candidates[index]
	replacement, err := encodeCanonicalTextReplacement(candidate.Show, mutation.Replace)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	if candidate.Show.Start < 0 || candidate.Show.End < candidate.Show.Start || candidate.Show.End > len(candidate.Decoded) {
		return nil, core.Report{}, core.Verification{}, errors.New("unsafe replacement: decoded span is outside stream")
	}
	if !bytes.Equal(candidate.Decoded[candidate.Show.Start:candidate.Show.End], []byte(candidate.Show.Encoded)) {
		return nil, core.Report{}, core.Verification{}, errors.New("unsafe replacement: decoded span does not match encoded operand")
	}
	updated := replaceByteRange(candidate.Decoded, candidate.Show.Start, candidate.Show.End, []byte(replacement))
	stream := candidate.Stream
	stream.Data = updated
	stream.Dict = clonePDFDict(stream.Dict)
	delete(stream.Dict, "Filter")
	delete(stream.Dict, "DecodeParms")
	stream.Dict["Length"] = len(stream.Data)
	candidate.Object.Value = stream
	output, err := writeCanonicalPDFWithOptions(graph, writeOpts)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	if len(invariants) == 0 {
		invariants = []core.Invariant{
			core.InvariantReparse,
			core.InvariantOldGone,
			core.InvariantNewSelectable,
			core.InvariantPageUnchanged,
			core.InvariantNoFallbackUsed,
		}
	}
	plan := &core.EditPlan{
		Operation:  "pdf.canonical_content_stream_text_rewrite",
		OldText:    candidate.Show.Text,
		NewText:    mutation.Replace,
		PageCount:  graph.pageCount(),
		Invariants: invariants,
	}
	verification, err := verifyCanonicalEditOutput(output, plan, parseOpts)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	report := core.Report{
		Format:        "pdf",
		Edit:          plan.Operation,
		FallbackUsed:  false,
		NodesModified: 1,
		MatchIndex:    matchIndex,
		Invariants:    invariants,
	}
	return output, report, verification, nil
}

func verifyCanonicalEditOutput(output []byte, plan *core.EditPlan, parseOpts pdfGraphParseOptions) (core.Verification, error) {
	if !parseOpts.AllowSignature && !parseOpts.AllowEncryption && !parseOpts.AllowXFA {
		return NewAdapter().Verify(output, plan)
	}
	graph, err := parsePDFGraphWithOptions(output, parseOpts)
	if err != nil {
		return core.Verification{}, err
	}
	cmapContext := graph.cmapContext()
	oldMatches, err := graph.textShowCandidatesWithCMapContext(plan.OldText, cmapContext)
	if err != nil {
		return core.Verification{}, err
	}
	newMatches, err := graph.textShowCandidatesWithCMapContext(plan.NewText, cmapContext)
	if err != nil {
		return core.Verification{}, err
	}
	pageUnchanged := true
	if plan.PageCount > 0 {
		pageUnchanged = graph.pageCount() == plan.PageCount
	}
	return core.Verification{
		ReparseOK:      true,
		OldTextRemoved: len(oldMatches) == 0,
		NewSelectable:  len(newMatches) > 0,
		PageUnchanged:  pageUnchanged,
	}, nil
}

func (g *pdfGraph) textShowCandidates(text string) ([]canonicalTextCandidate, error) {
	return g.textShowCandidatesWithCMap(text, nil)
}

func (g *pdfGraph) textShowCandidatesWithCMap(text string, cmap *toUnicodeCMap) ([]canonicalTextCandidate, error) {
	return g.textShowCandidatesWithCMapContext(text, pdfCMapContext{fallback: cmap})
}

func (g *pdfGraph) textShowCandidatesWithCMapContext(text string, cmapContext pdfCMapContext) ([]canonicalTextCandidate, error) {
	candidates := make([]canonicalTextCandidate, 0)
	for _, object := range sortedPDFObjects(g.Objects) {
		stream, ok := object.Value.(pdfStreamObject)
		if !ok || dictHasType(stream.Dict, "ObjStm") || dictHasType(stream.Dict, "XRef") {
			continue
		}
		decoded, err := g.decodePDFGraphObjectStream(object.ID, stream)
		if err != nil {
			continue
		}
		shows := parseCanonicalTextShows(decoded, cmapContext.fallback, cmapContext.fontCMapsForStream(stream.SourceStart))
		for _, show := range shows {
			if text != "" && show.Text != text {
				continue
			}
			candidates = append(candidates, canonicalTextCandidate{
				Object:  object,
				Stream:  stream,
				Decoded: decoded,
				Show:    show,
			})
		}
	}
	return candidates, nil
}

func parseCanonicalTextShows(input []byte, cmap *toUnicodeCMap, fontCMaps map[string]*toUnicodeCMap) []parsedTextShow {
	shows := make([]parsedTextShow, 0)
	ctx := textShowContext{toUnicode: cmap, fontToUnicode: fontCMaps}
	activeFont := ""
	for i := 0; i < len(input); i++ {
		if font, next, ok := nextSetFontOperator(input, i, len(input)); ok {
			activeFont = font
			i = next - 1
			continue
		}
		var (
			closeAt      int
			operandEnd   int
			operandStart int
			encoded      string
			decoded      string
			encoding     string
			ok           bool
		)
		switch input[i] {
		case '[':
			arrayDecoded, arrayEncoded, arrayEnd, arrayUsedCMap, arrayOK := parseSimpleTJArrayText(input, i, len(input), ctx.cmapForFont(activeFont))
			if !arrayOK {
				continue
			}
			op, _ := nextOperator(input, arrayEnd, len(input))
			if op != "TJ" {
				i = arrayEnd - 1
				continue
			}
			closeAt = arrayEnd
			operandEnd = arrayEnd
			operandStart = i
			encoded = arrayEncoded
			decoded = arrayDecoded
			encoding = "tj-array"
			if arrayUsedCMap {
				encoding = "tj-array-cmap"
			}
		case '(':
			closeAt, ok = findLiteralEnd(input, i+1, len(input))
			if !ok {
				continue
			}
			operandEnd = closeAt + 1
			operandStart = i + 1
			encoded = string(input[operandStart:closeAt])
			decoded = decodeLiteralString(encoded)
			encoding = "literal"
		case '<':
			if i+1 < len(input) && input[i+1] == '<' {
				continue
			}
			closeAt, ok = findHexStringEnd(input, i+1, len(input))
			if !ok {
				continue
			}
			operandEnd = closeAt + 1
			operandStart = i + 1
			encoded = string(input[operandStart:closeAt])
			var usedCMap bool
			activeCMap := ctx.cmapForFont(activeFont)
			decoded, usedCMap, ok = decodeHexTextStringWithCMap([]byte(encoded), activeCMap)
			if !ok {
				i = closeAt
				continue
			}
			encoding = "hex"
			if usedCMap {
				encoding = "hex-cmap"
			}
		default:
			continue
		}
		op, _ := nextOperator(input, operandEnd, len(input))
		if op == "" {
			i = closeAt
			continue
		}
		shows = append(shows, parsedTextShow{
			Text:     decoded,
			Encoded:  encoded,
			Encoding: encoding,
			CMap:     ctx.cmapForFont(activeFont),
			Start:    operandStart,
			End:      closeAt,
		})
		i = closeAt
	}
	return shows
}

func encodeCanonicalTextReplacement(show parsedTextShow, replacement string) (string, error) {
	switch show.Encoding {
	case "", "literal":
		return encodeLiteralString(replacement), nil
	case "tj-array":
		return "[(" + encodeLiteralString(replacement) + ")]", nil
	case "tj-array-cmap":
		encoded, ok := show.CMap.EncodeHex(replacement)
		if !ok {
			return "", errors.New("replacement for ToUnicode TJ array text is not representable by the CMap")
		}
		return "[<" + encoded + ">]", nil
	case "hex":
		return encodeHexTextString(replacement)
	case "hex-cmap":
		encoded, ok := show.CMap.EncodeHex(replacement)
		if !ok {
			return "", errors.New("replacement for ToUnicode hex text show operand is not representable by the CMap")
		}
		return encoded, nil
	default:
		return "", fmt.Errorf("unsupported text show operand encoding %q", show.Encoding)
	}
}

func writeCanonicalPDF(graph *pdfGraph) ([]byte, error) {
	return writeCanonicalPDFWithOptions(graph, pdfCanonicalWriteOptions{})
}

func writeCanonicalPDFWithOptions(graph *pdfGraph, opts pdfCanonicalWriteOptions) ([]byte, error) {
	if graph.Boundaries.HasEncryption {
		if !opts.AllowEncryption {
			return nil, ErrEncryptedPDFPasswordRequired
		}
		if graph.Encryption == nil || graph.Encryption.security == nil {
			return nil, unsupportedPDFEncryption("canonical encrypted rewrite requires an authenticated Standard Security graph")
		}
	}
	if graph.Boundaries.HasSignature && !opts.AllowSignatureInvalidation {
		return nil, ErrSignedPDFRequiresInvalidation
	}
	objects := make([]*pdfIndirectObject, 0, len(graph.Objects))
	maxObject := 0
	hasNonZeroGeneration := false
	for _, object := range sortedPDFObjects(graph.Objects) {
		if stream, ok := object.Value.(pdfStreamObject); ok && (dictHasType(stream.Dict, "ObjStm") || dictHasType(stream.Dict, "XRef")) {
			continue
		}
		if object.ID.Generation != 0 {
			hasNonZeroGeneration = true
		}
		objects = append(objects, object)
		if object.ID.Number > maxObject {
			maxObject = object.ID.Number
		}
	}
	var out bytes.Buffer
	if graph.Header != "" {
		out.WriteString(graph.Header)
	} else {
		out.WriteString("%PDF-1.7")
	}
	out.WriteString("\n")
	offsets := make(map[pdfObjectID]int, len(objects))
	for _, object := range objects {
		offsets[object.ID] = out.Len()
		fmt.Fprintf(&out, "%d %d obj\n", object.ID.Number, object.ID.Generation)
		value := object.Value
		if graph.Boundaries.HasEncryption && (graph.Encryption.encryptObject == nil || object.ID != *graph.Encryption.encryptObject) {
			encrypted, err := encryptPDFObjectValue(graph.Encryption.security, graph.Encryption.fileKey, object.ID, value)
			if err != nil {
				return nil, fmt.Errorf("encrypt object %d %d: %w", object.ID.Number, object.ID.Generation, err)
			}
			value = encrypted
		}
		if err := writePDFValue(&out, value); err != nil {
			return nil, fmt.Errorf("write object %d %d: %w", object.ID.Number, object.ID.Generation, err)
		}
		out.WriteString("\nendobj\n")
	}
	xrefOffset := out.Len()
	if hasNonZeroGeneration {
		out.WriteString("xref\n")
		out.WriteString("0 1\n0000000000 65535 f \n")
		for _, object := range objects {
			offset := offsets[object.ID]
			fmt.Fprintf(&out, "%d 1\n%010d %05d n \n", object.ID.Number, offset, object.ID.Generation)
		}
	} else {
		fmt.Fprintf(&out, "xref\n0 %d\n", maxObject+1)
		out.WriteString("0000000000 65535 f \n")
		for i := 1; i <= maxObject; i++ {
			offset, ok := offsets[pdfObjectID{Number: i, Generation: 0}]
			if !ok {
				out.WriteString("0000000000 65535 f \n")
				continue
			}
			fmt.Fprintf(&out, "%010d 00000 n \n", offset)
		}
	}
	trailer := canonicalTrailer(graph, maxObject+1, graph.Boundaries.HasEncryption)
	out.WriteString("trailer\n")
	if err := writePDFValue(&out, trailer); err != nil {
		return nil, err
	}
	out.WriteString("\nstartxref\n")
	out.WriteString(strconv.Itoa(xrefOffset))
	out.WriteString("\n%%EOF\n")
	return out.Bytes(), nil
}

func canonicalTrailer(graph *pdfGraph, size int, preserveEncrypt bool) pdfDict {
	trailer := clonePDFDict(graph.Trailer)
	if trailer == nil {
		trailer = make(pdfDict)
	}
	delete(trailer, "Prev")
	delete(trailer, "XRefStm")
	if !preserveEncrypt {
		delete(trailer, "Encrypt")
	}
	delete(trailer, "Length")
	delete(trailer, "Filter")
	delete(trailer, "DecodeParms")
	delete(trailer, "Type")
	delete(trailer, "W")
	delete(trailer, "Index")
	trailer["Size"] = size
	if _, ok := trailer["Root"]; !ok && graph.Root != nil {
		trailer["Root"] = pdfRef{ID: *graph.Root}
	}
	return trailer
}

func writePDFValue(out *bytes.Buffer, value pdfValue) error {
	switch v := value.(type) {
	case nil:
		out.WriteString("null")
	case bool:
		if v {
			out.WriteString("true")
		} else {
			out.WriteString("false")
		}
	case int:
		out.WriteString(strconv.Itoa(v))
	case int64:
		out.WriteString(strconv.FormatInt(v, 10))
	case float64:
		out.WriteString(strconv.FormatFloat(v, 'f', -1, 64))
	case pdfName:
		out.WriteByte('/')
		out.WriteString(string(v))
	case pdfLiteralString:
		out.WriteByte('(')
		out.WriteString(string(v))
		out.WriteByte(')')
	case pdfHexString:
		out.WriteByte('<')
		out.WriteString(string(v))
		out.WriteByte('>')
	case pdfRef:
		fmt.Fprintf(out, "%d %d R", v.ID.Number, v.ID.Generation)
	case pdfArray:
		out.WriteByte('[')
		for i, item := range v {
			if i > 0 {
				out.WriteByte(' ')
			}
			if err := writePDFValue(out, item); err != nil {
				return err
			}
		}
		out.WriteByte(']')
	case pdfDict:
		writePDFDict(out, v)
	case pdfStreamObject:
		dict := clonePDFDict(v.Dict)
		dict["Length"] = len(v.Data)
		writePDFDict(out, dict)
		out.WriteString("\nstream\n")
		out.Write(v.Data)
		out.WriteString("\nendstream")
	default:
		return fmt.Errorf("unsupported PDF value type %T", value)
	}
	return nil
}

func writePDFDict(out *bytes.Buffer, dict pdfDict) {
	out.WriteString("<<")
	keys := make([]string, 0, len(dict))
	for key := range dict {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	for _, key := range keys {
		out.WriteByte(' ')
		out.WriteByte('/')
		out.WriteString(key)
		out.WriteByte(' ')
		_ = writePDFValue(out, dict[key])
	}
	if len(keys) > 0 {
		out.WriteByte(' ')
	}
	out.WriteString(">>")
}

func parseLastTrailerDictionary(input []byte) pdfDict {
	trailerAt := bytes.LastIndex(input, []byte("trailer"))
	if trailerAt == -1 {
		return nil
	}
	dictStartRel := bytes.Index(input[trailerAt:], []byte("<<"))
	if dictStartRel == -1 {
		return nil
	}
	dictStart := trailerAt + dictStartRel
	dictEnd, ok := findDictionaryEnd(input, dictStart)
	if !ok {
		return nil
	}
	parser := pdfValueParser{input: input[dictStart:dictEnd]}
	value, err := parser.parseValue()
	if err != nil {
		return nil
	}
	dict, _ := value.(pdfDict)
	return dict
}

func sortedPDFObjects(objects map[pdfObjectID]*pdfIndirectObject) []*pdfIndirectObject {
	out := make([]*pdfIndirectObject, 0, len(objects))
	for _, object := range objects {
		out = append(out, object)
	}
	sort.Slice(out, func(i, j int) bool {
		if out[i].ID.Number != out[j].ID.Number {
			return out[i].ID.Number < out[j].ID.Number
		}
		return out[i].ID.Generation < out[j].ID.Generation
	})
	return out
}

func clonePDFDict(in pdfDict) pdfDict {
	if in == nil {
		return nil
	}
	out := make(pdfDict, len(in))
	for key, value := range in {
		out[key] = value
	}
	return out
}

func decodePDFGraphStream(stream pdfStreamObject) ([]byte, error) {
	return decodeStreamFilterWithDecodeParms(pdfGraphStreamFilterString(stream.Dict), pdfGraphDecodeParmsString(stream.Dict), stream.Data)
}

func (g *pdfGraph) decodePDFGraphObjectStream(id pdfObjectID, stream pdfStreamObject) ([]byte, error) {
	return decodeStreamFilterWithDecodeParmsAndCrypt(
		pdfGraphStreamFilterString(stream.Dict),
		pdfGraphDecodeParmsString(stream.Dict),
		stream.Data,
		g.streamCryptHandler(id),
	)
}

func pdfGraphStreamFilterString(dict pdfDict) string {
	value, ok := dict["Filter"]
	if !ok {
		return ""
	}
	switch v := value.(type) {
	case pdfName:
		return "/" + string(v)
	case pdfArray:
		parts := make([]string, 0, len(v))
		for _, item := range v {
			name, ok := item.(pdfName)
			if !ok {
				return ""
			}
			parts = append(parts, "/"+string(name))
		}
		return "[" + strings.Join(parts, " ") + "]"
	default:
		return ""
	}
}

func pdfGraphDecodeParmsString(dict pdfDict) string {
	value, ok := dict["DecodeParms"]
	if !ok {
		return ""
	}
	var out bytes.Buffer
	if err := writePDFValue(&out, value); err != nil {
		return ""
	}
	return out.String()
}

func dictHasType(dict pdfDict, name string) bool {
	got, ok := dict["Type"].(pdfName)
	return ok && string(got) == name
}

func dictInt(dict pdfDict, key string) (int, bool) {
	value, ok := dict[key]
	if !ok {
		return 0, false
	}
	switch v := value.(type) {
	case int:
		return v, true
	case int64:
		return int(v), true
	case float64:
		return int(v), v == float64(int(v))
	default:
		return 0, false
	}
}

func dictIntArray(dict pdfDict, key string) ([]int, bool) {
	value, ok := dict[key].(pdfArray)
	if !ok {
		return nil, false
	}
	out := make([]int, 0, len(value))
	for _, item := range value {
		switch v := item.(type) {
		case int:
			out = append(out, v)
		case float64:
			if v != float64(int(v)) {
				return nil, false
			}
			out = append(out, int(v))
		default:
			return nil, false
		}
	}
	return out, true
}

func dictRef(dict pdfDict, key string) (pdfRef, bool) {
	value, ok := dict[key].(pdfRef)
	return value, ok
}

func treeNeedsCanonicalRewrite(tree *core.Tree) bool {
	if tree == nil {
		return false
	}
	root, ok := tree.Node(tree.Root)
	if !ok {
		return false
	}
	value, ok := root.Value.(map[string]any)
	if !ok {
		return false
	}
	xref, ok := value["xref"].(map[string]any)
	if !ok {
		return false
	}
	return pdfBoolMetadata(xref, "has_stream") || pdfBoolMetadata(xref, "has_object_stream")
}

func NeedsCanonicalRewrite(tree *core.Tree) bool {
	return treeNeedsCanonicalRewrite(tree)
}

func pdfBoolMetadata(values map[string]any, key string) bool {
	value, ok := values[key]
	if !ok {
		return false
	}
	got, ok := value.(bool)
	return ok && got
}

func parseXrefStreamEntries(input []byte, objects []xrefObjectOffset) ([]xrefObjectOffset, error) {
	xrefStreams := findXrefStreamObjects(input, actualXrefOffsets(objects))
	if len(xrefStreams) == 0 {
		return nil, nil
	}
	entries := make([]xrefObjectOffset, 0)
	for _, object := range xrefStreams {
		value, err := parsePDFObjectValueAt(input, object, objects)
		if err != nil {
			return nil, err
		}
		stream, ok := value.(pdfStreamObject)
		if !ok {
			return nil, fmt.Errorf("xref stream object %d %d is not a stream", object.Number, object.Generation)
		}
		xrefEntries, err := parsePDFXrefStream(stream.Dict, stream.Data)
		if err != nil {
			return nil, err
		}
		for _, entry := range xrefEntries {
			switch entry.Type {
			case 0:
				continue
			case 1:
				entries = append(entries, xrefObjectOffset{
					Number:     entry.ObjectNumber,
					Generation: entry.Generation,
					Offset:     entry.Offset,
				})
			case 2:
				entries = append(entries, xrefObjectOffset{
					Number:             entry.ObjectNumber,
					Generation:         0,
					Offset:             -1,
					Compressed:         true,
					ObjectStreamNumber: entry.StreamNumber,
					ObjectStreamIndex:  entry.StreamIndex,
				})
			}
		}
	}
	return entries, nil
}

func parseObjectStreamEntries(input []byte, objects []xrefObjectOffset) ([]xrefObjectOffset, error) {
	objectStreams := findObjectStreamObjects(input, actualXrefOffsets(objects))
	if len(objectStreams) == 0 {
		return nil, nil
	}
	entries := make([]xrefObjectOffset, 0)
	for _, object := range objectStreams {
		value, err := parsePDFObjectValueAt(input, object, objects)
		if err != nil {
			return nil, err
		}
		stream, ok := value.(pdfStreamObject)
		if !ok {
			return nil, fmt.Errorf("object stream object %d %d is not a stream", object.Number, object.Generation)
		}
		decoded, err := decodePDFGraphStream(stream)
		if err != nil {
			return nil, err
		}
		n, ok := dictInt(stream.Dict, "N")
		if !ok || n < 0 {
			return nil, fmt.Errorf("object stream %d %d missing /N", object.Number, object.Generation)
		}
		first, ok := dictInt(stream.Dict, "First")
		if !ok || first < 0 || first > len(decoded) {
			return nil, fmt.Errorf("object stream %d %d has invalid /First", object.Number, object.Generation)
		}
		fields := strings.Fields(string(decoded[:first]))
		if len(fields) < n*2 {
			return nil, fmt.Errorf("object stream %d %d has malformed object header", object.Number, object.Generation)
		}
		for i := 0; i < n; i++ {
			number, err := strconv.Atoi(fields[i*2])
			if err != nil {
				return nil, err
			}
			offset, err := strconv.Atoi(fields[i*2+1])
			if err != nil {
				return nil, err
			}
			if offset < 0 || first+offset > len(decoded) {
				return nil, fmt.Errorf("object stream %d %d object offset out of range", object.Number, object.Generation)
			}
			sourceOffset := stream.SourceStart + skipPDFSpaceAndComments(decoded, first+offset)
			if _, filtered := stream.Dict["Filter"]; filtered {
				sourceOffset = -1
			} else {
				for sourceOffset >= 0 && sourceOffset < stream.SourceStart+len(stream.Data) && sourceOffset < len(input) && isPDFSpace(input[sourceOffset]) {
					sourceOffset++
				}
			}
			entries = append(entries, xrefObjectOffset{
				Number:             number,
				Generation:         0,
				Offset:             sourceOffset,
				Compressed:         true,
				ObjectStreamNumber: object.Number,
				ObjectStreamIndex:  i,
			})
		}
	}
	return entries, nil
}

func actualXrefOffsets(objects []xrefObjectOffset) []xrefObjectOffset {
	out := make([]xrefObjectOffset, 0, len(objects))
	for _, object := range objects {
		if object.Compressed || object.Offset < 0 {
			continue
		}
		out = append(out, object)
	}
	return out
}

func parsePDFObjectValueAt(input []byte, object xrefObjectOffset, objects []xrefObjectOffset) (pdfValue, error) {
	end := len(input)
	for _, candidate := range actualXrefOffsets(objects) {
		if candidate.Offset > object.Offset && candidate.Offset < end {
			end = candidate.Offset
		}
	}
	if object.Offset < 0 || object.Offset >= end || end > len(input) {
		return nil, fmt.Errorf("object %d %d offset is out of range", object.Number, object.Generation)
	}
	bodyStartRel := bytes.Index(input[object.Offset:end], []byte("obj"))
	if bodyStartRel == -1 {
		return nil, fmt.Errorf("object %d %d missing obj marker", object.Number, object.Generation)
	}
	bodyStart := object.Offset + bodyStartRel + len("obj")
	bodyEndRel := bytes.Index(input[bodyStart:end], []byte("endobj"))
	if bodyEndRel == -1 {
		return nil, fmt.Errorf("object %d %d missing endobj marker", object.Number, object.Generation)
	}
	return parsePDFIndirectObjectValue(input, bodyStart, bodyStart+bodyEndRel)
}
