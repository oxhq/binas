package pdf

import (
	"bytes"
	"errors"
	"fmt"
	"sort"
	"strconv"
)

type incrementalObjectUpdate struct {
	ID    pdfObjectID
	Value pdfValue
}

func appendIncrementalUpdate(input []byte, updates []incrementalObjectUpdate, trailerOverrides pdfDict) ([]byte, error) {
	if !bytes.HasPrefix(input, []byte("%PDF-")) {
		return nil, errors.New("not a PDF file")
	}
	if len(updates) == 0 {
		return nil, errors.New("incremental update requires at least one indirect object")
	}
	previousXrefOffset, err := lastStartXrefOffset(input)
	if err != nil {
		return nil, err
	}
	updates = append([]incrementalObjectUpdate(nil), updates...)
	sort.Slice(updates, func(i, j int) bool {
		if updates[i].ID.Number != updates[j].ID.Number {
			return updates[i].ID.Number < updates[j].ID.Number
		}
		return updates[i].ID.Generation < updates[j].ID.Generation
	})
	seenNumbers := make(map[int]struct{}, len(updates))
	for _, update := range updates {
		if update.ID.Number <= 0 {
			return nil, fmt.Errorf("invalid incremental object number %d", update.ID.Number)
		}
		if update.ID.Generation < 0 || update.ID.Generation > 65535 {
			return nil, fmt.Errorf("invalid incremental object generation %d", update.ID.Generation)
		}
		if _, exists := seenNumbers[update.ID.Number]; exists {
			return nil, fmt.Errorf("duplicate incremental update for object %d", update.ID.Number)
		}
		seenNumbers[update.ID.Number] = struct{}{}
	}

	var out bytes.Buffer
	out.Grow(len(input) + len(updates)*64 + 256)
	out.Write(input)
	if len(input) > 0 && input[len(input)-1] != '\n' && input[len(input)-1] != '\r' {
		out.WriteByte('\n')
	}

	offsets := make(map[pdfObjectID]int, len(updates))
	for _, update := range updates {
		offsets[update.ID] = out.Len()
		fmt.Fprintf(&out, "%d %d obj\n", update.ID.Number, update.ID.Generation)
		if err := writePDFValue(&out, update.Value); err != nil {
			return nil, fmt.Errorf("write incremental object %d %d: %w", update.ID.Number, update.ID.Generation, err)
		}
		out.WriteString("\nendobj\n")
	}

	xrefOffset := out.Len()
	out.WriteString("xref\n")
	for _, update := range updates {
		offset := offsets[update.ID]
		fmt.Fprintf(&out, "%d 1\n%010d %05d n \n", update.ID.Number, offset, update.ID.Generation)
	}
	out.WriteString("trailer\n")
	trailer := incrementalTrailer(input, updates, trailerOverrides, previousXrefOffset)
	if err := writePDFValue(&out, trailer); err != nil {
		return nil, err
	}
	out.WriteString("\nstartxref\n")
	out.WriteString(strconv.Itoa(xrefOffset))
	out.WriteString("\n%%EOF\n")
	return out.Bytes(), nil
}

func incrementalTrailer(input []byte, updates []incrementalObjectUpdate, overrides pdfDict, previousXrefOffset int) pdfDict {
	trailer := clonePDFDict(parseLastTrailerDictionary(input))
	if trailer == nil {
		trailer = make(pdfDict)
	}
	delete(trailer, "Prev")
	delete(trailer, "XRefStm")
	for key, value := range overrides {
		trailer[key] = value
	}
	size := maxIncrementalObjectNumber(input, updates) + 1
	if overrideSize, ok := dictInt(trailer, "Size"); ok && overrideSize > size {
		size = overrideSize
	}
	trailer["Size"] = size
	trailer["Prev"] = previousXrefOffset
	return trailer
}

func maxIncrementalObjectNumber(input []byte, updates []incrementalObjectUpdate) int {
	maxObject := 0
	for _, object := range findXrefObjectOffsets(input) {
		if object.Number > maxObject {
			maxObject = object.Number
		}
	}
	if trailer := parseLastTrailerDictionary(input); trailer != nil {
		if size, ok := dictInt(trailer, "Size"); ok && size > 0 && size-1 > maxObject {
			maxObject = size - 1
		}
	}
	for _, update := range updates {
		if update.ID.Number > maxObject {
			maxObject = update.ID.Number
		}
	}
	return maxObject
}

func lastStartXrefOffset(input []byte) (int, error) {
	startxrefAt := bytes.LastIndex(input, []byte("startxref"))
	if startxrefAt == -1 {
		return 0, errors.New("malformed PDF: startxref not found")
	}
	i := startxrefAt + len("startxref")
	for i < len(input) && isPDFSpace(input[i]) {
		i++
	}
	valueStart := i
	for i < len(input) && isPDFDigit(input[i]) {
		i++
	}
	if valueStart == i {
		return 0, errors.New("malformed PDF: startxref offset not found")
	}
	offset, err := strconv.Atoi(string(input[valueStart:i]))
	if err != nil {
		return 0, err
	}
	if offset < 0 || offset >= len(input) {
		return 0, fmt.Errorf("malformed PDF: startxref offset %d is outside input", offset)
	}
	return offset, nil
}
