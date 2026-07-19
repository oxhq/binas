use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    PdfDocument, PdfEngine, PdfError,
    encryption::write_encrypted_pdf,
    forms::refuse_unsafe_interactive_edit,
    limits::OpenOptions,
    parser::{IndirectObject, ObjectRef, ParsedDocument, Value},
    streams::{StreamObjectRef, read_decoded_stream},
};

type ResolvedDictionary<'a> = (&'a BTreeMap<Vec<u8>, Value>, Option<ObjectRef>);

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocumentInfoMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub creator: Option<String>,
    pub producer: Option<String>,
    pub creation_date: Option<String>,
    pub modification_date: Option<String>,
    pub trapped: Option<String>,
    pub total_entries: usize,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocumentInfoUpdate {
    /// PDF name to string value. `None` removes the entry; unspecified names are preserved.
    pub entries: BTreeMap<String, Option<String>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct XmpMetadata {
    pub xml: Vec<u8>,
    pub object_number: u32,
    pub object_generation: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct XmpMetadataUpdate {
    /// `Some` creates or replaces XMP; `None` removes it.
    pub xml: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocumentStructureReport {
    pub operation: String,
    pub input_bytes: usize,
    pub output_bytes: usize,
    pub objects_added: usize,
    pub objects_removed: usize,
    pub entries_changed: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocumentStructureVerification {
    pub passed: bool,
    pub reparsed: bool,
    pub page_count_unchanged: bool,
    pub requested_state_matches: bool,
    pub catalog_reachable: bool,
    pub unknown_entries_preserved: bool,
    pub no_dangling_references: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DocumentStructureOutcome {
    pub bytes: Vec<u8>,
    pub report: DocumentStructureReport,
    pub verification: DocumentStructureVerification,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OutlineItem {
    pub index: usize,
    pub depth: usize,
    pub title: String,
    pub destination_name: Option<String>,
    pub object_number: u32,
    pub object_generation: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OutlineCreateRequest {
    pub title: String,
    pub destination_name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OutlineRemoveRequest {
    pub outline_index: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct NamedDestination {
    pub name: String,
    pub page_index: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NamedDestinationUpdate {
    pub name: String,
    pub page_index: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EmbeddedAttachment {
    pub name: String,
    pub size: usize,
    pub object_number: u32,
    pub object_generation: u16,
}

struct ResolvedEmbeddedAttachment {
    attachment: EmbeddedAttachment,
    stream: StreamObjectRef,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EmbeddedAttachmentUpdate {
    pub name: String,
    pub data: Option<Vec<u8>>,
}

/// Read-only inventory of JavaScript action dictionaries.
///
/// `direct` is the parsed-document scan; `name_tree` is limited to actions
/// reachable through the catalog `/Names /JavaScript` name tree. Scripts are
/// returned as text only and are never executed or rewritten.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct JavaScriptActionInventory {
    pub direct: Vec<JavaScriptAction>,
    pub name_tree: Vec<JavaScriptAction>,
}

/// One directly readable JavaScript action dictionary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JavaScriptAction {
    pub object_number: u32,
    pub object_generation: u16,
    /// The catalog name-tree key when this action was reached through `/Names /JavaScript`.
    pub name: Option<String>,
    pub script: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PageLabel {
    pub page_index: usize,
    pub label: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PageLabelStyle {
    Decimal,
    LowerRoman,
    UpperRoman,
    LowerAlpha,
    UpperAlpha,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PageLabelSpec {
    pub style: Option<PageLabelStyle>,
    pub prefix: String,
    pub start: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PageLabelUpdate {
    pub page_index: usize,
    /// `Some` sets the label specification; `None` removes its starting point.
    pub spec: Option<PageLabelSpec>,
}

#[derive(Clone, Debug)]
struct PageLabelTreeNode {
    reference: Option<ObjectRef>,
    dictionary: BTreeMap<Vec<u8>, Value>,
    kind: PageLabelTreeNodeKind,
}

#[derive(Clone, Debug)]
enum PageLabelTreeNodeKind {
    Leaf(Vec<(usize, Value)>),
    Internal(Vec<PageLabelTreeNode>),
}

#[derive(Clone, Debug)]
struct DestinationNameTreeNode {
    reference: Option<ObjectRef>,
    dictionary: BTreeMap<Vec<u8>, Value>,
    kind: DestinationNameTreeNodeKind,
}

#[derive(Clone, Debug)]
enum DestinationNameTreeNodeKind {
    Leaf(Vec<(Vec<u8>, Value)>),
    Internal(Vec<DestinationNameTreeNode>),
}

pub fn read_document_info(document: &PdfDocument) -> Result<DocumentInfoMetadata, PdfError> {
    let trailer = dictionary(&document.parsed().trailer, "trailer")?;
    let Some(info) = trailer.get(b"Info".as_slice()) else {
        return Ok(DocumentInfoMetadata::default());
    };
    let (info, _) = resolve_dict(document.parsed(), info, "trailer /Info")?;
    Ok(DocumentInfoMetadata {
        title: text_entry(info, b"Title")?,
        author: text_entry(info, b"Author")?,
        subject: text_entry(info, b"Subject")?,
        keywords: text_entry(info, b"Keywords")?,
        creator: text_entry(info, b"Creator")?,
        producer: text_entry(info, b"Producer")?,
        creation_date: text_entry(info, b"CreationDate")?,
        modification_date: text_entry(info, b"ModDate")?,
        trapped: match info.get(b"Trapped".as_slice()) {
            None => None,
            Some(Value::Name(value)) => Some(String::from_utf8_lossy(value).into_owned()),
            Some(_) => return Err(PdfError::syntax("Info /Trapped is not a name", 0)),
        },
        total_entries: info.len(),
    })
}

pub fn read_xmp_metadata(document: &PdfDocument) -> Result<Option<XmpMetadata>, PdfError> {
    let (_, catalog) = catalog(document.parsed())?;
    let Some(Value::Ref(reference)) = catalog.get(b"Metadata".as_slice()) else {
        if catalog.contains_key(b"Metadata".as_slice()) {
            return Err(PdfError::unsupported(
                "catalog /Metadata must be an indirect stream",
            ));
        }
        return Ok(None);
    };
    let object = document.parsed().object(*reference)?;
    let dict = dictionary(&object.value, "XMP metadata")?;
    require_xmp_dictionary(dict)?;
    let xml = read_decoded_stream(
        document,
        StreamObjectRef {
            object_number: reference.number,
            object_generation: reference.generation,
        },
    )?;
    validate_xmp(&xml, &document.parsed().limits)?;
    Ok(Some(XmpMetadata {
        xml,
        object_number: reference.number,
        object_generation: reference.generation,
    }))
}

pub fn read_outlines(document: &PdfDocument) -> Result<Vec<OutlineItem>, PdfError> {
    let (_, catalog) = catalog(document.parsed())?;
    let Some(outlines) = catalog.get(b"Outlines".as_slice()) else {
        return Ok(Vec::new());
    };
    let (root, root_ref) = resolve_dict(document.parsed(), outlines, "catalog /Outlines")?;
    let root_ref =
        root_ref.ok_or_else(|| PdfError::unsupported("outline root must be indirect"))?;
    let mut output = Vec::new();
    let mut seen = BTreeSet::new();
    match (root.get(b"First".as_slice()), root.get(b"Last".as_slice())) {
        (Some(Value::Ref(first)), Some(Value::Ref(last))) => {
            let actual_last = walk_outline_chain(
                document.parsed(),
                *first,
                root_ref,
                0,
                &mut seen,
                &mut output,
            )?;
            if actual_last != *last {
                return Err(PdfError::syntax("outline root has an invalid /Last", 0));
            }
        }
        (None, None) => {}
        _ => {
            return Err(PdfError::syntax(
                "outline root /First and /Last must be indirect and paired",
                0,
            ));
        }
    }
    Ok(output)
}

pub fn read_named_destinations(document: &PdfDocument) -> Result<Vec<NamedDestination>, PdfError> {
    let entries = name_tree_entries(document.parsed(), b"Dests")?;
    let pages = document.page_refs()?;
    entries
        .into_iter()
        .map(|(name, value)| {
            let destination = resolve_destination(document.parsed(), &value)?;
            let page = match destination.first() {
                Some(Value::Ref(reference)) => *reference,
                _ => {
                    return Err(PdfError::unsupported(
                        "named destination must start with an indirect page",
                    ));
                }
            };
            let page_index = pages
                .iter()
                .position(|reference| *reference == page)
                .ok_or_else(|| {
                    PdfError::unsafe_rewrite(
                        "named destination references a page outside the document",
                    )
                })?;
            Ok(NamedDestination {
                name: decode_text(&name),
                page_index,
            })
        })
        .collect()
}

pub fn read_embedded_attachments(
    document: &PdfDocument,
) -> Result<Vec<EmbeddedAttachment>, PdfError> {
    embedded_attachment_entries(document)?
        .into_iter()
        .map(|entry| Ok(entry.attachment))
        .collect()
}

/// Reads bytes for one exact embedded-attachment inventory entry.
///
/// The supplied metadata is re-resolved through the catalog `/Names` tree;
/// stale, forged, or ambiguous entries are rejected rather than using their
/// object number as a generic stream selector. Encoded and decoded bytes stay
/// within the document's configured limits.
pub fn read_embedded_attachment_bytes(
    document: &PdfDocument,
    selected: &EmbeddedAttachment,
) -> Result<Vec<u8>, PdfError> {
    let mut matches = embedded_attachment_entries(document)?
        .into_iter()
        .filter(|entry| entry.attachment == *selected);
    let entry = matches
        .next()
        .ok_or_else(|| PdfError::selection("embedded attachment inventory entry was not found"))?;
    if matches.next().is_some() {
        return Err(PdfError::unsupported(
            "embedded attachment inventory entry is not a unique name-tree selector",
        ));
    }
    read_decoded_stream(document, entry.stream)
}

fn embedded_attachment_entries(
    document: &PdfDocument,
) -> Result<Vec<ResolvedEmbeddedAttachment>, PdfError> {
    name_tree_entries(document.parsed(), b"EmbeddedFiles")?
        .into_iter()
        .map(|(name, value)| {
            let Value::Ref(filespec_ref) = value else {
                return Err(PdfError::unsupported(
                    "embedded file name tree values must be indirect filespecs",
                ));
            };
            let filespec = dictionary(
                &document.parsed().object(filespec_ref)?.value,
                "embedded filespec",
            )?;
            if !matches!(filespec.get(b"Type".as_slice()), Some(Value::Name(value)) if value == b"Filespec") {
                return Err(PdfError::unsupported(
                    "embedded attachment is not a Filespec",
                ));
            }
            let (ef, _) = resolve_dict(
                document.parsed(),
                filespec
                    .get(b"EF".as_slice())
                    .ok_or_else(|| PdfError::syntax("filespec has no /EF", 0))?,
                "filespec /EF",
            )?;
            let Some(Value::Ref(stream_ref)) = ef.get(b"F".as_slice()) else {
                return Err(PdfError::unsupported(
                    "filespec /EF /F must be indirect",
                ));
            };
            let stream = document
                .parsed()
                .object(*stream_ref)?
                .stream
                .as_ref()
                .ok_or_else(|| PdfError::syntax("embedded file is not a stream", 0))?;
            if stream.len() > document.parsed().limits.max_stream_bytes {
                return Err(PdfError::limit(
                    "embedded attachment exceeds max_stream_bytes",
                ));
            }
            Ok(ResolvedEmbeddedAttachment {
                attachment: EmbeddedAttachment {
                    name: decode_text(&name),
                    size: stream.len(),
                    object_number: filespec_ref.number,
                    object_generation: filespec_ref.generation,
                },
                stream: StreamObjectRef {
                    object_number: stream_ref.number,
                    object_generation: stream_ref.generation,
                },
            })
        })
        .collect()
}

/// Lists directly readable JavaScript actions without executing or rewriting scripts.
///
/// The direct scan matches every parsed indirect action dictionary. The name-tree
/// scan is restricted to catalog-reachable `/Names /JavaScript` entries.
pub fn read_javascript_actions(
    document: &PdfDocument,
) -> Result<JavaScriptActionInventory, PdfError> {
    let parsed = document.parsed();
    let direct = parsed
        .objects
        .iter()
        .filter_map(|(reference, object)| {
            javascript_action(*reference, &object.value, None).transpose()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let name_tree = name_tree_entries(parsed, b"JavaScript")?
        .into_iter()
        .map(|(name, value)| {
            let Value::Ref(reference) = value else {
                return Err(PdfError::unsupported(
                    "JavaScript name tree values must be indirect action dictionaries",
                ));
            };
            let name = readable_javascript_text(&name, "JavaScript name tree key")?;
            javascript_action(reference, &parsed.object(reference)?.value, Some(name))?.ok_or_else(
                || {
                    PdfError::unsupported(
                        "JavaScript name tree entry does not reference a JavaScript action",
                    )
                },
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(JavaScriptActionInventory { direct, name_tree })
}

pub fn read_page_labels(document: &PdfDocument) -> Result<Vec<PageLabel>, PdfError> {
    let page_count = document.page_count()?;
    let mut labels = (0..page_count)
        .map(|page_index| PageLabel {
            page_index,
            label: (page_index + 1).to_string(),
        })
        .collect::<Vec<_>>();
    let (_, catalog) = catalog(document.parsed())?;
    let Some(value) = catalog.get(b"PageLabels".as_slice()) else {
        return Ok(labels);
    };
    let (root, root_reference) = resolve_dict(document.parsed(), value, "catalog /PageLabels")?;
    let mut seen = BTreeSet::new();
    if let Some(reference) = root_reference {
        seen.insert(reference);
    }
    let mut nodes = 0;
    let mut entries = Vec::new();
    collect_page_label_entries(
        document.parsed(),
        root,
        0,
        &mut seen,
        &mut nodes,
        &mut entries,
    )?;
    entries.sort_by_key(|(index, _)| *index);
    if entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(PdfError::syntax("page label indices must be unique", 0));
    }
    let mut current = None;
    let mut next = 0;
    for label in &mut labels {
        while let Some((index, spec)) = entries.get(next) {
            if *index > label.page_index {
                break;
            }
            current = Some((*index, spec));
            next += 1;
        }
        if let Some((start_index, spec)) = current {
            let offset = label
                .page_index
                .checked_sub(start_index)
                .ok_or_else(|| PdfError::syntax("page label index ordering is invalid", 0))?;
            label.label = spec.label(offset)?;
        }
    }
    Ok(labels)
}

impl PageLabelSpec {
    fn parse(parsed: &ParsedDocument, value: &Value) -> Result<Self, PdfError> {
        let (dictionary, _) = resolve_dict(parsed, value, "page label spec")?;
        let style = match dictionary.get(b"S".as_slice()) {
            None => None,
            Some(Value::Name(value)) => Some(PageLabelStyle::from_pdf_name(value)?),
            Some(_) => return Err(PdfError::syntax("page label /S is not a name", 0)),
        };
        let prefix = match dictionary.get(b"P".as_slice()) {
            None => String::new(),
            Some(Value::String(value)) => decode_text(value),
            Some(_) => return Err(PdfError::syntax("page label /P is not a string", 0)),
        };
        let start = match dictionary.get(b"St".as_slice()) {
            None => 1,
            Some(Value::Integer(value)) if *value > 0 => usize::try_from(*value)
                .map_err(|_| PdfError::limit("page label /St exceeds usize"))?,
            Some(Value::Integer(_)) => {
                return Err(PdfError::syntax("page label /St must be positive", 0));
            }
            Some(_) => return Err(PdfError::syntax("page label /St is not an integer", 0)),
        };
        Ok(Self {
            style,
            prefix,
            start,
        })
    }

    fn label(&self, offset: usize) -> Result<String, PdfError> {
        let number = self
            .start
            .checked_add(offset)
            .ok_or_else(|| PdfError::limit("page label number overflows"))?;
        let suffix = match self.style {
            Some(PageLabelStyle::LowerRoman) => roman_page_label(number).to_ascii_lowercase(),
            Some(PageLabelStyle::UpperRoman) => roman_page_label(number),
            Some(PageLabelStyle::LowerAlpha) => alpha_page_label(number).to_ascii_lowercase(),
            Some(PageLabelStyle::UpperAlpha) => alpha_page_label(number),
            Some(PageLabelStyle::Decimal) => number.to_string(),
            None => String::new(),
        };
        Ok(format!("{}{}", self.prefix, suffix))
    }
}

impl PageLabelStyle {
    fn from_pdf_name(value: &[u8]) -> Result<Self, PdfError> {
        match value {
            b"D" => Ok(Self::Decimal),
            b"r" => Ok(Self::LowerRoman),
            b"R" => Ok(Self::UpperRoman),
            b"a" => Ok(Self::LowerAlpha),
            b"A" => Ok(Self::UpperAlpha),
            _ => Err(PdfError::unsupported("page label style is not supported")),
        }
    }

    const fn pdf_name(self) -> &'static [u8] {
        match self {
            Self::Decimal => b"D",
            Self::LowerRoman => b"r",
            Self::UpperRoman => b"R",
            Self::LowerAlpha => b"a",
            Self::UpperAlpha => b"A",
        }
    }
}

fn collect_page_label_entries(
    parsed: &ParsedDocument,
    node: &BTreeMap<Vec<u8>, Value>,
    depth: usize,
    seen: &mut BTreeSet<ObjectRef>,
    nodes: &mut usize,
    entries: &mut Vec<(usize, PageLabelSpec)>,
) -> Result<Option<(usize, usize)>, PdfError> {
    if depth > parsed.limits.max_parser_depth {
        return Err(PdfError::limit(
            "page label tree depth exceeds parser limit",
        ));
    }
    *nodes = nodes
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("page label tree node count overflows"))?;
    if *nodes > parsed.limits.max_container_items {
        return Err(PdfError::limit(
            "page label tree node count exceeds container limit",
        ));
    }
    let limits = page_label_limits(node)?;
    let actual = match (node.get(b"Nums".as_slice()), node.get(b"Kids".as_slice())) {
        (Some(_), Some(_)) => {
            return Err(PdfError::syntax(
                "page label node cannot contain both /Nums and /Kids",
                0,
            ));
        }
        (Some(Value::Array(values)), None) => {
            if !values.len().is_multiple_of(2) {
                return Err(PdfError::syntax("page label /Nums must contain pairs", 0));
            }
            if values.len() / 2 > parsed.limits.max_container_items {
                return Err(PdfError::limit("page label /Nums exceeds container limit"));
            }
            let mut bounds = None;
            for pair in values.chunks_exact(2) {
                let Value::Integer(index) = &pair[0] else {
                    return Err(PdfError::syntax(
                        "page label /Nums index is not an integer",
                        0,
                    ));
                };
                if *index < 0 {
                    return Err(PdfError::syntax(
                        "page label /Nums index must be non-negative",
                        0,
                    ));
                }
                let index = usize::try_from(*index)
                    .map_err(|_| PdfError::limit("page label index exceeds usize"))?;
                if let Some((_, previous)) = bounds
                    && previous >= index
                {
                    return Err(PdfError::syntax(
                        "page label /Nums indices are not strictly ordered",
                        0,
                    ));
                }
                if entries.len() >= parsed.limits.max_container_items {
                    return Err(PdfError::limit("page label entry count exceeds limit"));
                }
                entries.push((index, PageLabelSpec::parse(parsed, &pair[1])?));
                bounds = Some((bounds.map_or(index, |(first, _)| first), index));
            }
            bounds
        }
        (Some(_), None) => {
            return Err(PdfError::syntax(
                "page label /Nums must be a direct array",
                0,
            ));
        }
        (None, Some(Value::Array(kids))) => {
            if kids.len() > parsed.limits.max_container_items {
                return Err(PdfError::limit("page label /Kids exceeds container limit"));
            }
            let mut bounds = None;
            for child in kids {
                let Value::Ref(reference) = child else {
                    return Err(PdfError::syntax(
                        "page label /Kids must contain indirect dictionaries",
                        0,
                    ));
                };
                if !seen.insert(*reference) {
                    return Err(PdfError::syntax("cycle in page label tree", 0));
                }
                let child = dictionary(&parsed.object(*reference)?.value, "page label child")?;
                let child_bounds =
                    collect_page_label_entries(parsed, child, depth + 1, seen, nodes, entries)?
                        .ok_or_else(|| {
                            PdfError::syntax("page label /Kids contains an empty node", 0)
                        })?;
                if let Some((_, previous)) = bounds
                    && previous >= child_bounds.0
                {
                    return Err(PdfError::syntax(
                        "page label child ranges are not strictly ordered",
                        0,
                    ));
                }
                bounds = Some((
                    bounds.map_or(child_bounds.0, |(first, _)| first),
                    child_bounds.1,
                ));
            }
            bounds
        }
        (None, Some(_)) => {
            return Err(PdfError::syntax(
                "page label /Kids must be a direct array",
                0,
            ));
        }
        (None, None) => None,
    };
    match (limits, actual) {
        (None, _) => Ok(actual),
        (Some(_), None) => Err(PdfError::syntax(
            "empty page label node must not have /Limits",
            0,
        )),
        (Some(expected), Some(actual)) if expected == actual => Ok(Some(actual)),
        _ => Err(PdfError::syntax(
            "page label /Limits do not match entries",
            0,
        )),
    }
}

fn page_label_limits(node: &BTreeMap<Vec<u8>, Value>) -> Result<Option<(usize, usize)>, PdfError> {
    let Some(value) = node.get(b"Limits".as_slice()) else {
        return Ok(None);
    };
    let Value::Array(values) = value else {
        return Err(PdfError::syntax(
            "page label /Limits must be a direct array",
            0,
        ));
    };
    let [Value::Integer(first), Value::Integer(last)] = values.as_slice() else {
        return Err(PdfError::syntax(
            "page label /Limits must contain two integers",
            0,
        ));
    };
    if *first < 0 || *last < *first {
        return Err(PdfError::syntax("page label /Limits are invalid", 0));
    }
    Ok(Some((
        usize::try_from(*first).map_err(|_| PdfError::limit("page label /Limits exceed usize"))?,
        usize::try_from(*last).map_err(|_| PdfError::limit("page label /Limits exceed usize"))?,
    )))
}

fn flat_page_label_entries(parsed: &ParsedDocument) -> Result<Vec<(usize, Value)>, PdfError> {
    let (_, catalog) = catalog(parsed)?;
    let Some(value) = catalog.get(b"PageLabels".as_slice()) else {
        return Ok(Vec::new());
    };
    let (root, root_reference) = resolve_dict(parsed, value, "catalog /PageLabels")?;
    if root.contains_key(b"Kids".as_slice()) {
        return Err(PdfError::unsupported(
            "page-label mutation requires a flat number tree",
        ));
    }
    let mut seen = BTreeSet::new();
    if let Some(reference) = root_reference {
        seen.insert(reference);
    }
    let mut nodes = 0;
    collect_page_label_entries(parsed, root, 0, &mut seen, &mut nodes, &mut Vec::new())?;

    let Some(Value::Array(values)) = root.get(b"Nums".as_slice()) else {
        return Ok(Vec::new());
    };
    values
        .chunks_exact(2)
        .map(|pair| match &pair[0] {
            Value::Integer(index) if *index >= 0 => usize::try_from(*index)
                .map(|index| (index, pair[1].clone()))
                .map_err(|_| PdfError::limit("page label index exceeds usize")),
            _ => Err(PdfError::syntax(
                "page label /Nums index is not a non-negative integer",
                0,
            )),
        })
        .collect()
}

fn page_label_spec_value(
    parsed: &ParsedDocument,
    existing: Option<&Value>,
    spec: &PageLabelSpec,
) -> Result<Value, PdfError> {
    validate_page_label_spec(spec, &parsed.limits)?;
    let mut dictionary = match existing {
        Some(value) => {
            let (dictionary, reference) = resolve_dict(parsed, value, "page label spec")?;
            if let Some(reference) = reference
                && parsed.object(reference)?.stream.is_some()
            {
                return Err(PdfError::unsupported(
                    "page label specifications must not be streams",
                ));
            }
            dictionary.clone()
        }
        None => BTreeMap::new(),
    };
    match spec.style {
        Some(style) => {
            dictionary.insert(b"S".to_vec(), Value::Name(style.pdf_name().to_vec()));
        }
        None => {
            dictionary.remove(b"S".as_slice());
        }
    }
    if spec.prefix.is_empty() {
        dictionary.remove(b"P".as_slice());
    } else {
        dictionary.insert(b"P".to_vec(), Value::String(encode_text(&spec.prefix)));
    }
    if spec.start == 1 {
        dictionary.remove(b"St".as_slice());
    } else {
        dictionary.insert(
            b"St".to_vec(),
            Value::Integer(
                i64::try_from(spec.start)
                    .map_err(|_| PdfError::limit("page label /St exceeds i64"))?,
            ),
        );
    }
    Ok(Value::Dict(dictionary))
}

fn validate_page_label_spec(spec: &PageLabelSpec, limits: &crate::Limits) -> Result<(), PdfError> {
    if spec.start == 0 {
        return Err(PdfError::unsafe_rewrite("page label /St must be positive"));
    }
    if encode_text(&spec.prefix).len() > limits.max_token_bytes {
        return Err(PdfError::limit("page label prefix exceeds max_token_bytes"));
    }
    Ok(())
}

fn install_flat_page_label_tree(
    parsed: &mut ParsedDocument,
    entries: Vec<(usize, Value)>,
) -> Result<(), PdfError> {
    let (catalog_ref, source_catalog) = catalog(parsed)?;
    let mut catalog = source_catalog.clone();
    let existing_tree = catalog.get(b"PageLabels".as_slice()).cloned();
    if entries.is_empty() {
        match existing_tree {
            Some(Value::Ref(reference)) => {
                let object = parsed.object(reference)?;
                if object.stream.is_some() {
                    return Err(PdfError::unsupported(
                        "catalog /PageLabels must not be a stream",
                    ));
                }
                let mut tree = dictionary(&object.value, "catalog /PageLabels")?.clone();
                if tree.contains_key(b"Kids".as_slice()) {
                    return Err(PdfError::unsupported(
                        "page-label mutation requires a flat number tree",
                    ));
                }
                tree.remove(b"Nums".as_slice());
                tree.remove(b"Limits".as_slice());
                parsed
                    .objects
                    .insert(reference, plain_object(Value::Dict(tree)));
            }
            Some(Value::Dict(mut tree)) => {
                if tree.contains_key(b"Kids".as_slice()) {
                    return Err(PdfError::unsupported(
                        "page-label mutation requires a flat number tree",
                    ));
                }
                tree.remove(b"Nums".as_slice());
                tree.remove(b"Limits".as_slice());
                catalog.insert(b"PageLabels".to_vec(), Value::Dict(tree));
            }
            Some(_) => {
                return Err(PdfError::unsupported(
                    "catalog /PageLabels must be a dictionary or reference",
                ));
            }
            None => {}
        }
    } else {
        if entries.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
            return Err(PdfError::unsafe_rewrite(
                "page label entries must be strictly ordered",
            ));
        }
        let (first, last) = match (entries.first(), entries.last()) {
            (Some((first, _)), Some((last, _))) => (first, last),
            _ => {
                return Err(PdfError::unsafe_rewrite(
                    "page label entries are unexpectedly empty",
                ));
            }
        };
        let replacement_limits = Value::Array(vec![
            Value::Integer(
                i64::try_from(*first)
                    .map_err(|_| PdfError::limit("page label index exceeds i64"))?,
            ),
            Value::Integer(
                i64::try_from(*last)
                    .map_err(|_| PdfError::limit("page label index exceeds i64"))?,
            ),
        ]);
        let mut values = Vec::with_capacity(
            entries
                .len()
                .checked_mul(2)
                .ok_or_else(|| PdfError::limit("page label /Nums capacity overflows"))?,
        );
        for (index, value) in entries {
            values
                .push(Value::Integer(i64::try_from(index).map_err(|_| {
                    PdfError::limit("page label index exceeds i64")
                })?));
            values.push(value);
        }
        match existing_tree {
            Some(Value::Ref(reference)) => {
                let object = parsed.object(reference)?;
                if object.stream.is_some() {
                    return Err(PdfError::unsupported(
                        "catalog /PageLabels must not be a stream",
                    ));
                }
                let mut tree = dictionary(&object.value, "catalog /PageLabels")?.clone();
                if tree.contains_key(b"Kids".as_slice()) {
                    return Err(PdfError::unsupported(
                        "page-label mutation requires a flat number tree",
                    ));
                }
                tree.insert(b"Nums".to_vec(), Value::Array(values));
                if tree.contains_key(b"Limits".as_slice()) {
                    tree.insert(b"Limits".to_vec(), replacement_limits.clone());
                }
                parsed
                    .objects
                    .insert(reference, plain_object(Value::Dict(tree)));
            }
            Some(Value::Dict(mut tree)) => {
                if tree.contains_key(b"Kids".as_slice()) {
                    return Err(PdfError::unsupported(
                        "page-label mutation requires a flat number tree",
                    ));
                }
                tree.insert(b"Nums".to_vec(), Value::Array(values));
                if tree.contains_key(b"Limits".as_slice()) {
                    tree.insert(b"Limits".to_vec(), replacement_limits);
                }
                catalog.insert(b"PageLabels".to_vec(), Value::Dict(tree));
            }
            Some(_) => {
                return Err(PdfError::unsupported(
                    "catalog /PageLabels must be a dictionary or reference",
                ));
            }
            None => {
                catalog.insert(
                    b"PageLabels".to_vec(),
                    Value::Dict(BTreeMap::from([(b"Nums".to_vec(), Value::Array(values))])),
                );
            }
        }
    }
    parsed
        .objects
        .insert(catalog_ref, plain_object(Value::Dict(catalog)));
    Ok(())
}

impl PageLabelTreeNode {
    fn bounds(&self) -> Option<(usize, usize)> {
        match &self.kind {
            PageLabelTreeNodeKind::Leaf(entries) => Some((entries.first()?.0, entries.last()?.0)),
            PageLabelTreeNodeKind::Internal(children) => {
                Some((children.first()?.bounds()?.0, children.last()?.bounds()?.1))
            }
        }
    }

    fn entry_count(&self) -> usize {
        match &self.kind {
            PageLabelTreeNodeKind::Leaf(entries) => entries.len(),
            PageLabelTreeNodeKind::Internal(children) => {
                children.iter().map(Self::entry_count).sum()
            }
        }
    }

    fn entry_value(&self, page_index: usize) -> Option<&Value> {
        match &self.kind {
            PageLabelTreeNodeKind::Leaf(entries) => entries
                .iter()
                .find(|(index, _)| *index == page_index)
                .map(|(_, value)| value),
            PageLabelTreeNodeKind::Internal(children) => children
                .iter()
                .find_map(|child| child.entry_value(page_index)),
        }
    }

    fn has_index_outside(&self, page_count: usize) -> bool {
        match &self.kind {
            PageLabelTreeNodeKind::Leaf(entries) => {
                entries.iter().any(|(index, _)| *index >= page_count)
            }
            PageLabelTreeNodeKind::Internal(children) => children
                .iter()
                .any(|child| child.has_index_outside(page_count)),
        }
    }
}

fn page_label_tree_is_hierarchical(parsed: &ParsedDocument) -> Result<bool, PdfError> {
    let (_, catalog) = catalog(parsed)?;
    let Some(value) = catalog.get(b"PageLabels".as_slice()) else {
        return Ok(false);
    };
    Ok(resolve_dict(parsed, value, "catalog /PageLabels")?
        .0
        .contains_key(b"Kids".as_slice()))
}

fn load_page_label_tree(parsed: &ParsedDocument) -> Result<Option<PageLabelTreeNode>, PdfError> {
    let (_, catalog) = catalog(parsed)?;
    let Some(value) = catalog.get(b"PageLabels".as_slice()) else {
        return Ok(None);
    };
    let mut seen = BTreeSet::new();
    let mut nodes = 0;
    let mut entries = 0;
    load_page_label_tree_node(parsed, value, true, 0, &mut seen, &mut nodes, &mut entries).map(Some)
}

fn load_page_label_tree_node(
    parsed: &ParsedDocument,
    value: &Value,
    allow_direct: bool,
    depth: usize,
    seen: &mut BTreeSet<ObjectRef>,
    nodes: &mut usize,
    entries: &mut usize,
) -> Result<PageLabelTreeNode, PdfError> {
    if depth > parsed.limits.max_parser_depth {
        return Err(PdfError::limit(
            "page label tree depth exceeds parser limit",
        ));
    }
    *nodes = nodes
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("page label tree node count overflows"))?;
    if *nodes > parsed.limits.max_container_items {
        return Err(PdfError::limit(
            "page label tree node count exceeds container limit",
        ));
    }

    let (reference, dictionary) = match value {
        Value::Dict(dictionary) if allow_direct => (None, dictionary.clone()),
        Value::Ref(reference) => {
            if !seen.insert(*reference) {
                return Err(PdfError::syntax("cycle in page label tree", 0));
            }
            let object = parsed.object(*reference)?;
            if object.stream.is_some() {
                return Err(PdfError::unsupported(
                    "page label tree nodes must not be streams",
                ));
            }
            (
                Some(*reference),
                dictionary(&object.value, "page label tree node")?.clone(),
            )
        }
        _ => {
            return Err(PdfError::syntax(
                "page label tree nodes must be direct root dictionaries or indirect dictionaries",
                0,
            ));
        }
    };
    let limits = page_label_limits(&dictionary)?;
    let kind = match (
        dictionary.get(b"Nums".as_slice()),
        dictionary.get(b"Kids".as_slice()),
    ) {
        (Some(_), Some(_)) => {
            return Err(PdfError::syntax(
                "page label node cannot contain both /Nums and /Kids",
                0,
            ));
        }
        (Some(Value::Array(values)), None) => {
            if !values.len().is_multiple_of(2) {
                return Err(PdfError::syntax("page label /Nums must contain pairs", 0));
            }
            if values.len() / 2 > parsed.limits.max_container_items {
                return Err(PdfError::limit("page label /Nums exceeds container limit"));
            }
            let mut previous = None;
            let mut output = Vec::with_capacity(values.len() / 2);
            for pair in values.chunks_exact(2) {
                let Value::Integer(index) = &pair[0] else {
                    return Err(PdfError::syntax(
                        "page label /Nums index is not an integer",
                        0,
                    ));
                };
                if *index < 0 {
                    return Err(PdfError::syntax(
                        "page label /Nums index must be non-negative",
                        0,
                    ));
                }
                let index = usize::try_from(*index)
                    .map_err(|_| PdfError::limit("page label index exceeds usize"))?;
                if previous.is_some_and(|previous| previous >= index) {
                    return Err(PdfError::syntax(
                        "page label /Nums indices are not strictly ordered",
                        0,
                    ));
                }
                *entries = entries
                    .checked_add(1)
                    .ok_or_else(|| PdfError::limit("page label entry count overflows"))?;
                if *entries > parsed.limits.max_container_items {
                    return Err(PdfError::limit("page label entry count exceeds limit"));
                }
                PageLabelSpec::parse(parsed, &pair[1])?;
                output.push((index, pair[1].clone()));
                previous = Some(index);
            }
            PageLabelTreeNodeKind::Leaf(output)
        }
        (Some(_), None) => {
            return Err(PdfError::syntax(
                "page label /Nums must be a direct array",
                0,
            ));
        }
        (None, Some(Value::Array(kids))) => {
            if kids.len() > parsed.limits.max_container_items {
                return Err(PdfError::limit("page label /Kids exceeds container limit"));
            }
            let mut children = Vec::with_capacity(kids.len());
            let mut previous = None;
            for child in kids {
                let Value::Ref(reference) = child else {
                    return Err(PdfError::syntax(
                        "page label /Kids must contain indirect dictionaries",
                        0,
                    ));
                };
                let child = load_page_label_tree_node(
                    parsed,
                    &Value::Ref(*reference),
                    false,
                    depth + 1,
                    seen,
                    nodes,
                    entries,
                )?;
                let (first, last) = child.bounds().ok_or_else(|| {
                    PdfError::syntax("page label /Kids contains an empty node", 0)
                })?;
                if previous.is_some_and(|previous| previous >= first) {
                    return Err(PdfError::syntax(
                        "page label child ranges are not strictly ordered",
                        0,
                    ));
                }
                previous = Some(last);
                children.push(child);
            }
            PageLabelTreeNodeKind::Internal(children)
        }
        (None, Some(_)) => {
            return Err(PdfError::syntax(
                "page label /Kids must be a direct array",
                0,
            ));
        }
        (None, None) => PageLabelTreeNodeKind::Leaf(Vec::new()),
    };
    let node = PageLabelTreeNode {
        reference,
        dictionary,
        kind,
    };
    match (limits, node.bounds()) {
        (None, _) => Ok(node),
        (Some(_), None) => Err(PdfError::syntax(
            "empty page label node must not have /Limits",
            0,
        )),
        (Some(expected), Some(actual)) if expected == actual => Ok(node),
        _ => Err(PdfError::syntax(
            "page label /Limits do not match entries",
            0,
        )),
    }
}

fn mutate_page_label_tree(
    node: &mut PageLabelTreeNode,
    parsed: &ParsedDocument,
    page_index: usize,
    spec: Option<&PageLabelSpec>,
    total_entries: &mut usize,
) -> Result<(), PdfError> {
    match &mut node.kind {
        PageLabelTreeNodeKind::Leaf(entries) => {
            let position = entries.iter().position(|(index, _)| *index == page_index);
            match spec {
                Some(spec) => {
                    let existing = position.map(|position| entries[position].1.clone());
                    let value = page_label_spec_value(parsed, existing.as_ref(), spec)?;
                    match position {
                        Some(position) => entries[position].1 = value,
                        None => {
                            if *total_entries >= parsed.limits.max_container_items {
                                return Err(PdfError::limit(
                                    "page label entry count exceeds limit",
                                ));
                            }
                            entries.push((page_index, value));
                            entries.sort_by_key(|(index, _)| *index);
                            *total_entries += 1;
                        }
                    }
                }
                None => {
                    let position = position.ok_or_else(|| {
                        PdfError::unsafe_rewrite(
                            "page label tree selection did not reach the requested entry",
                        )
                    })?;
                    entries.remove(position);
                    *total_entries -= 1;
                }
            }
            Ok(())
        }
        PageLabelTreeNodeKind::Internal(children) => {
            let child = page_label_tree_child_index(children, page_index)?;
            mutate_page_label_tree(
                &mut children[child],
                parsed,
                page_index,
                spec,
                total_entries,
            )
        }
    }
}

fn page_label_tree_child_index(
    children: &[PageLabelTreeNode],
    page_index: usize,
) -> Result<usize, PdfError> {
    let Some(first) = children.first() else {
        return Err(PdfError::unsupported(
            "empty hierarchical page label trees require rebuilding",
        ));
    };
    let (first_index, _) = first
        .bounds()
        .ok_or_else(|| PdfError::unsafe_rewrite("page label child has no bounds"))?;
    if page_index < first_index {
        return Ok(0);
    }
    for (index, child) in children.iter().enumerate() {
        let (first, last) = child
            .bounds()
            .ok_or_else(|| PdfError::unsafe_rewrite("page label child has no bounds"))?;
        if (first..=last).contains(&page_index) {
            return Ok(index);
        }
        if page_index < first {
            return Err(PdfError::unsupported(
                "page label insertion between child ranges requires rebalancing",
            ));
        }
    }
    Ok(children.len() - 1)
}

fn write_page_label_tree(
    parsed: &mut ParsedDocument,
    node: PageLabelTreeNode,
    is_root: bool,
) -> Result<Option<Value>, PdfError> {
    let PageLabelTreeNode {
        reference,
        mut dictionary,
        kind,
    } = node;
    match kind {
        PageLabelTreeNodeKind::Leaf(entries) if entries.is_empty() => {
            if !is_root {
                return Ok(None);
            }
            dictionary.remove(b"Nums".as_slice());
            dictionary.remove(b"Kids".as_slice());
            dictionary.remove(b"Limits".as_slice());
        }
        PageLabelTreeNodeKind::Leaf(entries) => {
            let bounds = match (entries.first(), entries.last()) {
                (Some((first, _)), Some((last, _))) => (*first, *last),
                _ => {
                    return Err(PdfError::unsafe_rewrite(
                        "non-empty page label leaf has no bounds",
                    ));
                }
            };
            let mut values = Vec::with_capacity(
                entries
                    .len()
                    .checked_mul(2)
                    .ok_or_else(|| PdfError::limit("page label /Nums capacity overflows"))?,
            );
            for (index, value) in entries {
                values
                    .push(Value::Integer(i64::try_from(index).map_err(|_| {
                        PdfError::limit("page label index exceeds i64")
                    })?));
                values.push(value);
            }
            dictionary.remove(b"Kids".as_slice());
            dictionary.insert(b"Nums".to_vec(), Value::Array(values));
            refresh_page_label_tree_limits(&mut dictionary, bounds)?;
        }
        PageLabelTreeNodeKind::Internal(children) => {
            let mut values = Vec::with_capacity(children.len());
            let mut bounds = None;
            for child in children {
                let child_bounds = child.bounds();
                if let Some(value) = write_page_label_tree(parsed, child, false)? {
                    let (first, last) = child_bounds.ok_or_else(|| {
                        PdfError::unsafe_rewrite("written page label child has no bounds")
                    })?;
                    if let Some((_, previous)) = bounds
                        && previous >= first
                    {
                        return Err(PdfError::unsafe_rewrite(
                            "page label child ranges became unordered",
                        ));
                    }
                    bounds = Some((bounds.map_or(first, |(first, _)| first), last));
                    values.push(value);
                }
            }
            let Some(bounds) = bounds else {
                if !is_root {
                    return Ok(None);
                }
                dictionary.remove(b"Nums".as_slice());
                dictionary.remove(b"Kids".as_slice());
                dictionary.remove(b"Limits".as_slice());
                return write_page_label_tree(
                    parsed,
                    PageLabelTreeNode {
                        reference,
                        dictionary,
                        kind: PageLabelTreeNodeKind::Leaf(Vec::new()),
                    },
                    true,
                );
            };
            dictionary.remove(b"Nums".as_slice());
            dictionary.insert(b"Kids".to_vec(), Value::Array(values));
            refresh_page_label_tree_limits(&mut dictionary, bounds)?;
        }
    }

    let value = Value::Dict(dictionary);
    match reference {
        Some(reference) => {
            parsed.objects.insert(reference, plain_object(value));
            Ok(Some(Value::Ref(reference)))
        }
        None if is_root => Ok(Some(value)),
        None => Err(PdfError::unsafe_rewrite(
            "page label child nodes must be indirect",
        )),
    }
}

fn refresh_page_label_tree_limits(
    dictionary: &mut BTreeMap<Vec<u8>, Value>,
    (first, last): (usize, usize),
) -> Result<(), PdfError> {
    if dictionary.contains_key(b"Limits".as_slice()) {
        dictionary.insert(
            b"Limits".to_vec(),
            Value::Array(vec![
                Value::Integer(
                    i64::try_from(first)
                        .map_err(|_| PdfError::limit("page label index exceeds i64"))?,
                ),
                Value::Integer(
                    i64::try_from(last)
                        .map_err(|_| PdfError::limit("page label index exceeds i64"))?,
                ),
            ]),
        );
    }
    Ok(())
}

fn roman_page_label(mut number: usize) -> String {
    if number == 0 {
        return "0".into();
    }
    let mut label = String::new();
    for (value, symbol) in [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ] {
        while number >= value {
            label.push_str(symbol);
            number -= value;
        }
    }
    label
}

fn alpha_page_label(mut number: usize) -> String {
    if number == 0 {
        return "0".into();
    }
    let mut label = Vec::new();
    while number > 0 {
        number -= 1;
        label.push(b'A' + u8::try_from(number % 26).expect("alphabetic page label digit"));
        number /= 26;
    }
    label.reverse();
    String::from_utf8(label).expect("alphabetic page labels are ASCII")
}

impl PdfDocument {
    pub fn update_page_label(
        &self,
        update: PageLabelUpdate,
    ) -> Result<DocumentStructureOutcome, PdfError> {
        refuse_unsafe_interactive_edit(self)?;
        let page_count = self.page_count()?;
        if update.page_index >= page_count {
            return Err(PdfError::selection("page label page index is out of range"));
        }
        if let Some(spec) = update.spec.as_ref() {
            validate_page_label_spec(spec, &self.parsed().limits)?;
        }
        if page_label_tree_is_hierarchical(self.parsed())? {
            return self.update_hierarchical_page_label(update, page_count);
        }

        let mut entries = flat_page_label_entries(self.parsed())?;
        if entries.iter().any(|(index, _)| *index >= page_count) {
            return Err(PdfError::unsafe_rewrite(
                "page label tree contains an index outside the document",
            ));
        }
        let before = entries.len();
        let existing = entries
            .iter()
            .find(|(index, _)| *index == update.page_index)
            .map(|(_, value)| value.clone());
        let existed = existing.is_some();
        entries.retain(|(index, _)| *index != update.page_index);
        if let Some(spec) = update.spec.as_ref() {
            entries.push((
                update.page_index,
                page_label_spec_value(self.parsed(), existing.as_ref(), spec)?,
            ));
        }
        entries.sort_by_key(|(index, _)| *index);
        let desired = update.spec;
        let expected_count = match desired.as_ref() {
            Some(_) => before
                .checked_add(usize::from(!existed))
                .ok_or_else(|| PdfError::limit("page label entry count overflows"))?,
            None => before - usize::from(existed),
        };
        let page_index = update.page_index;
        let mut parsed = self.parsed().clone();
        install_flat_page_label_tree(&mut parsed, entries)?;
        finish_structure_mutation(
            self,
            parsed,
            "update_page_label",
            0,
            0,
            1,
            move |document| {
                let entries = flat_page_label_entries(document.parsed())?;
                let actual = entries.iter().find(|(index, _)| *index == page_index);
                let requested_state_matches = match (desired.as_ref(), actual) {
                    (Some(spec), Some((_, value))) => {
                        PageLabelSpec::parse(document.parsed(), value)? == *spec
                    }
                    (None, None) => true,
                    _ => false,
                };
                Ok(requested_state_matches && entries.len() == expected_count)
            },
        )
    }

    fn update_hierarchical_page_label(
        &self,
        update: PageLabelUpdate,
        page_count: usize,
    ) -> Result<DocumentStructureOutcome, PdfError> {
        let mut tree = load_page_label_tree(self.parsed())?.ok_or_else(|| {
            PdfError::unsafe_rewrite("hierarchical page label tree is missing from the catalog")
        })?;
        if !matches!(tree.kind, PageLabelTreeNodeKind::Internal(_)) {
            return Err(PdfError::unsafe_rewrite(
                "page label tree changed shape during mutation setup",
            ));
        }
        if tree.has_index_outside(page_count) {
            return Err(PdfError::unsafe_rewrite(
                "page label tree contains an index outside the document",
            ));
        }
        let existed = tree.entry_value(update.page_index).is_some();
        let mut expected_count = tree.entry_count();
        let desired = update.spec;
        if desired.is_some() || existed {
            mutate_page_label_tree(
                &mut tree,
                self.parsed(),
                update.page_index,
                desired.as_ref(),
                &mut expected_count,
            )?;
        }
        let (catalog_ref, source_catalog) = catalog(self.parsed())?;
        let mut parsed = self.parsed().clone();
        let root = write_page_label_tree(&mut parsed, tree, true)?.ok_or_else(|| {
            PdfError::unsafe_rewrite("hierarchical page label root was unexpectedly removed")
        })?;
        let mut catalog = source_catalog.clone();
        catalog.insert(b"PageLabels".to_vec(), root);
        parsed
            .objects
            .insert(catalog_ref, plain_object(Value::Dict(catalog)));
        let page_index = update.page_index;
        let entries_changed = usize::from(desired.is_some() || existed);
        finish_structure_mutation(
            self,
            parsed,
            "update_page_label",
            0,
            0,
            entries_changed,
            move |document| {
                let tree = load_page_label_tree(document.parsed())?.ok_or_else(|| {
                    PdfError::verification("page label tree disappeared after mutation")
                })?;
                let actual = tree.entry_value(page_index);
                let requested_state_matches = match (desired.as_ref(), actual) {
                    (Some(spec), Some(value)) => {
                        PageLabelSpec::parse(document.parsed(), value)? == *spec
                    }
                    (None, None) => true,
                    _ => false,
                };
                Ok(requested_state_matches && tree.entry_count() == expected_count)
            },
        )
    }

    pub fn update_document_info(
        &self,
        update: DocumentInfoUpdate,
    ) -> Result<DocumentStructureOutcome, PdfError> {
        refuse_unsafe_interactive_edit(self)?;
        validate_info_update(&update, &self.parsed().limits)?;
        if update.entries.is_empty() {
            return Err(PdfError::unsafe_rewrite(
                "document Info update has no entries",
            ));
        }
        let before = info_dictionary(self.parsed())?.cloned().unwrap_or_default();
        let mut after = before.clone();
        for (name, value) in &update.entries {
            match value {
                Some(value) if name == "Trapped" => {
                    after.insert(
                        name.as_bytes().to_vec(),
                        Value::Name(value.as_bytes().to_vec()),
                    );
                }
                Some(value) => {
                    after.insert(name.as_bytes().to_vec(), Value::String(encode_text(value)));
                }
                None => {
                    after.remove(name.as_bytes());
                }
            }
        }
        let mut parsed = self.parsed().clone();
        let (added, removed) = install_info_dictionary(&mut parsed, after)?;
        let bytes = write_structure(self, parsed)?;
        let rewritten = reopen(self, &bytes, "document Info")?;
        let requested_state_matches = info_update_matches(&rewritten, &update)?;
        let empty = BTreeMap::new();
        let rewritten_info = info_dictionary(rewritten.parsed())?.unwrap_or(&empty);
        let unknown_entries_preserved =
            unspecified_entries_preserved(&before, rewritten_info, &update);
        structure_outcome(
            self,
            &rewritten,
            bytes,
            DocumentStructureReport {
                operation: "update_document_info".into(),
                input_bytes: self.source_len(),
                output_bytes: 0,
                objects_added: added,
                objects_removed: removed,
                entries_changed: update.entries.len(),
            },
            requested_state_matches,
            true,
            unknown_entries_preserved,
        )
    }

    pub fn update_xmp_metadata(
        &self,
        update: XmpMetadataUpdate,
    ) -> Result<DocumentStructureOutcome, PdfError> {
        refuse_unsafe_interactive_edit(self)?;
        if let Some(xml) = &update.xml {
            validate_xmp(xml, &self.parsed().limits)?;
        }
        let before = read_xmp_metadata(self)?;
        let expected_xml = update.xml.clone();
        let (catalog_ref, catalog) = catalog(self.parsed())?;
        let mut parsed = self.parsed().clone();
        let mut new_catalog = catalog.clone();
        let mut added = 0;
        let mut removed = 0;
        match update.xml {
            Some(xml) => {
                let metadata_ref = match new_catalog.get(b"Metadata".as_slice()) {
                    Some(Value::Ref(reference)) => *reference,
                    Some(_) => {
                        return Err(PdfError::unsupported("catalog /Metadata must be indirect"));
                    }
                    None => {
                        added = 1;
                        allocate_reference(&parsed)?
                    }
                };
                let dict = match parsed.objects.get(&metadata_ref) {
                    Some(object) => {
                        let dict = dictionary(&object.value, "XMP metadata")?;
                        require_xmp_dictionary(dict)?;
                        if dict.contains_key(b"Filter".as_slice())
                            || dict.contains_key(b"DecodeParms".as_slice())
                        {
                            return Err(PdfError::unsupported(
                                "filtered XMP metadata streams are not supported",
                            ));
                        }
                        dict.clone()
                    }
                    None => BTreeMap::from([
                        (b"Type".to_vec(), Value::Name(b"Metadata".to_vec())),
                        (b"Subtype".to_vec(), Value::Name(b"XML".to_vec())),
                    ]),
                };
                parsed.objects.insert(
                    metadata_ref,
                    IndirectObject {
                        value: Value::Dict(dict),
                        stream: Some(xml),
                        stream_offset: 0,
                        offset: 0,
                    },
                );
                new_catalog.insert(b"Metadata".to_vec(), Value::Ref(metadata_ref));
            }
            None => {
                if let Some(Value::Ref(reference)) = new_catalog.remove(b"Metadata".as_slice())
                    && !is_referenced_by_objects(&parsed, reference, Some(catalog_ref))
                {
                    parsed.objects.remove(&reference);
                    removed = 1;
                }
            }
        }
        parsed
            .objects
            .insert(catalog_ref, plain_object(Value::Dict(new_catalog)));
        let bytes = write_structure(self, parsed)?;
        let rewritten = reopen(self, &bytes, "XMP metadata")?;
        let after = read_xmp_metadata(&rewritten)?;
        let exact_xml = match (&after, &expected_xml) {
            (Some(after), Some(expected)) => after.xml == *expected,
            (None, None) => true,
            _ => false,
        };
        structure_outcome(
            self,
            &rewritten,
            bytes,
            DocumentStructureReport {
                operation: "update_xmp_metadata".into(),
                input_bytes: self.source_len(),
                output_bytes: 0,
                objects_added: added,
                objects_removed: removed,
                entries_changed: usize::from(
                    before.as_ref().map(|value| &value.xml)
                        != after.as_ref().map(|value| &value.xml),
                ),
            },
            exact_xml,
            catalog_has_metadata(rewritten.parsed())? == after.is_some(),
            xmp_unknown_dictionary_preserved(self.parsed(), rewritten.parsed())?,
        )
    }
}

impl PdfDocument {
    pub fn create_outline(
        &self,
        request: OutlineCreateRequest,
    ) -> Result<DocumentStructureOutcome, PdfError> {
        self.create_outline_at(request, None)
    }

    pub fn create_child_outline(
        &self,
        parent_outline_index: usize,
        request: OutlineCreateRequest,
    ) -> Result<DocumentStructureOutcome, PdfError> {
        self.create_outline_at(request, Some(parent_outline_index))
    }

    fn create_outline_at(
        &self,
        request: OutlineCreateRequest,
        parent_outline_index: Option<usize>,
    ) -> Result<DocumentStructureOutcome, PdfError> {
        refuse_unsafe_interactive_edit(self)?;
        validate_simple_text(&request.title, "outline title", &self.parsed().limits)?;
        validate_simple_text(
            &request.destination_name,
            "outline destination",
            &self.parsed().limits,
        )?;
        if !read_named_destinations(self)?
            .iter()
            .any(|destination| destination.name == request.destination_name)
        {
            return Err(PdfError::selection(
                "outline destination name was not found",
            ));
        }
        let before = read_outlines(self)?;
        let expected_depth = parent_outline_index
            .and_then(|index| before.get(index))
            .map_or(0, |item| item.depth + 1);
        let selected_parent = parent_outline_index
            .map(|index| {
                before
                    .get(index)
                    .map(|item| ObjectRef {
                        number: item.object_number,
                        generation: item.object_generation,
                    })
                    .ok_or_else(|| PdfError::selection("parent outline index is out of range"))
            })
            .transpose()?;
        let (catalog_ref, catalog) = catalog(self.parsed())?;
        let mut parsed = self.parsed().clone();
        let mut catalog = catalog.clone();
        let (root_ref, root, root_added) = match catalog.get(b"Outlines".as_slice()) {
            Some(Value::Ref(reference)) => {
                let root = dictionary(&parsed.object(*reference)?.value, "outline root")?.clone();
                (*reference, root, 0)
            }
            Some(_) => return Err(PdfError::unsupported("catalog /Outlines must be indirect")),
            None => {
                let reference = allocate_reference(&parsed)?;
                (
                    reference,
                    BTreeMap::from([(b"Type".to_vec(), Value::Name(b"Outlines".to_vec()))]),
                    1,
                )
            }
        };
        let item_ref = allocate_references_after(&parsed, root_added + 1)?[root_added];
        parsed
            .objects
            .insert(root_ref, plain_object(Value::Dict(root)));
        let parent_ref = selected_parent.unwrap_or(root_ref);
        let mut parent = dictionary(&parsed.object(parent_ref)?.value, "outline parent")?.clone();
        let last = match parent.get(b"Last".as_slice()) {
            None => None,
            Some(Value::Ref(reference)) => Some(*reference),
            Some(_) => {
                return Err(PdfError::unsupported(
                    "outline parent /Last must be indirect",
                ));
            }
        };
        let mut item = BTreeMap::from([
            (
                b"Title".to_vec(),
                Value::String(encode_text(&request.title)),
            ),
            (b"Parent".to_vec(), Value::Ref(parent_ref)),
            (
                b"Dest".to_vec(),
                Value::String(encode_text(&request.destination_name)),
            ),
        ]);
        if let Some(last) = last {
            let mut previous = dictionary(&parsed.object(last)?.value, "outline item")?.clone();
            if previous.contains_key(b"Next".as_slice())
                || !matches!(previous.get(b"Parent".as_slice()), Some(Value::Ref(value)) if *value == parent_ref)
            {
                return Err(PdfError::unsafe_rewrite(
                    "outline parent /Last has invalid links",
                ));
            }
            previous.insert(b"Next".to_vec(), Value::Ref(item_ref));
            parsed
                .objects
                .insert(last, plain_object(Value::Dict(previous)));
            item.insert(b"Prev".to_vec(), Value::Ref(last));
        } else {
            if parent.contains_key(b"First".as_slice()) {
                return Err(PdfError::unsafe_rewrite(
                    "outline parent has /First without /Last",
                ));
            }
            parent.insert(b"First".to_vec(), Value::Ref(item_ref));
        }
        parent.insert(b"Last".to_vec(), Value::Ref(item_ref));
        parsed
            .objects
            .insert(parent_ref, plain_object(Value::Dict(parent)));
        parsed
            .objects
            .insert(item_ref, plain_object(Value::Dict(item)));
        adjust_outline_counts(&mut parsed, parent_ref, root_ref, 1)?;
        catalog.insert(b"Outlines".to_vec(), Value::Ref(root_ref));
        parsed
            .objects
            .insert(catalog_ref, plain_object(Value::Dict(catalog)));
        finish_structure_mutation(
            self,
            parsed,
            "create_outline",
            root_added + 1,
            0,
            1,
            |document| {
                Ok(read_outlines(document)?.len() == before.len() + 1
                    && read_outlines(document)?
                        .iter()
                        .any(|item| item.title == request.title && item.depth == expected_depth))
            },
        )
    }

    pub fn remove_outline(
        &self,
        request: OutlineRemoveRequest,
    ) -> Result<DocumentStructureOutcome, PdfError> {
        refuse_unsafe_interactive_edit(self)?;
        let outlines = read_outlines(self)?;
        let selected = outlines
            .get(request.outline_index)
            .ok_or_else(|| PdfError::selection("outline index is out of range"))?;
        let item_ref = ObjectRef {
            number: selected.object_number,
            generation: selected.object_generation,
        };
        let item = dictionary(&self.parsed().object(item_ref)?.value, "outline item")?;
        let Some(Value::Ref(parent_ref)) = item.get(b"Parent".as_slice()) else {
            return Err(PdfError::unsupported(
                "outline item /Parent must be indirect",
            ));
        };
        let previous = match item.get(b"Prev".as_slice()) {
            Some(Value::Ref(value)) => Some(*value),
            None => None,
            _ => return Err(PdfError::unsupported("outline /Prev must be indirect")),
        };
        let next = match item.get(b"Next".as_slice()) {
            Some(Value::Ref(value)) => Some(*value),
            None => None,
            _ => return Err(PdfError::unsupported("outline /Next must be indirect")),
        };
        let subtree_end = outlines
            .iter()
            .enumerate()
            .skip(request.outline_index + 1)
            .find_map(|(index, item)| (item.depth <= selected.depth).then_some(index))
            .unwrap_or(outlines.len());
        let subtree = &outlines[request.outline_index..subtree_end];
        let subtree_refs = subtree
            .iter()
            .map(|item| ObjectRef {
                number: item.object_number,
                generation: item.object_generation,
            })
            .collect::<BTreeSet<_>>();
        if subtree_refs.len() != subtree.len()
            || subtree_refs.contains(parent_ref)
            || previous.is_some_and(|reference| subtree_refs.contains(&reference))
            || next.is_some_and(|reference| subtree_refs.contains(&reference))
        {
            return Err(PdfError::unsafe_rewrite(
                "outline subtree has shared or invalid links",
            ));
        }
        for reference in &subtree_refs {
            match dictionary(&self.parsed().object(*reference)?.value, "outline item")?
                .get(b"Count".as_slice())
            {
                None => {}
                Some(Value::Integer(value)) if *value >= 0 => {}
                _ => {
                    return Err(PdfError::unsupported(
                        "outline subtree removal requires non-negative /Count values",
                    ));
                }
            }
        }
        let removed_count = i64::try_from(subtree.len())
            .map_err(|_| PdfError::limit("outline subtree count exceeds i64"))?;
        let mut parsed = self.parsed().clone();
        let mut parent = dictionary(&parsed.object(*parent_ref)?.value, "outline parent")?.clone();
        if let Some(previous) = previous {
            let mut dict = dictionary(&parsed.object(previous)?.value, "outline previous")?.clone();
            match next {
                Some(next) => {
                    dict.insert(b"Next".to_vec(), Value::Ref(next));
                }
                None => {
                    dict.remove(b"Next".as_slice());
                }
            }
            parsed
                .objects
                .insert(previous, plain_object(Value::Dict(dict)));
        } else {
            match next {
                Some(next) => {
                    parent.insert(b"First".to_vec(), Value::Ref(next));
                }
                None => {
                    parent.remove(b"First".as_slice());
                }
            }
        }
        if let Some(next) = next {
            let mut dict = dictionary(&parsed.object(next)?.value, "outline next")?.clone();
            match previous {
                Some(previous) => {
                    dict.insert(b"Prev".to_vec(), Value::Ref(previous));
                }
                None => {
                    dict.remove(b"Prev".as_slice());
                }
            }
            parsed.objects.insert(next, plain_object(Value::Dict(dict)));
        } else {
            match previous {
                Some(previous) => {
                    parent.insert(b"Last".to_vec(), Value::Ref(previous));
                }
                None => {
                    parent.remove(b"Last".as_slice());
                }
            }
        }
        parsed
            .objects
            .insert(*parent_ref, plain_object(Value::Dict(parent)));
        for reference in &subtree_refs {
            parsed.objects.remove(reference);
        }
        let (_, catalog) = catalog(&parsed)?;
        let Some(Value::Ref(outline_root)) = catalog.get(b"Outlines".as_slice()) else {
            return Err(PdfError::unsafe_rewrite("catalog outline root is invalid"));
        };
        let outline_root = *outline_root;
        adjust_outline_counts(&mut parsed, *parent_ref, outline_root, -removed_count)?;
        if !all_references_resolve(&parsed) {
            return Err(PdfError::unsafe_rewrite(
                "outline subtree has external references",
            ));
        }
        let expected_remaining = outlines.len() - subtree.len();
        finish_structure_mutation(
            self,
            parsed,
            "remove_outline",
            0,
            subtree.len(),
            subtree.len(),
            |document| Ok(read_outlines(document)?.len() == expected_remaining),
        )
    }

    pub fn update_named_destination(
        &self,
        update: NamedDestinationUpdate,
    ) -> Result<DocumentStructureOutcome, PdfError> {
        refuse_unsafe_interactive_edit(self)?;
        validate_simple_text(&update.name, "destination name", &self.parsed().limits)?;
        let page =
            match update.page_index {
                Some(page_index) => {
                    let pages = self.page_refs()?;
                    Some(*pages.get(page_index).ok_or_else(|| {
                        PdfError::selection("destination page index is out of range")
                    })?)
                }
                None => None,
            };
        if destination_name_tree_is_hierarchical(self.parsed())? {
            return self.update_hierarchical_named_destination(update, page);
        }
        let mut entries = flat_name_tree(self.parsed(), b"Dests")?;
        let before = entries.len();
        let existed = entries
            .iter()
            .any(|(name, _)| decode_text(name) == update.name);
        entries.retain(|(name, _)| decode_text(name) != update.name);
        if let Some(page) = page {
            entries.push((
                encode_text(&update.name),
                Value::Array(vec![Value::Ref(page), Value::Name(b"Fit".to_vec())]),
            ));
        }
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        let mut parsed = self.parsed().clone();
        install_flat_name_tree(&mut parsed, b"Dests", entries)?;
        finish_structure_mutation(
            self,
            parsed,
            "update_named_destination",
            0,
            0,
            1,
            |document| {
                let found = read_named_destinations(document)?
                    .iter()
                    .any(|value| value.name == update.name);
                let expected = if update.page_index.is_some() {
                    before + usize::from(!existed)
                } else {
                    before - usize::from(existed)
                };
                Ok(found == update.page_index.is_some()
                    && read_named_destinations(document)?.len() == expected)
            },
        )
    }

    fn update_hierarchical_named_destination(
        &self,
        update: NamedDestinationUpdate,
        page: Option<ObjectRef>,
    ) -> Result<DocumentStructureOutcome, PdfError> {
        let mut tree = load_destination_name_tree(self.parsed())?.ok_or_else(|| {
            PdfError::unsafe_rewrite("hierarchical destination tree is missing from the catalog")
        })?;
        if !matches!(tree.kind, DestinationNameTreeNodeKind::Internal(_)) {
            return Err(PdfError::unsafe_rewrite(
                "destination tree changed shape during mutation setup",
            ));
        }

        let mut matches = Vec::new();
        tree.matching_entries(&update.name, &mut matches);
        if matches.len() > 1 {
            return Err(PdfError::unsafe_rewrite(
                "destination tree contains multiple entries for the requested name",
            ));
        }
        let existing = matches.pop();
        let target = existing
            .as_ref()
            .map(|(name, _)| name.clone())
            .unwrap_or_else(|| encode_text(&update.name));
        let replacement = page
            .map(|page| {
                destination_name_tree_value(existing.as_ref().map(|(_, value)| value), page)
            })
            .transpose()?;
        let mut expected_count = tree.entry_count();
        if replacement.is_some() || existing.is_some() {
            mutate_destination_name_tree(
                &mut tree,
                &target,
                replacement,
                self.parsed().limits.max_container_items,
                &mut expected_count,
            )?;
        }

        let mut parsed = self.parsed().clone();
        let root = write_destination_name_tree(&mut parsed, tree, true)?.ok_or_else(|| {
            PdfError::unsafe_rewrite("hierarchical destination tree root was unexpectedly removed")
        })?;
        install_destination_name_tree_root(&mut parsed, root)?;
        let requested_name = update.name;
        let requested_page = update.page_index;
        let entries_changed = usize::from(requested_page.is_some() || existing.is_some());
        finish_structure_mutation(
            self,
            parsed,
            "update_named_destination",
            0,
            0,
            entries_changed,
            move |document| {
                let tree = load_destination_name_tree(document.parsed())?.ok_or_else(|| {
                    PdfError::verification("destination tree disappeared after mutation")
                })?;
                let matches = read_named_destinations(document)?
                    .into_iter()
                    .filter(|destination| destination.name == requested_name)
                    .collect::<Vec<_>>();
                let requested_state_matches = match requested_page {
                    Some(page_index) => matches.len() == 1 && matches[0].page_index == page_index,
                    None => matches.is_empty(),
                };
                Ok(requested_state_matches && tree.entry_count() == expected_count)
            },
        )
    }

    pub fn update_embedded_attachment(
        &self,
        update: EmbeddedAttachmentUpdate,
    ) -> Result<DocumentStructureOutcome, PdfError> {
        refuse_unsafe_interactive_edit(self)?;
        validate_simple_text(&update.name, "attachment name", &self.parsed().limits)?;
        if update
            .data
            .as_ref()
            .is_some_and(|data| data.len() > self.parsed().limits.max_stream_bytes)
        {
            return Err(PdfError::limit(
                "embedded attachment exceeds max_stream_bytes",
            ));
        }
        let desired_present = update.data.is_some();
        let mut entries = flat_name_tree(self.parsed(), b"EmbeddedFiles")?;
        let removed = entries
            .iter()
            .find(|(name, _)| decode_text(name) == update.name)
            .map(|(_, value)| value.clone());
        entries.retain(|(name, _)| decode_text(name) != update.name);
        let mut parsed = self.parsed().clone();
        let mut added = 0;
        if let Some(data) = update.data {
            let refs = allocate_references_after(&parsed, 2)?;
            let stream_ref = refs[0];
            let filespec_ref = refs[1];
            parsed.objects.insert(
                stream_ref,
                IndirectObject {
                    value: Value::Dict(BTreeMap::from([(
                        b"Type".to_vec(),
                        Value::Name(b"EmbeddedFile".to_vec()),
                    )])),
                    stream: Some(data),
                    stream_offset: 0,
                    offset: 0,
                },
            );
            parsed.objects.insert(
                filespec_ref,
                plain_object(Value::Dict(BTreeMap::from([
                    (b"Type".to_vec(), Value::Name(b"Filespec".to_vec())),
                    (b"F".to_vec(), Value::String(encode_text(&update.name))),
                    (b"UF".to_vec(), Value::String(encode_text(&update.name))),
                    (
                        b"EF".to_vec(),
                        Value::Dict(BTreeMap::from([(b"F".to_vec(), Value::Ref(stream_ref))])),
                    ),
                ]))),
            );
            entries.push((encode_text(&update.name), Value::Ref(filespec_ref)));
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            added = 2;
        }
        install_flat_name_tree(&mut parsed, b"EmbeddedFiles", entries)?;
        let mut removed_objects = 0;
        if let Some(Value::Ref(filespec)) = removed.as_ref()
            && !is_referenced_by_objects(&parsed, *filespec, None)
        {
            let dependencies = direct_references(&parsed.object(*filespec)?.value);
            parsed.objects.remove(filespec);
            removed_objects += 1;
            for dependency in dependencies {
                if !is_referenced_by_objects(&parsed, dependency, None) {
                    parsed.objects.remove(&dependency);
                    removed_objects += 1;
                }
            }
        }
        finish_structure_mutation(
            self,
            parsed,
            "update_embedded_attachment",
            added,
            removed_objects,
            1,
            |document| {
                Ok(read_embedded_attachments(document)?
                    .iter()
                    .any(|value| value.name == update.name)
                    == desired_present)
            },
        )
    }
}

fn walk_outline_chain(
    parsed: &ParsedDocument,
    mut current: ObjectRef,
    expected_parent: ObjectRef,
    depth: usize,
    seen: &mut BTreeSet<ObjectRef>,
    output: &mut Vec<OutlineItem>,
) -> Result<ObjectRef, PdfError> {
    if depth > parsed.limits.max_parser_depth {
        return Err(PdfError::limit("outline depth exceeds parser limit"));
    }
    let mut previous = None;
    loop {
        if !seen.insert(current) {
            return Err(PdfError::syntax("cycle in outline tree", 0));
        }
        if output.len() >= parsed.limits.max_container_items {
            return Err(PdfError::limit("outline count exceeds container limit"));
        }
        let dict = dictionary(&parsed.object(current)?.value, "outline item")?;
        if !matches!(dict.get(b"Parent".as_slice()), Some(Value::Ref(reference)) if *reference == expected_parent)
        {
            return Err(PdfError::syntax("outline item has an invalid /Parent", 0));
        }
        match (previous, dict.get(b"Prev".as_slice())) {
            (None, None) => {}
            (Some(expected), Some(Value::Ref(actual))) if expected == *actual => {}
            _ => return Err(PdfError::syntax("outline item has an invalid /Prev", 0)),
        }
        let title = match dict.get(b"Title".as_slice()) {
            Some(Value::String(value)) => decode_text(value),
            _ => return Err(PdfError::syntax("outline item has no string /Title", 0)),
        };
        let destination_name = match dict.get(b"Dest".as_slice()) {
            None => None,
            Some(Value::String(value)) => Some(decode_text(value)),
            Some(Value::Name(value)) => Some(String::from_utf8_lossy(value).into_owned()),
            Some(_) => None,
        };
        output.push(OutlineItem {
            index: output.len(),
            depth,
            title,
            destination_name,
            object_number: current.number,
            object_generation: current.generation,
        });
        match (dict.get(b"First".as_slice()), dict.get(b"Last".as_slice())) {
            (Some(Value::Ref(first)), Some(Value::Ref(last))) => {
                let actual_last =
                    walk_outline_chain(parsed, *first, current, depth + 1, seen, output)?;
                if actual_last != *last {
                    return Err(PdfError::syntax("outline item has an invalid /Last", 0));
                }
            }
            (None, None) => {}
            _ => {
                return Err(PdfError::syntax(
                    "outline item /First and /Last must be indirect and paired",
                    0,
                ));
            }
        }
        let next = match dict.get(b"Next".as_slice()) {
            None => return Ok(current),
            Some(Value::Ref(reference)) => *reference,
            Some(_) => return Err(PdfError::syntax("outline item /Next must be indirect", 0)),
        };
        previous = Some(current);
        current = next;
    }
}

fn adjust_outline_counts(
    parsed: &mut ParsedDocument,
    mut current: ObjectRef,
    root: ObjectRef,
    delta: i64,
) -> Result<(), PdfError> {
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(current) || seen.len() > parsed.limits.max_parser_depth {
            return Err(PdfError::unsafe_rewrite("cycle in outline parent chain"));
        }
        let mut dict = dictionary(&parsed.object(current)?.value, "outline parent")?.clone();
        let count = match dict.get(b"Count".as_slice()) {
            None if delta > 0 => 0,
            Some(Value::Integer(value)) if *value >= 0 => *value,
            _ => {
                return Err(PdfError::unsupported(
                    "outline mutation requires non-negative /Count values",
                ));
            }
        };
        let count = count
            .checked_add(delta)
            .filter(|value| *value >= 0)
            .ok_or_else(|| PdfError::unsafe_rewrite("outline /Count is inconsistent"))?;
        dict.insert(b"Count".to_vec(), Value::Integer(count));
        let parent = match dict.get(b"Parent".as_slice()) {
            Some(Value::Ref(parent)) => Some(*parent),
            None if current == root => None,
            _ => return Err(PdfError::unsafe_rewrite("outline parent chain is invalid")),
        };
        parsed
            .objects
            .insert(current, plain_object(Value::Dict(dict)));
        if current == root {
            return Ok(());
        }
        current = parent.ok_or_else(|| PdfError::unsafe_rewrite("outline root was not reached"))?;
    }
}

fn resolve_destination<'a>(
    parsed: &'a ParsedDocument,
    mut value: &'a Value,
) -> Result<&'a [Value], PdfError> {
    let mut seen = BTreeSet::new();
    while let Value::Ref(reference) = value {
        if !seen.insert(*reference) || seen.len() > parsed.limits.max_parser_depth {
            return Err(PdfError::syntax("cycle resolving named destination", 0));
        }
        value = &parsed.object(*reference)?.value;
    }
    match value {
        Value::Array(values) => Ok(values),
        Value::Dict(values) => match values.get(b"D".as_slice()) {
            Some(Value::Array(values)) => Ok(values),
            _ => Err(PdfError::unsupported(
                "destination dictionary must contain a direct /D array",
            )),
        },
        _ => Err(PdfError::unsupported(
            "named destination value must be an array or destination dictionary",
        )),
    }
}

type NameTreeBounds = (Vec<u8>, Vec<u8>);

fn name_tree_entries(
    parsed: &ParsedDocument,
    key: &[u8],
) -> Result<Vec<(Vec<u8>, Value)>, PdfError> {
    let Some((tree, reference)) = name_tree_root(parsed, key)? else {
        return Ok(Vec::new());
    };
    let mut seen = BTreeSet::new();
    if let Some(reference) = reference {
        seen.insert(reference);
    }
    let mut nodes = 0;
    let mut entries = Vec::new();
    walk_name_tree(parsed, tree, 0, &mut seen, &mut nodes, &mut entries)?;
    Ok(entries)
}

fn flat_name_tree(parsed: &ParsedDocument, key: &[u8]) -> Result<Vec<(Vec<u8>, Value)>, PdfError> {
    if let Some((tree, _)) = name_tree_root(parsed, key)?
        && tree.contains_key(b"Kids".as_slice())
    {
        return Err(PdfError::unsupported(
            "name-tree mutation requires a flat tree",
        ));
    }
    name_tree_entries(parsed, key)
}

fn name_tree_root<'a>(
    parsed: &'a ParsedDocument,
    key: &[u8],
) -> Result<Option<ResolvedDictionary<'a>>, PdfError> {
    let (_, catalog) = catalog(parsed)?;
    let Some(names_value) = catalog.get(b"Names".as_slice()) else {
        return Ok(None);
    };
    let (names, _) = resolve_dict(parsed, names_value, "catalog /Names")?;
    let Some(tree_value) = names.get(key) else {
        return Ok(None);
    };
    Ok(Some(resolve_dict(parsed, tree_value, "name tree")?))
}

fn walk_name_tree(
    parsed: &ParsedDocument,
    tree: &BTreeMap<Vec<u8>, Value>,
    depth: usize,
    seen: &mut BTreeSet<ObjectRef>,
    nodes: &mut usize,
    entries: &mut Vec<(Vec<u8>, Value)>,
) -> Result<Option<NameTreeBounds>, PdfError> {
    if depth > parsed.limits.max_parser_depth {
        return Err(PdfError::limit("name tree depth exceeds parser limit"));
    }
    *nodes = nodes
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("name tree node count overflows"))?;
    if *nodes > parsed.limits.max_container_items {
        return Err(PdfError::limit(
            "name tree node count exceeds container limit",
        ));
    }

    let limits = name_tree_limits(tree)?;
    match (tree.get(b"Names".as_slice()), tree.get(b"Kids".as_slice())) {
        (Some(_), Some(_)) => Err(PdfError::syntax(
            "name tree node cannot contain both /Names and /Kids",
            0,
        )),
        (Some(Value::Array(values)), None) => {
            if !values.len().is_multiple_of(2) {
                return Err(PdfError::syntax("name tree /Names must contain pairs", 0));
            }
            if values.len() / 2 > parsed.limits.max_container_items {
                return Err(PdfError::limit("name tree leaf exceeds container limit"));
            }

            let mut bounds = None;
            for pair in values.chunks_exact(2) {
                let Value::String(name) = &pair[0] else {
                    return Err(PdfError::syntax("name tree keys must be strings", 0));
                };
                if let Some((_, previous)) = bounds.as_ref()
                    && previous >= name
                {
                    return Err(PdfError::syntax(
                        "name tree keys are not strictly ordered",
                        0,
                    ));
                }
                if entries.len() >= parsed.limits.max_container_items {
                    return Err(PdfError::limit(
                        "name tree entry count exceeds container limit",
                    ));
                }
                entries.push((name.clone(), pair[1].clone()));
                match &mut bounds {
                    Some((_, last)) => *last = name.clone(),
                    None => bounds = Some((name.clone(), name.clone())),
                }
            }
            validate_name_tree_limits(limits.as_ref(), bounds.as_ref())?;
            Ok(bounds)
        }
        (Some(_), None) => Err(PdfError::syntax(
            "name tree /Names must be a direct array",
            0,
        )),
        (None, Some(Value::Array(kids))) => {
            if kids.len() > parsed.limits.max_container_items {
                return Err(PdfError::limit("name tree /Kids exceeds container limit"));
            }
            let mut bounds: Option<NameTreeBounds> = None;
            for child in kids {
                let Value::Ref(reference) = child else {
                    return Err(PdfError::syntax(
                        "name tree /Kids must contain indirect dictionaries",
                        0,
                    ));
                };
                if !seen.insert(*reference) {
                    return Err(PdfError::syntax("cycle in name tree", 0));
                }
                let child = dictionary(&parsed.object(*reference)?.value, "name tree child")?;
                let child_bounds = walk_name_tree(parsed, child, depth + 1, seen, nodes, entries)?
                    .ok_or_else(|| PdfError::syntax("name tree /Kids contains an empty node", 0))?;
                if let Some((_, previous)) = bounds.as_ref()
                    && previous >= &child_bounds.0
                {
                    return Err(PdfError::syntax(
                        "name tree child ranges are not strictly ordered",
                        0,
                    ));
                }
                match &mut bounds {
                    Some((_, last)) => *last = child_bounds.1,
                    None => bounds = Some(child_bounds),
                }
            }
            validate_name_tree_limits(limits.as_ref(), bounds.as_ref())?;
            Ok(bounds)
        }
        (None, Some(_)) => Err(PdfError::syntax(
            "name tree /Kids must be a direct array",
            0,
        )),
        (None, None) => {
            validate_name_tree_limits(limits.as_ref(), None)?;
            Ok(None)
        }
    }
}

fn name_tree_limits(tree: &BTreeMap<Vec<u8>, Value>) -> Result<Option<NameTreeBounds>, PdfError> {
    let Some(value) = tree.get(b"Limits".as_slice()) else {
        return Ok(None);
    };
    let Value::Array(values) = value else {
        return Err(PdfError::syntax(
            "name tree /Limits must be a direct array",
            0,
        ));
    };
    let [Value::String(first), Value::String(last)] = values.as_slice() else {
        return Err(PdfError::syntax(
            "name tree /Limits must contain two strings",
            0,
        ));
    };
    if first > last {
        return Err(PdfError::syntax("name tree /Limits are not ordered", 0));
    }
    Ok(Some((first.clone(), last.clone())))
}

fn validate_name_tree_limits(
    declared: Option<&NameTreeBounds>,
    actual: Option<&NameTreeBounds>,
) -> Result<(), PdfError> {
    match (declared, actual) {
        (None, _) => Ok(()),
        (Some(_), None) => Err(PdfError::syntax(
            "empty name tree node must not have /Limits",
            0,
        )),
        (Some((first, last)), Some((actual_first, actual_last)))
            if first == actual_first && last == actual_last =>
        {
            Ok(())
        }
        _ => Err(PdfError::syntax(
            "name tree /Limits do not match entries",
            0,
        )),
    }
}

impl DestinationNameTreeNode {
    fn bounds(&self) -> Option<NameTreeBounds> {
        match &self.kind {
            DestinationNameTreeNodeKind::Leaf(entries) => {
                Some((entries.first()?.0.clone(), entries.last()?.0.clone()))
            }
            DestinationNameTreeNodeKind::Internal(children) => {
                Some((children.first()?.bounds()?.0, children.last()?.bounds()?.1))
            }
        }
    }

    fn entry_count(&self) -> usize {
        match &self.kind {
            DestinationNameTreeNodeKind::Leaf(entries) => entries.len(),
            DestinationNameTreeNodeKind::Internal(children) => {
                children.iter().map(Self::entry_count).sum()
            }
        }
    }

    fn matching_entries(&self, requested_name: &str, output: &mut Vec<(Vec<u8>, Value)>) {
        match &self.kind {
            DestinationNameTreeNodeKind::Leaf(entries) => output.extend(
                entries
                    .iter()
                    .filter(|(name, _)| decode_text(name) == requested_name)
                    .cloned(),
            ),
            DestinationNameTreeNodeKind::Internal(children) => children
                .iter()
                .for_each(|child| child.matching_entries(requested_name, output)),
        }
    }
}

fn destination_name_tree_root_value(parsed: &ParsedDocument) -> Result<Option<&Value>, PdfError> {
    let (_, catalog) = catalog(parsed)?;
    let Some(names_value) = catalog.get(b"Names".as_slice()) else {
        return Ok(None);
    };
    let names = match names_value {
        Value::Dict(names) => names,
        Value::Ref(reference) => {
            let object = parsed.object(*reference)?;
            if object.stream.is_some() {
                return Err(PdfError::unsupported(
                    "catalog /Names must not be a stream for destination tree mutation",
                ));
            }
            dictionary(&object.value, "catalog /Names")?
        }
        _ => {
            return Err(PdfError::unsupported(
                "catalog /Names must be a direct or indirect dictionary for destination tree mutation",
            ));
        }
    };
    Ok(names.get(b"Dests".as_slice()))
}

fn destination_name_tree_is_hierarchical(parsed: &ParsedDocument) -> Result<bool, PdfError> {
    let Some(value) = destination_name_tree_root_value(parsed)? else {
        return Ok(false);
    };
    let dictionary = match value {
        Value::Dict(dictionary) => dictionary,
        Value::Ref(reference) => {
            let object = parsed.object(*reference)?;
            if object.stream.is_some() {
                return Err(PdfError::unsupported(
                    "destination tree nodes must not be streams",
                ));
            }
            dictionary(&object.value, "destination tree root")?
        }
        _ => {
            return Err(PdfError::unsupported(
                "destination tree root must be a direct or indirect dictionary",
            ));
        }
    };
    Ok(dictionary.contains_key(b"Kids".as_slice()))
}

fn load_destination_name_tree(
    parsed: &ParsedDocument,
) -> Result<Option<DestinationNameTreeNode>, PdfError> {
    let Some(value) = destination_name_tree_root_value(parsed)? else {
        return Ok(None);
    };
    let mut seen = BTreeSet::new();
    let mut nodes = 0;
    let mut entries = 0;
    load_destination_name_tree_node(parsed, value, true, 0, &mut seen, &mut nodes, &mut entries)
        .map(Some)
}

fn load_destination_name_tree_node(
    parsed: &ParsedDocument,
    value: &Value,
    allow_direct: bool,
    depth: usize,
    seen: &mut BTreeSet<ObjectRef>,
    nodes: &mut usize,
    entries: &mut usize,
) -> Result<DestinationNameTreeNode, PdfError> {
    if depth > parsed.limits.max_parser_depth {
        return Err(PdfError::limit(
            "destination tree depth exceeds parser limit",
        ));
    }
    *nodes = nodes
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("destination tree node count overflows"))?;
    if *nodes > parsed.limits.max_container_items {
        return Err(PdfError::limit(
            "destination tree node count exceeds container limit",
        ));
    }

    let (reference, dictionary) = match value {
        Value::Dict(dictionary) if allow_direct => (None, dictionary.clone()),
        Value::Ref(reference) => {
            if !seen.insert(*reference) {
                return Err(PdfError::syntax("cycle in destination tree", 0));
            }
            let object = parsed.object(*reference)?;
            if object.stream.is_some() {
                return Err(PdfError::unsupported(
                    "destination tree nodes must not be streams",
                ));
            }
            (
                Some(*reference),
                dictionary(&object.value, "destination tree node")?.clone(),
            )
        }
        _ => {
            return Err(PdfError::syntax(
                "destination tree nodes must be direct root dictionaries or indirect dictionaries",
                0,
            ));
        }
    };
    let limits = name_tree_limits(&dictionary)?;
    let kind = match (
        dictionary.get(b"Names".as_slice()),
        dictionary.get(b"Kids".as_slice()),
    ) {
        (Some(_), Some(_)) => {
            return Err(PdfError::syntax(
                "destination tree node cannot contain both /Names and /Kids",
                0,
            ));
        }
        (Some(Value::Array(values)), None) => {
            if !values.len().is_multiple_of(2) {
                return Err(PdfError::syntax(
                    "destination tree /Names must contain pairs",
                    0,
                ));
            }
            if values.len() / 2 > parsed.limits.max_container_items {
                return Err(PdfError::limit(
                    "destination tree leaf exceeds container limit",
                ));
            }
            let mut previous = None;
            let mut output = Vec::with_capacity(values.len() / 2);
            for pair in values.chunks_exact(2) {
                let Value::String(name) = &pair[0] else {
                    return Err(PdfError::syntax("destination tree keys must be strings", 0));
                };
                if previous
                    .as_ref()
                    .is_some_and(|previous: &Vec<u8>| previous >= name)
                {
                    return Err(PdfError::syntax(
                        "destination tree keys are not strictly ordered",
                        0,
                    ));
                }
                *entries = entries
                    .checked_add(1)
                    .ok_or_else(|| PdfError::limit("destination tree entry count overflows"))?;
                if *entries > parsed.limits.max_container_items {
                    return Err(PdfError::limit(
                        "destination tree entry count exceeds limit",
                    ));
                }
                output.push((name.clone(), pair[1].clone()));
                previous = Some(name.clone());
            }
            DestinationNameTreeNodeKind::Leaf(output)
        }
        (Some(_), None) => {
            return Err(PdfError::syntax(
                "destination tree /Names must be a direct array",
                0,
            ));
        }
        (None, Some(Value::Array(kids))) => {
            if kids.len() > parsed.limits.max_container_items {
                return Err(PdfError::limit(
                    "destination tree /Kids exceeds container limit",
                ));
            }
            let mut children = Vec::with_capacity(kids.len());
            let mut previous = None;
            for child in kids {
                let Value::Ref(reference) = child else {
                    return Err(PdfError::syntax(
                        "destination tree /Kids must contain indirect dictionaries",
                        0,
                    ));
                };
                let child = load_destination_name_tree_node(
                    parsed,
                    &Value::Ref(*reference),
                    false,
                    depth + 1,
                    seen,
                    nodes,
                    entries,
                )?;
                let (first, last) = child.bounds().ok_or_else(|| {
                    PdfError::syntax("destination tree /Kids contains an empty node", 0)
                })?;
                if previous
                    .as_ref()
                    .is_some_and(|previous: &Vec<u8>| previous >= &first)
                {
                    return Err(PdfError::syntax(
                        "destination tree child ranges are not strictly ordered",
                        0,
                    ));
                }
                previous = Some(last);
                children.push(child);
            }
            DestinationNameTreeNodeKind::Internal(children)
        }
        (None, Some(_)) => {
            return Err(PdfError::syntax(
                "destination tree /Kids must be a direct array",
                0,
            ));
        }
        (None, None) => DestinationNameTreeNodeKind::Leaf(Vec::new()),
    };
    let node = DestinationNameTreeNode {
        reference,
        dictionary,
        kind,
    };
    let bounds = node.bounds();
    validate_name_tree_limits(limits.as_ref(), bounds.as_ref())?;
    Ok(node)
}

fn destination_name_tree_value(
    existing: Option<&Value>,
    page: ObjectRef,
) -> Result<Value, PdfError> {
    let destination = Value::Array(vec![Value::Ref(page), Value::Name(b"Fit".to_vec())]);
    match existing {
        None | Some(Value::Array(_)) => Ok(destination),
        Some(Value::Dict(dictionary)) => {
            let mut dictionary = dictionary.clone();
            dictionary.insert(b"D".to_vec(), destination);
            Ok(Value::Dict(dictionary))
        }
        Some(Value::Ref(_)) => Err(PdfError::unsupported(
            "updating indirect destination values requires rebuilding the destination tree",
        )),
        Some(_) => Err(PdfError::unsupported(
            "named destination values must be direct arrays or destination dictionaries",
        )),
    }
}

fn mutate_destination_name_tree(
    node: &mut DestinationNameTreeNode,
    target: &[u8],
    replacement: Option<Value>,
    max_entries: usize,
    total_entries: &mut usize,
) -> Result<(), PdfError> {
    match &mut node.kind {
        DestinationNameTreeNodeKind::Leaf(entries) => {
            let position = entries
                .iter()
                .position(|(name, _)| name.as_slice() == target);
            match replacement {
                Some(value) => match position {
                    Some(position) => entries[position].1 = value,
                    None => {
                        if *total_entries >= max_entries {
                            return Err(PdfError::limit(
                                "destination tree entry count exceeds limit",
                            ));
                        }
                        entries.push((target.to_vec(), value));
                        entries.sort_by(|left, right| left.0.cmp(&right.0));
                        *total_entries += 1;
                    }
                },
                None => {
                    let position = position.ok_or_else(|| {
                        PdfError::unsafe_rewrite(
                            "destination tree selection did not reach the requested entry",
                        )
                    })?;
                    entries.remove(position);
                    *total_entries -= 1;
                }
            }
            Ok(())
        }
        DestinationNameTreeNodeKind::Internal(children) => {
            let child = destination_name_tree_child_index(children, target)?;
            mutate_destination_name_tree(
                &mut children[child],
                target,
                replacement,
                max_entries,
                total_entries,
            )
        }
    }
}

fn destination_name_tree_child_index(
    children: &[DestinationNameTreeNode],
    target: &[u8],
) -> Result<usize, PdfError> {
    let Some(first) = children.first() else {
        return Err(PdfError::unsupported(
            "empty hierarchical destination trees require rebuilding",
        ));
    };
    let (first_name, _) = first
        .bounds()
        .ok_or_else(|| PdfError::unsafe_rewrite("destination tree child has no bounds"))?;
    if target < first_name.as_slice() {
        return Ok(0);
    }
    for (index, child) in children.iter().enumerate() {
        let (first_name, last_name) = child
            .bounds()
            .ok_or_else(|| PdfError::unsafe_rewrite("destination tree child has no bounds"))?;
        if target >= first_name.as_slice() && target <= last_name.as_slice() {
            return Ok(index);
        }
        if target < first_name.as_slice() {
            return Err(PdfError::unsupported(
                "destination insertion between child ranges requires rebalancing",
            ));
        }
    }
    Ok(children.len() - 1)
}

fn write_destination_name_tree(
    parsed: &mut ParsedDocument,
    node: DestinationNameTreeNode,
    is_root: bool,
) -> Result<Option<Value>, PdfError> {
    let DestinationNameTreeNode {
        reference,
        mut dictionary,
        kind,
    } = node;
    match kind {
        DestinationNameTreeNodeKind::Leaf(entries) if entries.is_empty() => {
            if !is_root {
                return Ok(None);
            }
            dictionary.remove(b"Names".as_slice());
            dictionary.remove(b"Kids".as_slice());
            dictionary.remove(b"Limits".as_slice());
        }
        DestinationNameTreeNodeKind::Leaf(entries) => {
            let (first, last) = match (entries.first(), entries.last()) {
                (Some((first, _)), Some((last, _))) => (first.clone(), last.clone()),
                _ => {
                    return Err(PdfError::unsafe_rewrite(
                        "non-empty destination tree leaf has no bounds",
                    ));
                }
            };
            let mut values =
                Vec::with_capacity(entries.len().checked_mul(2).ok_or_else(|| {
                    PdfError::limit("destination tree /Names capacity overflows")
                })?);
            for (name, value) in entries {
                values.push(Value::String(name));
                values.push(value);
            }
            dictionary.remove(b"Kids".as_slice());
            dictionary.insert(b"Names".to_vec(), Value::Array(values));
            refresh_destination_name_tree_limits(&mut dictionary, (first, last));
        }
        DestinationNameTreeNodeKind::Internal(children) => {
            let mut values = Vec::with_capacity(children.len());
            let mut bounds = None;
            for child in children {
                let child_bounds = child.bounds();
                if let Some(value) = write_destination_name_tree(parsed, child, false)? {
                    let (first, last) = child_bounds.ok_or_else(|| {
                        PdfError::unsafe_rewrite("written destination tree child has no bounds")
                    })?;
                    if let Some((_, previous)) = bounds.as_ref()
                        && previous >= &first
                    {
                        return Err(PdfError::unsafe_rewrite(
                            "destination tree child ranges became unordered",
                        ));
                    }
                    let root_first = bounds.map_or(first.clone(), |(first, _)| first);
                    bounds = Some((root_first, last));
                    values.push(value);
                }
            }
            let Some(bounds) = bounds else {
                if !is_root {
                    return Ok(None);
                }
                dictionary.remove(b"Names".as_slice());
                dictionary.remove(b"Kids".as_slice());
                dictionary.remove(b"Limits".as_slice());
                return write_destination_name_tree(
                    parsed,
                    DestinationNameTreeNode {
                        reference,
                        dictionary,
                        kind: DestinationNameTreeNodeKind::Leaf(Vec::new()),
                    },
                    true,
                );
            };
            dictionary.remove(b"Names".as_slice());
            dictionary.insert(b"Kids".to_vec(), Value::Array(values));
            refresh_destination_name_tree_limits(&mut dictionary, bounds);
        }
    }

    let value = Value::Dict(dictionary);
    match reference {
        Some(reference) => {
            parsed.objects.insert(reference, plain_object(value));
            Ok(Some(Value::Ref(reference)))
        }
        None if is_root => Ok(Some(value)),
        None => Err(PdfError::unsafe_rewrite(
            "destination tree child nodes must be indirect",
        )),
    }
}

fn refresh_destination_name_tree_limits(
    dictionary: &mut BTreeMap<Vec<u8>, Value>,
    (first, last): NameTreeBounds,
) {
    if dictionary.contains_key(b"Limits".as_slice()) {
        dictionary.insert(
            b"Limits".to_vec(),
            Value::Array(vec![Value::String(first), Value::String(last)]),
        );
    }
}

fn install_destination_name_tree_root(
    parsed: &mut ParsedDocument,
    root: Value,
) -> Result<(), PdfError> {
    let (catalog_ref, source_catalog) = catalog(parsed)?;
    let mut catalog = source_catalog.clone();
    let names_value = catalog
        .get(b"Names".as_slice())
        .cloned()
        .ok_or_else(|| PdfError::unsafe_rewrite("catalog /Names disappeared during mutation"))?;
    match names_value {
        Value::Dict(mut names) => {
            names.insert(b"Dests".to_vec(), root);
            catalog.insert(b"Names".to_vec(), Value::Dict(names));
        }
        Value::Ref(reference) => {
            let object = parsed.object(reference)?;
            if object.stream.is_some() {
                return Err(PdfError::unsupported(
                    "catalog /Names must not be a stream for destination tree mutation",
                ));
            }
            let mut names = dictionary(&object.value, "catalog /Names")?.clone();
            names.insert(b"Dests".to_vec(), root);
            parsed
                .objects
                .insert(reference, plain_object(Value::Dict(names)));
        }
        _ => {
            return Err(PdfError::unsupported(
                "catalog /Names must be a direct or indirect dictionary for destination tree mutation",
            ));
        }
    }
    parsed
        .objects
        .insert(catalog_ref, plain_object(Value::Dict(catalog)));
    Ok(())
}

fn install_flat_name_tree(
    parsed: &mut ParsedDocument,
    key: &[u8],
    entries: Vec<(Vec<u8>, Value)>,
) -> Result<(), PdfError> {
    let (catalog_ref, source_catalog) = catalog(parsed)?;
    let mut catalog = source_catalog.clone();
    let names_value = catalog.get(b"Names".as_slice()).cloned();
    let (names_ref, mut names) = match names_value {
        None => (None, BTreeMap::new()),
        Some(Value::Ref(reference)) => (
            Some(reference),
            dictionary(&parsed.object(reference)?.value, "catalog /Names")?.clone(),
        ),
        Some(Value::Dict(values)) => (None, values),
        Some(_) => {
            return Err(PdfError::unsupported(
                "catalog /Names must be a dictionary or reference",
            ));
        }
    };
    let existing_tree = names.get(key).cloned();
    if entries.is_empty() {
        if let Some(Value::Ref(reference)) = names.remove(key)
            && !is_referenced_by_objects(parsed, reference, names_ref.or(Some(catalog_ref)))
        {
            parsed.objects.remove(&reference);
        } else {
            names.remove(key);
        }
    } else {
        let (first, last) = match (entries.first(), entries.last()) {
            (Some((first, _)), Some((last, _))) => (first, last),
            _ => {
                return Err(PdfError::unsafe_rewrite(
                    "name tree entries are unexpectedly empty",
                ));
            }
        };
        let replacement_limits = Value::Array(vec![
            Value::String(first.clone()),
            Value::String(last.clone()),
        ]);
        let mut values = Vec::with_capacity(entries.len() * 2);
        for (name, value) in entries {
            values.push(Value::String(name));
            values.push(value);
        }
        match existing_tree {
            Some(Value::Ref(reference)) => {
                let mut tree = dictionary(&parsed.object(reference)?.value, "name tree")?.clone();
                if tree.contains_key(b"Kids".as_slice()) {
                    return Err(PdfError::unsupported(
                        "hierarchical name trees are not supported",
                    ));
                }
                tree.insert(b"Names".to_vec(), Value::Array(values));
                if tree.contains_key(b"Limits".as_slice()) {
                    tree.insert(b"Limits".to_vec(), replacement_limits.clone());
                }
                parsed
                    .objects
                    .insert(reference, plain_object(Value::Dict(tree)));
            }
            Some(Value::Dict(mut tree)) => {
                if tree.contains_key(b"Kids".as_slice()) {
                    return Err(PdfError::unsupported(
                        "hierarchical name trees are not supported",
                    ));
                }
                tree.insert(b"Names".to_vec(), Value::Array(values));
                if tree.contains_key(b"Limits".as_slice()) {
                    tree.insert(b"Limits".to_vec(), replacement_limits);
                }
                names.insert(key.to_vec(), Value::Dict(tree));
            }
            Some(_) => {
                return Err(PdfError::unsupported(
                    "name tree must be a dictionary or reference",
                ));
            }
            None => {
                names.insert(
                    key.to_vec(),
                    Value::Dict(BTreeMap::from([(b"Names".to_vec(), Value::Array(values))])),
                );
            }
        }
    }
    if let Some(reference) = names_ref {
        parsed
            .objects
            .insert(reference, plain_object(Value::Dict(names)));
    } else if names.is_empty() {
        catalog.remove(b"Names".as_slice());
    } else {
        catalog.insert(b"Names".to_vec(), Value::Dict(names));
    }
    parsed
        .objects
        .insert(catalog_ref, plain_object(Value::Dict(catalog)));
    Ok(())
}

fn validate_simple_text(value: &str, label: &str, limits: &crate::Limits) -> Result<(), PdfError> {
    if value.is_empty() || value.len() > limits.max_token_bytes {
        Err(PdfError::unsafe_rewrite(format!(
            "{label} is empty or exceeds limits"
        )))
    } else {
        Ok(())
    }
}

fn allocate_references_after(
    parsed: &ParsedDocument,
    count: usize,
) -> Result<Vec<ObjectRef>, PdfError> {
    if parsed
        .objects
        .len()
        .checked_add(count)
        .is_none_or(|value| value > parsed.limits.max_objects)
    {
        return Err(PdfError::limit(
            "document structure object allocation exceeds limits",
        ));
    }
    let mut number = parsed
        .objects
        .keys()
        .map(|reference| reference.number)
        .max()
        .unwrap_or(0);
    let mut output = Vec::with_capacity(count);
    for _ in 0..count {
        number = number
            .checked_add(1)
            .ok_or_else(|| PdfError::limit("object number overflows"))?;
        if usize::try_from(number)
            .ok()
            .and_then(|value| value.checked_add(1))
            .is_none_or(|value| value > parsed.limits.max_xref_entries)
        {
            return Err(PdfError::limit(
                "document structure allocation exceeds xref limit",
            ));
        }
        output.push(ObjectRef {
            number,
            generation: 0,
        });
    }
    Ok(output)
}

fn direct_references(value: &Value) -> BTreeSet<ObjectRef> {
    fn collect(value: &Value, output: &mut BTreeSet<ObjectRef>) {
        match value {
            Value::Ref(reference) => {
                output.insert(*reference);
            }
            Value::Array(values) => values.iter().for_each(|value| collect(value, output)),
            Value::Dict(values) => values.values().for_each(|value| collect(value, output)),
            _ => {}
        }
    }
    let mut output = BTreeSet::new();
    collect(value, &mut output);
    output
}

fn finish_structure_mutation(
    document: &PdfDocument,
    parsed: ParsedDocument,
    operation: &str,
    objects_added: usize,
    objects_removed: usize,
    entries_changed: usize,
    verify: impl FnOnce(&PdfDocument) -> Result<bool, PdfError>,
) -> Result<DocumentStructureOutcome, PdfError> {
    let bytes = write_structure(document, parsed)?;
    let rewritten = reopen(document, &bytes, operation)?;
    let requested = verify(&rewritten)?;
    structure_outcome(
        document,
        &rewritten,
        bytes,
        DocumentStructureReport {
            operation: operation.into(),
            input_bytes: document.source_len(),
            output_bytes: 0,
            objects_added,
            objects_removed,
            entries_changed,
        },
        requested,
        true,
        true,
    )
}

fn validate_info_update(
    update: &DocumentInfoUpdate,
    limits: &crate::Limits,
) -> Result<(), PdfError> {
    if update.entries.len() > limits.max_container_items {
        return Err(PdfError::limit(
            "document Info update exceeds container limit",
        ));
    }
    for (name, value) in &update.entries {
        if name.is_empty()
            || name.len() > limits.max_token_bytes
            || !name
                .bytes()
                .all(|byte| (33..=126).contains(&byte) && !b"()<>[]{}/%#".contains(&byte))
        {
            return Err(PdfError::unsafe_rewrite(
                "document Info entry names must be simple PDF names",
            ));
        }
        if value
            .as_ref()
            .is_some_and(|value| value.len() > limits.max_token_bytes)
        {
            return Err(PdfError::limit("document Info value exceeds token limit"));
        }
        if name == "Trapped"
            && value
                .as_ref()
                .is_some_and(|value| !matches!(value.as_str(), "True" | "False" | "Unknown"))
        {
            return Err(PdfError::unsafe_rewrite(
                "Info /Trapped must be True, False, or Unknown",
            ));
        }
    }
    Ok(())
}

fn validate_xmp(xml: &[u8], limits: &crate::Limits) -> Result<(), PdfError> {
    if xml.is_empty() || xml.len() > limits.max_stream_bytes {
        return Err(PdfError::limit(
            "XMP metadata is empty or exceeds max_stream_bytes",
        ));
    }
    let text = std::str::from_utf8(xml)
        .map_err(|_| PdfError::unsafe_rewrite("XMP metadata must be UTF-8 XML"))?;
    if !text.trim_start().starts_with('<') {
        return Err(PdfError::unsafe_rewrite(
            "XMP metadata must start with an XML element or declaration",
        ));
    }
    let mut reader = quick_xml::Reader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut depth = 0usize;
    let mut roots = 0usize;
    let mut events = 0usize;
    loop {
        events = events
            .checked_add(1)
            .ok_or_else(|| PdfError::limit("XMP event count overflows"))?;
        if events > limits.max_container_items {
            return Err(PdfError::limit("XMP event count exceeds container limit"));
        }
        match reader.read_event_into(&mut buffer) {
            Ok(quick_xml::events::Event::Start(_)) => {
                if depth == 0 {
                    roots += 1;
                }
                depth += 1;
                if depth > limits.max_parser_depth {
                    return Err(PdfError::limit("XMP element depth exceeds parser limit"));
                }
            }
            Ok(quick_xml::events::Event::Empty(_)) if depth == 0 => roots += 1,
            Ok(quick_xml::events::Event::End(_)) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| PdfError::unsafe_rewrite("XMP XML has an unmatched end tag"))?;
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Ok(_) => {}
            Err(_) => {
                return Err(PdfError::unsafe_rewrite(
                    "XMP metadata is not well-formed XML",
                ));
            }
        }
        buffer.clear();
    }
    if depth != 0 || roots != 1 {
        return Err(PdfError::unsafe_rewrite(
            "XMP metadata must contain exactly one complete root element",
        ));
    }
    Ok(())
}

fn text_entry(
    dictionary: &BTreeMap<Vec<u8>, Value>,
    key: &[u8],
) -> Result<Option<String>, PdfError> {
    match dictionary.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(decode_text(value))),
        Some(_) => Err(PdfError::syntax(
            format!("Info /{} is not a string", String::from_utf8_lossy(key)),
            0,
        )),
    }
}

fn info_dictionary(parsed: &ParsedDocument) -> Result<Option<&BTreeMap<Vec<u8>, Value>>, PdfError> {
    let trailer = dictionary(&parsed.trailer, "trailer")?;
    trailer
        .get(b"Info".as_slice())
        .map(|value| resolve_dict(parsed, value, "trailer /Info").map(|value| value.0))
        .transpose()
}

fn install_info_dictionary(
    parsed: &mut ParsedDocument,
    info: BTreeMap<Vec<u8>, Value>,
) -> Result<(usize, usize), PdfError> {
    let trailer = dictionary(&parsed.trailer, "trailer")?.clone();
    let mut new_trailer = trailer;
    let existing = new_trailer.get(b"Info".as_slice()).cloned();
    let mut added = 0;
    let mut removed = 0;
    if info.is_empty() {
        if let Some(Value::Ref(reference)) = new_trailer.remove(b"Info".as_slice()) {
            if !is_referenced_by_objects(parsed, reference, None) {
                parsed.objects.remove(&reference);
                removed = 1;
            }
        } else {
            new_trailer.remove(b"Info".as_slice());
        }
    } else {
        match existing {
            Some(Value::Ref(reference)) => {
                parsed
                    .objects
                    .insert(reference, plain_object(Value::Dict(info)));
            }
            Some(Value::Dict(_)) => {
                new_trailer.insert(b"Info".to_vec(), Value::Dict(info));
            }
            Some(_) => {
                return Err(PdfError::unsupported(
                    "trailer /Info must be a dictionary or reference",
                ));
            }
            None => {
                let reference = allocate_reference(parsed)?;
                parsed
                    .objects
                    .insert(reference, plain_object(Value::Dict(info)));
                new_trailer.insert(b"Info".to_vec(), Value::Ref(reference));
                added = 1;
            }
        }
    }
    parsed.trailer = Value::Dict(new_trailer);
    Ok((added, removed))
}

fn info_update_matches(
    document: &PdfDocument,
    update: &DocumentInfoUpdate,
) -> Result<bool, PdfError> {
    let info = info_dictionary(document.parsed())?;
    Ok(update.entries.iter().all(|(name, expected)| {
        let actual = info.and_then(|info| info.get(name.as_bytes()));
        match (name.as_str(), expected, actual) {
            (_, None, None) => true,
            ("Trapped", Some(expected), Some(Value::Name(actual))) => actual == expected.as_bytes(),
            (_, Some(expected), Some(Value::String(actual))) => decode_text(actual) == *expected,
            _ => false,
        }
    }))
}

fn unspecified_entries_preserved(
    before: &BTreeMap<Vec<u8>, Value>,
    after: &BTreeMap<Vec<u8>, Value>,
    update: &DocumentInfoUpdate,
) -> bool {
    before.iter().all(|(key, value)| {
        update
            .entries
            .contains_key(String::from_utf8_lossy(key).as_ref())
            || after.get(key) == Some(value)
    })
}

fn catalog(parsed: &ParsedDocument) -> Result<(ObjectRef, &BTreeMap<Vec<u8>, Value>), PdfError> {
    let trailer = dictionary(&parsed.trailer, "trailer")?;
    let Some(Value::Ref(reference)) = trailer.get(b"Root".as_slice()) else {
        return Err(PdfError::unsafe_rewrite(
            "document structures require an indirect catalog",
        ));
    };
    let catalog = dictionary(&parsed.object(*reference)?.value, "catalog")?;
    Ok((*reference, catalog))
}

fn require_xmp_dictionary(dictionary: &BTreeMap<Vec<u8>, Value>) -> Result<(), PdfError> {
    if !matches!(dictionary.get(b"Type".as_slice()), Some(Value::Name(value)) if value == b"Metadata")
        || !matches!(dictionary.get(b"Subtype".as_slice()), Some(Value::Name(value)) if value == b"XML")
    {
        return Err(PdfError::unsupported(
            "catalog metadata stream must be /Type /Metadata /Subtype /XML",
        ));
    }
    Ok(())
}

fn catalog_has_metadata(parsed: &ParsedDocument) -> Result<bool, PdfError> {
    Ok(catalog(parsed)?.1.contains_key(b"Metadata".as_slice()))
}

fn xmp_unknown_dictionary_preserved(
    before: &ParsedDocument,
    after: &ParsedDocument,
) -> Result<bool, PdfError> {
    let before_ref = match catalog(before)?.1.get(b"Metadata".as_slice()) {
        Some(Value::Ref(reference)) => Some(*reference),
        _ => None,
    };
    let after_ref = match catalog(after)?.1.get(b"Metadata".as_slice()) {
        Some(Value::Ref(reference)) => Some(*reference),
        _ => None,
    };
    let (Some(before_ref), Some(after_ref)) = (before_ref, after_ref) else {
        return Ok(true);
    };
    let before = dictionary(&before.object(before_ref)?.value, "XMP metadata")?;
    let after = dictionary(&after.object(after_ref)?.value, "XMP metadata")?;
    Ok(before.iter().all(|(key, value)| {
        matches!(key.as_slice(), b"Length" | b"Type" | b"Subtype") || after.get(key) == Some(value)
    }))
}

fn allocate_reference(parsed: &ParsedDocument) -> Result<ObjectRef, PdfError> {
    if parsed.objects.len() >= parsed.limits.max_objects {
        return Err(PdfError::limit(
            "document structure object allocation exceeds limit",
        ));
    }
    let number = parsed
        .objects
        .keys()
        .map(|reference| reference.number)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("document structure object number overflows"))?;
    if usize::try_from(number)
        .ok()
        .and_then(|value| value.checked_add(1))
        .is_none_or(|value| value > parsed.limits.max_xref_entries)
    {
        return Err(PdfError::limit(
            "document structure allocation exceeds xref limit",
        ));
    }
    Ok(ObjectRef {
        number,
        generation: 0,
    })
}

fn plain_object(value: Value) -> IndirectObject {
    IndirectObject {
        value,
        stream: None,
        stream_offset: 0,
        offset: 0,
    }
}

fn encode_text(value: &str) -> Vec<u8> {
    if value.is_ascii() {
        return value.as_bytes().to_vec();
    }
    let mut output = vec![0xfe, 0xff];
    for unit in value.encode_utf16() {
        output.extend_from_slice(&unit.to_be_bytes());
    }
    output
}

fn decode_text(value: &[u8]) -> String {
    if let Some(value) = value.strip_prefix(&[0xfe, 0xff]) {
        return String::from_utf16_lossy(
            &value
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>(),
        );
    }
    String::from_utf8_lossy(value).into_owned()
}

fn javascript_action(
    reference: ObjectRef,
    value: &Value,
    name: Option<String>,
) -> Result<Option<JavaScriptAction>, PdfError> {
    let Value::Dict(dictionary) = value else {
        return Ok(None);
    };
    if !matches!(dictionary.get(b"S".as_slice()), Some(Value::Name(value)) if value == b"JavaScript")
    {
        return Ok(None);
    }
    let script = match dictionary.get(b"JS".as_slice()) {
        Some(Value::String(value)) => readable_javascript_text(value, "JavaScript action /JS")?,
        Some(_) => {
            return Err(PdfError::unsupported(format!(
                "JavaScript action {} {} R /JS must be a direct readable string",
                reference.number, reference.generation
            )));
        }
        None => {
            return Err(PdfError::unsupported(format!(
                "JavaScript action {} {} R has no /JS string",
                reference.number, reference.generation
            )));
        }
    };
    Ok(Some(JavaScriptAction {
        object_number: reference.number,
        object_generation: reference.generation,
        name,
        script,
    }))
}

fn readable_javascript_text(value: &[u8], label: &str) -> Result<String, PdfError> {
    let Some(utf16) = value.strip_prefix(&[0xfe, 0xff]) else {
        return std::str::from_utf8(value)
            .map(str::to_owned)
            .map_err(|_| PdfError::unsupported(format!("{label} is not valid UTF-8 or UTF-16BE")));
    };
    if !utf16.len().is_multiple_of(2) {
        return Err(PdfError::unsupported(format!(
            "{label} has an incomplete UTF-16BE code unit"
        )));
    }
    String::from_utf16(
        &utf16
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>(),
    )
    .map_err(|_| PdfError::unsupported(format!("{label} is not valid UTF-16BE")))
}

fn resolve_dict<'a>(
    parsed: &'a ParsedDocument,
    mut value: &'a Value,
    label: &str,
) -> Result<ResolvedDictionary<'a>, PdfError> {
    let mut seen = BTreeSet::new();
    let mut reference = None;
    while let Value::Ref(next) = value {
        if seen.len() >= parsed.limits.max_parser_depth {
            return Err(PdfError::limit(format!(
                "{label} reference depth exceeds limit"
            )));
        }
        if !seen.insert(*next) {
            return Err(PdfError::syntax(format!("cycle resolving {label}"), 0));
        }
        value = &parsed.object(*next)?.value;
        reference = Some(*next);
    }
    Ok((dictionary(value, label)?, reference))
}

fn dictionary<'a>(value: &'a Value, label: &str) -> Result<&'a BTreeMap<Vec<u8>, Value>, PdfError> {
    match value {
        Value::Dict(value) => Ok(value),
        _ => Err(PdfError::syntax(format!("{label} is not a dictionary"), 0)),
    }
}

fn all_references_resolve(parsed: &ParsedDocument) -> bool {
    fn resolve(value: &Value, parsed: &ParsedDocument, depth: usize) -> bool {
        if depth > parsed.limits.max_parser_depth {
            return false;
        }
        match value {
            Value::Ref(reference) => parsed.objects.contains_key(reference),
            Value::Array(values) => values.iter().all(|value| resolve(value, parsed, depth + 1)),
            Value::Dict(values) => values
                .values()
                .all(|value| resolve(value, parsed, depth + 1)),
            _ => true,
        }
    }
    resolve(&parsed.trailer, parsed, 0)
        && parsed
            .objects
            .values()
            .all(|object| resolve(&object.value, parsed, 0))
}

fn is_referenced_by_objects(
    parsed: &ParsedDocument,
    expected: ObjectRef,
    except: Option<ObjectRef>,
) -> bool {
    fn contains(value: &Value, expected: ObjectRef) -> bool {
        match value {
            Value::Ref(reference) => *reference == expected,
            Value::Array(values) => values.iter().any(|value| contains(value, expected)),
            Value::Dict(values) => values.values().any(|value| contains(value, expected)),
            _ => false,
        }
    }
    parsed
        .objects
        .iter()
        .any(|(reference, object)| Some(*reference) != except && contains(&object.value, expected))
}

fn write_structure(document: &PdfDocument, parsed: ParsedDocument) -> Result<Vec<u8>, PdfError> {
    write_encrypted_pdf(document, &parsed)
}

fn reopen(document: &PdfDocument, bytes: &[u8], label: &str) -> Result<PdfDocument, PdfError> {
    PdfEngine::new(document.engine_config().clone())
        .open(bytes, OpenOptions::default())
        .map_err(|error| PdfError::verification(format!("{label} output did not reparse: {error}")))
}

fn structure_outcome(
    original: &PdfDocument,
    rewritten: &PdfDocument,
    bytes: Vec<u8>,
    mut report: DocumentStructureReport,
    requested_state_matches: bool,
    catalog_reachable: bool,
    unknown_entries_preserved: bool,
) -> Result<DocumentStructureOutcome, PdfError> {
    let page_count_unchanged = original.page_count()? == rewritten.page_count()?;
    let no_dangling_references = all_references_resolve(rewritten.parsed());
    let verification = DocumentStructureVerification {
        passed: page_count_unchanged
            && requested_state_matches
            && catalog_reachable
            && unknown_entries_preserved
            && no_dangling_references,
        reparsed: true,
        page_count_unchanged,
        requested_state_matches,
        catalog_reachable,
        unknown_entries_preserved,
        no_dangling_references,
    };
    if !verification.passed {
        return Err(PdfError::verification(
            "document structure mutation failed post-write verification",
        ));
    }
    report.output_bytes = bytes.len();
    Ok(DocumentStructureOutcome {
        bytes,
        report,
        verification,
    })
}
