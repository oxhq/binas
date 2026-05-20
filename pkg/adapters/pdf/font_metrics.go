package pdf

type pdfSimpleFontMetrics struct {
	FirstChar    int
	Widths       []int
	MissingWidth *int
}

func (g *pdfGraph) streamFontMetrics() map[int]map[string]pdfSimpleFontMetrics {
	out := make(map[int]map[string]pdfSimpleFontMetrics)
	for _, object := range sortedPDFObjects(g.Objects) {
		page, ok := object.Value.(pdfDict)
		if !ok || !dictHasType(page, "Page") {
			continue
		}
		fonts := g.pageFontMetrics(page)
		if len(fonts) == 0 {
			continue
		}
		for _, stream := range g.pageContentStreams(page) {
			out[stream.SourceStart] = fonts
		}
	}
	return out
}

func (g *pdfGraph) pageFontMetrics(page pdfDict) map[string]pdfSimpleFontMetrics {
	resources, ok := g.pageResources(page)
	if !ok {
		return nil
	}
	fonts, ok := g.resolvePDFDict(resources["Font"])
	if !ok {
		return nil
	}
	out := make(map[string]pdfSimpleFontMetrics)
	for name, value := range fonts {
		fontDict, ok := g.resolvePDFDict(value)
		if !ok {
			continue
		}
		metrics, ok := parseSimpleFontMetrics(fontDict)
		if !ok {
			continue
		}
		out[name] = metrics
	}
	if len(out) == 0 {
		return nil
	}
	return out
}

func parseSimpleFontMetrics(font pdfDict) (pdfSimpleFontMetrics, bool) {
	firstChar, ok := dictInt(font, "FirstChar")
	if !ok {
		return pdfSimpleFontMetrics{}, false
	}
	widths, ok := dictIntArray(font, "Widths")
	if !ok || len(widths) == 0 {
		return pdfSimpleFontMetrics{}, false
	}
	metrics := pdfSimpleFontMetrics{
		FirstChar: firstChar,
		Widths:    widths,
	}
	if missingWidth, ok := dictInt(font, "MissingWidth"); ok {
		metrics.MissingWidth = &missingWidth
	}
	return metrics, true
}

func enrichTextShowFontWidthMetadata(meta map[string]any, font string, encoded string, encoding string, fontMetrics map[string]pdfSimpleFontMetrics) {
	if font == "" || fontMetrics == nil {
		return
	}
	metrics, ok := fontMetrics[font]
	if !ok {
		return
	}
	meta["font"] = font
	codes, ok := textShowSimpleFontCodes(encoded, encoding)
	if !ok {
		return
	}
	widthUnits, missingUsed, ok := metrics.widthUnits(codes)
	if !ok {
		return
	}
	meta["width_units"] = widthUnits
	annotateTextShowLayoutProofMetadata(meta, &widthUnits, nil)
	meta["font_first_char"] = metrics.FirstChar
	meta["font_widths"] = append([]int(nil), metrics.Widths...)
	if missingUsed {
		meta["width_source"] = "/Widths+/MissingWidth"
		meta["missing_width_used"] = true
		if metrics.MissingWidth != nil {
			meta["font_missing_width"] = *metrics.MissingWidth
		}
		return
	}
	meta["width_source"] = "/Widths"
}

func textShowReplacementLayoutProofMetadata(nodeMeta map[string]any, newEncoded string) map[string]any {
	if nodeMeta == nil {
		return nil
	}
	oldWidth, ok := metadataInt(nodeMeta["width_units"])
	if !ok {
		return nil
	}
	metrics, ok := simpleFontMetricsFromMetadata(nodeMeta)
	if !ok {
		return textShowCIDReplacementLayoutProofMetadata(nodeMeta, newEncoded)
	}
	encoding, _ := nodeMeta["encoding"].(string)
	codes, ok := textShowSimpleFontCodes(newEncoded, encoding)
	if !ok {
		return map[string]any{
			"layout_proof": layoutProofStatusWidthUnproven,
		}
	}
	newWidth, missingUsed, ok := metrics.widthUnits(codes)
	if !ok {
		return map[string]any{
			"layout_proof": layoutProofStatusWidthUnproven,
		}
	}
	out := map[string]any{
		"old_width_units": oldWidth,
		"new_width_units": newWidth,
	}
	annotateTextShowLayoutProofMetadata(out, &oldWidth, &newWidth)
	if missingUsed {
		out["missing_width_used"] = true
	}
	return out
}

func simpleFontMetricsFromMetadata(meta map[string]any) (pdfSimpleFontMetrics, bool) {
	firstChar, ok := metadataInt(meta["font_first_char"])
	if !ok {
		return pdfSimpleFontMetrics{}, false
	}
	widths, ok := metadataIntSlice(meta["font_widths"])
	if !ok || len(widths) == 0 {
		return pdfSimpleFontMetrics{}, false
	}
	metrics := pdfSimpleFontMetrics{
		FirstChar: firstChar,
		Widths:    widths,
	}
	if missingWidth, ok := metadataInt(meta["font_missing_width"]); ok {
		metrics.MissingWidth = &missingWidth
	}
	return metrics, true
}

func metadataInt(value any) (int, bool) {
	switch v := value.(type) {
	case int:
		return v, true
	case int64:
		return int(v), true
	case float64:
		if v == float64(int(v)) {
			return int(v), true
		}
	}
	return 0, false
}

func metadataIntSlice(value any) ([]int, bool) {
	switch v := value.(type) {
	case []int:
		return append([]int(nil), v...), true
	case []any:
		out := make([]int, 0, len(v))
		for _, item := range v {
			n, ok := metadataInt(item)
			if !ok {
				return nil, false
			}
			out = append(out, n)
		}
		return out, true
	}
	return nil, false
}

func (m pdfSimpleFontMetrics) widthUnits(codes []byte) (int, bool, bool) {
	total := 0
	missingUsed := false
	for _, code := range codes {
		index := int(code) - m.FirstChar
		if index >= 0 && index < len(m.Widths) {
			total += m.Widths[index]
			continue
		}
		if m.MissingWidth == nil {
			return 0, false, false
		}
		total += *m.MissingWidth
		missingUsed = true
	}
	return total, missingUsed, true
}

func textShowSimpleFontCodes(encoded string, encoding string) ([]byte, bool) {
	switch encoding {
	case "literal":
		return []byte(decodeLiteralString(encoded)), true
	case "hex", "hex-cmap":
		return decodeHexBytes([]byte(encoded))
	case "tj-array", "tj-array-cmap":
		return simpleTJArrayFontCodes([]byte(encoded))
	default:
		return nil, false
	}
}

func simpleTJArrayFontCodes(input []byte) ([]byte, bool) {
	if len(input) == 0 || input[0] != '[' {
		return nil, false
	}
	out := make([]byte, 0)
	i := 1
	for {
		i = skipPDFSpaceAndComments(input, i)
		if i >= len(input) {
			return nil, false
		}
		switch input[i] {
		case ']':
			return out, true
		case '(':
			closeAt, ok := findLiteralEnd(input, i+1, len(input))
			if !ok {
				return nil, false
			}
			out = append(out, []byte(decodeLiteralString(string(input[i+1:closeAt])))...)
			i = closeAt + 1
		case '<':
			if i+1 < len(input) && input[i+1] == '<' {
				return nil, false
			}
			closeAt, ok := findHexStringEnd(input, i+1, len(input))
			if !ok {
				return nil, false
			}
			decoded, ok := decodeHexBytes(input[i+1 : closeAt])
			if !ok {
				return nil, false
			}
			out = append(out, decoded...)
			i = closeAt + 1
		default:
			numberEnd, ok := scanPDFNumber(input, i, len(input))
			if !ok || !isPDFTokenEnd(input, numberEnd) {
				return nil, false
			}
			i = numberEnd
		}
	}
}
