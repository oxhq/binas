package pdf

func applyPDFTextStateOperators(input []byte, start, end int, tracker *pdfTextStateTracker) []string {
	if tracker == nil {
		return nil
	}
	operands := make([]string, 0, 6)
	for i := start; i < end; {
		i = skipPDFSpaceAndComments(input, i)
		if i >= end {
			break
		}
		switch input[i] {
		case '(':
			closeAt, ok := findLiteralEnd(input, i+1, end)
			if !ok {
				return operands
			}
			i = closeAt + 1
			continue
		case '<':
			if i+1 < end && input[i+1] == '<' {
				dictEnd, ok := findDictionaryEnd(input, i)
				if !ok {
					return operands
				}
				i = dictEnd
				continue
			}
			closeAt, ok := findHexStringEnd(input, i+1, end)
			if !ok {
				return operands
			}
			i = closeAt + 1
			continue
		case '[':
			arrayEnd, ok := findArrayEnd(input, i)
			if !ok || arrayEnd > end {
				return operands
			}
			i = arrayEnd
			continue
		case '/':
			tokenEnd := i + 1
			for tokenEnd < end && !isPDFSpace(input[tokenEnd]) && !isPDFDelimiter(input[tokenEnd]) {
				tokenEnd++
			}
			operands = append(operands, string(input[i:tokenEnd]))
			i = tokenEnd
			continue
		}
		if numberEnd, ok := scanPDFNumber(input, i, end); ok {
			operands = append(operands, string(input[i:numberEnd]))
			i = numberEnd
			continue
		}
		tokenEnd := i
		for tokenEnd < end && !isPDFSpace(input[tokenEnd]) && !isPDFDelimiter(input[tokenEnd]) {
			tokenEnd++
		}
		if tokenEnd == i {
			i++
			continue
		}
		operator := string(input[i:tokenEnd])
		if isPDFTextStateOperator(operator) {
			tracker.Apply(operator, operands...)
		}
		operands = operands[:0]
		i = tokenEnd
	}
	return operands
}

func isPDFTextStateOperator(operator string) bool {
	switch operator {
	case "BT", "ET", "Tf", "TL", "Tc", "Tw", "Tz", "Ts", "Tr", "Td", "TD", "Tm", "T*":
		return true
	default:
		return false
	}
}

func applyPDFTextShowOperatorState(operator string, trailingOperands []string, tracker *pdfTextStateTracker) bool {
	switch operator {
	case "Tj", "TJ":
		return len(trailingOperands) == 0
	case "'":
		if len(trailingOperands) != 0 {
			return false
		}
		if tracker != nil {
			return tracker.Apply("T*")
		}
		return true
	case "\"":
		if len(trailingOperands) != 2 {
			return false
		}
		if _, ok := parsePDFTextStateNumber(trailingOperands[0]); !ok {
			return false
		}
		if _, ok := parsePDFTextStateNumber(trailingOperands[1]); !ok {
			return false
		}
		if tracker != nil {
			if !tracker.Apply("Tw", trailingOperands[0]) {
				return false
			}
			if !tracker.Apply("Tc", trailingOperands[1]) {
				return false
			}
			return tracker.Apply("T*")
		}
		return true
	default:
		return false
	}
}

func enrichTextShowTextStateMetadata(meta map[string]any, state pdfTextStateSnapshot) {
	if meta == nil || !state.InTextObject {
		return
	}
	if state.FontName != "" {
		meta["font"] = state.FontName
		meta["font_size"] = state.FontSize
	}
	meta["text_x"] = state.X
	meta["text_y"] = state.Y
	meta["text_leading"] = state.Leading
	meta["char_spacing"] = state.CharSpacing
	meta["word_spacing"] = state.WordSpacing
	meta["horizontal_scaling"] = state.HorizontalScaling
	meta["text_rise"] = state.TextRise
	meta["rendering_mode"] = state.RenderingMode
	if state.TextMatrix != ([6]float64{}) {
		meta["text_matrix"] = state.TextMatrix
	}
}
