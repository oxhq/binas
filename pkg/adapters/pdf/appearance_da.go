package pdf

import "strconv"

type defaultAppearance struct {
	FontResourceName string
	FontSize         float64
	FillGray         *float64
	FillRGB          *[3]float64
	TextMatrix       *[6]float64
}

type defaultAppearanceOperand struct {
	kind   defaultAppearanceOperandKind
	name   string
	number float64
}

type defaultAppearanceOperandKind int

const (
	defaultAppearanceOperandName defaultAppearanceOperandKind = iota + 1
	defaultAppearanceOperandNumber
)

func parseDefaultAppearance(input string) (defaultAppearance, bool) {
	data := []byte(input)
	operands := make([]defaultAppearanceOperand, 0, 4)
	var appearance defaultAppearance
	hasFont := false

	for i := 0; i < len(data); {
		i = skipDefaultAppearanceSpaceAndComments(data, i)
		if i >= len(data) {
			break
		}

		if data[i] == '/' {
			nameStart := i + 1
			nameEnd := nameStart
			for nameEnd < len(data) && !isPDFSpace(data[nameEnd]) && !isPDFDelimiter(data[nameEnd]) {
				nameEnd++
			}
			if nameEnd == nameStart {
				return defaultAppearance{}, false
			}
			operands = append(operands, defaultAppearanceOperand{
				kind: defaultAppearanceOperandName,
				name: string(data[nameStart:nameEnd]),
			})
			i = nameEnd
			continue
		}

		if isDefaultAppearanceNumberStart(data[i]) {
			numberEnd, ok := scanPDFNumber(data, i, len(data))
			if ok {
				number, err := strconv.ParseFloat(string(data[i:numberEnd]), 64)
				if err != nil {
					return defaultAppearance{}, false
				}
				operands = append(operands, defaultAppearanceOperand{
					kind:   defaultAppearanceOperandNumber,
					number: number,
				})
				i = numberEnd
				continue
			}
		}

		tokenEnd := i
		for tokenEnd < len(data) && !isPDFSpace(data[tokenEnd]) && !isPDFDelimiter(data[tokenEnd]) {
			tokenEnd++
		}
		if tokenEnd == i {
			operands = operands[:0]
			i++
			continue
		}

		switch string(data[i:tokenEnd]) {
		case "Tf":
			if len(operands) < 2 {
				return defaultAppearance{}, false
			}
			font := operands[len(operands)-2]
			size := operands[len(operands)-1]
			if font.kind != defaultAppearanceOperandName || size.kind != defaultAppearanceOperandNumber {
				return defaultAppearance{}, false
			}
			appearance.FontResourceName = font.name
			appearance.FontSize = size.number
			hasFont = true
		case "g":
			if len(operands) >= 1 {
				gray := operands[len(operands)-1]
				if gray.kind == defaultAppearanceOperandNumber {
					value := gray.number
					appearance.FillGray = &value
					appearance.FillRGB = nil
				}
			}
		case "rg":
			if len(operands) >= 3 {
				red := operands[len(operands)-3]
				green := operands[len(operands)-2]
				blue := operands[len(operands)-1]
				if red.kind == defaultAppearanceOperandNumber && green.kind == defaultAppearanceOperandNumber && blue.kind == defaultAppearanceOperandNumber {
					value := [3]float64{red.number, green.number, blue.number}
					appearance.FillRGB = &value
					appearance.FillGray = nil
				}
			}
		case "Tm":
			if len(operands) < 6 {
				return defaultAppearance{}, false
			}
			var matrix [6]float64
			for j := 0; j < 6; j++ {
				operand := operands[len(operands)-6+j]
				if operand.kind != defaultAppearanceOperandNumber {
					return defaultAppearance{}, false
				}
				matrix[j] = operand.number
			}
			appearance.TextMatrix = &matrix
		}
		operands = operands[:0]
		i = tokenEnd
	}

	if !hasFont {
		return defaultAppearance{}, false
	}
	return appearance, true
}

func skipDefaultAppearanceSpaceAndComments(input []byte, start int) int {
	i := start
	for i < len(input) {
		if isPDFSpace(input[i]) {
			i++
			continue
		}
		if input[i] == '%' {
			for i < len(input) && input[i] != '\n' && input[i] != '\r' {
				i++
			}
			continue
		}
		return i
	}
	return i
}

func isDefaultAppearanceNumberStart(b byte) bool {
	return isPDFDigit(b) || b == '-' || b == '+' || b == '.'
}
