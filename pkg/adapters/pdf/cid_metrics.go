package pdf

type pdfCIDFontMetrics struct {
	Encoding     string
	Widths       map[int]int
	DefaultWidth *int
	Registry     string
	Ordering     string
	Supplement   *int
}

func (g *pdfGraph) streamCIDFontMetrics() map[int]map[string]pdfCIDFontMetrics {
	out := make(map[int]map[string]pdfCIDFontMetrics)
	for _, object := range sortedPDFObjects(g.Objects) {
		page, ok := object.Value.(pdfDict)
		if !ok || !dictHasType(page, "Page") {
			continue
		}
		fonts := g.pageCIDFontMetrics(page)
		if len(fonts) == 0 {
			continue
		}
		for _, stream := range g.pageContentStreams(page) {
			out[stream.SourceStart] = fonts
		}
	}
	return out
}

func (g *pdfGraph) pageCIDFontMetrics(page pdfDict) map[string]pdfCIDFontMetrics {
	resources, ok := g.pageResources(page)
	if !ok {
		return nil
	}
	return g.cidFontMetricsForResources(resources)
}

func (g *pdfGraph) cidFontMetricsForResources(resources pdfDict) map[string]pdfCIDFontMetrics {
	fonts, ok := g.resolvePDFDict(resources["Font"])
	if !ok {
		return nil
	}
	out := make(map[string]pdfCIDFontMetrics)
	for name, value := range fonts {
		fontDict, ok := g.resolvePDFDict(value)
		if !ok {
			continue
		}
		metrics, ok := g.parseType0CIDFontMetrics(fontDict)
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

func (g *pdfGraph) parseType0CIDFontMetrics(font pdfDict) (pdfCIDFontMetrics, bool) {
	if !pdfDictHasSubtype(font, "Type0") {
		return pdfCIDFontMetrics{}, false
	}
	encoding, ok := pdfNameValue(font["Encoding"])
	if !ok || (encoding != "Identity-H" && encoding != "Identity-V") {
		return pdfCIDFontMetrics{}, false
	}
	descendants, ok := font["DescendantFonts"].(pdfArray)
	if !ok || len(descendants) != 1 {
		return pdfCIDFontMetrics{}, false
	}
	descendant, ok := g.resolvePDFDict(descendants[0])
	if !ok || (!pdfDictHasSubtype(descendant, "CIDFontType0") && !pdfDictHasSubtype(descendant, "CIDFontType2")) {
		return pdfCIDFontMetrics{}, false
	}
	metrics := pdfCIDFontMetrics{
		Encoding: encoding,
		Widths:   make(map[int]int),
	}
	if widthsArray, ok := descendant["W"].(pdfArray); ok {
		widths, ok := parseCIDWidthArray(widthsArray)
		if !ok {
			return pdfCIDFontMetrics{}, false
		}
		metrics.Widths = widths
	}
	if defaultWidth, ok := dictInt(descendant, "DW"); ok {
		metrics.DefaultWidth = &defaultWidth
	}
	if len(metrics.Widths) == 0 && metrics.DefaultWidth == nil {
		return pdfCIDFontMetrics{}, false
	}
	if systemInfo, ok := g.resolvePDFDict(descendant["CIDSystemInfo"]); ok {
		metrics.Registry, _ = pdfTextStringValue(systemInfo["Registry"])
		metrics.Ordering, _ = pdfTextStringValue(systemInfo["Ordering"])
		if supplement, ok := dictInt(systemInfo, "Supplement"); ok {
			metrics.Supplement = &supplement
		}
	}
	return metrics, true
}

func parseCIDWidthArray(values pdfArray) (map[int]int, bool) {
	out := make(map[int]int)
	for i := 0; i < len(values); {
		start, ok := pdfIntValue(values[i])
		if !ok || start < 0 {
			return nil, false
		}
		i++
		if i >= len(values) {
			return nil, false
		}
		if widths, ok := values[i].(pdfArray); ok {
			for offset, value := range widths {
				width, ok := pdfIntValue(value)
				if !ok {
					return nil, false
				}
				out[start+offset] = width
			}
			i++
			continue
		}
		end, ok := pdfIntValue(values[i])
		if !ok || end < start {
			return nil, false
		}
		i++
		if i >= len(values) {
			return nil, false
		}
		width, ok := pdfIntValue(values[i])
		if !ok {
			return nil, false
		}
		for cid := start; cid <= end; cid++ {
			out[cid] = width
		}
		i++
	}
	return out, true
}

func enrichTextShowCIDWidthMetadata(meta map[string]any, font string, encoded string, encoding string, cidMetrics map[string]pdfCIDFontMetrics) {
	if font == "" || cidMetrics == nil || (encoding != "hex-cmap" && encoding != "tj-array-cmap") {
		return
	}
	metrics, ok := cidMetrics[font]
	if !ok {
		return
	}
	cids, ok := textShowCIDFontCodes(encoded, encoding, metrics.Encoding)
	if !ok {
		return
	}
	widthUnits, defaultWidthUsed, ok := metrics.widthUnits(cids)
	if !ok {
		return
	}
	meta["font"] = font
	meta["width_units"] = widthUnits
	annotateTextShowLayoutProofMetadata(meta, &widthUnits, nil)
	meta["width_source"] = metrics.widthSource(defaultWidthUsed)
	meta["width_proof"] = textWidthProofStatusKnown
	meta["font_metrics_source"] = "cid_font_widths"
	meta["text_editability_status"] = textEditabilityStatusReplaceableCandidate
	if defaultWidthUsed {
		meta["cid_default_width_used"] = true
		if metrics.DefaultWidth != nil {
			meta["cid_default_width"] = *metrics.DefaultWidth
		}
	}
	meta["cid_encoding"] = metrics.Encoding
	meta["cid_widths"] = copyIntMap(metrics.Widths)
	if metrics.Registry != "" {
		meta["cid_system_registry"] = metrics.Registry
	}
	if metrics.Ordering != "" {
		meta["cid_system_ordering"] = metrics.Ordering
	}
	if metrics.Supplement != nil {
		meta["cid_system_supplement"] = *metrics.Supplement
	}
}

func textShowCIDReplacementLayoutProofMetadata(nodeMeta map[string]any, newEncoded string) map[string]any {
	if nodeMeta == nil {
		return nil
	}
	oldWidth, ok := metadataInt(nodeMeta["width_units"])
	if !ok {
		return nil
	}
	metrics, ok := cidFontMetricsFromMetadata(nodeMeta)
	if !ok {
		return nil
	}
	encoding, _ := nodeMeta["encoding"].(string)
	cids, ok := textShowCIDFontCodes(newEncoded, encoding, metrics.Encoding)
	if !ok {
		return map[string]any{
			"layout_proof": layoutProofStatusWidthUnproven,
			"width_proof":  textWidthProofStatusUnproven,
		}
	}
	newWidth, defaultWidthUsed, ok := metrics.widthUnits(cids)
	if !ok {
		return map[string]any{
			"layout_proof": layoutProofStatusWidthUnproven,
			"width_proof":  textWidthProofStatusUnproven,
		}
	}
	out := map[string]any{
		"old_width_units": oldWidth,
		"new_width_units": newWidth,
	}
	annotateTextShowLayoutProofMetadata(out, &oldWidth, &newWidth)
	if defaultWidthUsed {
		out["cid_default_width_used"] = true
	}
	return out
}

func cidFontMetricsFromMetadata(meta map[string]any) (pdfCIDFontMetrics, bool) {
	encoding, ok := meta["cid_encoding"].(string)
	if !ok || (encoding != "Identity-H" && encoding != "Identity-V") {
		return pdfCIDFontMetrics{}, false
	}
	widths, ok := metadataIntMap(meta["cid_widths"])
	if !ok {
		return pdfCIDFontMetrics{}, false
	}
	metrics := pdfCIDFontMetrics{
		Encoding: encoding,
		Widths:   widths,
	}
	if defaultWidth, ok := metadataInt(meta["cid_default_width"]); ok {
		metrics.DefaultWidth = &defaultWidth
	}
	if len(metrics.Widths) == 0 && metrics.DefaultWidth == nil {
		return pdfCIDFontMetrics{}, false
	}
	return metrics, true
}

func (m pdfCIDFontMetrics) widthUnits(cids []int) (int, bool, bool) {
	total := 0
	defaultWidthUsed := false
	for _, cid := range cids {
		width, ok := m.Widths[cid]
		if !ok {
			if m.DefaultWidth == nil {
				return 0, false, false
			}
			width = *m.DefaultWidth
			defaultWidthUsed = true
		}
		total += width
	}
	return total, defaultWidthUsed, true
}

func (m pdfCIDFontMetrics) widthSource(defaultWidthUsed bool) string {
	if !defaultWidthUsed {
		return "/DescendantFonts/W"
	}
	if len(m.Widths) == 0 {
		return "/DescendantFonts/DW"
	}
	return "/DescendantFonts/W+/DW"
}

func textShowCIDFontCodes(encoded string, textEncoding string, cidEncoding string) ([]int, bool) {
	switch textEncoding {
	case "hex-cmap":
		return cidFontCodes(encoded, cidEncoding)
	case "tj-array-cmap":
		return cidTJArrayFontCodes([]byte(encoded), cidEncoding)
	default:
		return nil, false
	}
}

func cidFontCodes(encoded string, encoding string) ([]int, bool) {
	if encoding != "Identity-H" && encoding != "Identity-V" {
		return nil, false
	}
	bytes, ok := decodeHexBytes([]byte(encoded))
	if !ok || len(bytes) == 0 || len(bytes)%2 != 0 {
		return nil, false
	}
	out := make([]int, 0, len(bytes)/2)
	for i := 0; i < len(bytes); i += 2 {
		out = append(out, int(bytes[i])<<8|int(bytes[i+1]))
	}
	return out, true
}

func cidTJArrayFontCodes(input []byte, encoding string) ([]int, bool) {
	bytes, ok := simpleTJArrayFontCodes(input)
	if !ok {
		return nil, false
	}
	return identityCIDCodes(bytes, encoding)
}

func identityCIDCodes(bytes []byte, encoding string) ([]int, bool) {
	if encoding != "Identity-H" && encoding != "Identity-V" {
		return nil, false
	}
	if len(bytes) == 0 || len(bytes)%2 != 0 {
		return nil, false
	}
	out := make([]int, 0, len(bytes)/2)
	for i := 0; i < len(bytes); i += 2 {
		out = append(out, int(bytes[i])<<8|int(bytes[i+1]))
	}
	return out, true
}

func copyIntMap(in map[int]int) map[int]int {
	if in == nil {
		return nil
	}
	out := make(map[int]int, len(in))
	for key, value := range in {
		out[key] = value
	}
	return out
}

func metadataIntMap(value any) (map[int]int, bool) {
	switch v := value.(type) {
	case map[int]int:
		return copyIntMap(v), true
	case map[string]any:
		out := make(map[int]int, len(v))
		for key, value := range v {
			cid, ok := metadataStringInt(key)
			if !ok {
				return nil, false
			}
			width, ok := metadataInt(value)
			if !ok {
				return nil, false
			}
			out[cid] = width
		}
		return out, true
	}
	return nil, false
}

func metadataStringInt(value string) (int, bool) {
	n := 0
	if value == "" {
		return 0, false
	}
	for _, r := range value {
		if r < '0' || r > '9' {
			return 0, false
		}
		n = n*10 + int(r-'0')
	}
	return n, true
}

func pdfNameValue(value pdfValue) (string, bool) {
	name, ok := value.(pdfName)
	return string(name), ok
}

func pdfIntValue(value pdfValue) (int, bool) {
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

func pdfTextStringValue(value pdfValue) (string, bool) {
	switch v := value.(type) {
	case pdfLiteralString:
		return decodeLiteralString(string(v)), true
	case pdfHexString:
		bytes, ok := decodeHexBytes([]byte(v))
		if !ok {
			return "", false
		}
		return string(bytes), true
	case pdfName:
		return string(v), true
	default:
		return "", false
	}
}

func (c pdfCMapContext) cidMetricsForStream(sourceStart int) map[string]pdfCIDFontMetrics {
	if c.streamCIDFontMetrics == nil {
		return nil
	}
	return c.streamCIDFontMetrics[sourceStart]
}
