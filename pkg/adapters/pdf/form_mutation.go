package pdf

import (
	"errors"
	"fmt"
)

const (
	formFieldCreateOperation  = "pdf.create_form_field"
	formFieldRemoveOperation  = "pdf.remove_form_field"
	formFieldFlattenOperation = "pdf.flatten_form_fields"
)

type FormFieldCreateOptions struct {
	Name         string     `json:"name"`
	PageIndex    int        `json:"page_index"`
	Rect         [4]float64 `json:"rect"`
	DefaultValue string     `json:"default_value,omitempty"`
	FieldType    string     `json:"field_type,omitempty"`
}

type FormFieldMutationReport struct {
	Operation         string `json:"operation"`
	FieldName         string `json:"field_name,omitempty"`
	FieldType         string `json:"field_type,omitempty"`
	PageIndex         int    `json:"page_index,omitempty"`
	NodesModified     int    `json:"nodes_modified,omitempty"`
	UnsupportedReason string `json:"unsupported_reason,omitempty"`
}

type FormFieldMutationVerification struct {
	ReparseOK          bool `json:"reparse_ok"`
	FieldPresent       bool `json:"field_present,omitempty"`
	WidgetOnPage       bool `json:"widget_on_page,omitempty"`
	NeedAppearancesSet bool `json:"need_appearances_set,omitempty"`
	Flattened          bool `json:"flattened,omitempty"`
}

type UnsupportedFormFlatteningError struct {
	Reason string
}

func (e *UnsupportedFormFlatteningError) Error() string {
	return "unsupported AcroForm flattening: " + e.Reason
}

func CreateFormField(input []byte, options FormFieldCreateOptions) ([]byte, FormFieldMutationReport, FormFieldMutationVerification, error) {
	report := FormFieldMutationReport{
		Operation: formFieldCreateOperation,
		FieldName: options.Name,
		FieldType: formFieldCreateType(options),
		PageIndex: options.PageIndex,
	}
	if err := validateFormFieldCreateOptions(options); err != nil {
		return nil, report, FormFieldMutationVerification{}, err
	}
	graph, err := parsePDFGraph(input)
	if err != nil {
		return nil, report, FormFieldMutationVerification{}, err
	}
	if existing := formFieldMatches(graph, options.Name); len(existing) > 0 {
		return nil, report, FormFieldMutationVerification{}, fmt.Errorf("AcroForm field %q already exists", options.Name)
	}
	pages, err := graph.orderedPages()
	if err != nil {
		return nil, report, FormFieldMutationVerification{}, err
	}
	if options.PageIndex < 0 || options.PageIndex >= len(pages) {
		return nil, report, FormFieldMutationVerification{}, fmt.Errorf("page index %d out of range for %d pages", options.PageIndex, len(pages))
	}
	acroForm, err := ensureCatalogAcroFormDict(graph)
	if err != nil {
		return nil, report, FormFieldMutationVerification{}, err
	}
	fields, ok := acroForm["Fields"].(pdfArray)
	if !ok {
		if _, exists := acroForm["Fields"]; exists {
			return nil, report, FormFieldMutationVerification{}, errors.New("unsupported AcroForm field creation: /AcroForm /Fields is not an array")
		}
		fields = pdfArray{}
	}

	fieldID := pdfObjectID{Number: nextPDFObjectNumber(graph), Generation: 0}
	fieldRef := pdfRef{ID: fieldID}
	field := pdfDict{
		"FT":      pdfName("Tx"),
		"Subtype": pdfName("Widget"),
		"T":       pdfLiteralString(encodeLiteralString(options.Name)),
		"Rect":    formMutationRectArray(options.Rect),
		"P":       pdfRef{ID: pages[options.PageIndex].ID},
	}
	if options.DefaultValue != "" {
		field["V"] = pdfLiteralString(encodeLiteralString(options.DefaultValue))
	}
	graph.Objects[fieldID] = &pdfIndirectObject{ID: fieldID, Value: field}
	acroForm["Fields"] = append(fields, fieldRef)
	acroForm["NeedAppearances"] = true
	pageAnnots, err := appendFormWidgetToPageAnnots(pages[options.PageIndex].Dict, fieldRef)
	if err != nil {
		delete(graph.Objects, fieldID)
		return nil, report, FormFieldMutationVerification{}, err
	}
	pages[options.PageIndex].Dict["Annots"] = pageAnnots

	output, err := writeCanonicalPDF(graph)
	if err != nil {
		return nil, report, FormFieldMutationVerification{}, err
	}
	verification, err := verifyFormFieldMutation(output, options.Name)
	if err != nil {
		return nil, report, verification, err
	}
	report.NodesModified = 3
	return output, report, verification, nil
}

func RemoveFormField(input []byte, fieldName string, matchIndexArg ...*int) ([]byte, FormFieldMutationReport, FormFieldMutationVerification, error) {
	report := FormFieldMutationReport{Operation: formFieldRemoveOperation, FieldName: fieldName}
	if fieldName == "" {
		return nil, report, FormFieldMutationVerification{}, errors.New("form field removal requires a field name")
	}
	if len(matchIndexArg) > 1 {
		return nil, report, FormFieldMutationVerification{}, errors.New("form field removal accepts at most one match index")
	}
	graph, err := parsePDFGraph(input)
	if err != nil {
		return nil, report, FormFieldMutationVerification{}, err
	}
	matches := formFieldMatches(graph, fieldName)
	if len(matches) == 0 {
		return nil, report, FormFieldMutationVerification{}, fmt.Errorf("no AcroForm field matches %q", fieldName)
	}
	index := 0
	var matchIndex *int
	if len(matchIndexArg) == 1 {
		matchIndex = matchIndexArg[0]
	}
	if len(matches) == 1 && matchIndex != nil {
		if *matchIndex != 0 {
			return nil, report, FormFieldMutationVerification{}, fmt.Errorf("match index %d out of range for 1 fields (zero-based)", *matchIndex)
		}
	}
	if len(matches) > 1 {
		if matchIndex == nil {
			return nil, report, FormFieldMutationVerification{}, fmt.Errorf("field %q matched %d dictionaries; pass match index to choose one", fieldName, len(matches))
		}
		if *matchIndex < 0 || *matchIndex >= len(matches) {
			return nil, report, FormFieldMutationVerification{}, fmt.Errorf("match index %d out of range for %d fields (zero-based)", *matchIndex, len(matches))
		}
		index = *matchIndex
	}
	match := matches[index]
	if err := validateSimpleFormFieldRemoval(match); err != nil {
		return nil, report, FormFieldMutationVerification{}, err
	}
	acroForm, ok := catalogAcroFormDict(graph)
	if !ok {
		return nil, report, FormFieldMutationVerification{}, errors.New("AcroForm dictionary missing")
	}
	fieldRef := pdfRef{ID: match.id}
	if err := removeFormFieldRefFromAcroForm(acroForm, fieldRef); err != nil {
		return nil, report, FormFieldMutationVerification{}, err
	}
	pages, err := graph.orderedPages()
	if err != nil {
		return nil, report, FormFieldMutationVerification{}, err
	}
	removedAnnot := false
	for _, page := range pages {
		removed, err := removeFormWidgetFromPageAnnots(page.Dict, fieldRef)
		if err != nil {
			return nil, report, FormFieldMutationVerification{}, err
		}
		removedAnnot = removedAnnot || removed
	}
	if !removedAnnot {
		return nil, report, FormFieldMutationVerification{}, errors.New("unsupported AcroForm field removal: widget reference was not present in any page /Annots array")
	}
	delete(graph.Objects, match.id)
	acroForm["NeedAppearances"] = true

	output, err := writeCanonicalPDF(graph)
	if err != nil {
		return nil, report, FormFieldMutationVerification{}, err
	}
	verification, err := verifyFormFieldMutation(output, fieldName)
	if err != nil {
		return nil, report, verification, err
	}
	report.NodesModified = 3
	return output, report, verification, nil
}

func FlattenFormFields(input []byte) ([]byte, FormFieldMutationReport, FormFieldMutationVerification, error) {
	report := FormFieldMutationReport{
		Operation:         formFieldFlattenOperation,
		UnsupportedReason: "flattening requires appearance stream painting into page contents and safe removal of field/widget structures",
	}
	err := &UnsupportedFormFlatteningError{Reason: report.UnsupportedReason}
	return nil, report, FormFieldMutationVerification{}, err
}

func validateFormFieldCreateOptions(options FormFieldCreateOptions) error {
	if options.Name == "" {
		return errors.New("form field creation requires a field name")
	}
	if fieldType := formFieldCreateType(options); fieldType != "Tx" {
		return fmt.Errorf("unsupported AcroForm field creation: field type %q is not supported", fieldType)
	}
	rect := options.Rect
	if rect[0] >= rect[2] || rect[1] >= rect[3] {
		return errors.New("form field creation requires a rectangle with x1 < x2 and y1 < y2")
	}
	return nil
}

func formFieldCreateType(options FormFieldCreateOptions) string {
	if options.FieldType == "" {
		return "Tx"
	}
	return options.FieldType
}

func formMutationRectArray(rect [4]float64) pdfArray {
	return pdfArray{rect[0], rect[1], rect[2], rect[3]}
}

func ensureCatalogAcroFormDict(graph *pdfGraph) (pdfDict, error) {
	catalog, ok := graph.catalogDict()
	if !ok {
		return nil, errors.New("missing PDF catalog")
	}
	if acroForm, ok := catalogAcroFormDict(graph); ok {
		if _, fieldsIsArray := acroForm["Fields"].(pdfArray); !fieldsIsArray {
			if _, exists := acroForm["Fields"]; exists {
				return nil, errors.New("unsupported AcroForm mutation: /AcroForm /Fields is not an array")
			}
		}
		return acroForm, nil
	}
	acroForm := pdfDict{"Fields": pdfArray{}}
	catalog["AcroForm"] = acroForm
	return acroForm, nil
}

func catalogAcroFormDict(graph *pdfGraph) (pdfDict, bool) {
	value, ok := catalogAcroForm(graph)
	if !ok {
		return nil, false
	}
	return resolvePDFDict(graph, value)
}

func appendFormWidgetToPageAnnots(page pdfDict, fieldRef pdfRef) (pdfArray, error) {
	annots, ok := page["Annots"].(pdfArray)
	if !ok {
		if _, exists := page["Annots"]; exists {
			return nil, errors.New("unsupported AcroForm field creation: page /Annots is not an array")
		}
		return pdfArray{fieldRef}, nil
	}
	return append(annots, fieldRef), nil
}

func validateSimpleFormFieldRemoval(match formFieldMatch) error {
	if match.id == (pdfObjectID{}) {
		return errors.New("unsupported AcroForm field removal: direct field dictionaries are not supported")
	}
	if _, hasKids := match.dict["Kids"]; hasKids {
		return errors.New("unsupported AcroForm field removal: parent/kid field trees are not supported")
	}
	if _, hasParent := match.dict["Parent"]; hasParent {
		return errors.New("unsupported AcroForm field removal: child field dictionaries are not supported")
	}
	if subtype, ok := match.dict["Subtype"].(pdfName); !ok || subtype != "Widget" {
		return errors.New("unsupported AcroForm field removal: field is not a terminal widget dictionary")
	}
	if ft, ok := match.fieldType(); !ok || ft != "Tx" {
		return errors.New("unsupported AcroForm field removal: only terminal text widgets are supported")
	}
	return nil
}

func removeFormFieldRefFromAcroForm(acroForm pdfDict, fieldRef pdfRef) error {
	fields, ok := acroForm["Fields"].(pdfArray)
	if !ok {
		return errors.New("unsupported AcroForm field removal: /AcroForm /Fields is not an array")
	}
	out := make(pdfArray, 0, len(fields))
	removed := false
	for _, field := range fields {
		if ref, ok := field.(pdfRef); ok && ref == fieldRef {
			removed = true
			continue
		}
		out = append(out, field)
	}
	if !removed {
		return errors.New("unsupported AcroForm field removal: field reference was not present in /AcroForm /Fields")
	}
	acroForm["Fields"] = out
	return nil
}

func removeFormWidgetFromPageAnnots(page pdfDict, fieldRef pdfRef) (bool, error) {
	annotsValue, exists := page["Annots"]
	if !exists {
		return false, nil
	}
	annots, ok := annotsValue.(pdfArray)
	if !ok {
		return false, errors.New("unsupported AcroForm field removal: page /Annots is not an array")
	}
	out := make(pdfArray, 0, len(annots))
	removed := false
	for _, annot := range annots {
		if ref, ok := annot.(pdfRef); ok && ref == fieldRef {
			removed = true
			continue
		}
		out = append(out, annot)
	}
	if len(out) == 0 {
		delete(page, "Annots")
	} else {
		page["Annots"] = out
	}
	return removed, nil
}

func verifyFormFieldMutation(output []byte, fieldName string) (FormFieldMutationVerification, error) {
	graph, err := parsePDFGraph(output)
	if err != nil {
		return FormFieldMutationVerification{}, err
	}
	matches := formFieldMatches(graph, fieldName)
	widgetOnPage := false
	if len(matches) > 0 && matches[0].id != (pdfObjectID{}) {
		pages, err := graph.orderedPages()
		if err != nil {
			return FormFieldMutationVerification{}, err
		}
		fieldRef := pdfRef{ID: matches[0].id}
		for _, page := range pages {
			annots, ok := page.Dict["Annots"].(pdfArray)
			if !ok {
				continue
			}
			for _, annot := range annots {
				if ref, ok := annot.(pdfRef); ok && ref == fieldRef {
					widgetOnPage = true
				}
			}
		}
	}
	return FormFieldMutationVerification{
		ReparseOK:          true,
		FieldPresent:       len(matches) > 0,
		WidgetOnPage:       widgetOnPage,
		NeedAppearancesSet: formNeedAppearancesSet(graph),
	}, nil
}
