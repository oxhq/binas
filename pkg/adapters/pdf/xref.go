package pdf

import (
	"bytes"
	"regexp"
	"sort"
	"strconv"
)

type xrefSummary struct {
	Objects                 []xrefObjectOffset
	HasTable                bool
	TableOffset             int
	HasHybridStream         bool
	HybridStreamOffset      int
	HybridStreamObject      xrefObjectOffset
	HasStream               bool
	StreamObjects           []xrefObjectOffset
	UnsupportedXrefStream   bool
	HasObjectStream         bool
	ObjectStreamObjects     []xrefObjectOffset
	UnsupportedObjectStream bool
}

type xrefObjectOffset struct {
	Number             int  `json:"number"`
	Generation         int  `json:"generation"`
	Offset             int  `json:"offset"`
	Compressed         bool `json:"compressed,omitempty"`
	ObjectStreamNumber int  `json:"object_stream_number,omitempty"`
	ObjectStreamIndex  int  `json:"object_stream_index,omitempty"`
}

var (
	indirectObjectHeaderRe = regexp.MustCompile(`(?m)^(\d+)\s+(\d+)\s+obj\b`)
)

func summarizeXref(input []byte) xrefSummary {
	objects := findXrefObjectOffsets(input)
	tableOffset, hasTable := findXrefTableOffset(input)
	streamObjects := findXrefStreamObjects(input, objects)
	hybridStreamOffset := -1
	hybridStreamObject := xrefObjectOffset{}
	hasHybridStream := false
	hybridStreamUnsupported := false
	if trailer := parseLastTrailerDictionary(input); trailer != nil {
		if _, exists := trailer["XRefStm"]; exists {
			hasHybridStream = true
			offset, ok := dictInt(trailer, "XRefStm")
			if !ok {
				hybridStreamUnsupported = true
			} else {
				hybridStreamOffset = offset
				object, found := findXrefObjectOffsetAt(objects, offset)
				if !found || !containsXrefObjectOffset(streamObjects, object) {
					hybridStreamUnsupported = true
				} else {
					hybridStreamObject = object
				}
			}
		}
	}
	objectStreamObjects := findObjectStreamObjects(input, objects)
	unsupportedXrefStream := len(streamObjects) > 0
	if streamEntries, err := parseXrefStreamEntries(input, objects); err == nil {
		objects = mergeXrefObjectOffsets(objects, streamEntries)
		unsupportedXrefStream = false
	}
	unsupportedObjectStream := len(objectStreamObjects) > 0
	if objectStreamEntries, err := parseObjectStreamEntries(input, objects); err == nil {
		objects = mergeXrefObjectOffsets(objects, objectStreamEntries)
		unsupportedObjectStream = false
	}

	return xrefSummary{
		Objects:                 objects,
		HasTable:                hasTable,
		TableOffset:             tableOffset,
		HasHybridStream:         hasHybridStream,
		HybridStreamOffset:      hybridStreamOffset,
		HybridStreamObject:      hybridStreamObject,
		HasStream:               len(streamObjects) > 0,
		StreamObjects:           streamObjects,
		UnsupportedXrefStream:   unsupportedXrefStream || hybridStreamUnsupported,
		HasObjectStream:         len(objectStreamObjects) > 0,
		ObjectStreamObjects:     objectStreamObjects,
		UnsupportedObjectStream: unsupportedObjectStream,
	}
}

func mergeXrefObjectOffsets(base, extra []xrefObjectOffset) []xrefObjectOffset {
	if len(extra) == 0 {
		return base
	}
	merged := append([]xrefObjectOffset(nil), base...)
	indexByID := make(map[[2]int]int, len(merged))
	for i, object := range merged {
		indexByID[[2]int{object.Number, object.Generation}] = i
	}
	for _, object := range extra {
		key := [2]int{object.Number, object.Generation}
		if index, ok := indexByID[key]; ok {
			merged[index] = object
			continue
		}
		indexByID[key] = len(merged)
		merged = append(merged, object)
	}
	sort.Slice(merged, func(i, j int) bool {
		if merged[i].Number == merged[j].Number {
			return merged[i].Generation < merged[j].Generation
		}
		return merged[i].Number < merged[j].Number
	})
	return merged
}

func findXrefObjectOffsets(input []byte) []xrefObjectOffset {
	matches := indirectObjectHeaderRe.FindAllSubmatchIndex(input, -1)
	objects := make([]xrefObjectOffset, 0, len(matches))
	for _, match := range matches {
		number, numberErr := strconv.Atoi(string(input[match[2]:match[3]]))
		generation, generationErr := strconv.Atoi(string(input[match[4]:match[5]]))
		if numberErr != nil || generationErr != nil {
			continue
		}
		objects = append(objects, xrefObjectOffset{
			Number:     number,
			Generation: generation,
			Offset:     match[0],
		})
	}
	return objects
}

func findXrefObjectOffsetAt(objects []xrefObjectOffset, offset int) (xrefObjectOffset, bool) {
	for _, object := range objects {
		if !object.Compressed && object.Offset == offset {
			return object, true
		}
	}
	return xrefObjectOffset{}, false
}

func containsXrefObjectOffset(objects []xrefObjectOffset, object xrefObjectOffset) bool {
	for _, candidate := range objects {
		if candidate.Number == object.Number && candidate.Generation == object.Generation && candidate.Offset == object.Offset {
			return true
		}
	}
	return false
}

func findXrefTableOffset(input []byte) (int, bool) {
	offset := -1
	for lineStart := 0; lineStart <= len(input); {
		nextNewline := bytes.IndexByte(input[lineStart:], '\n')
		lineEnd := len(input)
		nextLineStart := len(input) + 1
		if nextNewline != -1 {
			lineEnd = lineStart + nextNewline
			nextLineStart = lineEnd + 1
		}
		if lineEnd > lineStart && input[lineEnd-1] == '\r' {
			lineEnd--
		}
		if bytes.Equal(input[lineStart:lineEnd], []byte("xref")) {
			offset = lineStart
		}
		if nextLineStart > len(input) {
			break
		}
		lineStart = nextLineStart
	}
	return offset, offset != -1
}

func findXrefStreamObjects(input []byte, objects []xrefObjectOffset) []xrefObjectOffset {
	return findStreamObjects(input, objects, "Type", "XRef")
}

func findObjectStreamObjects(input []byte, objects []xrefObjectOffset) []xrefObjectOffset {
	return findStreamObjects(input, objects, "Type", "ObjStm")
}

func findStreamObjects(input []byte, objects []xrefObjectOffset, key, value string) []xrefObjectOffset {
	if len(objects) == 0 {
		return nil
	}
	sortedObjects := append([]xrefObjectOffset(nil), objects...)
	sort.Slice(sortedObjects, func(i, j int) bool {
		return sortedObjects[i].Offset < sortedObjects[j].Offset
	})

	streamObjects := make([]xrefObjectOffset, 0)
	for i, object := range sortedObjects {
		end := len(input)
		if i+1 < len(sortedObjects) {
			end = sortedObjects[i+1].Offset
		}
		if object.Offset < 0 || object.Offset >= end || end > len(input) {
			continue
		}
		body := input[object.Offset:end]
		dictionary, hasStream := findDictionaryBeforeStream(body)
		if hasStream && dictionaryHasDirectNameEntry(dictionary, key, value) {
			streamObjects = append(streamObjects, object)
		}
	}
	return streamObjects
}

func dictionaryHasDirectNameEntry(dictionary []byte, key, value string) bool {
	dictDepth := 0
	arrayDepth := 0
	for i := 0; i < len(dictionary); {
		switch {
		case i+1 < len(dictionary) && dictionary[i] == '<' && dictionary[i+1] == '<':
			dictDepth++
			i += 2
			continue
		case i+1 < len(dictionary) && dictionary[i] == '>' && dictionary[i+1] == '>':
			if dictDepth > 0 {
				dictDepth--
			}
			i += 2
			continue
		case dictionary[i] == '(':
			i = skipPDFLiteralString(dictionary, i+1)
			continue
		case dictionary[i] == '<':
			i = skipPDFHexString(dictionary, i+1)
			continue
		case dictionary[i] == '%':
			i = skipPDFComment(dictionary, i+1)
			continue
		case dictionary[i] == '[':
			if dictDepth == 1 {
				arrayDepth++
			}
			i++
			continue
		case dictionary[i] == ']':
			if dictDepth == 1 && arrayDepth > 0 {
				arrayDepth--
			}
			i++
			continue
		case dictDepth == 1 && arrayDepth == 0 && dictionary[i] == '/':
			name, next := readPDFName(dictionary, i)
			if name != key {
				i = next
				continue
			}
			next = skipPDFSpaceAndComments(dictionary, next)
			if next >= len(dictionary) || dictionary[next] != '/' {
				i = next
				continue
			}
			gotValue, _ := readPDFName(dictionary, next)
			return gotValue == value
		default:
			i++
		}
	}
	return false
}

func readPDFName(input []byte, start int) (string, int) {
	if start >= len(input) || input[start] != '/' {
		return "", start
	}
	end := start + 1
	for end < len(input) && !isPDFSpace(input[end]) && !isPDFDelimiter(input[end]) {
		end++
	}
	return string(input[start+1 : end]), end
}

func skipPDFSpaceAndComments(input []byte, start int) int {
	i := start
	for i < len(input) {
		if isPDFSpace(input[i]) {
			i++
			continue
		}
		if input[i] == '%' {
			i = skipPDFComment(input, i+1)
			continue
		}
		return i
	}
	return i
}

func skipPDFComment(input []byte, start int) int {
	i := start
	for i < len(input) && input[i] != '\n' && input[i] != '\r' {
		i++
	}
	return i
}

func skipPDFLiteralString(input []byte, start int) int {
	depth := 1
	escaped := false
	for i := start; i < len(input); i++ {
		if escaped {
			escaped = false
			continue
		}
		switch input[i] {
		case '\\':
			escaped = true
		case '(':
			depth++
		case ')':
			depth--
			if depth == 0 {
				return i + 1
			}
		}
	}
	return len(input)
}

func skipPDFHexString(input []byte, start int) int {
	for i := start; i < len(input); i++ {
		if input[i] == '>' {
			return i + 1
		}
	}
	return len(input)
}

func findDictionaryBeforeStream(body []byte) ([]byte, bool) {
	streamOffset := findStreamKeyword(body)
	if streamOffset == -1 {
		return nil, false
	}

	prefix := body[:streamOffset]
	dictionaryStart := bytes.Index(prefix, []byte("<<"))
	dictionaryEnd := bytes.LastIndex(prefix, []byte(">>"))
	if dictionaryStart == -1 || dictionaryEnd == -1 || dictionaryEnd < dictionaryStart {
		return nil, false
	}
	return prefix[dictionaryStart : dictionaryEnd+len(">>")], true
}

func findStreamKeyword(body []byte) int {
	for offset := 0; offset < len(body); {
		index := bytes.Index(body[offset:], []byte("stream"))
		if index == -1 {
			return -1
		}
		index += offset
		end := index + len("stream")
		if isPDFTokenBoundary(body, index-1) && isPDFTokenBoundary(body, end) {
			return index
		}
		offset = end
	}
	return -1
}

func isPDFTokenBoundary(body []byte, index int) bool {
	if index < 0 || index >= len(body) {
		return true
	}
	switch body[index] {
	case 0x00, '\t', '\n', '\f', '\r', ' ', '(', ')', '<', '>', '[', ']', '{', '}', '/', '%':
		return true
	default:
		return false
	}
}
