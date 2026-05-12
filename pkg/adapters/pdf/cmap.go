package pdf

import (
	"bytes"
	"encoding/hex"
	"fmt"
	"sort"
	"strconv"
	"strings"
	"unicode/utf16"
)

type pdfToUnicodeMap map[string]string
type toUnicodeCMap = pdfToUnicodeMap

type pdfCMapContext struct {
	fallback          *toUnicodeCMap
	streamFontCMaps   map[int]map[string]*toUnicodeCMap
	streamFontMetrics map[int]map[string]pdfSimpleFontMetrics
}

func singleToUnicodeCMap(input []byte) *toUnicodeCMap {
	return singleToUnicodeCMapWithOptions(input, pdfGraphParseOptions{})
}

func singleToUnicodeCMapWithOptions(input []byte, opts pdfGraphParseOptions) *toUnicodeCMap {
	return pdfCMapContextForInput(input, opts).fallback
}

func pdfCMapContextForInput(input []byte, opts pdfGraphParseOptions) pdfCMapContext {
	graph, err := parsePDFGraphWithOptions(input, opts)
	if err != nil {
		return pdfCMapContext{}
	}
	return graph.cmapContext()
}

func (g *pdfGraph) cmapContext() pdfCMapContext {
	ctx := pdfCMapContext{
		streamFontCMaps:   g.streamFontToUnicodeCMaps(),
		streamFontMetrics: g.streamFontMetrics(),
	}
	cmap, ok := g.singleToUnicodeMap()
	if !ok {
		return ctx
	}
	ctx.fallback = &cmap
	return ctx
}

func (g *pdfGraph) singleToUnicodeMap() (pdfToUnicodeMap, bool) {
	var maps []pdfToUnicodeMap
	for _, object := range sortedPDFObjects(g.Objects) {
		dict, ok := object.Value.(pdfDict)
		if !ok {
			continue
		}
		ref, ok := dict["ToUnicode"].(pdfRef)
		if !ok {
			continue
		}
		streamObject, ok := g.Objects[ref.ID]
		if !ok {
			continue
		}
		stream, ok := streamObject.Value.(pdfStreamObject)
		if !ok {
			continue
		}
		decoded, err := g.decodePDFGraphObjectStream(ref.ID, stream)
		if err != nil {
			continue
		}
		cmap, ok := parseToUnicodeCMap(decoded)
		if ok {
			maps = append(maps, cmap)
		}
	}
	if len(maps) != 1 {
		return nil, false
	}
	return maps[0], true
}

func (g *pdfGraph) streamFontToUnicodeCMaps() map[int]map[string]*toUnicodeCMap {
	out := make(map[int]map[string]*toUnicodeCMap)
	for _, object := range sortedPDFObjects(g.Objects) {
		page, ok := object.Value.(pdfDict)
		if !ok || !dictHasType(page, "Page") {
			continue
		}
		fonts := g.pageFontToUnicodeCMaps(page)
		if len(fonts) == 0 {
			continue
		}
		for _, stream := range g.pageContentStreams(page) {
			out[stream.SourceStart] = fonts
		}
	}
	return out
}

func (g *pdfGraph) pageFontToUnicodeCMaps(page pdfDict) map[string]*toUnicodeCMap {
	resources, ok := g.pageResources(page)
	if !ok {
		return nil
	}
	fonts, ok := g.resolvePDFDict(resources["Font"])
	if !ok {
		return nil
	}
	out := make(map[string]*toUnicodeCMap)
	for name, value := range fonts {
		ref, ok := value.(pdfRef)
		if !ok {
			continue
		}
		cmap, ok := g.toUnicodeCMapForFont(ref.ID)
		if !ok {
			continue
		}
		cmapCopy := cmap
		out[name] = &cmapCopy
	}
	if len(out) == 0 {
		return nil
	}
	return out
}

func (g *pdfGraph) pageResources(page pdfDict) (pdfDict, bool) {
	if resourcesValue, ok := page["Resources"]; ok {
		return g.resolvePDFDict(resourcesValue)
	}
	return g.inheritedPageResources(page["Parent"])
}

func (g *pdfGraph) inheritedPageResources(parent pdfValue) (pdfDict, bool) {
	visited := make(map[pdfObjectID]bool)
	for {
		ref, ok := parent.(pdfRef)
		if !ok {
			return nil, false
		}
		if visited[ref.ID] {
			return nil, false
		}
		visited[ref.ID] = true

		object, ok := g.Objects[ref.ID]
		if !ok {
			return nil, false
		}
		parentDict, ok := object.Value.(pdfDict)
		if !ok {
			return nil, false
		}
		if resourcesValue, ok := parentDict["Resources"]; ok {
			return g.resolvePDFDict(resourcesValue)
		}
		parent = parentDict["Parent"]
	}
}

func (g *pdfGraph) pageContentStreams(page pdfDict) []pdfStreamObject {
	value, ok := page["Contents"]
	if !ok {
		return nil
	}
	refs := make([]pdfRef, 0, 1)
	switch v := value.(type) {
	case pdfRef:
		refs = append(refs, v)
	case pdfArray:
		for _, item := range v {
			ref, ok := item.(pdfRef)
			if ok {
				refs = append(refs, ref)
			}
		}
	default:
		return nil
	}
	out := make([]pdfStreamObject, 0, len(refs))
	for _, ref := range refs {
		object, ok := g.Objects[ref.ID]
		if !ok {
			continue
		}
		stream, ok := object.Value.(pdfStreamObject)
		if ok {
			out = append(out, stream)
		}
	}
	return out
}

func (g *pdfGraph) resolvePDFDict(value pdfValue) (pdfDict, bool) {
	switch v := value.(type) {
	case pdfDict:
		return v, true
	case pdfRef:
		object, ok := g.Objects[v.ID]
		if !ok {
			return nil, false
		}
		dict, ok := object.Value.(pdfDict)
		return dict, ok
	default:
		return nil, false
	}
}

func (g *pdfGraph) toUnicodeCMapForFont(fontID pdfObjectID) (pdfToUnicodeMap, bool) {
	object, ok := g.Objects[fontID]
	if !ok {
		return nil, false
	}
	var dict pdfDict
	switch value := object.Value.(type) {
	case pdfDict:
		dict = value
	case pdfStreamObject:
		dict = value.Dict
	default:
		return nil, false
	}
	ref, ok := dict["ToUnicode"].(pdfRef)
	if !ok {
		return nil, false
	}
	streamObject, ok := g.Objects[ref.ID]
	if !ok {
		return nil, false
	}
	stream, ok := streamObject.Value.(pdfStreamObject)
	if !ok {
		return nil, false
	}
	decoded, err := g.decodePDFGraphObjectStream(ref.ID, stream)
	if err != nil {
		return nil, false
	}
	return parseToUnicodeCMap(decoded)
}

func (c pdfCMapContext) fontCMapsForStream(sourceStart int) map[string]*toUnicodeCMap {
	if c.streamFontCMaps == nil {
		return nil
	}
	return c.streamFontCMaps[sourceStart]
}

func (c pdfCMapContext) fontMetricsForStream(sourceStart int) map[string]pdfSimpleFontMetrics {
	if c.streamFontMetrics == nil {
		return nil
	}
	return c.streamFontMetrics[sourceStart]
}

func parseToUnicodeCMap(input []byte) (pdfToUnicodeMap, bool) {
	fields := bytes.Fields(input)
	out := make(pdfToUnicodeMap)
	for i := 0; i < len(fields); i++ {
		switch string(fields[i]) {
		case "beginbfchar":
			if i == 0 {
				continue
			}
			count, err := strconv.Atoi(string(fields[i-1]))
			if err != nil {
				continue
			}
			j := i + 1
			for n := 0; n < count && j+1 < len(fields); n++ {
				src, srcOK := pdfHexToken(fields[j])
				dst, dstOK := pdfHexToken(fields[j+1])
				if srcOK && dstOK {
					out[strings.ToUpper(src)] = decodeUTF16BEHex(dst)
				}
				j += 2
			}
			i = j - 1
		case "beginbfrange":
			if i == 0 {
				continue
			}
			count, err := strconv.Atoi(string(fields[i-1]))
			if err != nil {
				continue
			}
			j := i + 1
			for n := 0; n < count && j+2 < len(fields); n++ {
				startHex, startOK := pdfHexToken(fields[j])
				endHex, endOK := pdfHexToken(fields[j+1])
				if startOK && endOK && len(fields[j+2]) > 0 && fields[j+2][0] == '[' {
					j = addCMapArrayRange(out, startHex, endHex, fields, j+2)
					continue
				}
				dstHex, dstOK := pdfHexToken(fields[j+2])
				if startOK && endOK && dstOK {
					addCMapSequentialRange(out, startHex, endHex, dstHex)
				}
				j += 3
			}
			i = j - 1
		}
	}
	return out, len(out) > 0
}

func (m pdfToUnicodeMap) DecodeHex(encoded []byte) (string, bool) {
	compact := make([]byte, 0, len(encoded))
	for _, b := range encoded {
		if !isPDFSpace(b) {
			compact = append(compact, b)
		}
	}
	if len(compact) == 0 || len(compact)%2 != 0 {
		return "", false
	}
	var out strings.Builder
	for i := 0; i < len(compact); {
		matched := false
		for width := min(len(compact)-i, 8); width >= 2; width -= 2 {
			token := strings.ToUpper(string(compact[i : i+width]))
			if value, ok := m[token]; ok {
				out.WriteString(value)
				i += width
				matched = true
				break
			}
		}
		if !matched {
			return "", false
		}
	}
	return out.String(), true
}

func (m pdfToUnicodeMap) EncodeHex(text string) (string, bool) {
	keys := make([]string, 0, len(m))
	for encoded := range m {
		keys = append(keys, encoded)
	}
	sort.Strings(keys)

	reverse := make(map[string]string, len(m))
	decodedKeys := make([]string, 0, len(m))
	for _, encoded := range keys {
		decoded := m[encoded]
		if _, exists := reverse[decoded]; !exists {
			reverse[decoded] = encoded
			decodedKeys = append(decodedKeys, decoded)
		}
	}
	sort.Slice(decodedKeys, func(i, j int) bool {
		if len(decodedKeys[i]) == len(decodedKeys[j]) {
			return decodedKeys[i] < decodedKeys[j]
		}
		return len(decodedKeys[i]) > len(decodedKeys[j])
	})

	var out strings.Builder
	for len(text) > 0 {
		matched := false
		for _, decoded := range decodedKeys {
			if decoded == "" || !strings.HasPrefix(text, decoded) {
				continue
			}
			out.WriteString(reverse[decoded])
			text = text[len(decoded):]
			matched = true
			break
		}
		if !matched {
			return "", false
		}
	}
	return out.String(), true
}

func decodeHexBytes(encoded []byte) ([]byte, bool) {
	compact := make([]byte, 0, len(encoded))
	for _, b := range encoded {
		if !isPDFSpace(b) {
			compact = append(compact, b)
		}
	}
	if len(compact)%2 != 0 {
		return nil, false
	}
	decoded := make([]byte, 0, len(compact)/2)
	for i := 0; i < len(compact); i += 2 {
		hi, ok := hexNibble(compact[i])
		if !ok {
			return nil, false
		}
		lo, ok := hexNibble(compact[i+1])
		if !ok {
			return nil, false
		}
		decoded = append(decoded, hi<<4|lo)
	}
	return decoded, true
}

func pdfHexToken(token []byte) (string, bool) {
	token = bytes.TrimPrefix(token, []byte("["))
	token = bytes.TrimSuffix(token, []byte("]"))
	if len(token) < 2 || token[0] != '<' || token[len(token)-1] != '>' {
		return "", false
	}
	raw := string(token[1 : len(token)-1])
	if _, err := hex.DecodeString(raw); err != nil {
		return "", false
	}
	return raw, true
}

func addCMapSequentialRange(out pdfToUnicodeMap, startHex, endHex, dstHex string) {
	start, err1 := strconv.ParseInt(startHex, 16, 64)
	end, err2 := strconv.ParseInt(endHex, 16, 64)
	dst, err3 := strconv.ParseInt(dstHex, 16, 64)
	if err1 != nil || err2 != nil || err3 != nil || end < start {
		return
	}
	width := len(startHex)
	for value := start; value <= end; value++ {
		out[fmt.Sprintf("%0*X", width, value)] = decodeUTF16BECodepoint(dst + (value - start))
	}
}

func addCMapArrayRange(out pdfToUnicodeMap, startHex, endHex string, fields [][]byte, startField int) int {
	start, err1 := strconv.ParseInt(startHex, 16, 64)
	end, err2 := strconv.ParseInt(endHex, 16, 64)
	if err1 != nil || err2 != nil || end < start {
		return skipCMapArray(fields, startField)
	}
	width := len(startHex)
	value := start
	j := startField
	for j < len(fields) && value <= end {
		token, closed := trimCMapArrayToken(fields[j])
		if token != nil {
			if dstHex, ok := pdfHexToken(token); ok {
				out[fmt.Sprintf("%0*X", width, value)] = decodeUTF16BEHex(dstHex)
				value++
			}
		}
		j++
		if closed {
			return j
		}
	}
	return skipCMapArray(fields, j)
}

func skipCMapArray(fields [][]byte, start int) int {
	for i := start; i < len(fields); i++ {
		if bytes.Contains(fields[i], []byte("]")) {
			return i + 1
		}
	}
	return len(fields)
}

func trimCMapArrayToken(token []byte) ([]byte, bool) {
	closed := bytes.Contains(token, []byte("]"))
	token = bytes.TrimPrefix(token, []byte("["))
	token = bytes.TrimSuffix(token, []byte("]"))
	if len(token) == 0 {
		return nil, closed
	}
	return token, closed
}

func decodeUTF16BEHex(raw string) string {
	if len(raw)%4 != 0 {
		return ""
	}
	values := make([]uint16, 0, len(raw)/4)
	for i := 0; i < len(raw); i += 4 {
		v, err := strconv.ParseUint(raw[i:i+4], 16, 16)
		if err != nil {
			return ""
		}
		values = append(values, uint16(v))
	}
	return string(utf16.Decode(values))
}

func decodeUTF16BECodepoint(value int64) string {
	if value < 0 || value > 0xffff {
		return ""
	}
	return string(utf16.Decode([]uint16{uint16(value)}))
}
