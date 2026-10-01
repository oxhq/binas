//! Assembly import identities and conservative immutable resource sharing.
//! Unknown resource extensions remain occurrence-local. This is not a complete
//! PDF ownership index or a promise of zero-copy dictionaries/whole-process bounds.

use crate::{
    Limits, PdfDocument, PdfError,
    object::{IndirectObject, ObjectRef, Value},
    store::ObjectStore,
};
use binas_core::SourceId;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum CloneScope {
    SharedAsset,
    PageOccurrence(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum Role {
    Resources,
    Names,
    General,
    Stream,
    Font,
    Descriptor,
    Encoding,
    CidInfo,
    DecodeParams,
    Image,
    Form,
    Group,
    CharProcs,
}

#[derive(Default)]
pub(crate) struct PageImportMap {
    sources: Vec<(SourceId, ObjectStore)>,
    mapped: BTreeMap<(usize, ObjectRef, CloneScope), ObjectRef>,
    occurrence: usize,
}

impl PageImportMap {
    fn source_slot(&mut self, document: &PdfDocument) -> usize {
        let source = document.source_id();
        if let Some(slot) = self.sources.iter().position(|(id, objects)| {
            *id == source && objects.shares_snapshot_with(&document.parsed().objects)
        }) {
            return slot;
        }
        let slot = self.sources.len();
        self.sources
            .push((source, document.parsed().objects.clone()));
        slot
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn import_page(
        &mut self,
        document: &PdfDocument,
        original: ObjectRef,
        dictionary: BTreeMap<Vec<u8>, Value>,
        parent: ObjectRef,
        output: &mut BTreeMap<ObjectRef, IndirectObject>,
        next: &mut u32,
        limits: &Limits,
    ) -> Result<ObjectRef, PdfError> {
        checkpoint(document)?;
        let source = self.source_slot(document);
        let scope = CloneScope::PageOccurrence(self.occurrence);
        self.occurrence = self
            .occurrence
            .checked_add(1)
            .ok_or_else(|| PdfError::limit("page import occurrence overflows"))?;
        let page = allocate(next, limits)?;
        self.mapped.insert((source, original, scope), page);
        let mut importer = Importer {
            map: self,
            document,
            source,
            output,
            next,
            limits,
        };
        let mut copied = BTreeMap::new();
        for (key, value) in dictionary {
            let role = match key.as_slice() {
                b"Resources" => Some(Role::Resources),
                b"Contents" => Some(Role::Stream),
                _ => None,
            };
            let selected = if let Some(role) = role
                && shareable(
                    document,
                    &value,
                    role,
                    limits,
                    0,
                    &mut BTreeSet::new(),
                    &mut 0,
                )? {
                CloneScope::SharedAsset
            } else {
                scope
            };
            copied.insert(key, importer.remap(&value, selected, 0)?);
        }
        copied.insert(b"Parent".to_vec(), Value::Ref(parent));
        importer.output.insert(
            page,
            IndirectObject {
                value: Value::Dict(copied),
                stream: None,
                stream_offset: 0,
                offset: 0,
            },
        );
        Ok(page)
    }
}

struct Importer<'a> {
    map: &'a mut PageImportMap,
    document: &'a PdfDocument,
    source: usize,
    output: &'a mut BTreeMap<ObjectRef, IndirectObject>,
    next: &'a mut u32,
    limits: &'a Limits,
}

impl Importer<'_> {
    fn remap(&mut self, value: &Value, scope: CloneScope, depth: usize) -> Result<Value, PdfError> {
        checkpoint(self.document)?;
        if depth > self.limits.max_parser_depth {
            return Err(PdfError::limit("imported object graph depth exceeds limit"));
        }
        match value {
            Value::Ref(reference) => {
                let key = (self.source, *reference, scope);
                if let Some(mapped) = self.map.mapped.get(&key) {
                    return Ok(Value::Ref(*mapped));
                }
                let original = self.document.parsed().object(*reference)?;
                reject_ownership_boundary(&original.value)?;
                let mapped = allocate(self.next, self.limits)?;
                self.map.mapped.insert(key, mapped);
                let copied = self.remap(&original.value, scope, depth + 1)?;
                self.output.insert(
                    mapped,
                    IndirectObject {
                        value: copied,
                        stream: original.stream.clone(),
                        stream_offset: 0,
                        offset: 0,
                    },
                );
                Ok(Value::Ref(mapped))
            }
            Value::Array(values) => {
                if values.len() > self.limits.max_container_items {
                    return Err(PdfError::limit("imported array exceeds container limit"));
                }
                values
                    .iter()
                    .map(|value| self.remap(value, scope, depth + 1))
                    .collect::<Result<Vec<_>, _>>()
                    .map(Value::Array)
            }
            Value::Dict(dictionary) => {
                if dictionary.len() > self.limits.max_container_items {
                    return Err(PdfError::limit(
                        "imported dictionary exceeds container limit",
                    ));
                }
                // Inline widgets require the same refusal as indirect widgets.
                if matches!(dictionary.get(b"Subtype".as_slice()), Some(Value::Name(kind)) if kind == b"Widget")
                {
                    return Err(PdfError::unsupported(
                        "copying widget annotations without their document AcroForm is not supported",
                    ));
                }
                dictionary
                    .iter()
                    .map(|(key, value)| Ok((key.clone(), self.remap(value, scope, depth + 1)?)))
                    .collect::<Result<BTreeMap<_, _>, PdfError>>()
                    .map(Value::Dict)
            }
            _ => Ok(value.clone()),
        }
    }
}

fn reject_ownership_boundary(value: &Value) -> Result<(), PdfError> {
    if let Value::Dict(dictionary) = value {
        if matches!(dictionary.get(b"Type".as_slice()), Some(Value::Name(kind)) if matches!(kind.as_slice(), b"Page" | b"Pages" | b"Catalog"))
        {
            return Err(PdfError::unsupported(
                "page dependencies that reference another page, page-tree node or catalog are not supported",
            ));
        }
        if matches!(dictionary.get(b"Subtype".as_slice()), Some(Value::Name(kind)) if kind == b"Widget")
        {
            return Err(PdfError::unsupported(
                "copying widget annotations without their document AcroForm is not supported",
            ));
        }
    }
    Ok(())
}

/// Roles identify dictionary *keys*, never inferred ownership from object numbers.
/// A reference closure must be entirely understood before any of it is shared.
fn shareable(
    document: &PdfDocument,
    value: &Value,
    role: Role,
    limits: &Limits,
    depth: usize,
    seen: &mut BTreeSet<(ObjectRef, Role)>,
    work: &mut usize,
) -> Result<bool, PdfError> {
    checkpoint(document)?;
    *work = work
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("import analysis work overflows"))?;
    if depth > limits.max_parser_depth || *work > limits.max_container_items {
        return Err(PdfError::limit(
            "import sharing analysis exceeds traversal limit",
        ));
    }
    match value {
        Value::Ref(reference) => {
            if !seen.insert((*reference, role)) {
                return Ok(true);
            }
            let object = document.parsed().object(*reference)?;
            shareable(document, &object.value, role, limits, depth + 1, seen, work)
        }
        Value::Array(values) => {
            for value in values {
                if !shareable(document, value, role, limits, depth + 1, seen, work)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Value::Dict(dictionary) => {
            let role = if role == Role::General {
                match dictionary.get(b"Type".as_slice()) {
                    Some(Value::Name(kind)) => match kind.as_slice() {
                        b"Font" => Role::Font,
                        b"FontDescriptor" => Role::Descriptor,
                        b"XObject" => match dictionary.get(b"Subtype".as_slice()) {
                            Some(Value::Name(kind)) if kind == b"Image" => Role::Image,
                            Some(Value::Name(kind)) if kind == b"Form" => Role::Form,
                            _ => return Ok(false),
                        },
                        b"Encoding" => Role::Encoding,
                        _ => return Ok(false),
                    },
                    _ => return Ok(false),
                }
            } else {
                role
            };
            if !matches!(role, Role::Names | Role::CharProcs)
                && let Some(kind) = dictionary.get(b"Type".as_slice())
            {
                let expected: Option<&[u8]> = match role {
                    Role::Font => Some(b"Font"),
                    Role::Descriptor => Some(b"FontDescriptor"),
                    Role::Encoding => Some(b"Encoding"),
                    Role::Image | Role::Form => Some(b"XObject"),
                    Role::Group => Some(b"Group"),
                    _ => None,
                };
                if !matches!(kind, Value::Name(name) if expected == Some(name.as_slice())) {
                    return Ok(false);
                }
            }
            if role == Role::Font
                && !matches!(dictionary.get(b"Subtype".as_slice()), Some(Value::Name(kind)) if matches!(kind.as_slice(), b"Type0" | b"Type1" | b"MMType1" | b"Type3" | b"TrueType" | b"CIDFontType0" | b"CIDFontType2"))
            {
                return Ok(false);
            }
            if role == Role::Stream
                && let Some(subtype) = dictionary.get(b"Subtype".as_slice())
                && !matches!(subtype, Value::Name(kind) if matches!(kind.as_slice(), b"Type1C" | b"CIDFontType0C" | b"OpenType"))
            {
                return Ok(false);
            }
            for (key, value) in dictionary {
                let Some(nested) = child_role(role, key, dictionary) else {
                    return Ok(false);
                };
                if !shareable(document, value, nested, limits, depth + 1, seen, work)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => Ok(true),
    }
}

fn child_role(role: Role, key: &[u8], dictionary: &BTreeMap<Vec<u8>, Value>) -> Option<Role> {
    let scalar = Role::General;
    match role {
        Role::Names => Some(Role::General),
        Role::CharProcs => Some(Role::Stream),
        Role::Resources => match key {
            b"Font" | b"XObject" => Some(Role::Names),
            b"ProcSet" => Some(scalar),
            // ExtGState, color, pattern, shading and optional-content graphs
            // remain local until their own key-role contracts are implemented.
            _ => None,
        },
        Role::Font => match key {
            b"Type" | b"Subtype" | b"BaseFont" | b"Name" | b"FirstChar" | b"LastChar"
            | b"Widths" | b"DW" | b"W" | b"DW2" | b"W2" => Some(scalar),
            b"Encoding" => Some(Role::Encoding),
            b"FontDescriptor" | b"DescendantFonts" => Some(Role::General),
            b"ToUnicode" | b"CIDToGIDMap" => Some(Role::Stream),
            b"CIDSystemInfo" => Some(Role::CidInfo),
            b"Resources" | b"CharProcs" | b"FontBBox" | b"FontMatrix" if matches!(dictionary.get(b"Subtype".as_slice()), Some(Value::Name(kind)) if kind == b"Type3") => {
                Some(match key {
                    b"Resources" => Role::Resources,
                    b"CharProcs" => Role::CharProcs,
                    _ => scalar,
                })
            }
            _ => None,
        },
        Role::Descriptor => match key {
            b"Type" | b"FontName" | b"FontFamily" | b"FontStretch" | b"FontWeight" | b"Flags"
            | b"FontBBox" | b"ItalicAngle" | b"Ascent" | b"Descent" | b"Leading" | b"CapHeight"
            | b"XHeight" | b"StemV" | b"StemH" | b"AvgWidth" | b"MaxWidth" | b"MissingWidth"
            | b"CharSet" => Some(scalar),
            b"FontFile" | b"FontFile2" | b"FontFile3" => Some(Role::Stream),
            _ => None,
        },
        Role::Encoding => match key {
            b"Type" | b"BaseEncoding" | b"Differences" => Some(scalar),
            _ => None,
        },
        Role::CidInfo => match key {
            b"Registry" | b"Ordering" | b"Supplement" => Some(scalar),
            _ => None,
        },
        Role::DecodeParams => match key {
            b"Predictor"
            | b"Colors"
            | b"BitsPerComponent"
            | b"Columns"
            | b"EarlyChange"
            | b"K"
            | b"EndOfLine"
            | b"EncodedByteAlign"
            | b"Rows"
            | b"EndOfBlock"
            | b"BlackIs1"
            | b"DamagedRowsBeforeError"
            | b"ColorTransform" => Some(scalar),
            _ => None,
        },
        Role::Group => match key {
            b"Type" | b"S" | b"CS" | b"I" | b"K" => Some(scalar),
            _ => None,
        },
        Role::Form => match key {
            b"Type" | b"Subtype" | b"FormType" | b"BBox" | b"Matrix" => Some(scalar),
            b"Resources" => Some(Role::Resources),
            b"Group" => Some(Role::Group),
            _ => stream_key(key),
        },
        Role::Image => match key {
            b"Type" | b"Subtype" | b"Width" | b"Height" | b"ColorSpace" | b"BitsPerComponent"
            | b"ImageMask" | b"Decode" | b"Interpolate" | b"Matte" => Some(scalar),
            b"Mask" | b"SMask" => Some(Role::General),
            _ => stream_key(key),
        },
        Role::Stream => stream_key(key),
        Role::General => None,
    }
}

fn stream_key(key: &[u8]) -> Option<Role> {
    match key {
        b"Length" | b"Length1" | b"Length2" | b"Length3" | b"Filter" | b"Subtype" => {
            Some(Role::General)
        }
        b"DecodeParms" => Some(Role::DecodeParams),
        _ => None,
    }
}

fn allocate(next: &mut u32, limits: &Limits) -> Result<ObjectRef, PdfError> {
    if usize::try_from(*next).map_or(true, |next| next.saturating_sub(1) >= limits.max_objects) {
        return Err(PdfError::limit("copied object closure exceeds max_objects"));
    }
    let reference = ObjectRef {
        number: *next,
        generation: 0,
    };
    *next = next
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("copied object number overflows"))?;
    if usize::try_from(*next).map_or(true, |size| size > limits.max_xref_entries) {
        return Err(PdfError::limit(
            "copied object closure exceeds max_xref_entries",
        ));
    }
    Ok(reference)
}

fn checkpoint(document: &PdfDocument) -> Result<(), PdfError> {
    if document.parsed().decode_cancellation.is_cancelled() {
        Err(PdfError::from_context(binas_core::ContextError::Cancelled))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OpenOptions, PdfEngine};

    fn key(number: u32) -> ObjectRef {
        ObjectRef {
            number,
            generation: 0,
        }
    }
    fn dict(value: &Value) -> &BTreeMap<Vec<u8>, Value> {
        let Value::Dict(dict) = value else {
            panic!("not dict")
        };
        dict
    }
    fn reference(value: &Value) -> ObjectRef {
        let Value::Ref(reference) = value else {
            panic!("not ref")
        };
        *reference
    }

    fn fixture(font: &str, extension: &str) -> PdfDocument {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /Resources 5 0 R /MediaBox [0 0 300 300] /CropBox [10 10 290 290] /Rotate 90 /BleedBox [20 20 280 280] /TrimBox [30 30 270 270] /ArtBox [40 40 260 260] >>".into(),
            "<< /Type /Page /Parent 2 0 R /Contents 6 0 R /Annots [8 0 R] /Resources null /MediaBox null /CropBox null /Rotate null >>".into(),
            "<< /Type /Page /Parent 2 0 R /Contents 6 0 R >>".into(),
            format!("<< /Font << /F1 7 0 R >> {extension} >>"),
            "<< /Length 3 >>\nstream\nq Q\nendstream".into(),
            font.into(),
            "<< /Type /Annot /Subtype /Text /P 3 0 R /Rect [10 10 20 20] /Contents (note) >>".into(),
            "<< /Marker (opaque) >>".into(),
            "<< /Length 3 >>\nstream\nq Q\nendstream".into(),
        ];
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = vec![0];
        for (index, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
        }
        let xref = bytes.len();
        bytes.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes(),
        );
        for offset in offsets.iter().skip(1) {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                offsets.len()
            )
            .as_bytes(),
        );
        PdfEngine::default()
            .open_owned(bytes, OpenOptions::default())
            .unwrap()
    }

    fn open(bytes: Vec<u8>) -> PdfDocument {
        PdfEngine::default()
            .open_owned(bytes, OpenOptions::default())
            .unwrap()
    }
    const FONT: &str = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>";

    #[test]
    fn actual_assembly_shares_known_resources_but_duplicates_annotations_and_pages() {
        let original = fixture(FONT, "");
        let outcome = original.copy_pages(&[0, 0, 1]).unwrap();
        let copied = open(outcome.bytes);
        let refs = copied.page_refs().unwrap();
        assert_ne!(refs[0], refs[1]);
        let pages = refs
            .iter()
            .map(|reference| dict(&copied.parsed().object(*reference).unwrap().value))
            .collect::<Vec<_>>();
        assert_eq!(
            pages[0][b"Resources".as_slice()],
            pages[1][b"Resources".as_slice()]
        );
        assert_eq!(
            pages[0][b"Resources".as_slice()],
            pages[2][b"Resources".as_slice()]
        );
        assert_eq!(
            pages[0][b"Contents".as_slice()],
            pages[2][b"Contents".as_slice()]
        );
        let annots = pages[..2]
            .iter()
            .map(|page| {
                let Value::Array(values) = &page[b"Annots".as_slice()] else {
                    panic!()
                };
                reference(&values[0])
            })
            .collect::<Vec<_>>();
        assert_ne!(annots[0], annots[1]);
        for index in 0..2 {
            assert_eq!(
                dict(&copied.parsed().object(annots[index]).unwrap().value)[b"P".as_slice()],
                Value::Ref(refs[index])
            );
        }
    }

    #[test]
    fn same_number_foreign_documents_and_unknown_extensions_do_not_share() {
        let first = fixture(FONT, "");
        let second = fixture("<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>", "");
        let merged = open(first.merge_pages(&[&second]).unwrap().bytes);
        let pages = merged.page_refs().unwrap();
        let resource = |index| {
            reference(
                &dict(&merged.parsed().object(pages[index]).unwrap().value)
                    [b"Resources".as_slice()],
            )
        };
        assert_ne!(resource(0), resource(2));
        let font = |r| {
            let resources = dict(&merged.parsed().object(r).unwrap().value);
            let fonts = dict(&resources[b"Font".as_slice()]);
            let r = reference(&fonts[b"F1".as_slice()]);
            dict(&merged.parsed().object(r).unwrap().value)[b"BaseFont".as_slice()].clone()
        };
        assert_eq!(font(resource(0)), Value::Name(b"Helvetica".to_vec()));
        assert_eq!(font(resource(2)), Value::Name(b"Courier".to_vec()));
        let unknown = fixture(FONT, "/Vendor 9 0 R");
        let copied = open(unknown.copy_pages(&[0, 0]).unwrap().bytes);
        let pages = copied.page_refs().unwrap();
        let resources = pages
            .iter()
            .map(|r| {
                reference(
                    &dict(&copied.parsed().object(*r).unwrap().value)[b"Resources".as_slice()],
                )
            })
            .collect::<Vec<_>>();
        assert_ne!(resources[0], resources[1]);
        assert_ne!(
            dict(&copied.parsed().object(resources[0]).unwrap().value)[b"Vendor".as_slice()],
            dict(&copied.parsed().object(resources[1]).unwrap().value)[b"Vendor".as_slice()]
        );
    }

    #[test]
    fn type3_font_closure_is_shared_and_payload_provenance_is_retained() {
        let original = fixture(
            "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 100 100] /FontMatrix [1 0 0 1 0 0] /FirstChar 65 /LastChar 65 /Widths [100] /Encoding << /Differences [65 /A] >> /CharProcs << /A 10 0 R >> /Resources << >> >>",
            "",
        );
        let mut imports = PageImportMap::default();
        let mut output = BTreeMap::new();
        let mut next = 3;
        let page = BTreeMap::from([
            (b"Type".to_vec(), Value::Name(b"Page".to_vec())),
            (b"Resources".to_vec(), Value::Ref(key(5))),
            (b"Contents".to_vec(), Value::Ref(key(6))),
        ]);
        let first = imports
            .import_page(
                &original,
                key(3),
                page.clone(),
                key(2),
                &mut output,
                &mut next,
                &original.parsed().limits,
            )
            .unwrap();
        let second = imports
            .import_page(
                &original,
                key(3),
                page,
                key(2),
                &mut output,
                &mut next,
                &original.parsed().limits,
            )
            .unwrap();
        assert_eq!(
            dict(&output[&first].value)[b"Resources".as_slice()],
            dict(&output[&second].value)[b"Resources".as_slice()]
        );
        let source = original
            .parsed()
            .object(key(10))
            .unwrap()
            .stream
            .as_ref()
            .unwrap();
        let imported = imports.mapped[&(0, key(10), CloneScope::SharedAsset)];
        let payload = output[&imported].stream.as_ref().unwrap();
        assert_eq!(source.as_ptr(), payload.as_ptr());
        assert_eq!(source.source_span(), payload.source_span());
    }

    #[test]
    fn graph_branches_are_distinct_import_sources() {
        let original = fixture(FONT, "");
        let clone = original.clone();
        let mut staged = original.parsed().clone();
        staged.objects.get_mut(&key(7)).unwrap().value = Value::Dict(BTreeMap::new());
        let branch = original.with_parsed(staged);
        let mut imports = PageImportMap::default();
        assert_eq!(imports.source_slot(&original), imports.source_slot(&clone));
        assert_ne!(imports.source_slot(&original), imports.source_slot(&branch));
    }

    #[test]
    fn keys_known_in_another_pdf_role_do_not_authorize_font_sharing() {
        let original = fixture(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Width 9 0 R >>",
            "",
        );
        let copied = open(original.copy_pages(&[0, 0]).unwrap().bytes);
        let refs = copied.page_refs().unwrap();
        assert_ne!(
            dict(&copied.parsed().object(refs[0]).unwrap().value)[b"Resources".as_slice()],
            dict(&copied.parsed().object(refs[1]).unwrap().value)[b"Resources".as_slice()]
        );
    }

    #[test]
    fn cycles_and_object_limits_remain_bounded() {
        let original = fixture("<< /Type /Font /Subtype /Type3 /Resources 5 0 R >>", "");
        let copied = open(original.copy_pages(&[0, 0]).unwrap().bytes);
        assert_eq!(copied.page_count().unwrap(), 2);
        let mut limits = original.parsed().limits.clone();
        limits.max_objects = 2;
        let result = PageImportMap::default().import_page(
            &original,
            key(3),
            BTreeMap::new(),
            key(2),
            &mut BTreeMap::new(),
            &mut 3,
            &limits,
        );
        assert_eq!(
            result.err().unwrap().code,
            crate::PdfErrorCode::ResourceLimit
        );
        let cancelled = binas_core::CancellationToken::new();
        cancelled.cancel();
        let stopped = original.with_cancellation(cancelled);
        assert_eq!(
            stopped.copy_pages(&[0]).err().unwrap().code,
            crate::PdfErrorCode::Cancelled
        );
    }

    #[test]
    fn inheritance_materializes_only_four_keys_and_honors_null_defaults() {
        let original = fixture(FONT, "");
        let copied = open(original.copy_pages(&[0]).unwrap().bytes);
        let page = dict(
            &copied
                .parsed()
                .object(copied.page_refs().unwrap()[0])
                .unwrap()
                .value,
        );
        assert_eq!(page[b"Rotate".as_slice()], Value::Integer(90));
        assert_eq!(
            page[b"MediaBox".as_slice()],
            Value::Array(vec![
                Value::Integer(0),
                Value::Integer(0),
                Value::Integer(300),
                Value::Integer(300)
            ])
        );
        assert_eq!(
            page[b"CropBox".as_slice()],
            Value::Array(vec![
                Value::Integer(10),
                Value::Integer(10),
                Value::Integer(290),
                Value::Integer(290)
            ])
        );
        for key in [b"BleedBox".as_slice(), b"TrimBox", b"ArtBox"] {
            assert!(!page.contains_key(key));
        }
        let mut parsed = original.parsed().clone();
        let mut source_page = dict(&parsed.object(key(3)).unwrap().value).clone();
        let trim = Value::Array([15, 15, 285, 285].into_iter().map(Value::Integer).collect());
        source_page.insert(b"TrimBox".to_vec(), trim.clone());
        source_page.insert(b"BleedBox".to_vec(), Value::Null);
        source_page.insert(b"ArtBox".to_vec(), Value::Null);
        parsed.objects.get_mut(&key(3)).unwrap().value = Value::Dict(source_page);
        let source = original.with_parsed(parsed);
        let copied = open(source.copy_pages(&[0]).unwrap().bytes);
        let page = dict(
            &copied
                .parsed()
                .object(copied.page_refs().unwrap()[0])
                .unwrap()
                .value,
        );
        assert_eq!(page[b"TrimBox".as_slice()], trim);
        assert!(!page.contains_key(b"BleedBox".as_slice()));
        assert!(!page.contains_key(b"ArtBox".as_slice()));
    }
}
