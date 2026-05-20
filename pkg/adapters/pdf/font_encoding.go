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
			} else {
				encoding.decode[code] = ""
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
	case "StandardEncoding":
		return standardSimpleFontEncoding(), true
	case "MacRomanEncoding":
		return macRomanSimpleFontEncoding(), true
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
	return simpleFontEncodingFromRunes("WinAnsiEncoding", winAnsiEncodingRunes)
}

func standardSimpleFontEncoding() *pdfSimpleFontEncoding {
	encoding := &pdfSimpleFontEncoding{name: "StandardEncoding"}
	for code, name := range standardEncodingGlyphNames {
		if value, ok := glyphNameToUnicode(name); ok {
			encoding.decode[code] = value
		}
	}
	return encoding
}

func macRomanSimpleFontEncoding() *pdfSimpleFontEncoding {
	return simpleFontEncodingFromRunes("MacRomanEncoding", macRomanEncodingRunes)
}

func simpleFontEncodingFromRunes(name string, runes [256]rune) *pdfSimpleFontEncoding {
	encoding := &pdfSimpleFontEncoding{name: name}
	for code, r := range runes {
		if r != 0 {
			encoding.decode[code] = string(r)
		}
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
	if strings.HasPrefix(name, "u") && len(name) >= 5 && len(name) <= 7 {
		var value int
		if _, err := fmt.Sscanf(name, "u%X", &value); err == nil {
			return string(rune(value)), true
		}
	}
	glyphs := map[string]string{
		"AE":             "\u00c6",
		"Aacute":         "\u00c1",
		"Acircumflex":    "\u00c2",
		"Adieresis":      "\u00c4",
		"Agrave":         "\u00c0",
		"Aring":          "\u00c5",
		"Atilde":         "\u00c3",
		"Ccedilla":       "\u00c7",
		"Eacute":         "\u00c9",
		"Ecircumflex":    "\u00ca",
		"Edieresis":      "\u00cb",
		"Egrave":         "\u00c8",
		"Eth":            "\u00d0",
		"Euro":           "\u20ac",
		"Iacute":         "\u00cd",
		"Icircumflex":    "\u00ce",
		"Idieresis":      "\u00cf",
		"Igrave":         "\u00cc",
		"Lslash":         "\u0141",
		"Ntilde":         "\u00d1",
		"OE":             "\u0152",
		"Oacute":         "\u00d3",
		"Ocircumflex":    "\u00d4",
		"Odieresis":      "\u00d6",
		"Ograve":         "\u00d2",
		"Oslash":         "\u00d8",
		"Otilde":         "\u00d5",
		"Scaron":         "\u0160",
		"Thorn":          "\u00de",
		"Uacute":         "\u00da",
		"Ucircumflex":    "\u00db",
		"Udieresis":      "\u00dc",
		"Ugrave":         "\u00d9",
		"Yacute":         "\u00dd",
		"Ydieresis":      "\u0178",
		"Zcaron":         "\u017d",
		"aacute":         "\u00e1",
		"acircumflex":    "\u00e2",
		"acute":          "\u00b4",
		"adieresis":      "\u00e4",
		"ae":             "\u00e6",
		"agrave":         "\u00e0",
		"ampersand":      "&",
		"aring":          "\u00e5",
		"asciicircum":    "^",
		"asciitilde":     "~",
		"asterisk":       "*",
		"at":             "@",
		"atilde":         "\u00e3",
		"backslash":      "\\",
		"bar":            "|",
		"braceleft":      "{",
		"braceright":     "}",
		"bracketleft":    "[",
		"bracketright":   "]",
		"breve":          "\u02d8",
		"brokenbar":      "\u00a6",
		"bullet":         "\u2022",
		"caron":          "\u02c7",
		"ccedilla":       "\u00e7",
		"cedilla":        "\u00b8",
		"cent":           "\u00a2",
		"circumflex":     "\u02c6",
		"colon":          ":",
		"comma":          ",",
		"copyright":      "\u00a9",
		"currency":       "\u00a4",
		"dagger":         "\u2020",
		"daggerdbl":      "\u2021",
		"degree":         "\u00b0",
		"dieresis":       "\u00a8",
		"divide":         "\u00f7",
		"dollar":         "$",
		"dotaccent":      "\u02d9",
		"dotlessi":       "\u0131",
		"eacute":         "\u00e9",
		"ecircumflex":    "\u00ea",
		"edieresis":      "\u00eb",
		"egrave":         "\u00e8",
		"eight":          "8",
		"ellipsis":       "\u2026",
		"emdash":         "\u2014",
		"endash":         "\u2013",
		"equal":          "=",
		"eth":            "\u00f0",
		"exclam":         "!",
		"exclamdown":     "\u00a1",
		"fi":             "\ufb01",
		"five":           "5",
		"fl":             "\ufb02",
		"florin":         "\u0192",
		"four":           "4",
		"fraction":       "\u2044",
		"germandbls":     "\u00df",
		"grave":          "`",
		"greater":        ">",
		"guillemotleft":  "\u00ab",
		"guillemotright": "\u00bb",
		"guilsinglleft":  "\u2039",
		"guilsinglright": "\u203a",
		"hungarumlaut":   "\u02dd",
		"hyphen":         "-",
		"iacute":         "\u00ed",
		"icircumflex":    "\u00ee",
		"idieresis":      "\u00ef",
		"igrave":         "\u00ec",
		"less":           "<",
		"logicalnot":     "\u00ac",
		"lslash":         "\u0142",
		"macron":         "\u00af",
		"minus":          "-",
		"mu":             "\u00b5",
		"multiply":       "\u00d7",
		"nine":           "9",
		"ntilde":         "\u00f1",
		"numbersign":     "#",
		"oacute":         "\u00f3",
		"ocircumflex":    "\u00f4",
		"odieresis":      "\u00f6",
		"oe":             "\u0153",
		"ogonek":         "\u02db",
		"ograve":         "\u00f2",
		"one":            "1",
		"ordfeminine":    "\u00aa",
		"ordmasculine":   "\u00ba",
		"oslash":         "\u00f8",
		"otilde":         "\u00f5",
		"paragraph":      "\u00b6",
		"parenleft":      "(",
		"parenright":     ")",
		"percent":        "%",
		"period":         ".",
		"periodcentered": "\u00b7",
		"perthousand":    "\u2030",
		"plus":           "+",
		"question":       "?",
		"questiondown":   "\u00bf",
		"quotedbl":       "\"",
		"quotedblbase":   "\u201e",
		"quotedblleft":   "\u201c",
		"quotedblright":  "\u201d",
		"quoteleft":      "\u2018",
		"quoteright":     "\u2019",
		"quotesinglbase": "\u201a",
		"quotesingle":    "'",
		"registered":     "\u00ae",
		"ring":           "\u02da",
		"scaron":         "\u0161",
		"section":        "\u00a7",
		"semicolon":      ";",
		"seven":          "7",
		"six":            "6",
		"slash":          "/",
		"space":          " ",
		"sterling":       "\u00a3",
		"thorn":          "\u00fe",
		"three":          "3",
		"tilde":          "\u02dc",
		"trademark":      "\u2122",
		"two":            "2",
		"uacute":         "\u00fa",
		"ucircumflex":    "\u00fb",
		"udieresis":      "\u00fc",
		"ugrave":         "\u00f9",
		"underscore":     "_",
		"yacute":         "\u00fd",
		"ydieresis":      "\u00ff",
		"yen":            "\u00a5",
		"zcaron":         "\u017e",
		"zero":           "0",
	}
	value, ok := glyphs[name]
	return value, ok
}

var standardEncodingGlyphNames = map[int]string{
	32: "space", 33: "exclam", 34: "quotedbl", 35: "numbersign", 36: "dollar", 37: "percent", 38: "ampersand", 39: "quoteright",
	40: "parenleft", 41: "parenright", 42: "asterisk", 43: "plus", 44: "comma", 45: "hyphen", 46: "period", 47: "slash",
	48: "zero", 49: "one", 50: "two", 51: "three", 52: "four", 53: "five", 54: "six", 55: "seven",
	56: "eight", 57: "nine", 58: "colon", 59: "semicolon", 60: "less", 61: "equal", 62: "greater", 63: "question",
	64: "at", 65: "A", 66: "B", 67: "C", 68: "D", 69: "E", 70: "F", 71: "G",
	72: "H", 73: "I", 74: "J", 75: "K", 76: "L", 77: "M", 78: "N", 79: "O",
	80: "P", 81: "Q", 82: "R", 83: "S", 84: "T", 85: "U", 86: "V", 87: "W",
	88: "X", 89: "Y", 90: "Z", 91: "bracketleft", 92: "backslash", 93: "bracketright", 94: "asciicircum", 95: "underscore",
	96: "quoteleft", 97: "a", 98: "b", 99: "c", 100: "d", 101: "e", 102: "f", 103: "g",
	104: "h", 105: "i", 106: "j", 107: "k", 108: "l", 109: "m", 110: "n", 111: "o",
	112: "p", 113: "q", 114: "r", 115: "s", 116: "t", 117: "u", 118: "v", 119: "w",
	120: "x", 121: "y", 122: "z", 123: "braceleft", 124: "bar", 125: "braceright", 126: "asciitilde",
	161: "exclamdown", 162: "cent", 163: "sterling", 164: "fraction", 165: "yen", 166: "florin", 167: "section", 168: "currency",
	169: "quotesingle", 170: "quotedblleft", 171: "guillemotleft", 172: "guilsinglleft", 173: "guilsinglright", 174: "fi", 175: "fl",
	177: "endash", 178: "dagger", 179: "daggerdbl", 180: "periodcentered", 182: "paragraph", 183: "bullet", 184: "quotesinglbase",
	185: "quotedblbase", 186: "quotedblright", 187: "guillemotright", 188: "ellipsis", 189: "perthousand", 191: "questiondown",
	193: "grave", 194: "acute", 195: "circumflex", 196: "tilde", 197: "macron", 198: "breve", 199: "dotaccent", 200: "dieresis",
	202: "ring", 203: "cedilla", 205: "hungarumlaut", 206: "ogonek", 207: "caron", 208: "emdash",
	225: "AE", 227: "ordfeminine", 232: "Lslash", 233: "Oslash", 234: "OE", 235: "ordmasculine",
	241: "ae", 245: "dotlessi", 248: "lslash", 249: "oslash", 250: "oe", 251: "germandbls",
}

var winAnsiEncodingRunes = [256]rune{
	0x20: ' ', 0x21: '!', 0x22: '"', 0x23: '#', 0x24: '$', 0x25: '%', 0x26: '&', 0x27: '\'',
	0x28: '(', 0x29: ')', 0x2a: '*', 0x2b: '+', 0x2c: ',', 0x2d: '-', 0x2e: '.', 0x2f: '/',
	0x30: '0', 0x31: '1', 0x32: '2', 0x33: '3', 0x34: '4', 0x35: '5', 0x36: '6', 0x37: '7',
	0x38: '8', 0x39: '9', 0x3a: ':', 0x3b: ';', 0x3c: '<', 0x3d: '=', 0x3e: '>', 0x3f: '?',
	0x40: '@', 0x41: 'A', 0x42: 'B', 0x43: 'C', 0x44: 'D', 0x45: 'E', 0x46: 'F', 0x47: 'G',
	0x48: 'H', 0x49: 'I', 0x4a: 'J', 0x4b: 'K', 0x4c: 'L', 0x4d: 'M', 0x4e: 'N', 0x4f: 'O',
	0x50: 'P', 0x51: 'Q', 0x52: 'R', 0x53: 'S', 0x54: 'T', 0x55: 'U', 0x56: 'V', 0x57: 'W',
	0x58: 'X', 0x59: 'Y', 0x5a: 'Z', 0x5b: '[', 0x5c: '\\', 0x5d: ']', 0x5e: '^', 0x5f: '_',
	0x60: '`', 0x61: 'a', 0x62: 'b', 0x63: 'c', 0x64: 'd', 0x65: 'e', 0x66: 'f', 0x67: 'g',
	0x68: 'h', 0x69: 'i', 0x6a: 'j', 0x6b: 'k', 0x6c: 'l', 0x6d: 'm', 0x6e: 'n', 0x6f: 'o',
	0x70: 'p', 0x71: 'q', 0x72: 'r', 0x73: 's', 0x74: 't', 0x75: 'u', 0x76: 'v', 0x77: 'w',
	0x78: 'x', 0x79: 'y', 0x7a: 'z', 0x7b: '{', 0x7c: '|', 0x7d: '}', 0x7e: '~',
	0x80: '\u20ac', 0x82: '\u201a', 0x83: '\u0192', 0x84: '\u201e', 0x85: '\u2026', 0x86: '\u2020', 0x87: '\u2021', 0x88: '\u02c6',
	0x89: '\u2030', 0x8a: '\u0160', 0x8b: '\u2039', 0x8c: '\u0152', 0x8e: '\u017d',
	0x91: '\u2018', 0x92: '\u2019', 0x93: '\u201c', 0x94: '\u201d', 0x95: '\u2022', 0x96: '\u2013', 0x97: '\u2014', 0x98: '\u02dc',
	0x99: '\u2122', 0x9a: '\u0161', 0x9b: '\u203a', 0x9c: '\u0153', 0x9e: '\u017e', 0x9f: '\u0178',
	0xa0: '\u00a0', 0xa1: '\u00a1', 0xa2: '\u00a2', 0xa3: '\u00a3', 0xa4: '\u00a4', 0xa5: '\u00a5', 0xa6: '\u00a6', 0xa7: '\u00a7',
	0xa8: '\u00a8', 0xa9: '\u00a9', 0xaa: '\u00aa', 0xab: '\u00ab', 0xac: '\u00ac', 0xad: '\u00ad', 0xae: '\u00ae', 0xaf: '\u00af',
	0xb0: '\u00b0', 0xb1: '\u00b1', 0xb2: '\u00b2', 0xb3: '\u00b3', 0xb4: '\u00b4', 0xb5: '\u00b5', 0xb6: '\u00b6', 0xb7: '\u00b7',
	0xb8: '\u00b8', 0xb9: '\u00b9', 0xba: '\u00ba', 0xbb: '\u00bb', 0xbc: '\u00bc', 0xbd: '\u00bd', 0xbe: '\u00be', 0xbf: '\u00bf',
	0xc0: '\u00c0', 0xc1: '\u00c1', 0xc2: '\u00c2', 0xc3: '\u00c3', 0xc4: '\u00c4', 0xc5: '\u00c5', 0xc6: '\u00c6', 0xc7: '\u00c7',
	0xc8: '\u00c8', 0xc9: '\u00c9', 0xca: '\u00ca', 0xcb: '\u00cb', 0xcc: '\u00cc', 0xcd: '\u00cd', 0xce: '\u00ce', 0xcf: '\u00cf',
	0xd0: '\u00d0', 0xd1: '\u00d1', 0xd2: '\u00d2', 0xd3: '\u00d3', 0xd4: '\u00d4', 0xd5: '\u00d5', 0xd6: '\u00d6', 0xd7: '\u00d7',
	0xd8: '\u00d8', 0xd9: '\u00d9', 0xda: '\u00da', 0xdb: '\u00db', 0xdc: '\u00dc', 0xdd: '\u00dd', 0xde: '\u00de', 0xdf: '\u00df',
	0xe0: '\u00e0', 0xe1: '\u00e1', 0xe2: '\u00e2', 0xe3: '\u00e3', 0xe4: '\u00e4', 0xe5: '\u00e5', 0xe6: '\u00e6', 0xe7: '\u00e7',
	0xe8: '\u00e8', 0xe9: '\u00e9', 0xea: '\u00ea', 0xeb: '\u00eb', 0xec: '\u00ec', 0xed: '\u00ed', 0xee: '\u00ee', 0xef: '\u00ef',
	0xf0: '\u00f0', 0xf1: '\u00f1', 0xf2: '\u00f2', 0xf3: '\u00f3', 0xf4: '\u00f4', 0xf5: '\u00f5', 0xf6: '\u00f6', 0xf7: '\u00f7',
	0xf8: '\u00f8', 0xf9: '\u00f9', 0xfa: '\u00fa', 0xfb: '\u00fb', 0xfc: '\u00fc', 0xfd: '\u00fd', 0xfe: '\u00fe', 0xff: '\u00ff',
}

var macRomanEncodingRunes = [256]rune{
	0x20: ' ', 0x21: '!', 0x22: '"', 0x23: '#', 0x24: '$', 0x25: '%', 0x26: '&', 0x27: '\'',
	0x28: '(', 0x29: ')', 0x2a: '*', 0x2b: '+', 0x2c: ',', 0x2d: '-', 0x2e: '.', 0x2f: '/',
	0x30: '0', 0x31: '1', 0x32: '2', 0x33: '3', 0x34: '4', 0x35: '5', 0x36: '6', 0x37: '7',
	0x38: '8', 0x39: '9', 0x3a: ':', 0x3b: ';', 0x3c: '<', 0x3d: '=', 0x3e: '>', 0x3f: '?',
	0x40: '@', 0x41: 'A', 0x42: 'B', 0x43: 'C', 0x44: 'D', 0x45: 'E', 0x46: 'F', 0x47: 'G',
	0x48: 'H', 0x49: 'I', 0x4a: 'J', 0x4b: 'K', 0x4c: 'L', 0x4d: 'M', 0x4e: 'N', 0x4f: 'O',
	0x50: 'P', 0x51: 'Q', 0x52: 'R', 0x53: 'S', 0x54: 'T', 0x55: 'U', 0x56: 'V', 0x57: 'W',
	0x58: 'X', 0x59: 'Y', 0x5a: 'Z', 0x5b: '[', 0x5c: '\\', 0x5d: ']', 0x5e: '^', 0x5f: '_',
	0x60: '`', 0x61: 'a', 0x62: 'b', 0x63: 'c', 0x64: 'd', 0x65: 'e', 0x66: 'f', 0x67: 'g',
	0x68: 'h', 0x69: 'i', 0x6a: 'j', 0x6b: 'k', 0x6c: 'l', 0x6d: 'm', 0x6e: 'n', 0x6f: 'o',
	0x70: 'p', 0x71: 'q', 0x72: 'r', 0x73: 's', 0x74: 't', 0x75: 'u', 0x76: 'v', 0x77: 'w',
	0x78: 'x', 0x79: 'y', 0x7a: 'z', 0x7b: '{', 0x7c: '|', 0x7d: '}', 0x7e: '~',
	0x80: '\u00c4', 0x81: '\u00c5', 0x82: '\u00c7', 0x83: '\u00c9', 0x84: '\u00d1', 0x85: '\u00d6', 0x86: '\u00dc', 0x87: '\u00e1',
	0x88: '\u00e0', 0x89: '\u00e2', 0x8a: '\u00e4', 0x8b: '\u00e3', 0x8c: '\u00e5', 0x8d: '\u00e7', 0x8e: '\u00e9', 0x8f: '\u00e8',
	0x90: '\u00ea', 0x91: '\u00eb', 0x92: '\u00ed', 0x93: '\u00ec', 0x94: '\u00ee', 0x95: '\u00ef', 0x96: '\u00f1', 0x97: '\u00f3',
	0x98: '\u00f2', 0x99: '\u00f4', 0x9a: '\u00f6', 0x9b: '\u00f5', 0x9c: '\u00fa', 0x9d: '\u00f9', 0x9e: '\u00fb', 0x9f: '\u00fc',
	0xa0: '\u2020', 0xa1: '\u00b0', 0xa2: '\u00a2', 0xa3: '\u00a3', 0xa4: '\u00a7', 0xa5: '\u2022', 0xa6: '\u00b6', 0xa7: '\u00df',
	0xa8: '\u00ae', 0xa9: '\u00a9', 0xaa: '\u2122', 0xab: '\u00b4', 0xac: '\u00a8', 0xad: '\u2260', 0xae: '\u00c6', 0xaf: '\u00d8',
	0xb0: '\u221e', 0xb1: '\u00b1', 0xb2: '\u2264', 0xb3: '\u2265', 0xb4: '\u00a5', 0xb5: '\u00b5', 0xb6: '\u2202', 0xb7: '\u2211',
	0xb8: '\u220f', 0xb9: '\u03c0', 0xba: '\u222b', 0xbb: '\u00aa', 0xbc: '\u00ba', 0xbd: '\u03a9', 0xbe: '\u00e6', 0xbf: '\u00f8',
	0xc0: '\u00bf', 0xc1: '\u00a1', 0xc2: '\u00ac', 0xc3: '\u221a', 0xc4: '\u0192', 0xc5: '\u2248', 0xc6: '\u2206', 0xc7: '\u00ab',
	0xc8: '\u00bb', 0xc9: '\u2026', 0xca: '\u00a0', 0xcb: '\u00c0', 0xcc: '\u00c3', 0xcd: '\u00d5', 0xce: '\u0152', 0xcf: '\u0153',
	0xd0: '\u2013', 0xd1: '\u2014', 0xd2: '\u201c', 0xd3: '\u201d', 0xd4: '\u2018', 0xd5: '\u2019', 0xd6: '\u00f7', 0xd7: '\u25ca',
	0xd8: '\u00ff', 0xd9: '\u0178', 0xda: '\u2044', 0xdb: '\u20ac', 0xdc: '\u2039', 0xdd: '\u203a', 0xde: '\ufb01', 0xdf: '\ufb02',
	0xe0: '\u2021', 0xe1: '\u00b7', 0xe2: '\u201a', 0xe3: '\u201e', 0xe4: '\u2030', 0xe5: '\u00c2', 0xe6: '\u00ca', 0xe7: '\u00c1',
	0xe8: '\u00cb', 0xe9: '\u00c8', 0xea: '\u00cd', 0xeb: '\u00ce', 0xec: '\u00cf', 0xed: '\u00cc', 0xee: '\u00d3', 0xef: '\u00d4',
	0xf0: '\uf8ff', 0xf1: '\u00d2', 0xf2: '\u00da', 0xf3: '\u00db', 0xf4: '\u00d9', 0xf5: '\u0131', 0xf6: '\u02c6', 0xf7: '\u02dc',
	0xf8: '\u00af', 0xf9: '\u02d8', 0xfa: '\u02d9', 0xfb: '\u02da', 0xfc: '\u00b8', 0xfd: '\u02dd', 0xfe: '\u02db', 0xff: '\u02c7',
}
