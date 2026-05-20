package pdf

import (
	"fmt"
	"strings"
	"unicode/utf8"
)

type pdfSimpleFontEncoding struct {
	name   string
	decode [256]string
}

func (e *pdfSimpleFontEncoding) DecodeBytes(input []byte) string {
	if e == nil {
		return string(input)
	}
	var out strings.Builder
	for _, b := range input {
		value := e.decode[b]
		if value == "" {
			out.WriteRune(utf8.RuneError)
			continue
		}
		out.WriteString(value)
	}
	return out.String()
}

func (e *pdfSimpleFontEncoding) EncodeBytes(text string) ([]byte, bool) {
	if e == nil {
		return []byte(text), true
	}
	reverse := make(map[rune]byte, 256)
	for i, value := range e.decode {
		r, size := utf8.DecodeRuneInString(value)
		if r == utf8.RuneError || size != len(value) {
			continue
		}
		if _, exists := reverse[r]; !exists {
			reverse[r] = byte(i)
		}
	}
	out := make([]byte, 0, len(text))
	for _, r := range text {
		b, ok := reverse[r]
		if !ok {
			return nil, false
		}
		out = append(out, b)
	}
	return out, true
}

func (e *pdfSimpleFontEncoding) EncodeHex(text string) (string, bool) {
	encoded, ok := e.EncodeBytes(text)
	if !ok {
		return "", false
	}
	const hex = "0123456789ABCDEF"
	var out strings.Builder
	out.Grow(len(encoded) * 2)
	for _, b := range encoded {
		out.WriteByte(hex[b>>4])
		out.WriteByte(hex[b&0x0f])
	}
	return out.String(), true
}

func parseSimpleFontEncoding(value pdfValue) (*pdfSimpleFontEncoding, bool) {
	switch v := value.(type) {
	case pdfName:
		return baseSimpleFontEncoding(string(v))
	case pdfDict:
		encoding, ok := simpleFontEncodingFromDict(v)
		return encoding, ok
	default:
		return nil, false
	}
}

func simpleFontEncodingFromDict(dict pdfDict) (*pdfSimpleFontEncoding, bool) {
	baseName := "StandardEncoding"
	if base, ok := dict["BaseEncoding"].(pdfName); ok {
		baseName = string(base)
	}
	encoding, ok := baseSimpleFontEncoding(baseName)
	if !ok {
		return nil, false
	}
	encoding = cloneSimpleFontEncoding(encoding)
	encoding.name = baseName + "+Differences"
	differences, ok := dict["Differences"].(pdfArray)
	if !ok {
		return encoding, true
	}
	code := -1
	for _, item := range differences {
		switch v := item.(type) {
		case int:
			if v < 0 || v > 255 {
				code = -1
				continue
			}
			code = v
		case pdfName:
			if code < 0 || code > 255 {
				continue
			}
			r, ok := glyphNameToUnicode(string(v))
			if ok {
				encoding.decode[code] = r
			}
			code++
		default:
			code = -1
		}
	}
	return encoding, true
}

func baseSimpleFontEncoding(name string) (*pdfSimpleFontEncoding, bool) {
	switch name {
	case "WinAnsiEncoding":
		return winAnsiSimpleFontEncoding(), true
	case "StandardEncoding", "MacRomanEncoding":
		return conservativeSimpleFontEncoding(name), true
	default:
		return nil, false
	}
}

func cloneSimpleFontEncoding(in *pdfSimpleFontEncoding) *pdfSimpleFontEncoding {
	if in == nil {
		return nil
	}
	out := *in
	return &out
}

func conservativeSimpleFontEncoding(name string) *pdfSimpleFontEncoding {
	encoding := &pdfSimpleFontEncoding{name: name}
	for i := 0x20; i <= 0x7e; i++ {
		encoding.decode[i] = string(rune(i))
	}
	return encoding
}

func winAnsiSimpleFontEncoding() *pdfSimpleFontEncoding {
	encoding := conservativeSimpleFontEncoding("WinAnsiEncoding")
	for code, value := range map[byte]string{
		0x80: "€",
		0x82: "‚",
		0x83: "ƒ",
		0x84: "„",
		0x85: "…",
		0x86: "†",
		0x87: "‡",
		0x88: "ˆ",
		0x89: "‰",
		0x8a: "Š",
		0x8b: "‹",
		0x8c: "Œ",
		0x8e: "Ž",
		0x91: "‘",
		0x92: "’",
		0x93: "“",
		0x94: "”",
		0x95: "•",
		0x96: "–",
		0x97: "—",
		0x98: "˜",
		0x99: "™",
		0x9a: "š",
		0x9b: "›",
		0x9c: "œ",
		0x9e: "ž",
		0x9f: "Ÿ",
	} {
		encoding.decode[code] = value
	}
	for code := 0xa0; code <= 0xff; code++ {
		encoding.decode[code] = string(rune(code))
	}
	return encoding
}

func glyphNameToUnicode(name string) (string, bool) {
	if len(name) == 1 {
		return name, true
	}
	if strings.HasPrefix(name, "uni") && len(name) == 7 {
		var value int
		if _, err := fmt.Sscanf(name, "uni%04X", &value); err == nil {
			return string(rune(value)), true
		}
	}
	glyphs := map[string]string{
		"Aacute":    "Á",
		"aacute":    "á",
		"Adieresis": "Ä",
		"adieresis": "ä",
		"Aring":     "Å",
		"aring":     "å",
		"Ccedilla":  "Ç",
		"ccedilla":  "ç",
		"Eacute":    "É",
		"eacute":    "é",
		"Euro":      "€",
		"Ntilde":    "Ñ",
		"ntilde":    "ñ",
		"Oacute":    "Ó",
		"oacute":    "ó",
		"Odieresis": "Ö",
		"odieresis": "ö",
		"Uacute":    "Ú",
		"uacute":    "ú",
		"Udieresis": "Ü",
		"udieresis": "ü",
		"bullet":    "•",
		"emdash":    "—",
		"endash":    "–",
		"minus":     "-",
		"space":     " ",
	}
	value, ok := glyphs[name]
	return value, ok
}
