package pdf

import (
	"errors"
	"fmt"
	"sort"
	"strings"

	"github.com/oxhq/binas/pkg/core"
)

type FormFieldMetadata struct {
	Index                       int      `json:"index"`
	Name                        string   `json:"name"`
	ObjectNumber                *int     `json:"object_number,omitempty"`
	ObjectGeneration            *int     `json:"object_generation,omitempty"`
	FieldType                   string   `json:"field_type,omitempty"`
	AlternateName               *string  `json:"alternate_name,omitempty"`
	MappingName                 *string  `json:"mapping_name,omitempty"`
	Value                       *string  `json:"value,omitempty"`
	DefaultValue                *string  `json:"default_value,omitempty"`
	Flags                       *int     `json:"flags,omitempty"`
	FlagNames                   []string `json:"flag_names,omitempty"`
	TypeFlagNames               []string `json:"type_flag_names,omitempty"`
	ReadOnly                    bool     `json:"read_only,omitempty"`
	Required                    bool     `json:"required,omitempty"`
	NoExport                    bool     `json:"no_export,omitempty"`
	KidCount                    int      `json:"kid_count"`
	ButtonWidgetAppearanceProof bool     `json:"button_widget_appearance_proof"`
	ButtonStates                []string `json:"button_states,omitempty"`
	Options                     []string `json:"options,omitempty"`
}

type FormFieldEditOptions struct {
	MatchIndex           *int
	RegenerateAppearance bool
}

type FormFieldEditReport struct {
	core.Report
	AppearanceRegenerated bool `json:"appearance_regenerated"`
}

type FormFieldEditVerification struct {
	ReparseOK             bool `json:"reparse_ok"`
	FieldValueSet         bool `json:"field_value_set"`
	NeedAppearancesSet    bool `json:"need_appearances_set"`
	AppearanceRegenerated bool `json:"appearance_regenerated"`
}

func ListFormFields(input []byte) ([]FormFieldMetadata, error) {
	graph, err := parsePDFGraph(input)
	if err != nil {
		return nil, err
	}
	acroForm, ok := catalogAcroForm(graph)
	if !ok {
		return []FormFieldMetadata{}, nil
	}
	fields := collectFormFieldMetadata(graph, acroForm, formFieldContext{}, make(map[pdfObjectID]bool))
	for i := range fields {
		fields[i].Index = i
	}
	return fields, nil
}

func ApplyFormFieldEdit(input []byte, fieldName, value string, matchIndexArg ...*int) ([]byte, FormFieldEditReport, FormFieldEditVerification, error) {
	if len(matchIndexArg) > 1 {
		return nil, FormFieldEditReport{}, FormFieldEditVerification{}, errors.New("form field edit accepts at most one match index")
	}
	options := FormFieldEditOptions{}
	if len(matchIndexArg) == 1 {
		options.MatchIndex = matchIndexArg[0]
	}
	return ApplyFormFieldEditWithOptions(input, fieldName, value, options)
}

func ApplyFormFieldEditWithOptions(input []byte, fieldName, value string, options FormFieldEditOptions) ([]byte, FormFieldEditReport, FormFieldEditVerification, error) {
	if fieldName == "" {
		return nil, FormFieldEditReport{}, FormFieldEditVerification{}, errors.New("form field edit requires a field name")
	}
	graph, err := parsePDFGraph(input)
	if err != nil {
		return nil, FormFieldEditReport{}, FormFieldEditVerification{}, err
	}
	matches := formFieldMatches(graph, fieldName)
	if len(matches) == 0 {
		return nil, FormFieldEditReport{}, FormFieldEditVerification{}, fmt.Errorf("no AcroForm field matches %q", fieldName)
	}
	index := 0
	var selected *int
	if options.MatchIndex != nil {
		if *options.MatchIndex < 0 || *options.MatchIndex >= len(matches) {
			return nil, FormFieldEditReport{}, FormFieldEditVerification{}, fmt.Errorf("match index %d out of range for %d fields (zero-based)", *options.MatchIndex, len(matches))
		}
		index = *options.MatchIndex
		selectedValue := index
		selected = &selectedValue
	} else if len(matches) > 1 {
		return nil, FormFieldEditReport{}, FormFieldEditVerification{}, fmt.Errorf("field %q matched %d dictionaries; pass --match-index N (zero-based, 0..%d) to choose one", fieldName, len(matches), len(matches)-1)
	}
	appearanceRegenerated := false
	if isButtonFormField(matches[index]) {
		if options.RegenerateAppearance {
			return nil, FormFieldEditReport{}, FormFieldEditVerification{}, errors.New("unsupported AcroForm appearance regeneration for button fields; button appearances require proven existing /AP states")
		}
		if err := applyButtonFormFieldEdit(graph, matches[index].dict, value); err != nil {
			return nil, FormFieldEditReport{}, FormFieldEditVerification{}, err
		}
	} else if isChoiceFormField(matches[index]) {
		if err := applyChoiceFormFieldEdit(graph, matches[index], value); err != nil {
			return nil, FormFieldEditReport{}, FormFieldEditVerification{}, err
		}
		if options.RegenerateAppearance {
			generated, err := regenerateTextChoiceFieldAppearances(graph, matches[index], value)
			if err != nil {
				return nil, FormFieldEditReport{}, FormFieldEditVerification{}, err
			}
			appearanceRegenerated = generated > 0
		}
	} else {
		matches[index].dict["V"] = pdfLiteralString(encodeLiteralString(value))
		setNeedAppearances(graph)
		if options.RegenerateAppearance {
			generated, err := regenerateTextChoiceFieldAppearances(graph, matches[index], value)
			if err != nil {
				return nil, FormFieldEditReport{}, FormFieldEditVerification{}, err
			}
			appearanceRegenerated = generated > 0
		}
	}
	output, err := writeCanonicalPDF(graph)
	if err != nil {
		return nil, FormFieldEditReport{}, FormFieldEditVerification{}, err
	}
	verification, err := verifyFormFieldEdit(output, fieldName, value, selected, appearanceRegenerated)
	if err != nil {
		return nil, FormFieldEditReport{}, FormFieldEditVerification{}, err
	}
	return output, FormFieldEditReport{
		Report: core.Report{
			Format:        "pdf",
			Edit:          "pdf.acroform_field_value_update",
			FallbackUsed:  false,
			NodesModified: 1,
			MatchIndex:    selected,
			Invariants:    []core.Invariant{core.InvariantReparse, core.InvariantNoFallbackUsed},
		},
		AppearanceRegenerated: appearanceRegenerated,
	}, verification, nil
}

func isButtonFormField(match formFieldMatch) bool {
	ft, ok := match.fieldType()
	return ok && string(ft) == "Btn"
}

func isChoiceFormField(match formFieldMatch) bool {
	ft, ok := match.fieldType()
	return ok && string(ft) == "Ch"
}

func applyChoiceFormFieldEdit(graph *pdfGraph, match formFieldMatch, value string) error {
	options := formFieldChoiceOptions(match.dict, "Ch")
	if len(options) > 0 && !match.editableChoiceField() && !stringInSlice(options, value) {
		return fmt.Errorf("unsupported AcroForm choice field value %q: not present in direct /Opt options", value)
	}
	match.dict["V"] = pdfLiteralString(encodeLiteralString(value))
	setNeedAppearances(graph)
	return nil
}

func regenerateTextChoiceFieldAppearances(graph *pdfGraph, match formFieldMatch, value string) (int, error) {
	widgets, err := textChoiceAppearanceWidgets(graph, match)
	if err != nil {
		return 0, err
	}
	if len(widgets) == 0 {
		return 0, errors.New("unsupported AcroForm appearance regeneration: no widget dictionary with direct /Rect proof")
	}
	nextObjectNumber := nextPDFObjectNumber(graph)
	for _, widget := range widgets {
		rect, ok := formWidgetRect(widget)
		if !ok {
			return 0, errors.New("unsupported AcroForm appearance regeneration: widget /Rect is missing or not a direct numeric rectangle")
		}
		stream, err := textChoiceAppearanceStream(rect, value)
		if err != nil {
			return 0, err
		}
		id := pdfObjectID{Number: nextObjectNumber, Generation: 0}
		nextObjectNumber++
		graph.Objects[id] = &pdfIndirectObject{ID: id, Value: stream}
		widget["AP"] = pdfDict{"N": pdfRef{ID: id}}
	}
	return len(widgets), nil
}

func textChoiceAppearanceWidgets(graph *pdfGraph, match formFieldMatch) ([]pdfDict, error) {
	if kids, ok := match.dict["Kids"].(pdfArray); ok {
		widgets := make([]pdfDict, 0, len(kids))
		for _, kid := range kids {
			widget, ok := resolvePDFDict(graph, kid)
			if !ok {
				return nil, errors.New("unsupported AcroForm appearance regeneration: widget kid is not a dictionary")
			}
			widgets = append(widgets, widget)
		}
		return widgets, nil
	}
	if _, ok := match.dict["Rect"]; ok {
		return []pdfDict{match.dict}, nil
	}
	return nil, errors.New("unsupported AcroForm appearance regeneration: widget /Rect is missing or not a direct numeric rectangle")
}

func formWidgetRect(widget pdfDict) ([]float64, bool) {
	value, ok := widget["Rect"].(pdfArray)
	if !ok || len(value) != 4 {
		return nil, false
	}
	rect := make([]float64, 0, 4)
	for _, item := range value {
		number, ok := pdfNumericValue(item)
		if !ok {
			return nil, false
		}
		rect = append(rect, number)
	}
	if rect[2] <= rect[0] || rect[3] <= rect[1] {
		return nil, false
	}
	return rect, true
}

func textChoiceAppearanceStream(rect []float64, value string) (pdfStreamObject, error) {
	width := rect[2] - rect[0]
	height := rect[3] - rect[1]
	if width <= 0 || height <= 0 {
		return pdfStreamObject{}, errors.New("unsupported AcroForm appearance regeneration: widget /Rect has non-positive dimensions")
	}
	lines, err := simpleAppearanceTextLines(width, height, value)
	if err != nil {
		return pdfStreamObject{}, fmt.Errorf("unsupported AcroForm appearance regeneration: %w", err)
	}
	var data strings.Builder
	writeSimpleAppearanceText(&data, width, height, lines, false)
	return pdfStreamObject{
		Dict: pdfDict{
			"Type":     pdfName("XObject"),
			"Subtype":  pdfName("Form"),
			"FormType": 1,
			"BBox":     pdfArray{0, 0, width, height},
			"Resources": pdfDict{
				"Font": pdfDict{
					"Helv": pdfDict{
						"Type":     pdfName("Font"),
						"Subtype":  pdfName("Type1"),
						"BaseFont": pdfName("Helvetica"),
					},
				},
			},
		},
		Data: []byte(data.String()),
	}, nil
}

const (
	simpleAppearanceFontSize = 10.0
	simpleAppearanceLeading  = 12.0
	simpleAppearanceInset    = 2.0
)

func simpleAppearanceTextLines(width, height float64, text string) ([]string, error) {
	usableWidth := width - simpleAppearanceInset*2
	usableHeight := height - simpleAppearanceInset*2
	if usableWidth <= 0 || usableHeight < simpleAppearanceFontSize {
		return nil, errors.New("appearance rectangle is too small for safe text layout")
	}
	maxChars := int(usableWidth / (simpleAppearanceFontSize * 0.5))
	if maxChars < 1 {
		return nil, errors.New("appearance rectangle is too narrow for safe text layout")
	}
	maxLines := int(usableHeight / simpleAppearanceLeading)
	if maxLines < 1 {
		maxLines = 1
	}
	lines := wrapAppearanceText(text, maxChars)
	if len(lines) > maxLines {
		lines = lines[:maxLines]
	}
	if len(lines) == 0 {
		lines = []string{""}
	}
	return lines, nil
}

func writeSimpleAppearanceText(data *strings.Builder, width, height float64, lines []string, includeFillColor bool) {
	data.WriteString("q\n")
	fmt.Fprintf(data, "0 0 %s %s re W n\n", pdfNumberToken(width), pdfNumberToken(height))
	data.WriteString("BT\n")
	fmt.Fprintf(data, "/Helv %s Tf\n", pdfNumberToken(simpleAppearanceFontSize))
	if includeFillColor {
		data.WriteString("0 0 0 rg\n")
	}
	startY := height - simpleAppearanceInset - simpleAppearanceFontSize
	if startY < simpleAppearanceInset {
		startY = simpleAppearanceInset
	}
	fmt.Fprintf(data, "%s %s Td\n", pdfNumberToken(simpleAppearanceInset), pdfNumberToken(startY))
	for i, line := range lines {
		if i > 0 {
			fmt.Fprintf(data, "0 -%s Td\n", pdfNumberToken(simpleAppearanceLeading))
		}
		fmt.Fprintf(data, "(%s) Tj\n", encodeLiteralString(line))
	}
	data.WriteString("ET\n")
	data.WriteString("Q")
}

func wrapAppearanceText(text string, maxChars int) []string {
	normalized := strings.ReplaceAll(strings.ReplaceAll(text, "\r\n", "\n"), "\r", "\n")
	paragraphs := strings.Split(normalized, "\n")
	lines := make([]string, 0, len(paragraphs))
	for _, paragraph := range paragraphs {
		if paragraph == "" {
			lines = append(lines, "")
			continue
		}
		lines = append(lines, wrapAppearanceParagraph(paragraph, maxChars)...)
	}
	return lines
}

func wrapAppearanceParagraph(text string, maxChars int) []string {
	words := strings.Fields(text)
	if len(words) == 0 {
		return []string{""}
	}
	lines := make([]string, 0, len(words))
	current := ""
	for _, word := range words {
		for len([]rune(word)) > maxChars {
			prefix, rest := splitRunes(word, maxChars)
			if current != "" {
				lines = append(lines, current)
				current = ""
			}
			lines = append(lines, prefix)
			word = rest
		}
		if current == "" {
			current = word
			continue
		}
		if len([]rune(current))+1+len([]rune(word)) <= maxChars {
			current += " " + word
			continue
		}
		lines = append(lines, current)
		current = word
	}
	if current != "" {
		lines = append(lines, current)
	}
	return lines
}

func splitRunes(text string, count int) (string, string) {
	runes := []rune(text)
	if count >= len(runes) {
		return text, ""
	}
	return string(runes[:count]), string(runes[count:])
}

func pdfNumberToken(value float64) string {
	return strings.TrimRight(strings.TrimRight(fmt.Sprintf("%.4f", value), "0"), ".")
}

func nextPDFObjectNumber(graph *pdfGraph) int {
	next := 1
	for id := range graph.Objects {
		if id.Number >= next {
			next = id.Number + 1
		}
	}
	return next
}

func stringInSlice(values []string, value string) bool {
	for _, candidate := range values {
		if candidate == value {
			return true
		}
	}
	return false
}

func applyButtonFormFieldEdit(graph *pdfGraph, field pdfDict, value string) error {
	if ap, ok := field["AP"]; ok {
		appearance, err := buttonNormalAppearanceDict(ap)
		if err != nil {
			return err
		}
		state, err := buttonAppearanceState(appearance, value)
		if err != nil {
			return err
		}
		field["V"] = pdfName(state)
		field["AS"] = pdfName(state)
		return nil
	}
	kids, ok := field["Kids"].(pdfArray)
	if !ok {
		return errors.New("unsupported AcroForm button field: missing /AP proof")
	}
	if len(kids) != 1 {
		return applyButtonFormFieldEditToWidgetKids(graph, field, kids, value)
	}
	widget, ok := resolvePDFDict(graph, kids[0])
	if !ok {
		return errors.New("unsupported AcroForm button field: widget kid is not a dictionary")
	}
	ap, ok := widget["AP"]
	if !ok {
		return errors.New("unsupported AcroForm button field: missing widget /AP proof")
	}
	appearance, err := buttonNormalAppearanceDict(ap)
	if err != nil {
		return err
	}
	state, err := buttonAppearanceState(appearance, value)
	if err != nil {
		return err
	}
	field["V"] = pdfName(state)
	widget["AS"] = pdfName(state)
	return nil
}

type buttonWidgetAppearance struct {
	widget pdfDict
	normal pdfDict
	state  string
}

func applyButtonFormFieldEditToWidgetKids(graph *pdfGraph, field pdfDict, kids pdfArray, value string) error {
	widgets := make([]buttonWidgetAppearance, 0, len(kids))
	for _, kid := range kids {
		widget, ok := resolvePDFDict(graph, kid)
		if !ok {
			return errors.New("unsupported AcroForm button field: widget kid is not a dictionary")
		}
		ap, ok := widget["AP"]
		if !ok {
			return errors.New("unsupported AcroForm button field: missing widget /AP proof")
		}
		normal, err := buttonNormalAppearanceDict(ap)
		if err != nil {
			return err
		}
		state, err := singleButtonOnState(normal)
		if err != nil {
			return err
		}
		widgets = append(widgets, buttonWidgetAppearance{widget: widget, normal: normal, state: state})
	}

	switch value {
	case "false", "Off":
		field["V"] = pdfName("Off")
		for _, item := range widgets {
			item.widget["AS"] = pdfName("Off")
		}
		return nil
	case "", "true", "On":
		return fmt.Errorf("ambiguous button appearance states for %q", value)
	}

	selected := -1
	for i, item := range widgets {
		if _, ok := item.normal[value]; ok && item.state == value {
			if selected != -1 {
				return fmt.Errorf("unsupported AcroForm button field: duplicate button appearance state %q", value)
			}
			selected = i
		}
	}
	if selected == -1 {
		return fmt.Errorf("unsupported AcroForm button field value %q", value)
	}

	field["V"] = pdfName(value)
	for i, item := range widgets {
		if i == selected {
			item.widget["AS"] = pdfName(value)
		} else {
			item.widget["AS"] = pdfName("Off")
		}
	}
	return nil
}

func buttonNormalAppearanceDict(value pdfValue) (pdfDict, error) {
	ap, ok := value.(pdfDict)
	if !ok {
		return nil, errors.New("unsupported AcroForm button field: /AP is not a dictionary")
	}
	normal, ok := ap["N"].(pdfDict)
	if !ok {
		return nil, errors.New("unsupported AcroForm button field: /AP /N is not a state dictionary")
	}
	return normal, nil
}

func singleButtonOnState(normal pdfDict) (string, error) {
	if _, ok := normal["Off"]; !ok {
		return "", errors.New("unsupported AcroForm button field: missing /Off appearance state")
	}
	onStates := make([]string, 0)
	for state := range normal {
		if state != "Off" {
			onStates = append(onStates, state)
		}
	}
	if len(onStates) == 0 {
		return "", errors.New("unsupported AcroForm button field: missing checked appearance state")
	}
	if len(onStates) != 1 {
		return "", errors.New("unsupported AcroForm button field: ambiguous widget appearance states")
	}
	return onStates[0], nil
}

func buttonAppearanceState(normal pdfDict, value string) (string, error) {
	if _, ok := normal["Off"]; !ok {
		return "", errors.New("unsupported AcroForm button field: missing /Off appearance state")
	}
	onStates := make([]string, 0)
	for state := range normal {
		if state != "Off" {
			onStates = append(onStates, state)
		}
	}
	if len(onStates) == 0 {
		return "", errors.New("unsupported AcroForm button field: missing checked appearance state")
	}
	switch value {
	case "false", "Off":
		return "Off", nil
	case "true":
		if len(onStates) != 1 {
			return "", fmt.Errorf("ambiguous button appearance states for %q", value)
		}
		return onStates[0], nil
	case "On":
		if _, ok := normal["On"]; ok {
			return "On", nil
		}
		if len(onStates) != 1 {
			return "", fmt.Errorf("ambiguous button appearance states for %q", value)
		}
		return onStates[0], nil
	default:
		if value == "" {
			return "", errors.New("unsupported AcroForm button field: empty button value")
		}
		if _, ok := normal[value]; ok {
			return value, nil
		}
		return "", fmt.Errorf("unsupported AcroForm button field value %q", value)
	}
}

func resolvePDFDict(graph *pdfGraph, value pdfValue) (pdfDict, bool) {
	switch v := value.(type) {
	case pdfDict:
		return v, true
	case pdfRef:
		object, ok := graph.Objects[v.ID]
		if !ok {
			return nil, false
		}
		dict, ok := object.Value.(pdfDict)
		return dict, ok
	default:
		return nil, false
	}
}

type formFieldMatch struct {
	id             pdfObjectID
	dict           pdfDict
	inheritedFT    pdfName
	hasInheritedFT bool
	inheritedFlags int
	hasFlags       bool
}

func (match formFieldMatch) fieldType() (pdfName, bool) {
	if ft, ok := match.dict["FT"].(pdfName); ok {
		return ft, true
	}
	return match.inheritedFT, match.hasInheritedFT
}

func (match formFieldMatch) editableChoiceField() bool {
	flags := 0
	if direct, ok := dictInt(match.dict, "Ff"); ok {
		flags = direct
	} else if match.hasFlags {
		flags = match.inheritedFlags
	}
	return flags&formFieldFlagChEdit != 0
}

type formFieldContext struct {
	nameSegments []string
	fieldType    pdfName
	hasFT        bool
	flags        int
	hasFlags     bool
}

func collectFormFieldMetadata(graph *pdfGraph, value pdfValue, context formFieldContext, visited map[pdfObjectID]bool) []FormFieldMetadata {
	switch v := value.(type) {
	case pdfRef:
		if visited[v.ID] {
			return nil
		}
		visited[v.ID] = true
		object, ok := graph.Objects[v.ID]
		if !ok {
			return nil
		}
		fields := collectFormFieldMetadata(graph, object.Value, context, visited)
		dict, isDict := object.Value.(pdfDict)
		if isDict && len(fields) > 0 {
			if _, isField := dict["T"]; isField && fields[0].ObjectNumber == nil && fields[0].ObjectGeneration == nil {
				number := object.ID.Number
				generation := object.ID.Generation
				fields[0].ObjectNumber = &number
				fields[0].ObjectGeneration = &generation
			}
		}
		return fields
	case pdfArray:
		fields := make([]FormFieldMetadata, 0)
		for _, item := range v {
			fields = append(fields, collectFormFieldMetadata(graph, item, context, visited)...)
		}
		return fields
	case pdfDict:
		fields := make([]FormFieldMetadata, 0)
		childContext := context
		if ft, ok := v["FT"].(pdfName); ok {
			childContext.fieldType = ft
			childContext.hasFT = true
		}
		if flags, ok := dictInt(v, "Ff"); ok {
			childContext.flags = flags
			childContext.hasFlags = true
		}
		if name, ok := pdfTextValue(v["T"]); ok {
			fullName := name
			if len(context.nameSegments) > 0 {
				fullName = strings.Join(append(append([]string{}, context.nameSegments...), name), ".")
			}
			fieldType := ""
			if ft, ok := formFieldType(v, context); ok {
				fieldType = string(ft)
			}
			fields = append(fields, FormFieldMetadata{
				Name:                        fullName,
				FieldType:                   fieldType,
				AlternateName:               formFieldTextMetadata(v, "TU"),
				MappingName:                 formFieldTextMetadata(v, "TM"),
				Value:                       formFieldValue(v),
				DefaultValue:                formFieldDefaultValue(v),
				Flags:                       formFieldFlags(v, context),
				FlagNames:                   formFieldFlagNames(v, context),
				TypeFlagNames:               formFieldTypeFlagNames(v, context, fieldType),
				ReadOnly:                    formFieldFlagSet(v, context, formFieldFlagReadOnly),
				Required:                    formFieldFlagSet(v, context, formFieldFlagRequired),
				NoExport:                    formFieldFlagSet(v, context, formFieldFlagNoExport),
				KidCount:                    formFieldKidCount(v),
				ButtonWidgetAppearanceProof: formFieldButtonAppearanceProof(graph, v, fieldType),
				ButtonStates:                formFieldButtonStates(graph, v, fieldType),
				Options:                     formFieldChoiceOptions(v, fieldType),
			})
			childContext.nameSegments = append(append([]string{}, context.nameSegments...), name)
		}
		if fieldsValue, ok := v["Fields"]; ok {
			fields = append(fields, collectFormFieldMetadata(graph, fieldsValue, childContext, visited)...)
		}
		if kids, ok := v["Kids"]; ok {
			fields = append(fields, collectFormFieldMetadata(graph, kids, childContext, visited)...)
		}
		return fields
	default:
		return nil
	}
}

func formFieldType(dict pdfDict, context formFieldContext) (pdfName, bool) {
	if ft, ok := dict["FT"].(pdfName); ok {
		return ft, true
	}
	return context.fieldType, context.hasFT
}

const (
	formFieldFlagReadOnly = 1 << iota
	formFieldFlagRequired
	formFieldFlagNoExport
)

const (
	formFieldFlagTxMultiline         = 1 << 12
	formFieldFlagTxPassword          = 1 << 13
	formFieldFlagBtnNoToggleToOff    = 1 << 14
	formFieldFlagBtnRadio            = 1 << 15
	formFieldFlagBtnPushbutton       = 1 << 16
	formFieldFlagChCombo             = 1 << 17
	formFieldFlagChEdit              = 1 << 18
	formFieldFlagChSort              = 1 << 19
	formFieldFlagTxFileSelect        = 1 << 20
	formFieldFlagChMultiSelect       = 1 << 21
	formFieldFlagDoNotSpellCheck     = 1 << 22
	formFieldFlagTxDoNotScroll       = 1 << 23
	formFieldFlagTxComb              = 1 << 24
	formFieldFlagTxRichText          = 1 << 25
	formFieldFlagBtnRadiosInUnison   = 1 << 25
	formFieldFlagChCommitOnSelChange = 1 << 26
)

func formFieldFlags(dict pdfDict, context formFieldContext) *int {
	flags, ok := effectiveFormFieldFlags(dict, context)
	if !ok {
		return nil
	}
	return &flags
}

func effectiveFormFieldFlags(dict pdfDict, context formFieldContext) (int, bool) {
	if flags, ok := dictInt(dict, "Ff"); ok {
		return flags, true
	}
	return context.flags, context.hasFlags
}

func formFieldFlagSet(dict pdfDict, context formFieldContext, flag int) bool {
	flags, ok := effectiveFormFieldFlags(dict, context)
	return ok && flags&flag != 0
}

func formFieldFlagNames(dict pdfDict, context formFieldContext) []string {
	flags, ok := effectiveFormFieldFlags(dict, context)
	if !ok {
		return nil
	}
	names := make([]string, 0, 3)
	if flags&formFieldFlagReadOnly != 0 {
		names = append(names, "read_only")
	}
	if flags&formFieldFlagRequired != 0 {
		names = append(names, "required")
	}
	if flags&formFieldFlagNoExport != 0 {
		names = append(names, "no_export")
	}
	if len(names) == 0 {
		return nil
	}
	return names
}

func formFieldTypeFlagNames(dict pdfDict, context formFieldContext, fieldType string) []string {
	flags, ok := effectiveFormFieldFlags(dict, context)
	if !ok {
		return nil
	}
	var specs []formFieldTypeFlagSpec
	switch fieldType {
	case "Tx":
		specs = []formFieldTypeFlagSpec{
			{formFieldFlagTxMultiline, "multiline"},
			{formFieldFlagTxPassword, "password"},
			{formFieldFlagTxFileSelect, "file_select"},
			{formFieldFlagDoNotSpellCheck, "do_not_spell_check"},
			{formFieldFlagTxDoNotScroll, "do_not_scroll"},
			{formFieldFlagTxComb, "comb"},
			{formFieldFlagTxRichText, "rich_text"},
		}
	case "Btn":
		specs = []formFieldTypeFlagSpec{
			{formFieldFlagBtnNoToggleToOff, "no_toggle_to_off"},
			{formFieldFlagBtnRadio, "radio"},
			{formFieldFlagBtnPushbutton, "pushbutton"},
			{formFieldFlagBtnRadiosInUnison, "radios_in_unison"},
		}
	case "Ch":
		specs = []formFieldTypeFlagSpec{
			{formFieldFlagChCombo, "combo"},
			{formFieldFlagChEdit, "edit"},
			{formFieldFlagChSort, "sort"},
			{formFieldFlagChMultiSelect, "multi_select"},
			{formFieldFlagDoNotSpellCheck, "do_not_spell_check"},
			{formFieldFlagChCommitOnSelChange, "commit_on_sel_change"},
		}
	default:
		return nil
	}
	names := make([]string, 0, len(specs))
	for _, spec := range specs {
		if flags&spec.flag != 0 {
			names = append(names, spec.name)
		}
	}
	if len(names) == 0 {
		return nil
	}
	return names
}

type formFieldTypeFlagSpec struct {
	flag int
	name string
}

func formFieldTextMetadata(dict pdfDict, key string) *string {
	value, ok := dict[key]
	if !ok {
		return nil
	}
	text, ok := pdfDirectFormTextString(value)
	if !ok {
		return nil
	}
	return &text
}

func formFieldValue(dict pdfDict) *string {
	value, ok := dict["V"]
	if !ok {
		return nil
	}
	decoded := pdfValueText(value)
	return &decoded
}

func formFieldDefaultValue(dict pdfDict) *string {
	value, ok := dict["DV"]
	if !ok {
		return nil
	}
	decoded, ok := pdfDirectFormValueText(value)
	if !ok {
		return nil
	}
	return &decoded
}

func pdfDirectFormValueText(value pdfValue) (string, bool) {
	if text, ok := pdfDirectFormTextString(value); ok {
		return text, true
	}
	if name, ok := value.(pdfName); ok {
		return string(name), true
	}
	return "", false
}

func pdfDirectFormTextString(value pdfValue) (string, bool) {
	if hex, ok := value.(pdfHexString); ok {
		return decodeFormHexTextString([]byte(hex))
	}
	if text, ok := pdfTextValue(value); ok {
		return text, true
	}
	return "", false
}

func pdfValueText(value pdfValue) string {
	if hex, ok := value.(pdfHexString); ok {
		if text, ok := decodeFormHexTextString([]byte(hex)); ok {
			return text
		}
	}
	if text, ok := pdfTextValue(value); ok {
		return text
	}
	switch v := value.(type) {
	case pdfName:
		return string(v)
	default:
		return fmt.Sprint(v)
	}
}

func decodeFormHexTextString(encoded []byte) (string, bool) {
	decoded, ok := decodeHexBytes(encoded)
	if !ok {
		return "", false
	}
	if len(decoded) >= 2 && decoded[0] == 0xfe && decoded[1] == 0xff {
		compact := make([]byte, 0, len(encoded))
		for _, b := range encoded {
			if !isPDFSpace(b) {
				compact = append(compact, b)
			}
		}
		if len(compact) >= 4 {
			return decodeUTF16BEHex(string(compact[4:])), true
		}
	}
	return string(decoded), true
}

func formFieldKidCount(dict pdfDict) int {
	kids, ok := dict["Kids"].(pdfArray)
	if !ok {
		return 0
	}
	return len(kids)
}

func formFieldChoiceOptions(dict pdfDict, fieldType string) []string {
	if fieldType != "Ch" {
		return nil
	}
	opt, ok := dict["Opt"]
	if !ok {
		return nil
	}
	if text, ok := pdfTextValue(opt); ok {
		return []string{text}
	}
	options, ok := opt.(pdfArray)
	if !ok {
		return nil
	}
	out := make([]string, 0, len(options))
	for _, option := range options {
		if text, ok := pdfTextValue(option); ok {
			out = append(out, text)
			continue
		}
		pair, ok := option.(pdfArray)
		if !ok || len(pair) != 2 {
			continue
		}
		if display, ok := pdfTextValue(pair[1]); ok {
			out = append(out, display)
			continue
		}
		if export, ok := pdfTextValue(pair[0]); ok {
			out = append(out, export)
		}
	}
	if len(out) == 0 {
		return nil
	}
	return out
}

func formFieldButtonAppearanceProof(graph *pdfGraph, dict pdfDict, fieldType string) bool {
	if fieldType != "Btn" {
		return false
	}
	if _, err := buttonNormalAppearanceDict(dict["AP"]); err == nil {
		return true
	}
	kids, ok := dict["Kids"].(pdfArray)
	if !ok || len(kids) == 0 {
		return false
	}
	for _, kid := range kids {
		widget, ok := resolvePDFDict(graph, kid)
		if !ok {
			return false
		}
		if _, err := buttonNormalAppearanceDict(widget["AP"]); err != nil {
			return false
		}
	}
	return true
}

func formFieldButtonStates(graph *pdfGraph, dict pdfDict, fieldType string) []string {
	if fieldType != "Btn" {
		return nil
	}
	states := make(map[string]bool)
	collectButtonAppearanceStates(states, dict["AP"])
	if kids, ok := dict["Kids"].(pdfArray); ok {
		for _, kid := range kids {
			widget, ok := resolvePDFDict(graph, kid)
			if !ok {
				continue
			}
			collectButtonAppearanceStates(states, widget["AP"])
		}
	}
	if len(states) == 0 {
		return nil
	}
	out := make([]string, 0, len(states))
	for state := range states {
		out = append(out, state)
	}
	sort.Strings(out)
	return out
}

func collectButtonAppearanceStates(states map[string]bool, ap pdfValue) {
	normal, err := buttonNormalAppearanceDict(ap)
	if err != nil {
		return
	}
	for state := range normal {
		states[state] = true
	}
}

func formFieldMatches(graph *pdfGraph, fieldName string) []formFieldMatch {
	acroForm, ok := catalogAcroForm(graph)
	if !ok {
		return nil
	}
	return collectFormFieldMatches(graph, acroForm, fieldName, formFieldContext{}, make(map[pdfObjectID]bool))
}

func collectFormFieldMatches(graph *pdfGraph, value pdfValue, fieldName string, context formFieldContext, visited map[pdfObjectID]bool) []formFieldMatch {
	switch v := value.(type) {
	case pdfRef:
		if visited[v.ID] {
			return nil
		}
		visited[v.ID] = true
		object, ok := graph.Objects[v.ID]
		if !ok {
			return nil
		}
		matches := collectFormFieldMatches(graph, object.Value, fieldName, context, visited)
		for i := range matches {
			if matches[i].id == (pdfObjectID{}) {
				matches[i].id = object.ID
			}
		}
		return matches
	case pdfArray:
		matches := make([]formFieldMatch, 0)
		for _, item := range v {
			matches = append(matches, collectFormFieldMatches(graph, item, fieldName, context, visited)...)
		}
		return matches
	case pdfDict:
		matches := make([]formFieldMatch, 0)
		childContext := context
		if ft, ok := v["FT"].(pdfName); ok {
			childContext.fieldType = ft
			childContext.hasFT = true
		}
		if flags, ok := dictInt(v, "Ff"); ok {
			childContext.flags = flags
			childContext.hasFlags = true
		}
		if name, ok := pdfTextValue(v["T"]); ok {
			fullName := name
			if len(context.nameSegments) > 0 {
				fullName = strings.Join(append(append([]string{}, context.nameSegments...), name), ".")
			}
			if fullName == fieldName {
				matches = append(matches, formFieldMatch{
					dict:           v,
					inheritedFT:    context.fieldType,
					hasInheritedFT: context.hasFT,
					inheritedFlags: context.flags,
					hasFlags:       context.hasFlags,
				})
			}
			childContext.nameSegments = append(append([]string{}, context.nameSegments...), name)
		}
		if fields, ok := v["Fields"]; ok {
			matches = append(matches, collectFormFieldMatches(graph, fields, fieldName, childContext, visited)...)
		}
		if kids, ok := v["Kids"]; ok {
			matches = append(matches, collectFormFieldMatches(graph, kids, fieldName, childContext, visited)...)
		}
		return matches
	default:
		return nil
	}
}

func catalogAcroForm(graph *pdfGraph) (pdfValue, bool) {
	if graph.Root == nil {
		return nil, false
	}
	catalogObject, ok := graph.Objects[*graph.Root]
	if !ok {
		return nil, false
	}
	catalog, ok := catalogObject.Value.(pdfDict)
	if !ok {
		return nil, false
	}
	acroForm, ok := catalog["AcroForm"]
	return acroForm, ok
}

func setNeedAppearances(graph *pdfGraph) {
	acroFormValue, ok := catalogAcroForm(graph)
	if !ok {
		return
	}
	switch acroForm := acroFormValue.(type) {
	case pdfDict:
		acroForm["NeedAppearances"] = true
	case pdfRef:
		if object, ok := graph.Objects[acroForm.ID]; ok {
			if dict, ok := object.Value.(pdfDict); ok {
				dict["NeedAppearances"] = true
			}
		}
	}
}

func verifyFormFieldEdit(output []byte, fieldName, value string, matchIndex *int, expectAppearanceRegenerated bool) (FormFieldEditVerification, error) {
	graph, err := parsePDFGraph(output)
	if err != nil {
		return FormFieldEditVerification{}, err
	}
	matches := formFieldMatches(graph, fieldName)
	index := 0
	if matchIndex != nil {
		index = *matchIndex
	}
	fieldValueSet := false
	appearanceRegenerated := false
	if index >= 0 && index < len(matches) {
		if got := formFieldValue(matches[index].dict); got != nil && *got == value {
			fieldValueSet = true
		}
		if expectAppearanceRegenerated {
			appearanceRegenerated = formFieldHasNormalAppearanceStream(graph, matches[index])
		}
	}
	return FormFieldEditVerification{
		ReparseOK:             true,
		FieldValueSet:         fieldValueSet,
		NeedAppearancesSet:    formNeedAppearancesSet(graph),
		AppearanceRegenerated: appearanceRegenerated,
	}, nil
}

func formNeedAppearancesSet(graph *pdfGraph) bool {
	acroFormValue, ok := catalogAcroForm(graph)
	if !ok {
		return false
	}
	switch acroForm := acroFormValue.(type) {
	case pdfDict:
		return acroForm["NeedAppearances"] == true
	case pdfRef:
		object, ok := graph.Objects[acroForm.ID]
		if !ok {
			return false
		}
		dict, ok := object.Value.(pdfDict)
		return ok && dict["NeedAppearances"] == true
	default:
		return false
	}
}

func formFieldHasNormalAppearanceStream(graph *pdfGraph, match formFieldMatch) bool {
	widgets, err := textChoiceAppearanceWidgets(graph, match)
	if err != nil {
		return false
	}
	for _, widget := range widgets {
		ap, ok := widget["AP"].(pdfDict)
		if !ok {
			continue
		}
		ref, ok := ap["N"].(pdfRef)
		if !ok {
			continue
		}
		object, ok := graph.Objects[ref.ID]
		if !ok {
			continue
		}
		stream, ok := object.Value.(pdfStreamObject)
		if !ok {
			continue
		}
		if stream.Dict["Type"] == pdfName("XObject") && stream.Dict["Subtype"] == pdfName("Form") {
			return true
		}
	}
	return false
}
