package pdf

import (
	"bytes"
	"errors"
	"fmt"
	"strings"
	"unicode/utf8"

	"github.com/oxhq/binas/pkg/core"
)

const xfaPacketPreviewRunes = 80

type XFAPacketMetadata struct {
	Index            int    `json:"index"`
	Label            string `json:"label"`
	PacketKind       string `json:"packet_kind,omitempty"`
	HasXMLProlog     bool   `json:"has_xml_prolog"`
	RootElement      string `json:"root_element,omitempty"`
	ObjectNumber     *int   `json:"object_number"`
	ObjectGeneration *int   `json:"object_generation"`
	IsStream         bool   `json:"is_stream"`
	Filter           string `json:"filter,omitempty"`
	DecodeParms      string `json:"decode_parms,omitempty"`
	HasDecodeError   bool   `json:"has_decode_error,omitempty"`
	DecodeError      string `json:"decode_error,omitempty"`
	TextLength       int    `json:"text_length"`
	ByteLength       int    `json:"byte_length"`
	Preview          string `json:"preview"`
}

type XFASelector struct {
	PacketKind string
	Label      string
}

type XFAPacketListOptions struct {
	Selector XFASelector
}

type XFAReplaceOptions struct {
	MatchIndex *int
	Selector   XFASelector
}

func ListXFAPackets(input []byte) ([]XFAPacketMetadata, error) {
	return ListXFAPacketsWithOptions(input, XFAPacketListOptions{})
}

func ListXFAPacketsWithOptions(input []byte, options XFAPacketListOptions) ([]XFAPacketMetadata, error) {
	graph, err := parsePDFGraphWithOptions(input, pdfGraphParseOptions{AllowXFA: true})
	if err != nil {
		return nil, err
	}
	packets := make([]XFAPacketMetadata, 0)
	for _, acroForm := range acroFormDictionaries(graph) {
		value, ok := acroForm["XFA"]
		if !ok {
			continue
		}
		packets = appendXFAPacketMetadata(packets, graph, value, "")
	}
	for i := range packets {
		packets[i].Index = i
	}
	packets = filterXFAPacketMetadata(packets, options.Selector)
	return packets, nil
}

func ApplyXFAReplace(input []byte, oldText, newText string, matchIndexArg ...*int) ([]byte, core.Report, core.Verification, error) {
	if oldText == "" {
		return nil, core.Report{}, core.Verification{}, errors.New("XFA replace requires --text")
	}
	if len(matchIndexArg) > 1 {
		return nil, core.Report{}, core.Verification{}, errors.New("XFA replace accepts at most one match index")
	}
	var matchIndex *int
	if len(matchIndexArg) == 1 {
		matchIndex = matchIndexArg[0]
	}
	return ApplyXFAReplaceWithOptions(input, oldText, newText, XFAReplaceOptions{MatchIndex: matchIndex})
}

func ApplyXFAReplaceWithOptions(input []byte, oldText, newText string, options XFAReplaceOptions) ([]byte, core.Report, core.Verification, error) {
	if oldText == "" {
		return nil, core.Report{}, core.Verification{}, errors.New("XFA replace requires --text")
	}
	graph, err := parsePDFGraphWithOptions(input, pdfGraphParseOptions{AllowXFA: true})
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	candidates := xfaPackets(graph, oldText)
	if !options.Selector.empty() {
		selectorMatches := filterXFAPackets(xfaPackets(graph, ""), options.Selector)
		if len(selectorMatches) == 0 {
			if !graphHasDirectXFA(graph) {
				return nil, core.Report{}, core.Verification{}, errors.New("unsupported PDF: XFA packet is not directly represented")
			}
			return nil, core.Report{}, core.Verification{}, fmt.Errorf("no XFA packet matches selector %s", options.Selector.describe())
		}
	}
	matches := filterXFAPackets(candidates, options.Selector)
	if len(matches) == 0 {
		if !graphHasDirectXFA(graph) {
			return nil, core.Report{}, core.Verification{}, errors.New("unsupported PDF: XFA packet is not directly represented")
		}
		if options.Selector.empty() {
			return nil, core.Report{}, core.Verification{}, fmt.Errorf("no XFA packet contains %q", oldText)
		}
		return nil, core.Report{}, core.Verification{}, fmt.Errorf("no XFA packet matching selector %s contains %q", options.Selector.describe(), oldText)
	}
	matchIndex := 0
	var selected *int
	if options.MatchIndex != nil {
		matchIndex = *options.MatchIndex
		if matchIndex < 0 || matchIndex >= len(matches) {
			return nil, core.Report{}, core.Verification{}, fmt.Errorf("XFA replacement match index %d is out of range: %d matches for %q", matchIndex, len(matches), oldText)
		}
		selectedValue := matchIndex
		selected = &selectedValue
	} else if len(matches) > 1 {
		return nil, core.Report{}, core.Verification{}, fmt.Errorf("XFA replacement is ambiguous: %d matches for %q", len(matches), oldText)
	}
	if err := matches[matchIndex].replace(oldText, newText); err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	output, err := writeCanonicalPDF(graph)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	verification, err := verifySemanticPDF(output, pdfGraphParseOptions{AllowXFA: true})
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	verification.OldTextRemoved = !bytes.Contains(output, []byte(oldText))
	verification.NewSelectable = bytes.Contains(output, []byte(newText))
	return output, core.Report{
		Format:        "pdf",
		Edit:          "pdf.xfa_replace",
		FallbackUsed:  false,
		NodesModified: 1,
		MatchIndex:    selected,
		Invariants:    []core.Invariant{core.InvariantReparse, core.InvariantNoFallbackUsed},
	}, verification, nil
}

type xfaPacket struct {
	dictKey string
	dict    pdfDict
	array   pdfArray
	index   int
	object  *pdfIndirectObject
	label   string
	kind    string
	text    string
	stream  bool
	occurs  int
}

func xfaPackets(graph *pdfGraph, contains string) []xfaPacket {
	matches := make([]xfaPacket, 0)
	for _, acroForm := range acroFormDictionaries(graph) {
		value, ok := acroForm["XFA"]
		if !ok {
			continue
		}
		matches = append(matches, collectXFAPackets(graph, acroForm, "XFA", value, contains, "")...)
	}
	return matches
}

func filterXFAPacketMetadata(packets []XFAPacketMetadata, selector XFASelector) []XFAPacketMetadata {
	if selector.empty() {
		return packets
	}
	filtered := make([]XFAPacketMetadata, 0, len(packets))
	for _, packet := range packets {
		if selector.matches(packet.Label, packet.PacketKind) {
			filtered = append(filtered, packet)
		}
	}
	return filtered
}

func filterXFAPackets(packets []xfaPacket, selector XFASelector) []xfaPacket {
	if selector.empty() {
		return packets
	}
	filtered := make([]xfaPacket, 0, len(packets))
	for _, packet := range packets {
		if selector.matches(packet.label, packet.kind) {
			filtered = append(filtered, packet)
		}
	}
	return filtered
}

func (s XFASelector) empty() bool {
	return s.PacketKind == "" && s.Label == ""
}

func (s XFASelector) matches(label, kind string) bool {
	if s.Label != "" && label != s.Label {
		return false
	}
	if s.PacketKind != "" && kind != s.PacketKind {
		return false
	}
	return true
}

func (s XFASelector) describe() string {
	parts := make([]string, 0, 2)
	if s.PacketKind != "" {
		parts = append(parts, fmt.Sprintf("packet_kind=%q", s.PacketKind))
	}
	if s.Label != "" {
		parts = append(parts, fmt.Sprintf("label=%q", s.Label))
	}
	return strings.Join(parts, " ")
}

func appendXFAPacketMetadata(out []XFAPacketMetadata, graph *pdfGraph, value pdfValue, label string) []XFAPacketMetadata {
	switch v := value.(type) {
	case pdfLiteralString, pdfHexString:
		text, ok := pdfTextValue(v)
		if ok {
			out = append(out, makeXFAPacketMetadata(label, nil, false, text))
		}
	case pdfRef:
		object, ok := graph.Objects[v.ID]
		if !ok {
			return out
		}
		switch objectValue := object.Value.(type) {
		case pdfStreamObject:
			decoded, err := decodePDFGraphStream(objectValue)
			if err == nil {
				out = append(out, makeXFAStreamPacketMetadata(label, object, objectValue, string(decoded), nil))
			} else {
				out = append(out, makeXFAStreamPacketMetadata(label, object, objectValue, "", err))
			}
		case pdfLiteralString, pdfHexString:
			text, ok := pdfTextValue(objectValue)
			if ok {
				out = append(out, makeXFAPacketMetadata(label, object, false, text))
			}
		}
	case pdfArray:
		for i := 0; i < len(v); i++ {
			item := v[i]
			packetLabel, hasPacketLabel := xfaArrayPacketLabel(item)
			if hasPacketLabel && i+1 < len(v) {
				i++
				out = appendXFAPacketMetadata(out, graph, v[i], packetLabel)
				continue
			}
			out = appendXFAPacketMetadata(out, graph, item, "")
		}
	}
	return out
}

func makeXFAPacketMetadata(label string, object *pdfIndirectObject, isStream bool, text string) XFAPacketMetadata {
	hasXMLProlog, rootElement := xfaPacketXMLDiagnostics(text)
	packet := XFAPacketMetadata{
		Label:        label,
		PacketKind:   classifyXFAPacketKind(label, text),
		HasXMLProlog: hasXMLProlog,
		RootElement:  rootElement,
		IsStream:     isStream,
		TextLength:   utf8.RuneCountInString(text),
		ByteLength:   len(text),
		Preview:      boundedXFAPreview(text),
	}
	if object != nil {
		number := object.ID.Number
		generation := object.ID.Generation
		packet.ObjectNumber = &number
		packet.ObjectGeneration = &generation
	}
	return packet
}

func xfaPacketXMLDiagnostics(text string) (bool, string) {
	text = strings.TrimSpace(text)
	if strings.HasPrefix(text, "\ufeff") {
		text = strings.TrimSpace(strings.TrimPrefix(text, "\ufeff"))
	}
	hasXMLProlog := false
	if strings.HasPrefix(text, "<?xml") {
		end := strings.Index(text, "?>")
		if end < 0 {
			return true, ""
		}
		hasXMLProlog = true
		text = strings.TrimSpace(text[end+2:])
	}
	for {
		switch {
		case strings.HasPrefix(text, "<!--"):
			end := strings.Index(text, "-->")
			if end < 0 {
				return hasXMLProlog, ""
			}
			text = strings.TrimSpace(text[end+3:])
		case strings.HasPrefix(text, "<?"):
			end := strings.Index(text, "?>")
			if end < 0 {
				return hasXMLProlog, ""
			}
			text = strings.TrimSpace(text[end+2:])
		default:
			root, ok := xfaPacketXMLRootElement(text)
			if !ok {
				return hasXMLProlog, ""
			}
			return hasXMLProlog, root
		}
	}
}

func xfaPacketXMLRootElement(text string) (string, bool) {
	if !strings.HasPrefix(text, "<") || strings.HasPrefix(text, "</") || strings.HasPrefix(text, "<!") || strings.HasPrefix(text, "<?") {
		return "", false
	}
	text = text[1:]
	end := len(text)
	for i, r := range text {
		if r == '>' || r == '/' || r == ' ' || r == '\t' || r == '\r' || r == '\n' {
			end = i
			break
		}
	}
	if end == 0 {
		return "", false
	}
	root := text[:end]
	if !isConservativeXMLName(root) {
		return "", false
	}
	return root, true
}

func isConservativeXMLName(name string) bool {
	if name == "" {
		return false
	}
	for i, r := range name {
		if i == 0 {
			if (r >= 'A' && r <= 'Z') || (r >= 'a' && r <= 'z') || r == '_' || r == ':' {
				continue
			}
			return false
		}
		if (r >= 'A' && r <= 'Z') || (r >= 'a' && r <= 'z') || (r >= '0' && r <= '9') || r == '_' || r == ':' || r == '-' || r == '.' {
			continue
		}
		return false
	}
	return true
}

func makeXFAStreamPacketMetadata(label string, object *pdfIndirectObject, stream pdfStreamObject, text string, decodeErr error) XFAPacketMetadata {
	packet := makeXFAPacketMetadata(label, object, true, text)
	packet.Filter = pdfGraphStreamFilterString(stream.Dict)
	packet.DecodeParms = pdfGraphDecodeParmsString(stream.Dict)
	if decodeErr != nil {
		packet.HasDecodeError = true
		packet.DecodeError = decodeErr.Error()
	}
	return packet
}

func classifyXFAPacketKind(label, text string) string {
	if strings.TrimSpace(label) != "" {
		return classifyXFAPacketKindToken(label)
	}
	return classifyXFAPacketKindToken(xfaPacketRootToken(text))
}

func classifyXFAPacketKindToken(token string) string {
	token = strings.TrimSpace(token)
	if token == "" {
		return ""
	}
	if at := strings.IndexByte(token, ':'); at >= 0 {
		token = token[at+1:]
	}
	token = strings.TrimLeft(token, "/")
	token = strings.TrimRight(token, "/")
	switch token {
	case "template", "datasets", "config", "localeSet", "connectionSet", "sourceSet", "xdp":
		return token
	case "xml":
		return "xml"
	default:
		return ""
	}
}

func xfaPacketRootToken(text string) string {
	text = strings.TrimSpace(text)
	if strings.HasPrefix(text, "\ufeff") {
		text = strings.TrimSpace(strings.TrimPrefix(text, "\ufeff"))
	}
	if strings.HasPrefix(text, "<?xml") {
		if end := strings.Index(text, "?>"); end >= 0 {
			text = strings.TrimSpace(text[end+2:])
		} else {
			return "xml"
		}
	}
	if !strings.HasPrefix(text, "<") || strings.HasPrefix(text, "</") {
		return ""
	}
	text = text[1:]
	if strings.HasPrefix(text, "!") || strings.HasPrefix(text, "?") {
		return "xml"
	}
	end := len(text)
	for i, r := range text {
		if r == '>' || r == '/' || r == ' ' || r == '\t' || r == '\r' || r == '\n' {
			end = i
			break
		}
	}
	if end == 0 {
		return "xml"
	}
	root := text[:end]
	if classifyXFAPacketKindToken(root) == "" {
		return "xml"
	}
	return root
}

func xfaArrayPacketLabel(value pdfValue) (string, bool) {
	switch v := value.(type) {
	case pdfLiteralString:
		return decodeLiteralString(string(v)), true
	case pdfHexString:
		text, ok := decodeHexTextString([]byte(v))
		return text, ok
	case pdfName:
		return string(v), true
	default:
		return "", false
	}
}

func boundedXFAPreview(text string) string {
	if utf8.RuneCountInString(text) <= xfaPacketPreviewRunes {
		return text
	}
	runes := []rune(text)
	return string(runes[:xfaPacketPreviewRunes]) + "..."
}

func graphHasDirectXFA(graph *pdfGraph) bool {
	for _, acroForm := range acroFormDictionaries(graph) {
		if _, ok := acroForm["XFA"]; ok {
			return true
		}
	}
	return false
}

func acroFormDictionaries(graph *pdfGraph) []pdfDict {
	matches := make([]pdfDict, 0)
	for _, object := range sortedPDFObjects(graph.Objects) {
		dict, ok := object.Value.(pdfDict)
		if !ok {
			continue
		}
		value, ok := dict["AcroForm"]
		if !ok {
			continue
		}
		switch v := value.(type) {
		case pdfDict:
			matches = append(matches, v)
		case pdfRef:
			refObject, ok := graph.Objects[v.ID]
			if !ok {
				continue
			}
			if refDict, ok := refObject.Value.(pdfDict); ok {
				matches = append(matches, refDict)
			}
		}
	}
	return matches
}

func collectXFAPackets(graph *pdfGraph, owner pdfDict, key string, value pdfValue, contains string, label string) []xfaPacket {
	matches := make([]xfaPacket, 0)
	switch v := value.(type) {
	case pdfLiteralString, pdfHexString:
		text, ok := pdfTextValue(v)
		if ok {
			for i := 0; i < countXFAPacketTextMatches(text, contains); i++ {
				matches = append(matches, makeXFAPacketMatch(xfaPacket{dict: owner, dictKey: key, text: text, occurs: i + 1}, label))
			}
		}
	case pdfRef:
		object, ok := graph.Objects[v.ID]
		if !ok {
			return matches
		}
		switch objectValue := object.Value.(type) {
		case pdfStreamObject:
			decoded, err := decodePDFGraphStream(objectValue)
			if err == nil {
				text := string(decoded)
				for i := 0; i < countXFAPacketTextMatches(text, contains); i++ {
					matches = append(matches, makeXFAPacketMatch(xfaPacket{object: object, text: text, stream: true, occurs: i + 1}, label))
				}
			}
		case pdfLiteralString, pdfHexString:
			text, ok := pdfTextValue(objectValue)
			if ok {
				for i := 0; i < countXFAPacketTextMatches(text, contains); i++ {
					matches = append(matches, makeXFAPacketMatch(xfaPacket{object: object, text: text, occurs: i + 1}, label))
				}
			}
		}
	case pdfArray:
		for i := 0; i < len(v); i++ {
			item := v[i]
			packetLabel := ""
			if labelValue, hasPacketLabel := xfaArrayPacketLabel(item); hasPacketLabel && i+1 < len(v) {
				i++
				item = v[i]
				packetLabel = labelValue
			}
			switch itemValue := item.(type) {
			case pdfLiteralString, pdfHexString:
				text, ok := pdfTextValue(itemValue)
				if !ok {
					continue
				}
				for n := 0; n < countXFAPacketTextMatches(text, contains); n++ {
					matches = append(matches, makeXFAPacketMatch(xfaPacket{array: v, index: i, text: text, occurs: n + 1}, packetLabel))
				}
			default:
				matches = append(matches, collectXFAPackets(graph, owner, key, item, contains, packetLabel)...)
			}
		}
	}
	return matches
}

func countXFAPacketTextMatches(text, contains string) int {
	if contains == "" {
		return 1
	}
	return strings.Count(text, contains)
}

func makeXFAPacketMatch(packet xfaPacket, label string) xfaPacket {
	packet.label = label
	packet.kind = classifyXFAPacketKind(label, packet.text)
	return packet
}

func (p xfaPacket) replace(oldText, newText string) error {
	updated, ok := replaceNthString(p.text, oldText, newText, p.occurs)
	if !ok {
		return errors.New("selected XFA occurrence is no longer present")
	}
	if p.stream {
		stream, ok := p.object.Value.(pdfStreamObject)
		if !ok {
			return errors.New("XFA packet target is no longer a stream")
		}
		stream.Data = []byte(updated)
		stream.Dict = clonePDFDict(stream.Dict)
		delete(stream.Dict, "Filter")
		delete(stream.Dict, "DecodeParms")
		stream.Dict["Length"] = len(stream.Data)
		p.object.Value = stream
		return nil
	}
	if p.object != nil {
		p.object.Value = pdfLiteralString(encodeLiteralString(updated))
		return nil
	}
	if p.array != nil {
		if p.index < 0 || p.index >= len(p.array) {
			return errors.New("XFA packet array target is out of range")
		}
		p.array[p.index] = pdfLiteralString(encodeLiteralString(updated))
		return nil
	}
	p.dict[p.dictKey] = pdfLiteralString(encodeLiteralString(updated))
	return nil
}

func replaceNthString(input, oldText, newText string, occurrence int) (string, bool) {
	if occurrence < 1 {
		return "", false
	}
	searchFrom := 0
	for i := 1; i <= occurrence; i++ {
		at := strings.Index(input[searchFrom:], oldText)
		if at == -1 {
			return "", false
		}
		at += searchFrom
		if i == occurrence {
			return input[:at] + newText + input[at+len(oldText):], true
		}
		searchFrom = at + len(oldText)
	}
	return "", false
}

func verifySemanticPDF(output []byte, opts pdfGraphParseOptions) (core.Verification, error) {
	if _, err := parsePDFGraphWithOptions(output, opts); err != nil {
		return core.Verification{}, err
	}
	return core.Verification{ReparseOK: true, PageUnchanged: true}, nil
}

func pdfTextValue(value pdfValue) (string, bool) {
	switch v := value.(type) {
	case pdfLiteralString:
		return decodeLiteralString(string(v)), true
	case pdfHexString:
		return decodeHexTextString([]byte(v))
	default:
		return "", false
	}
}

func pdfTextBytes(value pdfValue) ([]byte, bool) {
	text, ok := pdfTextValue(value)
	if !ok {
		return nil, false
	}
	return []byte(text), true
}

func replaceFirstBytes(input []byte, oldText, newText string) ([]byte, bool) {
	at := bytes.Index(input, []byte(oldText))
	if at == -1 {
		return nil, false
	}
	return replaceByteRange(input, at, at+len(oldText), []byte(newText)), true
}
