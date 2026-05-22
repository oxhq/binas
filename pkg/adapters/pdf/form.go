package pdf

import (
	"encoding/json"
	"errors"
	"fmt"
	"sort"
	"strings"

	"github.com/oxhq/binas/pkg/core"
)

type FormFieldMetadata struct {
	Index                        int      `json:"index"`
	Name                         string   `json:"name"`
	ObjectNumber                 *int     `json:"object_number,omitempty"`
	ObjectGeneration             *int     `json:"object_generation,omitempty"`
	FieldType                    string   `json:"field_type,omitempty"`
	AlternateName                *string  `json:"alternate_name,omitempty"`
	MappingName                  *string  `json:"mapping_name,omitempty"`
	Value                        *string  `json:"value,omitempty"`
	DefaultValue                 *string  `json:"default_value,omitempty"`
	Flags                        *int     `json:"flags,omitempty"`
	FlagNames                    []string `json:"flag_names,omitempty"`
	TypeFlagNames                []string `json:"type_flag_names,omitempty"`
	ReadOnly                     bool     `json:"read_only,omitempty"`
	Required                     bool     `json:"required,omitempty"`
	NoExport                     bool     `json:"no_export,omitempty"`
	KidCount                     int      `json:"kid_count"`
	ButtonWidgetAppearanceProof  bool     `json:"button_widget_appearance_proof"`
	ButtonStates                 []string `json:"button_states,omitempty"`
	Options                      []string `json:"options,omitempty"`
	SelectedIndexes              []int    `json:"selected_indexes,omitempty"`
	AppearanceGenerationStatus   string   `json:"appearance_generation_status,omitempty"`
	AppearanceGenerationBlockers []string `json:"appearance_generation_blockers,omitempty"`
	FillStatus                   string   `json:"fill_status,omitempty"`
	FillBlockers                 []string `json:"fill_blockers,omitempty"`
}

type FormFieldEditOptions struct {
	MatchIndex           *int
	RegenerateAppearance bool
}

type FormFieldEditReport struct {
	core.Report
	AppearanceRegenerated       bool   `json:"appearance_regenerated"`
	AppearanceStatus            string `json:"appearance_status"`
	AppearanceGenerationStatus  string `json:"appearance_generation_status"`
	AppearanceGenerationDetails string `json:"appearance_generation_details,omitempty"`
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
	if blockers := formFieldEditBlockers(graph, matches[index].dict, matches[index].context(), options.RegenerateAppearance); len(blockers) > 0 {
		return nil, FormFieldEditReport{}, FormFieldEditVerification{}, fmt.Errorf("unsupported AcroForm field edit: %s", strings.Join(blockers, ", "))
	}
	appearanceRegenerated := false
	appearanceStatus := "not_requested"
	appearanceDetails := "appearance regeneration was not requested"
	if isButtonFormField(matches[index]) {
		generated := 0
		if options.RegenerateAppearance {
			generated, err = regenerateButtonFieldAppearances(graph, matches[index], value)
			if err != nil {
				return nil, FormFieldEditReport{}, FormFieldEditVerification{}, err
			}
		}
		if err := applyButtonFormFieldEdit(graph, matches[index].dict, value); err != nil {
			return nil, FormFieldEditReport{}, FormFieldEditVerification{}, err
		}
		if options.RegenerateAppearance {
			appearanceRegenerated = true
			appearanceStatus = "regenerated"
			if generated > 0 {
				appearanceDetails = fmt.Sprintf("regenerated %d simple checkbox appearance stream(s) and synchronized /V and /AS", generated)
			} else {
				appearanceDetails = "selected proven button /AP state and synchronized /V and /AS"
			}
		}
	} else if isChoiceFormField(matches[index]) {
		appearanceValue, err := applyChoiceFormFieldEdit(graph, matches[index], value)
		if err != nil {
			return nil, FormFieldEditReport{}, FormFieldEditVerification{}, err
		}
		if options.RegenerateAppearance {
			generated, err := regenerateTextChoiceFieldAppearances(graph, matches[index], appearanceValue)
			if err != nil {
				return nil, FormFieldEditReport{}, FormFieldEditVerification{}, err
			}
			appearanceRegenerated = generated > 0
			appearanceStatus, appearanceDetails = formAppearanceGenerationReport(generated)
		}
	} else {
		encodedValue := pdfLiteralString(encodeLiteralString(value))
		matches[index].dict["V"] = encodedValue
		if err := synchronizeTextChoiceFormFieldValue(graph, matches[index], encodedValue); err != nil {
			return nil, FormFieldEditReport{}, FormFieldEditVerification{}, err
		}
		setNeedAppearances(graph)
		if options.RegenerateAppearance {
			generated, err := regenerateTextChoiceFieldAppearances(graph, matches[index], value)
			if err != nil {
				return nil, FormFieldEditReport{}, FormFieldEditVerification{}, err
			}
			appearanceRegenerated = generated > 0
			appearanceStatus, appearanceDetails = formAppearanceGenerationReport(generated)
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
			Meta: map[string]any{
				"appearance_generation_status":  appearanceStatus,
				"appearance_generation_details": appearanceDetails,
			},
		},
		AppearanceRegenerated:       appearanceRegenerated,
		AppearanceStatus:            formAppearanceEditStatus(appearanceRegenerated),
		AppearanceGenerationStatus:  appearanceStatus,
		AppearanceGenerationDetails: appearanceDetails,
	}, verification, nil
}

func formAppearanceEditStatus(regenerated bool) string {
	if regenerated {
		return "regenerated"
	}
	return "preserved"
}

func formAppearanceGenerationReport(generated int) (string, string) {
	if generated > 0 {
		return "regenerated", fmt.Sprintf("regenerated %d simple text/choice widget appearance stream(s)", generated)
	}
	return "not_generated", "no widget appearance stream was regenerated"
}

func isButtonFormField(match formFieldMatch) bool {
	ft, ok := match.fieldType()
	return ok && string(ft) == "Btn"
}

func isChoiceFormField(match formFieldMatch) bool {
	ft, ok := match.fieldType()
	return ok && string(ft) == "Ch"
}

func applyChoiceFormFieldEdit(graph *pdfGraph, match formFieldMatch, value string) (string, error) {
	if match.multiSelectChoiceField() {
		return applyMultiSelectChoiceFormFieldEdit(graph, match, value)
	}
	options := formFieldChoiceOptionEntries(match.dict, "Ch")
	selected, hasSelectedIndex := uniqueChoiceOptionSelection(options, value)
	if len(options) > 0 && !match.editableChoiceField() && !hasSelectedIndex {
		return "", fmt.Errorf("unsupported AcroForm choice field value %q: not present in direct /Opt options", value)
	}
	if hasSelectedIndex {
		match.dict["V"] = pdfLiteralString(encodeLiteralString(selected.export))
		match.dict["I"] = pdfArray{selected.index}
	} else {
		match.dict["V"] = pdfLiteralString(encodeLiteralString(value))
		delete(match.dict, "I")
	}
	if err := synchronizeTextChoiceFormFieldValue(graph, match, match.dict["V"]); err != nil {
		return "", err
	}
	setNeedAppearances(graph)
	if hasSelectedIndex {
		return selected.display, nil
	}
	return value, nil
}

func applyMultiSelectChoiceFormFieldEdit(graph *pdfGraph, match formFieldMatch, value string) (string, error) {
	var requested []string
	if err := json.Unmarshal([]byte(value), &requested); err != nil {
		return "", errors.New("unsupported AcroForm choice field edit: multi-select fields require a JSON array of string values")
	}
	if len(requested) == 0 {
		return "", errors.New("unsupported AcroForm choice field edit: multi-select fields require at least one selected value")
	}
	options := formFieldChoiceOptionEntries(match.dict, "Ch")
	if len(options) == 0 {
		return "", errors.New("unsupported AcroForm choice field edit: multi-select fields require direct /Opt options to prove array /V values")
	}
	selectedOptions := make([]formFieldChoiceOption, 0, len(requested))
	seenIndexes := make(map[int]bool, len(requested))
	for _, item := range requested {
		selected, ok := uniqueChoiceOptionSelection(options, item)
		if !ok {
			return "", fmt.Errorf("unsupported AcroForm choice field value %q: ambiguous or not present in direct /Opt options", item)
		}
		if seenIndexes[selected.index] {
			return "", fmt.Errorf("unsupported AcroForm choice field value %q: duplicate multi-select option index %d", item, selected.index)
		}
		seenIndexes[selected.index] = true
		selectedOptions = append(selectedOptions, selected)
	}
	sort.Slice(selectedOptions, func(i, j int) bool {
		return selectedOptions[i].index < selectedOptions[j].index
	})
	valueArray := make(pdfArray, 0, len(selectedOptions))
	indexArray := make(pdfArray, 0, len(selectedOptions))
	displayValues := make([]string, 0, len(selectedOptions))
	for _, selected := range selectedOptions {
		valueArray = append(valueArray, pdfLiteralString(encodeLiteralString(selected.export)))
		indexArray = append(indexArray, selected.index)
		displayValues = append(displayValues, selected.display)
	}
	match.dict["V"] = valueArray
	match.dict["I"] = indexArray
	if err := synchronizeTextChoiceFormFieldValue(graph, match, valueArray); err != nil {
		return "", err
	}
	setNeedAppearances(graph)
	return strings.Join(displayValues, "\n"), nil
}

func synchronizeTextChoiceFormFieldValue(graph *pdfGraph, match formFieldMatch, value pdfValue) error {
	if err := synchronizeTerminalKidValues(graph, match, value); err != nil {
		return err
	}
	synchronizeSingleTerminalParentValue(graph, match, value)
	return nil
}

func synchronizeTerminalKidValues(graph *pdfGraph, match formFieldMatch, value pdfValue) error {
	kids, ok := match.dict["Kids"].(pdfArray)
	if !ok || len(kids) == 0 {
		return nil
	}
	for _, kidValue := range kids {
		kid, ok := resolvePDFDict(graph, kidValue)
		if !ok {
			return errors.New("unsupported AcroForm value synchronization: field kid is not a dictionary")
		}
		if err := safeTerminalValueSyncKid(match, kid); err != nil {
			return err
		}
		kid["V"] = value
	}
	return nil
}

func synchronizeSingleTerminalParentValue(graph *pdfGraph, match formFieldMatch, value pdfValue) {
	parentValue, ok := match.dict["Parent"]
	if !ok {
		return
	}
	parent, ok := resolvePDFDict(graph, parentValue)
	if !ok || !safeSingleTerminalParentValueSync(parent, match) {
		return
	}
	parent["V"] = value
}

func safeTerminalValueSyncKid(match formFieldMatch, kid pdfDict) error {
	if _, hasGrandkids := kid["Kids"]; hasGrandkids {
		return errors.New("unsupported AcroForm value synchronization: complex child field tree has nested /Kids")
	}
	name, hasName := pdfTextValue(kid["T"])
	if !hasName {
		if kid["Subtype"] == pdfName("Widget") {
			return nil
		}
		return errors.New("unsupported AcroForm value synchronization: unnamed field kid is not a widget")
	}
	if name == match.localName || name == match.fullName {
		return nil
	}
	return fmt.Errorf("unsupported AcroForm value synchronization: child field name %q does not match %q", name, match.fullName)
}

func safeSingleTerminalParentValueSync(parent pdfDict, match formFieldMatch) bool {
	kids, ok := parent["Kids"].(pdfArray)
	if !ok || len(kids) != 1 {
		return false
	}
	kidRef, ok := kids[0].(pdfRef)
	if !ok || match.id == (pdfObjectID{}) || kidRef.ID != match.id {
		return false
	}
	if _, hasGrandkids := match.dict["Kids"]; hasGrandkids {
		return false
	}
	_, parentHasFT := parent["FT"].(pdfName)
	return parentHasFT
}

func regenerateTextChoiceFieldAppearances(graph *pdfGraph, match formFieldMatch, value string) (int, error) {
	if formFieldHasRichAppearance(match) {
		return 0, errors.New("unsupported AcroForm appearance regeneration: rich text/default appearance cannot be regenerated safely")
	}
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
		style, err := textChoiceAppearanceStyle(graph, match, widget)
		if err != nil {
			return 0, err
		}
		stream, err := textChoiceAppearanceStream(rect, value, style)
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

func formFieldHasRichAppearance(match formFieldMatch) bool {
	if flags, ok := match.effectiveFlags(); ok && flags&formFieldFlagTxRichText != 0 {
		return true
	}
	_, hasRichValue := match.dict["RV"]
	return hasRichValue
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

func textChoiceAppearanceStream(rect []float64, value string, style simpleAppearanceStyle) (pdfStreamObject, error) {
	width := rect[2] - rect[0]
	height := rect[3] - rect[1]
	if width <= 0 || height <= 0 {
		return pdfStreamObject{}, errors.New("unsupported AcroForm appearance regeneration: widget /Rect has non-positive dimensions")
	}
	lines, err := simpleAppearanceTextLinesWithStyle(width, height, value, style)
	if err != nil {
		return pdfStreamObject{}, fmt.Errorf("unsupported AcroForm appearance regeneration: %w", err)
	}
	var data strings.Builder
	writeSimpleAppearanceTextWithStyle(&data, width, height, lines, style)
	fontResourceName, fontResourceValue := simpleAppearanceFontResource(style)
	return pdfStreamObject{
		Dict: pdfDict{
			"Type":     pdfName("XObject"),
			"Subtype":  pdfName("Form"),
			"FormType": 1,
			"BBox":     pdfArray{0, 0, width, height},
			"Resources": pdfDict{
				"Font": pdfDict{
					fontResourceName: fontResourceValue,
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

type simpleAppearanceStyle struct {
	FontResourceName  string
	FontResourceValue pdfValue
	FontSize          float64
	FillGray          *float64
	FillRGB           *[3]float64
	TextMatrix        *[6]float64
}

func defaultSimpleAppearanceStyle() simpleAppearanceStyle {
	return simpleAppearanceStyle{
		FontResourceName: "Helv",
		FontResourceValue: pdfDict{
			"Type":     pdfName("Font"),
			"Subtype":  pdfName("Type1"),
			"BaseFont": pdfName("Helvetica"),
		},
		FontSize: simpleAppearanceFontSize,
	}
}

func (style simpleAppearanceStyle) fontSize() float64 {
	if style.FontSize > 0 {
		return style.FontSize
	}
	return simpleAppearanceFontSize
}

func (style simpleAppearanceStyle) leading() float64 {
	return style.fontSize() * (simpleAppearanceLeading / simpleAppearanceFontSize)
}

func simpleAppearanceFontResource(style simpleAppearanceStyle) (string, pdfValue) {
	if style.FontResourceName != "" && style.FontResourceValue != nil {
		return style.FontResourceName, style.FontResourceValue
	}
	defaultStyle := defaultSimpleAppearanceStyle()
	return defaultStyle.FontResourceName, defaultStyle.FontResourceValue
}

func textChoiceAppearanceStyle(graph *pdfGraph, match formFieldMatch, widget pdfDict) (simpleAppearanceStyle, error) {
	for _, candidate := range textChoiceDefaultAppearanceCandidates(graph, match, widget) {
		appearance, ok := parseDefaultAppearance(candidate.defaultAppearance)
		if !ok || appearance.FontSize <= 0 {
			return simpleAppearanceStyle{}, errors.New("unsupported AcroForm appearance regeneration: default appearance could not be parsed safely")
		}
		style := simpleAppearanceStyle{
			FontResourceName:  "Helv",
			FontResourceValue: defaultSimpleAppearanceStyle().FontResourceValue,
			FontSize:          appearance.FontSize,
			FillGray:          appearance.FillGray,
			FillRGB:           appearance.FillRGB,
			TextMatrix:        appearance.TextMatrix,
		}
		if fontValue, ok := resolveDefaultAppearanceFontResource(graph, candidate.defaultResources, appearance.FontResourceName); ok {
			style.FontResourceName = appearance.FontResourceName
			style.FontResourceValue = fontValue
		} else if appearance.FontResourceName != defaultSimpleAppearanceStyle().FontResourceName {
			return simpleAppearanceStyle{}, fmt.Errorf("unsupported AcroForm appearance regeneration: default appearance font resource %q was not found in /DR", appearance.FontResourceName)
		}
		return style, nil
	}
	return defaultSimpleAppearanceStyle(), nil
}

type textChoiceDefaultAppearanceCandidate struct {
	defaultAppearance string
	defaultResources  pdfDict
}

func textChoiceDefaultAppearanceCandidates(graph *pdfGraph, match formFieldMatch, widget pdfDict) []textChoiceDefaultAppearanceCandidate {
	candidates := make([]textChoiceDefaultAppearanceCandidate, 0, 3)
	if da, ok := pdfDirectFormTextString(widget["DA"]); ok {
		candidates = append(candidates, textChoiceDefaultAppearanceCandidate{
			defaultAppearance: da,
			defaultResources:  formDefaultResources(graph, widget),
		})
	}
	if da, ok := pdfDirectFormTextString(match.dict["DA"]); ok {
		candidates = append(candidates, textChoiceDefaultAppearanceCandidate{
			defaultAppearance: da,
			defaultResources:  formDefaultResources(graph, match.dict),
		})
	}
	if match.hasInheritedDefaultAppearance {
		candidates = append(candidates, textChoiceDefaultAppearanceCandidate{
			defaultAppearance: match.inheritedDefaultAppearance,
			defaultResources:  match.inheritedDefaultResources,
		})
	}
	return candidates
}

func formDefaultResources(graph *pdfGraph, dict pdfDict) pdfDict {
	value, ok := dict["DR"]
	if !ok {
		return nil
	}
	resources, ok := resolvePDFDict(graph, value)
	if !ok {
		return nil
	}
	return resources
}

func resolveDefaultAppearanceFontResource(graph *pdfGraph, resources pdfDict, name string) (pdfValue, bool) {
	if resources == nil || name == "" {
		return nil, false
	}
	fonts, ok := resources["Font"].(pdfDict)
	if !ok {
		return nil, false
	}
	value, ok := fonts[name]
	if !ok {
		return nil, false
	}
	if _, ok := resolvePDFDict(graph, value); !ok {
		return nil, false
	}
	return value, true
}

func simpleAppearanceTextLines(width, height float64, text string) ([]string, error) {
	return simpleAppearanceTextLinesWithStyle(width, height, text, defaultSimpleAppearanceStyle())
}

func simpleAppearanceTextLinesWithStyle(width, height float64, text string, style simpleAppearanceStyle) ([]string, error) {
	fontSize := style.fontSize()
	leading := style.leading()
	usableWidth := width - simpleAppearanceInset*2
	usableHeight := height - simpleAppearanceInset*2
	if usableWidth <= 0 || usableHeight < fontSize {
		return nil, errors.New("appearance rectangle is too small for safe text layout")
	}
	maxChars := int(usableWidth / (fontSize * 0.5))
	if maxChars < 1 {
		return nil, errors.New("appearance rectangle is too narrow for safe text layout")
	}
	maxLines := int(usableHeight / leading)
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
	style := defaultSimpleAppearanceStyle()
	if includeFillColor {
		black := [3]float64{0, 0, 0}
		style.FillRGB = &black
	}
	writeSimpleAppearanceTextWithStyle(data, width, height, lines, style)
}

func writeSimpleAppearanceTextWithStyle(data *strings.Builder, width, height float64, lines []string, style simpleAppearanceStyle) {
	fontSize := style.fontSize()
	leading := style.leading()
	fontResourceName, _ := simpleAppearanceFontResource(style)
	data.WriteString("q\n")
	fmt.Fprintf(data, "0 0 %s %s re W n\n", pdfNumberToken(width), pdfNumberToken(height))
	data.WriteString("BT\n")
	fmt.Fprintf(data, "/%s %s Tf\n", fontResourceName, pdfNumberToken(fontSize))
	if style.FillGray != nil {
		fmt.Fprintf(data, "%s g\n", pdfNumberToken(*style.FillGray))
	} else if style.FillRGB != nil {
		fmt.Fprintf(data, "%s %s %s rg\n", pdfNumberToken(style.FillRGB[0]), pdfNumberToken(style.FillRGB[1]), pdfNumberToken(style.FillRGB[2]))
	}
	if style.TextMatrix != nil {
		matrix := *style.TextMatrix
		fmt.Fprintf(data, "%s %s %s %s %s %s Tm\n",
			pdfNumberToken(matrix[0]),
			pdfNumberToken(matrix[1]),
			pdfNumberToken(matrix[2]),
			pdfNumberToken(matrix[3]),
			pdfNumberToken(matrix[4]),
			pdfNumberToken(matrix[5]),
		)
	} else {
		startY := height - simpleAppearanceInset - fontSize
		if startY < simpleAppearanceInset {
			startY = simpleAppearanceInset
		}
		fmt.Fprintf(data, "%s %s Td\n", pdfNumberToken(simpleAppearanceInset), pdfNumberToken(startY))
	}
	for i, line := range lines {
		if i > 0 {
			fmt.Fprintf(data, "0 -%s Td\n", pdfNumberToken(leading))
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

func regenerateButtonFieldAppearances(graph *pdfGraph, match formFieldMatch, value string) (int, error) {
	if ap, ok := match.dict["AP"]; ok {
		normal, err := buttonNormalAppearanceDict(ap)
		if err != nil {
			return 0, err
		}
		if _, err := singleButtonOnState(normal); err == nil {
			return 0, nil
		} else if _, hasRect := match.dict["Rect"]; !hasRect {
			return 0, err
		}
	}
	if _, hasWidgetRect := match.dict["Rect"]; !hasWidgetRect {
		if _, hasKids := match.dict["Kids"]; !hasKids {
			return 0, nil
		}
	}
	if kids, ok := match.dict["Kids"].(pdfArray); ok && buttonKidsAlreadyHaveNormalAppearances(graph, kids) {
		return 0, nil
	}
	flags, hasFlags := match.effectiveFlags()
	if hasFlags && flags&(formFieldFlagBtnRadio|formFieldFlagBtnPushbutton|formFieldFlagBtnRadiosInUnison) != 0 {
		return 0, nil
	}
	widget, err := checkboxAppearanceWidget(graph, match)
	if err != nil {
		return 0, err
	}
	if err := validateCheckboxAppearanceSynthesisInput(widget); err != nil {
		return 0, err
	}
	onState, err := checkboxSynthesisOnState(match.dict, widget, value)
	if err != nil {
		return 0, err
	}
	if !checkboxNeedsAppearanceSynthesis(widget, onState) {
		return 0, nil
	}
	rect, ok := formWidgetRect(widget)
	if !ok {
		return 0, errors.New("unsupported AcroForm checkbox appearance regeneration: widget /Rect is missing or not a direct numeric rectangle")
	}
	offStream, err := checkboxAppearanceStream(rect, false)
	if err != nil {
		return 0, err
	}
	onStream, err := checkboxAppearanceStream(rect, true)
	if err != nil {
		return 0, err
	}
	nextObjectNumber := nextPDFObjectNumber(graph)
	offID := pdfObjectID{Number: nextObjectNumber, Generation: 0}
	onID := pdfObjectID{Number: nextObjectNumber + 1, Generation: 0}
	graph.Objects[offID] = &pdfIndirectObject{ID: offID, Value: offStream}
	graph.Objects[onID] = &pdfIndirectObject{ID: onID, Value: onStream}
	widget["AP"] = pdfDict{
		"N": pdfDict{
			"Off":   pdfRef{ID: offID},
			onState: pdfRef{ID: onID},
		},
	}
	return 2, nil
}

func buttonKidsAlreadyHaveNormalAppearances(graph *pdfGraph, kids pdfArray) bool {
	if len(kids) == 0 {
		return false
	}
	for _, kid := range kids {
		widget, ok := resolvePDFDict(graph, kid)
		if !ok {
			return false
		}
		ap, ok := widget["AP"]
		if !ok {
			return false
		}
		normal, err := buttonNormalAppearanceDict(ap)
		if err != nil {
			return false
		}
		if _, err := singleButtonOnState(normal); err != nil {
			return false
		}
	}
	return true
}

func checkboxAppearanceWidget(graph *pdfGraph, match formFieldMatch) (pdfDict, error) {
	if kids, ok := match.dict["Kids"].(pdfArray); ok {
		if len(kids) != 1 {
			return nil, errors.New("unsupported AcroForm checkbox appearance regeneration: radio/group button appearance synthesis is not supported")
		}
		widget, ok := resolvePDFDict(graph, kids[0])
		if !ok {
			return nil, errors.New("unsupported AcroForm checkbox appearance regeneration: widget kid is not a dictionary")
		}
		return widget, nil
	}
	if _, ok := match.dict["Rect"]; ok {
		return match.dict, nil
	}
	return nil, errors.New("unsupported AcroForm checkbox appearance regeneration: widget /Rect is missing or not a direct numeric rectangle")
}

func checkboxNeedsAppearanceSynthesis(widget pdfDict, onState string) bool {
	ap, ok := widget["AP"]
	if !ok {
		return true
	}
	normal, err := buttonNormalAppearanceDict(ap)
	if err != nil {
		return true
	}
	_, hasOff := normal["Off"]
	_, hasOn := normal[onState]
	return !hasOff || !hasOn
}

func validateCheckboxAppearanceSynthesisInput(widget pdfDict) error {
	if _, hasMK := widget["MK"]; hasMK {
		return errors.New("unsupported AcroForm checkbox appearance regeneration: widget appearance characteristics are not supported")
	}
	apValue, hasAP := widget["AP"]
	if !hasAP {
		return nil
	}
	ap, ok := apValue.(pdfDict)
	if !ok {
		return errors.New("unsupported AcroForm checkbox appearance regeneration: /AP is not a dictionary")
	}
	for key := range ap {
		if key != "N" {
			return errors.New("unsupported AcroForm checkbox appearance regeneration: rich /AP states are not supported")
		}
	}
	if _, ok := ap["N"].(pdfDict); !ok {
		return errors.New("unsupported AcroForm checkbox appearance regeneration: /AP /N is not a state dictionary")
	}
	return nil
}

func checkboxSynthesisOnState(field pdfDict, widget pdfDict, value string) (string, error) {
	if explicitCheckboxOnStateValue(value) {
		return value, nil
	}
	states := make(map[string]bool)
	for _, dict := range []pdfDict{field, widget} {
		for _, key := range []string{"V", "AS"} {
			if state, ok := dict[key].(pdfName); ok && state != "Off" {
				states[string(state)] = true
			}
		}
	}
	if len(states) != 1 {
		return "", errors.New("unsupported AcroForm checkbox appearance regeneration: missing checked appearance state")
	}
	for state := range states {
		if !safeSyntheticButtonStateName(state) {
			return "", fmt.Errorf("unsupported AcroForm checkbox appearance regeneration: unsafe checked appearance state %q", state)
		}
		return state, nil
	}
	return "", errors.New("unsupported AcroForm checkbox appearance regeneration: missing checked appearance state")
}

func explicitCheckboxOnStateValue(value string) bool {
	switch value {
	case "", "true", "false", "Off", "On":
		return false
	default:
		return safeSyntheticButtonStateName(value)
	}
}

func safeSyntheticButtonStateName(value string) bool {
	if value == "" || value == "Off" {
		return false
	}
	for _, r := range value {
		if r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z' || r >= '0' && r <= '9' || r == '_' || r == '-' || r == '.' {
			continue
		}
		return false
	}
	return true
}

func checkboxAppearanceStream(rect []float64, checked bool) (pdfStreamObject, error) {
	width := rect[2] - rect[0]
	height := rect[3] - rect[1]
	if width < 6 || height < 6 {
		return pdfStreamObject{}, errors.New("unsupported AcroForm checkbox appearance regeneration: widget /Rect is too small for safe checkbox drawing")
	}
	inset := 1.0
	var data strings.Builder
	data.WriteString("q\n")
	data.WriteString("0 0 0 RG\n")
	data.WriteString("0.75 w\n")
	fmt.Fprintf(&data, "%s %s %s %s re S\n", pdfNumberToken(inset), pdfNumberToken(inset), pdfNumberToken(width-inset*2), pdfNumberToken(height-inset*2))
	if checked {
		fmt.Fprintf(&data, "%s %s m\n", pdfNumberToken(width*0.25), pdfNumberToken(height*0.5))
		fmt.Fprintf(&data, "%s %s l\n", pdfNumberToken(width*0.42), pdfNumberToken(height*0.28))
		fmt.Fprintf(&data, "%s %s l\n", pdfNumberToken(width*0.78), pdfNumberToken(height*0.74))
		data.WriteString("S\n")
	}
	data.WriteString("Q")
	return pdfStreamObject{
		Dict: pdfDict{
			"Type":      pdfName("XObject"),
			"Subtype":   pdfName("Form"),
			"FormType":  1,
			"BBox":      pdfArray{0, 0, width, height},
			"Resources": pdfDict{},
		},
		Data: []byte(data.String()),
	}, nil
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
	for key := range ap {
		if key != "N" {
			return nil, errors.New("unsupported AcroForm button field: rich /AP states are not supported")
		}
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
	id                            pdfObjectID
	dict                          pdfDict
	fullName                      string
	localName                     string
	inheritedFT                   pdfName
	hasInheritedFT                bool
	inheritedFlags                int
	hasFlags                      bool
	inheritedDefaultAppearance    string
	hasInheritedDefaultAppearance bool
	inheritedDefaultResources     pdfDict
}

func (match formFieldMatch) context() formFieldContext {
	return formFieldContext{
		fieldType:            match.inheritedFT,
		hasFT:                match.hasInheritedFT,
		flags:                match.inheritedFlags,
		hasFlags:             match.hasFlags,
		defaultAppearance:    match.inheritedDefaultAppearance,
		hasDefaultAppearance: match.hasInheritedDefaultAppearance,
		defaultResources:     match.inheritedDefaultResources,
	}
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

func (match formFieldMatch) multiSelectChoiceField() bool {
	flags, ok := match.effectiveFlags()
	return ok && flags&formFieldFlagChMultiSelect != 0
}

func (match formFieldMatch) effectiveFlags() (int, bool) {
	flags := 0
	if direct, ok := dictInt(match.dict, "Ff"); ok {
		flags = direct
	} else if match.hasFlags {
		flags = match.inheritedFlags
	} else {
		return 0, false
	}
	return flags, true
}

type formFieldContext struct {
	nameSegments         []string
	fieldType            pdfName
	hasFT                bool
	flags                int
	hasFlags             bool
	defaultAppearance    string
	hasDefaultAppearance bool
	defaultResources     pdfDict
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
		if da, ok := pdfDirectFormTextString(v["DA"]); ok {
			childContext.defaultAppearance = da
			childContext.hasDefaultAppearance = true
		}
		if dr := formDefaultResources(graph, v); dr != nil {
			childContext.defaultResources = dr
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
			appearanceGenerationStatus, appearanceGenerationBlockers := formFieldAppearanceGenerationMetadata(graph, v, context, fieldType)
			fillStatus, fillBlockers := formFieldFillMetadata(graph, v, context, fieldType)
			fields = append(fields, FormFieldMetadata{
				Name:                         fullName,
				FieldType:                    fieldType,
				AlternateName:                formFieldTextMetadata(v, "TU"),
				MappingName:                  formFieldTextMetadata(v, "TM"),
				Value:                        formFieldValue(v),
				DefaultValue:                 formFieldDefaultValue(v),
				Flags:                        formFieldFlags(v, context),
				FlagNames:                    formFieldFlagNames(v, context),
				TypeFlagNames:                formFieldTypeFlagNames(v, context, fieldType),
				ReadOnly:                     formFieldFlagSet(v, context, formFieldFlagReadOnly),
				Required:                     formFieldFlagSet(v, context, formFieldFlagRequired),
				NoExport:                     formFieldFlagSet(v, context, formFieldFlagNoExport),
				KidCount:                     formFieldKidCount(v),
				ButtonWidgetAppearanceProof:  formFieldButtonAppearanceProof(graph, v, fieldType),
				ButtonStates:                 formFieldButtonStates(graph, v, fieldType),
				Options:                      formFieldChoiceOptions(v, fieldType),
				SelectedIndexes:              formFieldChoiceSelectedIndexes(v, fieldType),
				AppearanceGenerationStatus:   appearanceGenerationStatus,
				AppearanceGenerationBlockers: appearanceGenerationBlockers,
				FillStatus:                   fillStatus,
				FillBlockers:                 fillBlockers,
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

func formFieldAppearanceGenerationMetadata(graph *pdfGraph, dict pdfDict, context formFieldContext, fieldType string) (string, []string) {
	switch fieldType {
	case "Tx", "Ch":
		blockers := formFieldAppearanceGenerationBlockers(dict, context)
		if len(blockers) > 0 {
			return "unsafe", blockers
		}
		if formFieldHasDirectAppearanceWidgetRect(graph, dict) {
			return "approximate_supported", nil
		}
		return "unsupported", []string{"missing_widget_rect"}
	case "Btn":
		if formFieldButtonAppearanceProof(graph, dict, fieldType) {
			return "existing_button_state_only", nil
		}
		return "unsupported", []string{"button_appearance_synthesis_not_supported"}
	case "":
		return "unsupported", []string{"unknown_field_type"}
	default:
		return "unsupported", []string{"unsupported_field_type"}
	}
}

func formFieldFillMetadata(graph *pdfGraph, dict pdfDict, context formFieldContext, fieldType string) (string, []string) {
	blockers := formFieldFillBlockers(graph, dict, context)
	if len(blockers) > 0 {
		return "unsafe", blockers
	}
	switch fieldType {
	case "Tx":
		return "supported", nil
	case "Ch":
		if flags, ok := effectiveFormFieldFlags(dict, context); ok && flags&formFieldFlagChMultiSelect != 0 {
			if len(formFieldChoiceOptionEntries(dict, fieldType)) == 0 {
				return "unsupported", []string{"multi_select_missing_options"}
			}
		}
		return "supported", nil
	case "Btn":
		if formFieldButtonAppearanceProof(graph, dict, fieldType) {
			return "supported", nil
		}
		return "unsupported", []string{"missing_or_ambiguous_button_appearance_states"}
	case "":
		return "unsupported", []string{"unknown_field_type"}
	default:
		return "unsupported", []string{"unsupported_field_type"}
	}
}

func formFieldFillBlockers(graph *pdfGraph, dict pdfDict, context formFieldContext) []string {
	blockers := formFieldAppearanceGenerationBlockers(dict, context)
	blockers = append(blockers, formFieldActionBlockers(graph, dict)...)
	blockers = append(blockers, formFieldTreeShapeBlockers(graph, dict)...)
	return uniqueStringsPreserveOrder(blockers)
}

func formFieldEditBlockers(graph *pdfGraph, dict pdfDict, context formFieldContext, regenerateAppearance bool) []string {
	blockers := make([]string, 0)
	if !regenerateAppearance {
		blockers = append(blockers, formFieldAppearanceGenerationBlockers(dict, context)...)
	}
	blockers = append(blockers, formFieldActionBlockers(graph, dict)...)
	blockers = append(blockers, formFieldTreeShapeBlockers(graph, dict)...)
	return uniqueStringsPreserveOrder(blockers)
}

func formFieldAppearanceGenerationBlockers(dict pdfDict, context formFieldContext) []string {
	blockers := make([]string, 0, 2)
	if _, hasRichValue := dict["RV"]; hasRichValue {
		blockers = append(blockers, "rich_text_value")
		return blockers
	}
	if flags, ok := effectiveFormFieldFlags(dict, context); ok && flags&formFieldFlagTxRichText != 0 {
		blockers = append(blockers, "rich_text_flag")
	}
	return blockers
}

func formFieldActionBlockers(graph *pdfGraph, dict pdfDict) []string {
	blockers := make([]string, 0)
	blockers = append(blockers, actionBlockersForDict(dict, "field")...)
	if kids, ok := dict["Kids"].(pdfArray); ok {
		for _, kid := range kids {
			widget, ok := resolvePDFDict(graph, kid)
			if !ok {
				continue
			}
			blockers = append(blockers, actionBlockersForDict(widget, "widget")...)
		}
	}
	return blockers
}

func actionBlockersForDict(dict pdfDict, prefix string) []string {
	blockers := make([]string, 0, 3)
	if _, ok := dict["A"]; ok {
		blockers = append(blockers, prefix+"_action")
	}
	if _, ok := dict["AA"]; ok {
		blockers = append(blockers, prefix+"_additional_actions")
	}
	if pdfDictContainsJavaScriptAction(dict["A"]) || pdfDictContainsJavaScriptAction(dict["AA"]) {
		blockers = append(blockers, prefix+"_javascript_action")
	}
	return blockers
}

func pdfDictContainsJavaScriptAction(value pdfValue) bool {
	switch v := value.(type) {
	case pdfDict:
		if s, ok := v["S"].(pdfName); ok && s == "JavaScript" {
			return true
		}
		for _, item := range v {
			if pdfDictContainsJavaScriptAction(item) {
				return true
			}
		}
	case pdfArray:
		for _, item := range v {
			if pdfDictContainsJavaScriptAction(item) {
				return true
			}
		}
	}
	return false
}

func formFieldTreeShapeBlockers(graph *pdfGraph, dict pdfDict) []string {
	kids, ok := dict["Kids"].(pdfArray)
	if !ok {
		return nil
	}
	blockers := make([]string, 0)
	for _, kid := range kids {
		child, ok := resolvePDFDict(graph, kid)
		if !ok {
			blockers = append(blockers, "unresolved_kid")
			continue
		}
		if _, hasGrandkids := child["Kids"]; hasGrandkids {
			blockers = append(blockers, "nested_kids")
		}
		if _, hasName := pdfTextValue(child["T"]); hasName && child["Subtype"] != pdfName("Widget") {
			blockers = append(blockers, "named_non_widget_kid")
		}
	}
	return blockers
}

func uniqueStringsPreserveOrder(values []string) []string {
	if len(values) == 0 {
		return nil
	}
	seen := make(map[string]bool, len(values))
	out := make([]string, 0, len(values))
	for _, value := range values {
		if seen[value] {
			continue
		}
		seen[value] = true
		out = append(out, value)
	}
	return out
}

func formFieldHasDirectAppearanceWidgetRect(graph *pdfGraph, dict pdfDict) bool {
	if kids, ok := dict["Kids"].(pdfArray); ok {
		if len(kids) == 0 {
			return false
		}
		for _, kid := range kids {
			widget, ok := resolvePDFDict(graph, kid)
			if !ok {
				return false
			}
			if _, ok := formWidgetRect(widget); !ok {
				return false
			}
		}
		return true
	}
	_, ok := formWidgetRect(dict)
	return ok
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
	entries := formFieldChoiceOptionEntries(dict, fieldType)
	if len(entries) == 0 {
		return nil
	}
	out := make([]string, 0, len(entries))
	for _, entry := range entries {
		out = append(out, entry.display)
	}
	return out
}

type formFieldChoiceOption struct {
	index   int
	export  string
	display string
}

func formFieldChoiceOptionEntries(dict pdfDict, fieldType string) []formFieldChoiceOption {
	if fieldType != "Ch" {
		return nil
	}
	opt, ok := dict["Opt"]
	if !ok {
		return nil
	}
	if text, ok := pdfTextValue(opt); ok {
		return []formFieldChoiceOption{{index: 0, export: text, display: text}}
	}
	options, ok := opt.(pdfArray)
	if !ok {
		return nil
	}
	out := make([]formFieldChoiceOption, 0, len(options))
	for i, option := range options {
		if text, ok := pdfTextValue(option); ok {
			out = append(out, formFieldChoiceOption{index: i, export: text, display: text})
			continue
		}
		pair, ok := option.(pdfArray)
		if !ok || len(pair) != 2 {
			continue
		}
		export, hasExport := pdfTextValue(pair[0])
		display, hasDisplay := pdfTextValue(pair[1])
		if hasDisplay {
			if !hasExport {
				export = display
			}
			out = append(out, formFieldChoiceOption{index: i, export: export, display: display})
			continue
		}
		if hasExport {
			out = append(out, formFieldChoiceOption{index: i, export: export, display: export})
		}
	}
	if len(out) == 0 {
		return nil
	}
	return out
}

func uniqueChoiceOptionSelection(options []formFieldChoiceOption, value string) (formFieldChoiceOption, bool) {
	selected := formFieldChoiceOption{}
	matches := 0
	for _, option := range options {
		if value != option.export && value != option.display {
			continue
		}
		selected = option
		matches++
	}
	return selected, matches == 1
}

func formFieldChoiceSelectedIndexes(dict pdfDict, fieldType string) []int {
	if fieldType != "Ch" {
		return nil
	}
	indexes, ok := dictIntArray(dict, "I")
	if !ok || len(indexes) == 0 {
		return nil
	}
	if len(formFieldChoiceOptionEntries(dict, fieldType)) == 0 {
		return nil
	}
	return indexes
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
		if da, ok := pdfDirectFormTextString(v["DA"]); ok {
			childContext.defaultAppearance = da
			childContext.hasDefaultAppearance = true
		}
		if dr := formDefaultResources(graph, v); dr != nil {
			childContext.defaultResources = dr
		}
		if name, ok := pdfTextValue(v["T"]); ok {
			fullName := name
			if len(context.nameSegments) > 0 {
				fullName = strings.Join(append(append([]string{}, context.nameSegments...), name), ".")
			}
			if fullName == fieldName {
				matches = append(matches, formFieldMatch{
					dict:                          v,
					fullName:                      fullName,
					localName:                     name,
					inheritedFT:                   context.fieldType,
					hasInheritedFT:                context.hasFT,
					inheritedFlags:                context.flags,
					hasFlags:                      context.hasFlags,
					inheritedDefaultAppearance:    context.defaultAppearance,
					hasInheritedDefaultAppearance: context.hasDefaultAppearance,
					inheritedDefaultResources:     context.defaultResources,
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
		if got := formFieldValue(matches[index].dict); got != nil && formFieldValueMatchesEdit(graph, matches[index], *got, value) {
			fieldValueSet = true
		}
		if expectAppearanceRegenerated {
			if isButtonFormField(matches[index]) {
				appearanceRegenerated = formFieldButtonAppearanceMatchesEdit(graph, matches[index], value)
			} else {
				appearanceRegenerated = formFieldHasRegeneratedNormalAppearance(graph, matches[index], formFieldAppearanceValueForEdit(matches[index], value))
			}
		}
	}
	return FormFieldEditVerification{
		ReparseOK:             true,
		FieldValueSet:         fieldValueSet,
		NeedAppearancesSet:    formNeedAppearancesSet(graph),
		AppearanceRegenerated: appearanceRegenerated,
	}, nil
}

func formFieldValueMatchesEdit(graph *pdfGraph, match formFieldMatch, got string, value string) bool {
	if isButtonFormField(match) {
		return formFieldButtonAppearanceMatchesEdit(graph, match, value)
	}
	if got == value {
		return true
	}
	if !isChoiceFormField(match) {
		return false
	}
	if match.multiSelectChoiceField() {
		return formFieldMultiSelectValueMatchesEdit(match, value)
	}
	selected, ok := uniqueChoiceOptionSelection(formFieldChoiceOptionEntries(match.dict, "Ch"), value)
	return ok && got == selected.export
}

func formFieldAppearanceValueForEdit(match formFieldMatch, value string) string {
	if !isChoiceFormField(match) {
		return value
	}
	if match.multiSelectChoiceField() {
		var requested []string
		if err := json.Unmarshal([]byte(value), &requested); err != nil {
			return value
		}
		options := formFieldChoiceOptionEntries(match.dict, "Ch")
		selectedOptions := make([]formFieldChoiceOption, 0, len(requested))
		for _, item := range requested {
			selected, ok := uniqueChoiceOptionSelection(options, item)
			if !ok {
				return value
			}
			selectedOptions = append(selectedOptions, selected)
		}
		sort.Slice(selectedOptions, func(i, j int) bool {
			return selectedOptions[i].index < selectedOptions[j].index
		})
		displayValues := make([]string, 0, len(selectedOptions))
		for _, selected := range selectedOptions {
			displayValues = append(displayValues, selected.display)
		}
		return strings.Join(displayValues, "\n")
	}
	selected, ok := uniqueChoiceOptionSelection(formFieldChoiceOptionEntries(match.dict, "Ch"), value)
	if !ok {
		return value
	}
	return selected.display
}

func formFieldMultiSelectValueMatchesEdit(match formFieldMatch, value string) bool {
	var requested []string
	if err := json.Unmarshal([]byte(value), &requested); err != nil {
		return false
	}
	options := formFieldChoiceOptionEntries(match.dict, "Ch")
	selectedExports := make([]string, 0, len(requested))
	for _, item := range requested {
		selected, ok := uniqueChoiceOptionSelection(options, item)
		if !ok {
			return false
		}
		selectedExports = append(selectedExports, selected.export)
	}
	sort.Strings(selectedExports)
	gotExports, ok := pdfTextArray(match.dict["V"])
	if !ok {
		return false
	}
	sort.Strings(gotExports)
	return reflectStringSlicesEqual(gotExports, selectedExports)
}

func pdfTextArray(value pdfValue) ([]string, bool) {
	array, ok := value.(pdfArray)
	if !ok {
		return nil, false
	}
	out := make([]string, 0, len(array))
	for _, item := range array {
		text, ok := pdfDirectFormValueText(item)
		if !ok {
			return nil, false
		}
		out = append(out, text)
	}
	return out, true
}

func reflectStringSlicesEqual(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
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

func formFieldHasRegeneratedNormalAppearance(graph *pdfGraph, match formFieldMatch, value string) bool {
	widgets, err := textChoiceAppearanceWidgets(graph, match)
	if err != nil {
		return false
	}
	if len(widgets) == 0 {
		return false
	}
	for _, widget := range widgets {
		stream, ok := formFieldNormalAppearanceStream(graph, widget)
		if !ok || !formFieldAppearanceStreamMatchesRegeneratedContent(graph, match, widget, stream, value) {
			return false
		}
	}
	return true
}

func formFieldNormalAppearanceStream(graph *pdfGraph, widget pdfDict) (pdfStreamObject, bool) {
	ap, ok := widget["AP"].(pdfDict)
	if !ok {
		return pdfStreamObject{}, false
	}
	ref, ok := ap["N"].(pdfRef)
	if !ok {
		return pdfStreamObject{}, false
	}
	object, ok := graph.Objects[ref.ID]
	if !ok {
		return pdfStreamObject{}, false
	}
	stream, ok := object.Value.(pdfStreamObject)
	return stream, ok
}

func formFieldAppearanceStreamMatchesRegeneratedContent(graph *pdfGraph, match formFieldMatch, widget pdfDict, stream pdfStreamObject, value string) bool {
	if stream.Dict["Type"] != pdfName("XObject") || stream.Dict["Subtype"] != pdfName("Form") {
		return false
	}
	if !formFieldAppearanceHasFontResource(stream) {
		return false
	}
	data := string(stream.Data)
	for _, token := range []string{"q\n", " re W n\n", "BT\n", " Tf\n", " Tj\n", "ET\n", "Q"} {
		if !strings.Contains(data, token) {
			return false
		}
	}
	rect, ok := formWidgetRect(widget)
	if !ok {
		return false
	}
	style, err := textChoiceAppearanceStyle(graph, match, widget)
	if err != nil {
		return false
	}
	lines, err := simpleAppearanceTextLinesWithStyle(rect[2]-rect[0], rect[3]-rect[1], value, style)
	if err != nil || len(lines) == 0 {
		return false
	}
	for _, line := range lines {
		if line == "" {
			continue
		}
		if strings.Contains(data, "("+encodeLiteralString(line)+") Tj") {
			return true
		}
	}
	return value == "" && strings.Contains(data, "() Tj")
}

func formFieldAppearanceHasFontResource(stream pdfStreamObject) bool {
	resources, ok := stream.Dict["Resources"].(pdfDict)
	if !ok {
		return false
	}
	fonts, ok := resources["Font"].(pdfDict)
	return ok && len(fonts) > 0
}

func formFieldButtonAppearanceMatchesEdit(graph *pdfGraph, match formFieldMatch, value string) bool {
	if ap, ok := match.dict["AP"]; ok {
		normal, err := buttonNormalAppearanceDict(ap)
		if err != nil {
			return false
		}
		state, err := buttonAppearanceState(normal, value)
		return err == nil && match.dict["V"] == pdfName(state) && match.dict["AS"] == pdfName(state)
	}
	kids, ok := match.dict["Kids"].(pdfArray)
	if !ok || len(kids) == 0 {
		return false
	}
	if graph == nil {
		return false
	}
	if len(kids) == 1 {
		widget, ok := resolvePDFDict(graph, kids[0])
		if !ok {
			return false
		}
		normal, err := buttonNormalAppearanceDict(widget["AP"])
		if err != nil {
			return false
		}
		state, err := buttonAppearanceState(normal, value)
		return err == nil && match.dict["V"] == pdfName(state) && widget["AS"] == pdfName(state)
	}
	if value == "false" || value == "Off" {
		if match.dict["V"] != pdfName("Off") {
			return false
		}
		for _, kid := range kids {
			widget, ok := resolvePDFDict(graph, kid)
			if !ok || widget["AS"] != pdfName("Off") {
				return false
			}
		}
		return true
	}
	if value == "" || value == "true" || value == "On" || match.dict["V"] != pdfName(value) {
		return false
	}
	selected := 0
	for _, kid := range kids {
		widget, ok := resolvePDFDict(graph, kid)
		if !ok {
			return false
		}
		normal, err := buttonNormalAppearanceDict(widget["AP"])
		if err != nil {
			return false
		}
		_, hasState := normal[value]
		if hasState && widget["AS"] == pdfName(value) {
			selected++
			continue
		}
		if widget["AS"] != pdfName("Off") {
			return false
		}
	}
	return selected == 1
}
