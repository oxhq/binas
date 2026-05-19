package pdf

import "testing"

func TestTextStateTracksFontLeadingAndPosition(t *testing.T) {
	tracker := newPDFTextStateTracker()

	if !tracker.Apply("BT") {
		t.Fatal("BT was not applied")
	}
	if !tracker.Apply("Tf", "/F1", "12.5") {
		t.Fatal("Tf was not applied")
	}
	if !tracker.Apply("TL", "14") {
		t.Fatal("TL was not applied")
	}
	if !tracker.Apply("Td", "10", "20") {
		t.Fatal("Td was not applied")
	}
	if !tracker.Apply("T*") {
		t.Fatal("T* was not applied")
	}

	got := tracker.Snapshot()
	if !got.InTextObject {
		t.Fatal("expected tracker to be inside a text object")
	}
	if got.FontName != "F1" || got.FontSize != 12.5 {
		t.Fatalf("font state = %q %.2f, want F1 12.5", got.FontName, got.FontSize)
	}
	if got.Leading != 14 {
		t.Fatalf("leading = %.2f, want 14", got.Leading)
	}
	if got.X != 10 || got.Y != 6 {
		t.Fatalf("position = %.2f %.2f, want 10 6", got.X, got.Y)
	}
}

func TestTextStateTDUpdatesLeadingAndMovesPosition(t *testing.T) {
	tracker := newPDFTextStateTracker()
	tracker.Apply("BT")

	if !tracker.Apply("TD", "3", "-18") {
		t.Fatal("TD was not applied")
	}
	got := tracker.Snapshot()
	if got.Leading != 18 {
		t.Fatalf("leading = %.2f, want 18", got.Leading)
	}
	if got.X != 3 || got.Y != -18 {
		t.Fatalf("position after TD = %.2f %.2f, want 3 -18", got.X, got.Y)
	}

	if !tracker.Apply("T*") {
		t.Fatal("T* was not applied")
	}
	got = tracker.Snapshot()
	if got.X != 3 || got.Y != -36 {
		t.Fatalf("position after T* = %.2f %.2f, want 3 -36", got.X, got.Y)
	}
}

func TestTextStateResetsAtBTAndET(t *testing.T) {
	tracker := newPDFTextStateTracker()
	tracker.Apply("BT")
	tracker.Apply("Tf", "/F2", "9")
	tracker.Apply("TL", "10")
	tracker.Apply("Td", "5", "6")

	if !tracker.Apply("ET") {
		t.Fatal("ET was not applied")
	}
	got := tracker.Snapshot()
	if got != (pdfTextStateSnapshot{}) {
		t.Fatalf("state after ET = %+v, want zero snapshot", got)
	}

	tracker.Apply("BT")
	got = tracker.Snapshot()
	want := pdfTextStateSnapshot{InTextObject: true}
	if got != want {
		t.Fatalf("state after fresh BT = %+v, want %+v", got, want)
	}
}
