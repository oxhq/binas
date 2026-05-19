package pdf

import "strconv"

type pdfTextStateSnapshot struct {
	InTextObject bool
	FontName     string
	FontSize     float64
	Leading      float64
	X            float64
	Y            float64
}

type pdfTextStateTracker struct {
	state pdfTextStateSnapshot
}

func newPDFTextStateTracker() *pdfTextStateTracker {
	return &pdfTextStateTracker{}
}

func (t *pdfTextStateTracker) Snapshot() pdfTextStateSnapshot {
	return t.state
}

func (t *pdfTextStateTracker) Apply(operator string, operands ...string) bool {
	switch operator {
	case "BT":
		t.reset()
		t.state.InTextObject = true
		return true
	case "ET":
		t.reset()
		return true
	case "Tf":
		if !t.state.InTextObject || len(operands) < 2 {
			return false
		}
		size, ok := parsePDFTextStateNumber(operands[len(operands)-1])
		if !ok {
			return false
		}
		fontName, ok := parsePDFTextStateFontName(operands[len(operands)-2])
		if !ok {
			return false
		}
		t.state.FontName = fontName
		t.state.FontSize = size
		return true
	case "TL":
		if !t.state.InTextObject || len(operands) < 1 {
			return false
		}
		leading, ok := parsePDFTextStateNumber(operands[len(operands)-1])
		if !ok {
			return false
		}
		t.state.Leading = leading
		return true
	case "Td", "TD":
		if !t.state.InTextObject || len(operands) < 2 {
			return false
		}
		tx, ok := parsePDFTextStateNumber(operands[len(operands)-2])
		if !ok {
			return false
		}
		ty, ok := parsePDFTextStateNumber(operands[len(operands)-1])
		if !ok {
			return false
		}
		if operator == "TD" {
			t.state.Leading = -ty
		}
		t.move(tx, ty)
		return true
	case "T*":
		if !t.state.InTextObject {
			return false
		}
		t.move(0, -t.state.Leading)
		return true
	default:
		return false
	}
}

func (t *pdfTextStateTracker) reset() {
	t.state = pdfTextStateSnapshot{}
}

func (t *pdfTextStateTracker) move(tx, ty float64) {
	t.state.X += tx
	t.state.Y += ty
}

func parsePDFTextStateFontName(token string) (string, bool) {
	if len(token) < 2 || token[0] != '/' {
		return "", false
	}
	return token[1:], true
}

func parsePDFTextStateNumber(token string) (float64, bool) {
	value, err := strconv.ParseFloat(token, 64)
	if err != nil {
		return 0, false
	}
	return value, true
}
