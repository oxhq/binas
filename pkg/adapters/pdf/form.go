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
	Value                       *string  `json:"value,omitempty"`
	Flags                       *int     `json:"flags,omitempty"`
	FlagNames                   []string `json:"flag_names,omitempty"`
	ReadOnly                    bool     `json:"read_only,omitempty"`
	Required                    bool     `json:"required,omitempty"`
	NoExport                    bool     `json:"no_export,omitempty"`
	KidCount                    int      `json:"kid_count"`
	ButtonWidgetAppearanceProof bool     `json:"button_widget_appearance_proof"`
	ButtonStates                []string `json:"button_states,omitempty"`
	Options                     []string `json:"options,omitempty"`
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

func ApplyFormFieldEdit(input []byte, fieldName, value string, matchIndexArg ...*int) ([]byte, core.Report, core.Verification, error) {
	if fieldName == "" {
		return nil, core.Report{}, core.Verification{}, errors.New("form field edit requires a field name")
	}
	var matchIndex *int
	if len(matchIndexArg) > 1 {
		return nil, core.Report{}, core.Verification{}, errors.New("form field edit accepts at most one match index")
	}
	if len(matchIndexArg) == 1 {
		matchIndex = matchIndexArg[0]
	}
	graph, err := parsePDFGraph(input)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	matches := formFieldMatches(graph, fieldName)
	if len(matches) == 0 {
		return nil, core.Report{}, core.Verification{}, fmt.Errorf("no AcroForm field matches %q", fieldName)
	}
	index := 0
	var selected *int
	if matchIndex != nil {
		if *matchIndex < 0 || *matchIndex >= len(matches) {
			return nil, core.Report{}, core.Verification{}, fmt.Errorf("match index %d out of range for %d fields (zero-based)", *matchIndex, len(matches))
		}
		index = *matchIndex
		selectedValue := index
		selected = &selectedValue
	} else if len(matches) > 1 {
		return nil, core.Report{}, core.Verification{}, fmt.Errorf("field %q matched %d dictionaries; pass --match-index N (zero-based, 0..%d) to choose one", fieldName, len(matches), len(matches)-1)
	}
	if isButtonFormField(matches[index]) {
		if err := applyButtonFormFieldEdit(graph, matches[index].dict, value); err != nil {
			return nil, core.Report{}, core.Verification{}, err
		}
	} else {
		matches[index].dict["V"] = pdfLiteralString(encodeLiteralString(value))
		setNeedAppearances(graph)
	}
	output, err := writeCanonicalPDF(graph)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	verification, err := verifySemanticPDF(output, pdfGraphParseOptions{})
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	return output, core.Report{
		Format:        "pdf",
		Edit:          "pdf.acroform_field_value_update",
		FallbackUsed:  false,
		NodesModified: 1,
		MatchIndex:    selected,
		Invariants:    []core.Invariant{core.InvariantReparse, core.InvariantNoFallbackUsed},
	}, verification, nil
}

func isButtonFormField(match formFieldMatch) bool {
	ft, ok := match.fieldType()
	return ok && string(ft) == "Btn"
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
}

func (match formFieldMatch) fieldType() (pdfName, bool) {
	if ft, ok := match.dict["FT"].(pdfName); ok {
		return ft, true
	}
	return match.inheritedFT, match.hasInheritedFT
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
				Value:                       formFieldValue(v),
				Flags:                       formFieldFlags(v, context),
				FlagNames:                   formFieldFlagNames(v, context),
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

func formFieldValue(dict pdfDict) *string {
	value, ok := dict["V"]
	if !ok {
		return nil
	}
	decoded := pdfValueText(value)
	return &decoded
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
