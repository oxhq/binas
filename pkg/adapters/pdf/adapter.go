package pdf

import (
	"bytes"
	"errors"
	"fmt"
	"regexp"
	"strconv"
	"strings"

	"github.com/oxhq/binas/pkg/core"
)

const (
	KindDocument = "pdf.document"
	KindStream   = "pdf.stream"
	KindTextShow = "pdf.content.text_show"
)

type Adapter struct{}

func NewAdapter() Adapter {
	return Adapter{}
}

func ParseWithPassword(input []byte, opts core.ParseOptions, password string) (*core.Tree, error) {
	return ParseWithSecurityOptions(input, opts, SecurityOptions{Password: password})
}

func ParseWithSecurityOptions(input []byte, opts core.ParseOptions, security SecurityOptions) (*core.Tree, error) {
	if strings.TrimSpace(security.Password) == "" &&
		security.SignatureInvalidation != SignatureInvalidationInvalidate &&
		security.SignatureInvalidation != SignatureInvalidationPreserveIncremental {
		return NewAdapter().Parse(input, opts)
	}
	if !bytes.HasPrefix(input, []byte("%PDF-")) {
		return nil, errors.New("not a PDF file")
	}
	if opts.Strict && !bytes.Contains(input, []byte("%%EOF")) {
		return nil, errors.New("malformed PDF: missing EOF marker")
	}
	parseOpts := pdfGraphParseOptions{
		AllowEncryption: strings.TrimSpace(security.Password) != "",
		AllowSignature:  security.SignatureInvalidation == SignatureInvalidationInvalidate || security.SignatureInvalidation == SignatureInvalidationPreserveIncremental,
		Password:        security.Password,
	}
	graph, err := parsePDFGraphWithOptions(input, parseOpts)
	if err != nil {
		return nil, err
	}
	tree := graph.toTree(input)
	enrichPDFStreamNodeMetadata(tree)
	return tree, nil
}

func (Adapter) Detect(input []byte) (core.Confidence, error) {
	if bytes.HasPrefix(input, []byte("%PDF-")) {
		return 1, nil
	}
	return 0, nil
}

func (Adapter) Parse(input []byte, opts core.ParseOptions) (*core.Tree, error) {
	if !bytes.HasPrefix(input, []byte("%PDF-")) {
		return nil, errors.New("not a PDF file")
	}
	if opts.Strict && !bytes.Contains(input, []byte("%%EOF")) {
		return nil, errors.New("malformed PDF: missing EOF marker")
	}
	boundaries := summarizeResidualBoundariesForInput(input)
	if err := rejectUnsupportedSecurityBoundaries(boundaries); err != nil {
		return nil, err
	}
	if boundaries.HasXFA {
		return nil, errors.New("unsupported PDF: XFA forms are not implemented")
	}
	xref := summarizeXref(input)
	tree := documentTree(input, boundaries, xref)
	if xref.UnsupportedXrefStream {
		return tree, errors.New("unsupported PDF: xref streams are not implemented")
	}
	if xref.HasStream {
		graphTree, err := parsePDFGraphTree(input, opts)
		if graphTree != nil {
			enrichPDFStreamNodeMetadata(graphTree)
			return graphTree, err
		}
		if err != nil {
			return tree, err
		}
	}
	if xref.HasObjectStream {
		graphTree, err := parsePDFGraphTree(input, opts)
		if graphTree != nil {
			enrichPDFStreamNodeMetadata(graphTree)
			return graphTree, err
		}
		if err != nil {
			return tree, err
		}
	}

	cmapContext := pdfCMapContextForInput(input, pdfGraphParseOptions{})
	if err := parseStreams(input, tree, tree.Root, cmapContext); err != nil {
		return nil, err
	}
	enrichPDFStreamNodeMetadata(tree)
	return tree, nil
}

func documentTree(input []byte, boundaries residualBoundarySummary, xref xrefSummary) *core.Tree {
	tree := &core.Tree{Format: "pdf"}
	hybridStreamObject := any(nil)
	if xref.HasHybridStream && xref.HybridStreamObject.Number > 0 {
		hybridStreamObject = xref.HybridStreamObject
	}
	tree.Root = tree.AddNode(core.Node{
		Kind: KindDocument,
		Span: core.Span{Start: 0, End: int64(len(input))},
		Value: map[string]any{
			"header": header(input),
			"size":   len(input),
			"pages":  countPages(input),
			"boundaries": map[string]any{
				"has_encrypt":           boundaries.HasEncryption,
				"has_signature":         boundaries.HasSignature,
				"has_acroform":          boundaries.HasAcroForm,
				"has_xfa":               boundaries.HasXFA,
				"has_annotations":       boundaries.HasAnnotations,
				"has_font_markers":      boundaries.HasFontMarkers,
				"has_cmap_markers":      boundaries.HasCMapMarkers,
				"has_tounicode_cmap":    boundaries.HasToUnicodeCMap,
				"has_cid_font_markers":  boundaries.HasCIDFontMarkers,
				"text_decoding_support": "simple literal operands, ASCII hex operands, literal/hex TJ arrays, page font-scoped ToUnicode CMaps for simple Tf flows, CMap-backed TJ hex arrays, and one unambiguous ToUnicode CMap fallback",
			},
			"xref": map[string]any{
				"has_table":             xref.HasTable,
				"table_offset":          xref.TableOffset,
				"has_hybrid_stream":     xref.HasHybridStream,
				"hybrid_stream_offset":  xref.HybridStreamOffset,
				"hybrid_stream_object":  hybridStreamObject,
				"has_stream":            xref.HasStream,
				"stream_count":          len(xref.StreamObjects),
				"has_object_stream":     xref.HasObjectStream,
				"object_count":          len(xref.Objects),
				"object_stream_count":   len(xref.ObjectStreamObjects),
				"objects":               xref.Objects,
				"stream_objects":        xref.StreamObjects,
				"object_stream_objects": xref.ObjectStreamObjects,
			},
		},
		Meta: map[string]any{
			"header": header(input),
		},
	})
	return tree
}

func (Adapter) PlanEdit(tree *core.Tree, selector core.Match, mutation core.Mutation) (*core.EditPlan, error) {
	if selector.Kind == "" {
		selector.Kind = KindTextShow
	}
	matches := tree.Query(selector)
	if len(matches) == 0 {
		return nil, fmt.Errorf("no nodes match kind=%q text=%q", selector.Kind, selector.Text)
	}
	if mutation.Index < 0 {
		return nil, errors.New("match index cannot be negative")
	}
	if mutation.Index == 0 && len(matches) > 1 {
		return nil, fmt.Errorf("selector matched %d nodes; pass --match-index to choose one", len(matches))
	}
	index := mutation.Index
	if index >= len(matches) {
		return nil, fmt.Errorf("match index %d out of range for %d matches", index, len(matches))
	}
	target := matches[index]
	oldEncoded, ok := target.Meta["encoded"].(string)
	if !ok || oldEncoded == "" {
		return nil, errors.New("matched text node has no editable encoded span")
	}
	newEncoded, err := encodeTextShowReplacement(target, mutation.Replace)
	if err != nil {
		return nil, err
	}
	if filter, _ := target.Meta["stream_filter"].(string); !isPassthroughPDFStreamFilter(filter) {
		decodeParms, _ := target.Meta["stream_decode_parms"].(string)
		streamBytes, ok := target.Meta["stream_encoded"].([]byte)
		if !ok {
			return nil, errors.New("matched text node has no editable encoded stream")
		}
		decodedStream, ok := target.Meta["decoded_stream"].([]byte)
		if !ok {
			return nil, errors.New("matched text node has no decoded stream")
		}
		decodedStart, ok := metaInt(target.Meta, "decoded_span_start")
		if !ok {
			return nil, errors.New("matched text node has no decoded span start")
		}
		decodedEnd, ok := metaInt(target.Meta, "decoded_span_end")
		if !ok {
			return nil, errors.New("matched text node has no decoded span end")
		}
		if decodedStart < 0 || decodedEnd < decodedStart || decodedEnd > len(decodedStream) {
			return nil, errors.New("unsafe replacement: decoded span is outside stream")
		}
		if !bytes.Equal(decodedStream[decodedStart:decodedEnd], []byte(oldEncoded)) {
			return nil, errors.New("unsafe replacement: decoded span does not match encoded operand")
		}
		updatedStream := replaceByteRange(decodedStream, decodedStart, decodedEnd, []byte(newEncoded))
		newStream, err := encodeStreamFilterWithDecodeParms(filter, decodeParms, updatedStream)
		if err != nil {
			return nil, err
		}
		if target.Span.Len() != int64(len(streamBytes)) {
			return nil, errors.New("unsafe replacement: stream span does not match encoded stream length")
		}
		pageCount := 0
		if root, ok := tree.Node(tree.Root); ok {
			if rootValue, ok := root.Value.(map[string]any); ok {
				if pages, ok := rootValue["pages"].(int); ok {
					pageCount = pages
				}
			}
		}
		return &core.EditPlan{
			Target:    target.ID,
			Operation: "pdf.content_stream_text_rewrite",
			OldText:   fmt.Sprint(target.Value),
			NewText:   mutation.Replace,
			Old:       streamBytes,
			New:       newStream,
			Span:      target.Span,
			PageCount: pageCount,
			Invariants: []core.Invariant{
				core.InvariantReparse,
				core.InvariantOldGone,
				core.InvariantNewSelectable,
				core.InvariantPageUnchanged,
				core.InvariantNoFallbackUsed,
			},
		}, nil
	}
	if target.Span.Len() != int64(len(oldEncoded)) {
		return nil, errors.New("unsafe replacement: node span does not match encoded operand length")
	}
	pageCount := 0
	if root, ok := tree.Node(tree.Root); ok {
		if rootValue, ok := root.Value.(map[string]any); ok {
			if pages, ok := rootValue["pages"].(int); ok {
				pageCount = pages
			}
		}
	}
	return &core.EditPlan{
		Target:    target.ID,
		Operation: "pdf.content_stream_text_rewrite",
		OldText:   fmt.Sprint(target.Value),
		NewText:   mutation.Replace,
		Old:       []byte(oldEncoded),
		New:       []byte(newEncoded),
		Span:      target.Span,
		PageCount: pageCount,
		Invariants: []core.Invariant{
			core.InvariantReparse,
			core.InvariantOldGone,
			core.InvariantNewSelectable,
			core.InvariantPageUnchanged,
			core.InvariantNoFallbackUsed,
		},
	}, nil
}

func (Adapter) Apply(input []byte, plan *core.EditPlan) ([]byte, core.Report, error) {
	if plan == nil {
		return nil, core.Report{}, errors.New("missing edit plan")
	}
	if plan.Span.Start < 0 || plan.Span.End > int64(len(input)) || plan.Span.Start > plan.Span.End {
		return nil, core.Report{}, errors.New("edit span is outside input")
	}
	if !bytes.Equal(input[plan.Span.Start:plan.Span.End], plan.Old) {
		return nil, core.Report{}, errors.New("edit precondition failed: source bytes no longer match plan")
	}
	stream, err := findStreamForSpan(input, plan.Span)
	if err != nil {
		return nil, core.Report{}, err
	}
	if stream.length.Start < 0 {
		return nil, core.Report{}, errors.New("unsupported stream: editable streams require a direct or indirect /Length")
	}
	delta := len(plan.New) - len(plan.Old)
	lengthSpan := stream.length
	newLength := stream.lengthValue + delta
	if newLength < 0 {
		return nil, core.Report{}, errors.New("stream length would become negative")
	}
	lengthReplacement := []byte(strconv.Itoa(newLength))
	output := input
	if lengthSpan.Start > plan.Span.Start {
		output = replaceSpan(output, lengthSpan, lengthReplacement)
		output = replaceSpan(output, plan.Span, plan.New)
	} else {
		output = replaceSpan(output, plan.Span, plan.New)
		output = replaceSpan(output, lengthSpan, lengthReplacement)
	}
	output, err = rebuildXref(output)
	if err != nil {
		return nil, core.Report{}, err
	}
	return output, core.Report{
		Format:        "pdf",
		Edit:          plan.Operation,
		FallbackUsed:  false,
		NodesModified: 1,
		Invariants:    plan.Invariants,
	}, nil
}

func (a Adapter) Verify(output []byte, plan *core.EditPlan) (core.Verification, error) {
	tree, err := a.Parse(output, core.ParseOptions{})
	if err != nil {
		return core.Verification{}, err
	}
	oldMatches := tree.Query(core.Match{Kind: KindTextShow, Text: plan.OldText})
	newMatches := tree.Query(core.Match{Kind: KindTextShow, Text: plan.NewText})
	pageUnchanged := true
	if plan.PageCount > 0 {
		pageUnchanged = false
		if root, ok := tree.Node(tree.Root); ok {
			if rootValue, ok := root.Value.(map[string]any); ok {
				if pages, ok := rootValue["pages"].(int); ok && pages == plan.PageCount {
					pageUnchanged = true
				}
			}
		}
	}
	verification := core.Verification{
		ReparseOK:      true,
		OldTextRemoved: len(oldMatches) == 0,
		NewSelectable:  len(newMatches) > 0,
		PageUnchanged:  pageUnchanged,
	}
	return verification, nil
}

func header(input []byte) string {
	end := bytes.IndexAny(input, "\r\n")
	if end == -1 {
		end = min(len(input), 16)
	}
	return string(input[:end])
}

type residualBoundarySummary struct {
	HasEncryption     bool
	HasSignature      bool
	HasAcroForm       bool
	HasXFA            bool
	HasAnnotations    bool
	HasFontMarkers    bool
	HasCMapMarkers    bool
	HasToUnicodeCMap  bool
	HasCIDFontMarkers bool
}

func summarizeResidualBoundaries(input []byte) residualBoundarySummary {
	hasToUnicode := hasPDFNameOutsideStringOrComment(input, "ToUnicode")
	hasCMap := hasToUnicode ||
		hasPDFNameOutsideStringOrComment(input, "CMap") ||
		hasRawCMapMarkerOutsideStringCommentOrHex(input)
	return residualBoundarySummary{
		HasEncryption:     hasPDFNameOutsideStringOrComment(input, "Encrypt"),
		HasSignature:      hasPDFSignatureBoundary(input),
		HasAcroForm:       hasPDFNameOutsideStringOrComment(input, "AcroForm"),
		HasXFA:            hasPDFNameOutsideStringOrComment(input, "XFA"),
		HasAnnotations:    hasPDFNameOutsideStringOrComment(input, "Annots"),
		HasFontMarkers:    hasPDFNameOutsideStringOrComment(input, "Font"),
		HasCMapMarkers:    hasCMap,
		HasToUnicodeCMap:  hasToUnicode,
		HasCIDFontMarkers: hasPDFNameOutsideStringOrComment(input, "CIDFontType0") || hasPDFNameOutsideStringOrComment(input, "CIDFontType2") || hasPDFNameOutsideStringOrComment(input, "CIDToGIDMap"),
	}
}

func summarizeResidualBoundariesForInput(input []byte) residualBoundarySummary {
	boundaries := summarizeResidualBoundaries(input)
	if trailer := parseLastTrailerDictionary(input); trailer != nil {
		if _, ok := trailer["Encrypt"]; ok {
			boundaries.HasEncryption = true
		}
	}
	return boundaries
}

func hasPDFSignatureBoundary(input []byte) bool {
	for _, name := range []string{"Sig", "ByteRange", "SigFlags"} {
		if hasPDFNameOutsideStringOrComment(input, name) {
			return true
		}
	}
	return false
}

func hasRawCMapMarkerOutsideStringCommentOrHex(input []byte) bool {
	markers := [][]byte{
		[]byte("begincmap"),
		[]byte("beginbfchar"),
		[]byte("beginbfrange"),
	}
	literalDepth := 0
	escaped := false
	for i := 0; i < len(input); i++ {
		if literalDepth > 0 {
			if escaped {
				escaped = false
				continue
			}
			switch input[i] {
			case '\\':
				escaped = true
			case '(':
				literalDepth++
			case ')':
				literalDepth--
			}
			continue
		}
		switch input[i] {
		case '(':
			literalDepth = 1
			continue
		case '%':
			for i < len(input) && input[i] != '\r' && input[i] != '\n' {
				i++
			}
			continue
		case '<':
			if i+1 < len(input) && input[i+1] == '<' {
				i++
				continue
			}
			for i++; i < len(input) && input[i] != '>'; i++ {
			}
			continue
		}
		for _, marker := range markers {
			if !bytes.HasPrefix(input[i:], marker) {
				continue
			}
			end := i + len(marker)
			if isBarePDFKeywordStart(input, i) && isPDFTokenEnd(input, end) {
				return true
			}
			i = end - 1
			break
		}
	}
	return false
}

func isBarePDFKeywordStart(input []byte, pos int) bool {
	if pos == 0 {
		return true
	}
	prev := input[pos-1]
	return prev != '/' && (isPDFSpace(prev) || isPDFDelimiter(prev))
}

func hasPDFNameOutsideStringOrComment(input []byte, name string) bool {
	needle := []byte("/" + name)
	literalDepth := 0
	escaped := false
	for i := 0; i < len(input); i++ {
		if literalDepth > 0 {
			if escaped {
				escaped = false
				continue
			}
			switch input[i] {
			case '\\':
				escaped = true
			case '(':
				literalDepth++
			case ')':
				literalDepth--
			}
			continue
		}
		if input[i] == '(' {
			literalDepth = 1
			continue
		}
		if input[i] == '%' {
			for i < len(input) && input[i] != '\r' && input[i] != '\n' {
				i++
			}
			continue
		}
		if input[i] == '<' {
			if i+1 < len(input) && input[i+1] == '<' {
				i++
				continue
			}
			for i++; i < len(input) && input[i] != '>'; i++ {
			}
			continue
		}
		if !bytes.HasPrefix(input[i:], needle) {
			continue
		}
		end := i + len(needle)
		if isPDFTokenEnd(input, end) {
			return true
		}
		i = end - 1
	}
	return false
}

func hasPDFName(input []byte, name string) bool {
	needle := []byte("/" + name)
	searchStart := 0
	for {
		at := bytes.Index(input[searchStart:], needle)
		if at == -1 {
			return false
		}
		at += searchStart
		end := at + len(needle)
		if isPDFTokenEnd(input, end) {
			return true
		}
		searchStart = end
	}
}

func countPages(input []byte) int {
	re := regexp.MustCompile(`/Type\s*/Page\b`)
	return len(re.FindAll(input, -1))
}

func parseStreams(input []byte, tree *core.Tree, root core.NodeID, cmapContext pdfCMapContext) error {
	pos := 0
	for {
		stream, ok, err := findNextStream(input, pos)
		if err != nil {
			return err
		}
		if !ok {
			return nil
		}
		streamBytes := input[stream.dataStart:stream.dataEnd]
		streamMeta := map[string]any{
			"raw":            isPassthroughPDFStreamFilter(stream.filter),
			"filter":         normalizePDFStreamFilter(stream.filter),
			"decode_parms":   stream.decodeParms,
			"direct_length":  stream.length.Start >= 0 && !stream.lengthIndirect,
			"length_ref":     stream.lengthIndirect,
			"encoded_length": len(streamBytes),
		}
		if filterChain := parsePDFStreamFilterChain(stream.filter); len(filterChain) > 0 {
			streamMeta["filter_chain"] = filterChain
		}
		if stream.unsupportedReason != "" {
			streamMeta["unsupported"] = stream.unsupportedReason
			streamID := tree.AddNode(core.Node{
				Kind: KindStream,
				Span: core.Span{Start: int64(stream.dataStart), End: int64(stream.dataEnd)},
				Meta: streamMeta,
			})
			tree.Nodes[root].Children = append(tree.Nodes[root].Children, streamID)
			pos = stream.endstreamAt + len("endstream")
			continue
		}
		decodedBytes, err := decodeStreamFilterWithDecodeParms(stream.filter, stream.decodeParms, streamBytes)
		if err != nil {
			streamMeta["unsupported"] = err.Error()
			streamID := tree.AddNode(core.Node{
				Kind: KindStream,
				Span: core.Span{Start: int64(stream.dataStart), End: int64(stream.dataEnd)},
				Meta: streamMeta,
			})
			tree.Nodes[root].Children = append(tree.Nodes[root].Children, streamID)
			pos = stream.endstreamAt + len("endstream")
			continue
		}
		streamMeta["decoded_length"] = len(decodedBytes)
		streamID := tree.AddNode(core.Node{
			Kind: KindStream,
			Span: core.Span{Start: int64(stream.dataStart), End: int64(stream.dataEnd)},
			Meta: streamMeta,
		})
		tree.Nodes[root].Children = append(tree.Nodes[root].Children, streamID)
		parseTextShow(decodedBytes, 0, len(decodedBytes), tree, streamID, textShowContext{
			sourceOffset:      stream.dataStart,
			streamSpan:        core.Span{Start: int64(stream.dataStart), End: int64(stream.dataEnd)},
			streamFilter:      stream.filter,
			streamDecodeParms: stream.decodeParms,
			streamEncoded:     bytes.Clone(streamBytes),
			decodedContent:    bytes.Clone(decodedBytes),
			toUnicode:         cmapContext.fallback,
			fontToUnicode:     cmapContext.fontCMapsForStream(stream.dataStart),
			fontMetrics:       cmapContext.fontMetricsForStream(stream.dataStart),
		})
		pos = stream.endstreamAt + len("endstream")
	}
}

func enrichPDFStreamNodeMetadata(tree *core.Tree) {
	if tree == nil {
		return
	}
	for i := range tree.Nodes {
		if tree.Nodes[i].Kind != KindStream {
			continue
		}
		if tree.Nodes[i].Meta == nil {
			tree.Nodes[i].Meta = make(map[string]any)
		}
		meta := tree.Nodes[i].Meta
		if _, ok := meta["encoded_length"]; !ok {
			meta["encoded_length"] = int(tree.Nodes[i].Span.Len())
		}
		filter, _ := meta["filter"].(string)
		if filterChain := parsePDFStreamFilterChain(filter); len(filterChain) > 0 {
			if _, ok := meta["filter_chain"]; !ok {
				meta["filter_chain"] = filterChain
			}
		}
		if _, ok := meta["decoded_length"]; !ok && meta["unsupported"] == nil {
			if raw, ok := meta["raw"].(bool); ok && raw {
				meta["decoded_length"] = int(tree.Nodes[i].Span.Len())
			}
		}
	}
}

type streamInfo struct {
	streamAt          int
	dataStart         int
	dataEnd           int
	endstreamAt       int
	length            core.Span
	lengthValue       int
	lengthIndirect    bool
	filter            string
	decodeParms       string
	unsupportedReason string
}

func findStreamForSpan(input []byte, span core.Span) (streamInfo, error) {
	pos := 0
	for {
		stream, ok, err := findNextStream(input, pos)
		if err != nil {
			return streamInfo{}, err
		}
		if !ok {
			return streamInfo{}, errors.New("no containing stream found for edit span")
		}
		if span.Start >= int64(stream.dataStart) && span.End <= int64(stream.dataEnd) {
			return stream, nil
		}
		pos = stream.endstreamAt + len("endstream")
	}
}

func findNextStream(input []byte, pos int) (streamInfo, bool, error) {
	streamAt := bytes.Index(input[pos:], []byte("stream"))
	if streamAt == -1 {
		return streamInfo{}, false, nil
	}
	streamAt += pos
	dataStart := streamAt + len("stream")
	if dataStart < len(input) && input[dataStart] == '\r' {
		dataStart++
	}
	if dataStart < len(input) && input[dataStart] == '\n' {
		dataStart++
	}
	endAt := bytes.Index(input[dataStart:], []byte("endstream"))
	if endAt == -1 {
		return streamInfo{}, false, errors.New("stream missing endstream")
	}
	endstreamAt := dataStart + endAt
	dataEnd := endstreamAt
	for dataEnd > dataStart && (input[dataEnd-1] == '\r' || input[dataEnd-1] == '\n') {
		dataEnd--
	}
	objStart := findObjectStart(input, streamAt)
	length, value, indirect, lengthUnsupported, err := findStreamLength(input, objStart, streamAt)
	if err != nil {
		return streamInfo{}, false, err
	}
	unsupportedReason := lengthUnsupported
	if length.Start >= 0 {
		dataEnd = dataStart + value
		if dataEnd > endstreamAt {
			return streamInfo{}, false, errors.New("stream length extends past endstream")
		}
	}
	filter, decodeParms, filterUnsupported := findDirectStreamFilterAndDecodeParms(input, objStart, streamAt)
	if unsupportedReason == "" {
		unsupportedReason = filterUnsupported
	}
	if !isPassthroughPDFStreamFilter(filter) && length.Start < 0 && unsupportedReason == "" {
		unsupportedReason = "unsupported stream: filtered streams require a direct or indirect /Length"
	}
	return streamInfo{
		streamAt:          streamAt,
		dataStart:         dataStart,
		dataEnd:           dataEnd,
		endstreamAt:       endstreamAt,
		length:            length,
		lengthValue:       value,
		lengthIndirect:    indirect,
		filter:            filter,
		decodeParms:       decodeParms,
		unsupportedReason: unsupportedReason,
	}, true, nil
}

func findDirectStreamFilter(input []byte, start, end int) (string, string) {
	filter, _, unsupported := findDirectStreamFilterAndDecodeParms(input, start, end)
	return filter, unsupported
}

func findDirectStreamFilterAndDecodeParms(input []byte, start, end int) (string, string, string) {
	dict := input[start:end]
	decodeParms, decodeParmsUnsupported := findDirectStreamDecodeParms(dict)
	if decodeParmsUnsupported != "" {
		return "", "", decodeParmsUnsupported
	}
	filterAt := bytes.Index(dict, []byte("/Filter"))
	if filterAt == -1 {
		return "", decodeParms, ""
	}
	i := filterAt + len("/Filter")
	for i < len(dict) && isPDFSpace(dict[i]) {
		i++
	}
	if i >= len(dict) {
		return "", "", "unsupported stream: /Filter is missing a value"
	}
	if dict[i] == '[' {
		filter, unsupported := findDirectStreamFilterArray(dict, i)
		return filter, decodeParms, unsupported
	}
	if dict[i] != '/' {
		return "", "", "unsupported stream: /Filter must be a direct name"
	}
	j := i + 1
	for j < len(dict) && !isPDFSpace(dict[j]) && !isPDFDelimiter(dict[j]) {
		j++
	}
	if j == i+1 {
		return "", "", "unsupported stream: /Filter name is empty"
	}
	return string(dict[i:j]), decodeParms, ""
}

func findDirectStreamDecodeParms(dict []byte) (string, string) {
	decodeParmsAt := bytes.Index(dict, []byte("/DecodeParms"))
	if decodeParmsAt == -1 {
		return "", ""
	}
	i := decodeParmsAt + len("/DecodeParms")
	if i < len(dict) && !isPDFSpace(dict[i]) && !isPDFDelimiter(dict[i]) {
		return "", ""
	}
	for i < len(dict) && isPDFSpace(dict[i]) {
		i++
	}
	if i >= len(dict) {
		return "", "unsupported stream: /DecodeParms is missing a value"
	}
	if i+1 < len(dict) && dict[i] == '<' && dict[i+1] == '<' {
		end, ok := findDictionaryEnd(dict, i)
		if !ok {
			return "", "unsupported stream: /DecodeParms dictionary is not closed"
		}
		return string(dict[i:end]), ""
	}
	if dict[i] == '[' {
		end, ok := findArrayEnd(dict, i)
		if !ok {
			return "", "unsupported stream: /DecodeParms array is not closed"
		}
		return string(dict[i:end]), ""
	}
	if bytes.HasPrefix(dict[i:], []byte("null")) && isPDFTokenEnd(dict, i+len("null")) {
		return "null", ""
	}
	return "", "unsupported stream: /DecodeParms must be a dictionary, array, or null"
}

func findDirectStreamFilterArray(dict []byte, start int) (string, string) {
	i := start + 1
	found := false
	for i < len(dict) && isPDFSpace(dict[i]) {
		i++
	}
	for i < len(dict) {
		if dict[i] == ']' {
			if !found {
				return "", "unsupported stream: /Filter array is empty"
			}
			return string(dict[start : i+1]), ""
		}
		if dict[i] != '/' {
			return "", "unsupported stream: /Filter array entries must be direct names"
		}
		j := i + 1
		for j < len(dict) && !isPDFSpace(dict[j]) && !isPDFDelimiter(dict[j]) {
			j++
		}
		if j == i+1 {
			return "", "unsupported stream: /Filter array name is empty"
		}
		found = true
		i = j
		for i < len(dict) && isPDFSpace(dict[i]) {
			i++
		}
	}
	return "", "unsupported stream: /Filter array is not closed"
}

func isPDFStreamFilterArray(filter string) bool {
	return strings.HasPrefix(strings.TrimSpace(filter), "[")
}

func findObjectStart(input []byte, before int) int {
	objAt := bytes.LastIndex(input[:before], []byte(" obj"))
	if objAt == -1 {
		return 0
	}
	lineStart := bytes.LastIndexAny(input[:objAt], "\r\n")
	if lineStart == -1 {
		return 0
	}
	return lineStart + 1
}

func findDirectLength(input []byte, start, end int) (core.Span, int, error) {
	length, value, indirect, unsupported, err := findStreamLength(input, start, end)
	if err != nil {
		return core.Span{}, 0, err
	}
	if indirect || unsupported != "" {
		return core.Span{Start: -1, End: -1}, 0, nil
	}
	return length, value, nil
}

func findStreamLength(input []byte, start, end int) (core.Span, int, bool, string, error) {
	if start < 0 {
		start = 0
	}
	if end > len(input) {
		end = len(input)
	}
	if start >= end {
		return core.Span{Start: -1, End: -1}, 0, false, "", nil
	}
	dict := input[start:end]
	searchStart := 0
	for {
		lengthAt := bytes.Index(dict[searchStart:], []byte("/Length"))
		if lengthAt == -1 {
			return core.Span{Start: -1, End: -1}, 0, false, "", nil
		}
		lengthAt += searchStart
		i := lengthAt + len("/Length")
		if i < len(dict) && !isPDFSpace(dict[i]) && !isPDFDelimiter(dict[i]) {
			searchStart = i
			continue
		}
		for i < len(dict) && isPDFSpace(dict[i]) {
			i++
		}
		if i >= len(dict) || !isPDFDigit(dict[i]) {
			return core.Span{Start: -1, End: -1}, 0, false, "unsupported stream: /Length must be an integer or indirect integer reference", nil
		}
		valueStart := i
		for i < len(dict) && isPDFDigit(dict[i]) {
			i++
		}
		valueEnd := i
		first, err := strconv.Atoi(string(dict[valueStart:valueEnd]))
		if err != nil {
			return core.Span{}, 0, false, "", err
		}
		j := i
		for j < len(dict) && isPDFSpace(dict[j]) {
			j++
		}
		if j < len(dict) && isPDFDigit(dict[j]) {
			generationStart := j
			for j < len(dict) && isPDFDigit(dict[j]) {
				j++
			}
			generation, err := strconv.Atoi(string(dict[generationStart:j]))
			if err != nil {
				return core.Span{}, 0, false, "", err
			}
			for j < len(dict) && isPDFSpace(dict[j]) {
				j++
			}
			if j < len(dict) && dict[j] == 'R' && isPDFTokenEnd(dict, j+1) {
				span, value, ok, err := findIntegerObject(input, first, generation)
				if err != nil {
					return core.Span{}, 0, true, "", err
				}
				if !ok {
					return core.Span{Start: -1, End: -1}, 0, true, "unsupported stream: /Length reference must resolve to an integer object", nil
				}
				return span, value, true, "", nil
			}
		}
		return core.Span{Start: int64(start + valueStart), End: int64(start + valueEnd)}, first, false, "", nil
	}
}

func findIntegerObject(input []byte, number, generation int) (core.Span, int, bool, error) {
	objects := findXrefObjectOffsets(input)
	for _, object := range objects {
		if object.Number != number || object.Generation != generation {
			continue
		}
		headerEndRel := bytes.Index(input[object.Offset:], []byte("obj"))
		if headerEndRel == -1 {
			return core.Span{}, 0, false, nil
		}
		bodyStart := object.Offset + headerEndRel + len("obj")
		endObjRel := bytes.Index(input[bodyStart:], []byte("endobj"))
		if endObjRel == -1 {
			return core.Span{}, 0, false, nil
		}
		bodyEnd := bodyStart + endObjRel
		valueStart := bodyStart
		for valueStart < bodyEnd && isPDFSpace(input[valueStart]) {
			valueStart++
		}
		valueEnd := valueStart
		for valueEnd < bodyEnd && isPDFDigit(input[valueEnd]) {
			valueEnd++
		}
		if valueStart == valueEnd {
			return core.Span{}, 0, false, nil
		}
		for i := valueEnd; i < bodyEnd; i++ {
			if !isPDFSpace(input[i]) {
				return core.Span{}, 0, false, nil
			}
		}
		value, err := strconv.Atoi(string(input[valueStart:valueEnd]))
		if err != nil {
			return core.Span{}, 0, false, err
		}
		return core.Span{Start: int64(valueStart), End: int64(valueEnd)}, value, true, nil
	}
	return core.Span{}, 0, false, nil
}

func replaceSpan(input []byte, span core.Span, replacement []byte) []byte {
	out := make([]byte, 0, len(input)-int(span.Len())+len(replacement))
	out = append(out, input[:span.Start]...)
	out = append(out, replacement...)
	out = append(out, input[span.End:]...)
	return out
}

func rebuildXref(input []byte) ([]byte, error) {
	bodyEnd := lastXrefTableStart(input)
	if bodyEnd == -1 {
		return nil, errors.New("unsupported PDF: xref table not found")
	}
	body := bytes.TrimRight(input[:bodyEnd], "\r\n")
	objectOffsets := findXrefObjectOffsets(body)
	if len(objectOffsets) == 0 {
		return nil, errors.New("cannot rebuild xref: no indirect objects found")
	}
	objects := make(map[int]xrefObjectOffset, len(objectOffsets))
	maxObj := 0
	for _, object := range objectOffsets {
		objects[object.Number] = object
		if object.Number > maxObj {
			maxObj = object.Number
		}
	}
	trailer := trailerDictionary(input[bodyEnd:], maxObj+1)
	var out bytes.Buffer
	out.Write(body)
	out.WriteString("\n")
	xrefOffset := out.Len()
	out.WriteString("xref\n")
	out.WriteString(fmt.Sprintf("0 %d\n", maxObj+1))
	out.WriteString("0000000000 65535 f \n")
	for i := 1; i <= maxObj; i++ {
		object, ok := objects[i]
		if !ok {
			out.WriteString("0000000000 65535 f \n")
			continue
		}
		out.WriteString(fmt.Sprintf("%010d %05d n \n", object.Offset, object.Generation))
	}
	out.WriteString("trailer\n")
	out.Write(trailer)
	out.WriteString("\nstartxref\n")
	out.WriteString(strconv.Itoa(xrefOffset))
	out.WriteString("\n%%EOF\n")
	return out.Bytes(), nil
}

func lastXrefTableStart(input []byte) int {
	candidates := [][]byte{[]byte("\nxref\n"), []byte("\rxref\r"), []byte("\r\nxref\r\n")}
	last := -1
	for _, marker := range candidates {
		if idx := bytes.LastIndex(input, marker); idx != -1 && idx+1 > last {
			last = idx + 1
		}
	}
	if bytes.HasPrefix(input, []byte("xref\n")) {
		last = 0
	}
	return last
}

func trailerDictionary(xrefAndTrailer []byte, size int) []byte {
	trailerAt := bytes.Index(xrefAndTrailer, []byte("trailer"))
	if trailerAt == -1 {
		return []byte(fmt.Sprintf("<< /Size %d >>", size))
	}
	dictStartRel := bytes.Index(xrefAndTrailer[trailerAt:], []byte("<<"))
	if dictStartRel == -1 {
		return []byte(fmt.Sprintf("<< /Size %d >>", size))
	}
	dictStart := trailerAt + dictStartRel
	dictEnd, ok := findDictionaryEnd(xrefAndTrailer, dictStart)
	if !ok {
		return []byte(fmt.Sprintf("<< /Size %d >>", size))
	}
	dict := bytes.Clone(xrefAndTrailer[dictStart:dictEnd])
	sizeRe := regexp.MustCompile(`/Size\s+\d+`)
	if sizeRe.Match(dict) {
		dict = sizeRe.ReplaceAll(dict, []byte(fmt.Sprintf("/Size %d", size)))
	} else {
		dict = bytes.TrimSuffix(dict, []byte(">>"))
		dict = append(dict, []byte(fmt.Sprintf(" /Size %d >>", size))...)
	}
	prevRe := regexp.MustCompile(`/Prev\s+\d+`)
	dict = prevRe.ReplaceAll(dict, nil)
	return bytes.TrimSpace(dict)
}

func findDictionaryEnd(input []byte, start int) (int, bool) {
	depth := 0
	for i := start; i+1 < len(input); i++ {
		if input[i] == '<' && input[i+1] == '<' {
			depth++
			i++
			continue
		}
		if input[i] == '>' && input[i+1] == '>' {
			depth--
			i++
			if depth == 0 {
				return i + 1, true
			}
		}
	}
	return 0, false
}

func findArrayEnd(input []byte, start int) (int, bool) {
	arrayDepth := 0
	dictDepth := 0
	for i := start; i < len(input); i++ {
		if i+1 < len(input) && input[i] == '<' && input[i+1] == '<' {
			dictDepth++
			i++
			continue
		}
		if i+1 < len(input) && input[i] == '>' && input[i+1] == '>' {
			if dictDepth == 0 {
				return 0, false
			}
			dictDepth--
			i++
			continue
		}
		if dictDepth > 0 {
			continue
		}
		switch input[i] {
		case '[':
			arrayDepth++
		case ']':
			arrayDepth--
			if arrayDepth == 0 {
				return i + 1, true
			}
			if arrayDepth < 0 {
				return 0, false
			}
		}
	}
	return 0, false
}

type textShowContext struct {
	sourceOffset      int
	streamSpan        core.Span
	streamFilter      string
	streamDecodeParms string
	streamEncoded     []byte
	decodedContent    []byte
	toUnicode         *toUnicodeCMap
	fontToUnicode     map[string]*toUnicodeCMap
	fontMetrics       map[string]pdfSimpleFontMetrics
}

func parseTextShow(input []byte, start, end int, tree *core.Tree, streamID core.NodeID, ctx textShowContext) {
	activeFont := ""
	for i := start; i < end; i++ {
		if font, next, ok := nextSetFontOperator(input, i, end); ok {
			activeFont = font
			i = next - 1
			continue
		}
		var (
			closeAt      int
			operandEnd   int
			encoded      string
			decoded      string
			encoding     string
			operandStart int
			ok           bool
		)
		switch input[i] {
		case '[':
			arrayDecoded, arrayEncoded, arrayEnd, arrayUsedCMap, arrayOK := parseSimpleTJArrayText(input, i, end, ctx.cmapForFont(activeFont))
			if !arrayOK {
				continue
			}
			op, _ := nextOperator(input, arrayEnd, end)
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
			closeAt, ok = findLiteralEnd(input, i+1, end)
			if !ok {
				continue
			}
			operandEnd = closeAt + 1
			operandStart = i + 1
			encoded = string(input[operandStart:closeAt])
			decoded = decodeLiteralString(encoded)
			encoding = "literal"
		case '<':
			if i+1 < end && input[i+1] == '<' {
				continue
			}
			closeAt, ok = findHexStringEnd(input, i+1, end)
			if !ok {
				continue
			}
			operandEnd = closeAt + 1
			operandStart = i + 1
			encoded = string(input[operandStart:closeAt])
			var usedCMap bool
			cmap := ctx.cmapForFont(activeFont)
			decoded, usedCMap, ok = decodeHexTextStringWithCMap([]byte(encoded), cmap)
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
		op, _ := nextOperator(input, operandEnd, end)
		if op == "" {
			i = closeAt
			continue
		}
		span := core.Span{Start: int64(ctx.sourceOffset + operandStart), End: int64(ctx.sourceOffset + closeAt)}
		meta := map[string]any{
			"operator": op,
			"encoded":  encoded,
			"encoding": encoding,
		}
		if encoding == "hex-cmap" || encoding == "tj-array-cmap" {
			meta["cmap"] = ctx.cmapForFont(activeFont)
			if activeFont != "" {
				meta["font"] = activeFont
			}
		}
		enrichTextShowFontWidthMetadata(meta, activeFont, encoded, encoding, ctx.fontMetrics)
		if !isPassthroughPDFStreamFilter(ctx.streamFilter) {
			span = ctx.streamSpan
			meta["stream_filter"] = ctx.streamFilter
			meta["stream_decode_parms"] = ctx.streamDecodeParms
			meta["stream_encoded"] = ctx.streamEncoded
			meta["decoded_stream"] = ctx.decodedContent
			meta["decoded_span_start"] = operandStart
			meta["decoded_span_end"] = closeAt
		}
		textID := tree.AddNode(core.Node{
			Kind:  KindTextShow,
			Span:  span,
			Value: decoded,
			Meta:  meta,
		})
		tree.Nodes[streamID].Children = append(tree.Nodes[streamID].Children, textID)
		i = closeAt
	}
}

func (ctx textShowContext) cmapForFont(font string) *toUnicodeCMap {
	if font != "" && ctx.fontToUnicode != nil {
		if cmap, ok := ctx.fontToUnicode[font]; ok {
			return cmap
		}
	}
	return ctx.toUnicode
}

func encodeTextShowReplacement(target core.Node, replacement string) (string, error) {
	encoding, _ := target.Meta["encoding"].(string)
	switch encoding {
	case "", "literal":
		return encodeLiteralString(replacement), nil
	case "tj-array":
		return "[(" + encodeLiteralString(replacement) + ")]", nil
	case "tj-array-cmap":
		cmap, _ := target.Meta["cmap"].(*toUnicodeCMap)
		encoded, ok := cmap.EncodeHex(replacement)
		if !ok {
			return "", errors.New("replacement for ToUnicode TJ array text is not representable by the CMap")
		}
		return "[<" + encoded + ">]", nil
	case "hex":
		return encodeHexTextString(replacement)
	case "hex-cmap":
		cmap, _ := target.Meta["cmap"].(*toUnicodeCMap)
		encoded, ok := cmap.EncodeHex(replacement)
		if !ok {
			return "", errors.New("replacement for ToUnicode hex text show operand is not representable by the CMap")
		}
		return encoded, nil
	default:
		return "", fmt.Errorf("unsupported text show operand encoding %q", encoding)
	}
}

func parseSimpleTJArrayText(input []byte, start, end int, cmap *toUnicodeCMap) (string, string, int, bool, bool) {
	if start >= end || input[start] != '[' {
		return "", "", start, false, false
	}
	var decoded strings.Builder
	usedCMap := false
	i := start + 1
	for {
		i = skipPDFSpaceAndComments(input, i)
		if i >= end {
			return "", "", start, false, false
		}
		switch input[i] {
		case ']':
			return decoded.String(), string(input[start : i+1]), i + 1, usedCMap, true
		case '(':
			closeAt, ok := findLiteralEnd(input, i+1, end)
			if !ok {
				return "", "", start, false, false
			}
			if cmap != nil {
				return "", "", start, false, false
			}
			decoded.WriteString(decodeLiteralString(string(input[i+1 : closeAt])))
			i = closeAt + 1
		case '<':
			if i+1 < end && input[i+1] == '<' {
				return "", "", start, false, false
			}
			closeAt, ok := findHexStringEnd(input, i+1, end)
			if !ok {
				return "", "", start, false, false
			}
			text, used, ok := decodeHexTextStringWithCMap(input[i+1:closeAt], cmap)
			if !ok || (!used && !isASCIIText(text)) {
				return "", "", start, false, false
			}
			usedCMap = usedCMap || used
			decoded.WriteString(text)
			i = closeAt + 1
		default:
			numberEnd, ok := scanPDFNumber(input, i, end)
			if !ok || !isPDFTokenEnd(input, numberEnd) {
				return "", "", start, false, false
			}
			i = numberEnd
		}
	}
}

func isASCIIText(text string) bool {
	for i := 0; i < len(text); i++ {
		if text[i] > 0x7f {
			return false
		}
	}
	return true
}

func findLiteralEnd(input []byte, start, end int) (int, bool) {
	depth := 1
	escaped := false
	for i := start; i < end; i++ {
		c := input[i]
		if escaped {
			escaped = false
			continue
		}
		switch c {
		case '\\':
			escaped = true
		case '(':
			depth++
		case ')':
			depth--
			if depth == 0 {
				return i, true
			}
		}
	}
	return 0, false
}

func findHexStringEnd(input []byte, start, end int) (int, bool) {
	for i := start; i < end; i++ {
		if input[i] == '>' {
			return i, true
		}
	}
	return 0, false
}

func nextOperator(input []byte, start, end int) (string, int) {
	i := start
	for i < end && isPDFSpace(input[i]) {
		i++
	}
	j := i
	for j < end && !isPDFSpace(input[j]) {
		j++
	}
	op := string(input[i:j])
	switch op {
	case "Tj", "TJ", "'", "\"":
		return op, j
	default:
		return "", j
	}
}

func nextSetFontOperator(input []byte, start, end int) (string, int, bool) {
	if start >= end || input[start] != '/' {
		return "", start, false
	}
	nameStart := start + 1
	nameEnd := nameStart
	for nameEnd < end && !isPDFSpace(input[nameEnd]) && !isPDFDelimiter(input[nameEnd]) {
		nameEnd++
	}
	if nameEnd == nameStart {
		return "", start, false
	}
	i := nameEnd
	for i < end && isPDFSpace(input[i]) {
		i++
	}
	numberEnd, ok := scanPDFNumber(input, i, end)
	if !ok {
		return "", start, false
	}
	i = numberEnd
	for i < end && isPDFSpace(input[i]) {
		i++
	}
	if i+2 > end || string(input[i:i+2]) != "Tf" || !isPDFTokenEnd(input, i+2) {
		return "", start, false
	}
	return string(input[nameStart:nameEnd]), i + 2, true
}

func scanPDFNumber(input []byte, start, end int) (int, bool) {
	i := start
	if i < end && (input[i] == '-' || input[i] == '+') {
		i++
	}
	digits := 0
	dot := false
	for i < end {
		switch {
		case isPDFDigit(input[i]):
			digits++
			i++
		case input[i] == '.' && !dot:
			dot = true
			i++
		default:
			if digits == 0 {
				return start, false
			}
			return i, true
		}
	}
	return i, digits > 0
}

func isPDFSpace(b byte) bool {
	switch b {
	case 0, '\t', '\n', '\f', '\r', ' ':
		return true
	default:
		return false
	}
}

func isPDFDigit(b byte) bool {
	return b >= '0' && b <= '9'
}

func isPDFDelimiter(b byte) bool {
	switch b {
	case '(', ')', '<', '>', '[', ']', '{', '}', '/', '%':
		return true
	default:
		return false
	}
}

func isPDFTokenEnd(input []byte, pos int) bool {
	return pos >= len(input) || isPDFSpace(input[pos]) || isPDFDelimiter(input[pos])
}

func metaInt(meta map[string]any, key string) (int, bool) {
	value, ok := meta[key]
	if !ok {
		return 0, false
	}
	switch v := value.(type) {
	case int:
		return v, true
	case int64:
		return int(v), true
	default:
		return 0, false
	}
}

func replaceByteRange(input []byte, start, end int, replacement []byte) []byte {
	out := make([]byte, 0, len(input)-(end-start)+len(replacement))
	out = append(out, input[:start]...)
	out = append(out, replacement...)
	out = append(out, input[end:]...)
	return out
}

func decodeLiteralString(encoded string) string {
	var out strings.Builder
	for i := 0; i < len(encoded); i++ {
		c := encoded[i]
		if c != '\\' {
			out.WriteByte(c)
			continue
		}
		if i+1 >= len(encoded) {
			break
		}
		n := encoded[i+1]
		switch n {
		case 'n':
			out.WriteByte('\n')
			i++
		case 'r':
			out.WriteByte('\r')
			i++
		case 't':
			out.WriteByte('\t')
			i++
		case 'b':
			out.WriteByte('\b')
			i++
		case 'f':
			out.WriteByte('\f')
			i++
		case '(', ')', '\\':
			out.WriteByte(n)
			i++
		case '\r', '\n':
			i++
			if n == '\r' && i+1 < len(encoded) && encoded[i+1] == '\n' {
				i++
			}
		default:
			if n >= '0' && n <= '7' {
				j := i + 1
				for j < len(encoded) && j < i+4 && encoded[j] >= '0' && encoded[j] <= '7' {
					j++
				}
				v, err := strconv.ParseInt(encoded[i+1:j], 8, 16)
				if err == nil {
					out.WriteByte(byte(v))
					i = j - 1
					continue
				}
			}
			out.WriteByte(n)
			i++
		}
	}
	return out.String()
}

func decodeHexTextString(encoded []byte) (string, bool) {
	decoded, ok := decodeHexBytes(encoded)
	if !ok {
		return "", false
	}
	return string(decoded), true
}

func decodeHexTextStringWithCMap(encoded []byte, cmap *toUnicodeCMap) (string, bool, bool) {
	if cmap != nil {
		if decoded, ok := cmap.DecodeHex(encoded); ok {
			return decoded, true, true
		}
	}
	decoded, ok := decodeHexTextString(encoded)
	return decoded, false, ok
}

func encodeHexTextString(text string) (string, error) {
	const hex = "0123456789ABCDEF"
	var out strings.Builder
	out.Grow(len(text) * 2)
	for i := 0; i < len(text); i++ {
		if text[i] > 0x7f {
			return "", errors.New("replacement for hex text show operand must be ASCII")
		}
		out.WriteByte(hex[text[i]>>4])
		out.WriteByte(hex[text[i]&0x0f])
	}
	return out.String(), nil
}

func hexNibble(b byte) (byte, bool) {
	switch {
	case b >= '0' && b <= '9':
		return b - '0', true
	case b >= 'A' && b <= 'F':
		return b - 'A' + 10, true
	case b >= 'a' && b <= 'f':
		return b - 'a' + 10, true
	default:
		return 0, false
	}
}

func encodeLiteralString(text string) string {
	var out strings.Builder
	for i := 0; i < len(text); i++ {
		switch text[i] {
		case '-':
			out.WriteString(`\055`)
		case '(', ')', '\\':
			out.WriteByte('\\')
			out.WriteByte(text[i])
		case '\n':
			out.WriteString(`\n`)
		case '\r':
			out.WriteString(`\r`)
		case '\t':
			out.WriteString(`\t`)
		default:
			out.WriteByte(text[i])
		}
	}
	return out.String()
}
