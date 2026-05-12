//go:build js && wasm

package main

import (
	"errors"
	"syscall/js"
)

func main() {
	js.Global().Set("binasInspectPDF", js.FuncOf(func(_ js.Value, args []js.Value) any {
		input, err := bytesFromJSArg(args, 0)
		if err != nil {
			return jsonString(map[string]any{"ok": false, "error": err.Error()})
		}
		return inspectPDFJSON(input)
	}))
	js.Global().Set("binasQueryPDF", js.FuncOf(func(_ js.Value, args []js.Value) any {
		input, err := bytesFromJSArg(args, 0)
		if err != nil {
			return jsonString(map[string]any{"ok": false, "error": err.Error()})
		}
		if len(args) < 2 || args[1].Type() != js.TypeString {
			return jsonString(map[string]any{"ok": false, "error": "binasQueryPDF requires a text string"})
		}
		return queryPDFJSON(input, args[1].String())
	}))
	js.Global().Set("binasEditPDFText", js.FuncOf(func(_ js.Value, args []js.Value) any {
		input, err := bytesFromJSArg(args, 0)
		if err != nil {
			return jsEditError(err)
		}
		if len(args) < 3 || args[1].Type() != js.TypeString || args[2].Type() != js.TypeString {
			return jsEditError(errors.New("binasEditPDFText requires bytes, oldText, and newText"))
		}
		return editResultToJS(editPDFText(input, args[1].String(), args[2].String()))
	}))
	select {}
}

func bytesFromJSArg(args []js.Value, index int) ([]byte, error) {
	if len(args) <= index {
		return nil, errors.New("missing Uint8Array argument")
	}
	value := args[index]
	byteLength := value.Get("byteLength")
	if byteLength.Type() != js.TypeNumber {
		return nil, errors.New("expected Uint8Array argument")
	}
	input := make([]byte, byteLength.Int())
	copied := js.CopyBytesToGo(input, value)
	return input[:copied], nil
}

func editResultToJS(result pdfEditResult) js.Value {
	if !result.OK {
		return jsEditError(errors.New(result.Error))
	}
	object := js.Global().Get("Object").New()
	object.Set("ok", true)
	bytes := js.Global().Get("Uint8Array").New(len(result.Bytes))
	js.CopyBytesToJS(bytes, result.Bytes)
	object.Set("bytes", bytes)
	object.Set("report", jsonToJS(jsonString(result.Report)))
	if result.Verification != nil {
		object.Set("verification", jsonToJS(jsonString(result.Verification)))
	} else {
		object.Set("verification", js.Null())
	}
	return object
}

func jsEditError(err error) js.Value {
	object := js.Global().Get("Object").New()
	object.Set("ok", false)
	if err != nil {
		object.Set("error", err.Error())
	}
	return object
}

func jsonToJS(raw string) js.Value {
	return js.Global().Get("JSON").Call("parse", raw)
}
