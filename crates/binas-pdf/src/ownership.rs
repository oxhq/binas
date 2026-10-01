//! Proof of explicit, direct page slots only. This is not a transitive resource
//! ownership classifier and does not interpret references in opaque streams.
use std::collections::BTreeMap;

use binas_core::{OperationContext, SnapshotId};

use crate::{
    PdfDocument, PdfError, PdfReferenceIndex, PdfReferenceOrigin, PdfReferencePathStep,
    PdfReferenceTarget,
    object::{ObjectRef, Value},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageOwnedRole {
    Contents,
    Annots,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum PageOwnedSlot {
    Scalar,
    ArrayIndex(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageOwnedReference {
    pub page: ObjectRef,
    pub slot: PageOwnedSlot,
}

impl PageOwnedRole {
    fn key(self) -> &'static [u8] {
        match self {
            Self::Contents => b"Contents",
            Self::Annots => b"Annots",
        }
    }
}

/// The caller must provide its current immutable revision and the index built
/// for that revision. Every indexed occurrence is checked against current typed
/// slots; mismatched source/revision bindings are rejected. Repeated slots on a
/// single page are retained separately; a page appearing twice in the page tree
/// is ambiguous and refused. Results follow document page order, then slot order.
/// Temporary accounting covers logical workspace, not allocator overhead. The
/// returned ordinary Vec is caller-owned, not a retained budget reservation.
pub(crate) fn direct_page_owners(
    document: &PdfDocument,
    index: &PdfReferenceIndex,
    snapshot: SnapshotId,
    target: ObjectRef,
    role: PageOwnedRole,
    context: &OperationContext,
) -> Result<Vec<PageOwnedReference>, PdfError> {
    context.charge_work(1).map_err(PdfError::from_context)?;
    if snapshot.source() != document.source_id() || snapshot != index.snapshot() {
        return Err(PdfError::selection(
            "page ownership source or revision mismatch",
        ));
    }
    let incoming = index.incoming(snapshot, public_ref(target))?;
    let object = document.parsed().object(target)?;
    if incoming.is_empty()
        || (role == PageOwnedRole::Contents && object.stream.is_none())
        || (role == PageOwnedRole::Annots
            && (object.stream.is_some() || !matches!(object.value, Value::Dict(_))))
    {
        return Err(PdfError::unsafe_rewrite(
            "target has no supported direct page owner",
        ));
    }
    let limits = &document.parsed().limits;
    if incoming.len() > limits.max_container_items {
        return Err(PdfError::limit(
            "page ownership occurrence count exceeds limit",
        ));
    }
    // page_refs has its own configured graph/page limits. Subsequent work polls
    // the supplied context; this does not make every legacy page-tree loop yield.
    let pages = document.page_refs()?;
    let workspace_bytes = pages
        .len()
        .checked_mul(std::mem::size_of::<ObjectRef>() + std::mem::size_of::<usize>())
        .and_then(|bytes| {
            incoming
                .len()
                .checked_mul(std::mem::size_of::<PageOwnedReference>())
                .and_then(|owners| bytes.checked_add(owners))
        })
        .ok_or_else(|| PdfError::limit("page ownership workspace overflows"))?;
    let _workspace = context
        .reserve(workspace_bytes as u64)
        .map_err(PdfError::from_context)?;
    let mut page_positions = BTreeMap::new();
    for (position, page) in pages.iter().enumerate() {
        context.charge_work(1).map_err(PdfError::from_context)?;
        if page_positions.insert(*page, position).is_some() {
            return Err(PdfError::unsafe_rewrite(
                "page has multiple occurrences in page tree",
            ));
        }
    }
    context
        .charge_work(incoming.len() as u64)
        .map_err(PdfError::from_context)?;
    let mut owners = Vec::new();
    owners
        .try_reserve_exact(incoming.len())
        .map_err(|_| PdfError::limit("page ownership allocation failed"))?;
    for entry in incoming {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let PdfReferenceOrigin::Object(owner) = entry.origin else {
            return Err(PdfError::unsafe_rewrite(
                "target has a trailer reference outside page ownership",
            ));
        };
        let owner = ObjectRef {
            number: owner.number,
            generation: owner.generation,
        };
        if entry.dangling || !page_positions.contains_key(&owner) {
            return Err(PdfError::unsafe_rewrite(
                "target has an incoming reference outside direct page ownership",
            ));
        }
        let slot = match entry.path.as_slice() {
            [PdfReferencePathStep::Key(key)]
                if role == PageOwnedRole::Contents && key == role.key() =>
            {
                PageOwnedSlot::Scalar
            }
            [
                PdfReferencePathStep::Key(key),
                PdfReferencePathStep::Index(index),
            ] if key == role.key() => PageOwnedSlot::ArrayIndex(*index),
            _ => {
                return Err(PdfError::unsafe_rewrite(
                    "target has a nested, indirect, or different-role reference",
                ));
            }
        };
        let Value::Dict(dictionary) = &document.parsed().object(owner)?.value else {
            return Err(PdfError::unsafe_rewrite("page owner is not a dictionary"));
        };
        let actual = match (slot, dictionary.get(role.key())) {
            (PageOwnedSlot::Scalar, value) => value,
            (PageOwnedSlot::ArrayIndex(index), Some(Value::Array(values))) => values.get(index),
            _ => None,
        };
        if actual != Some(&Value::Ref(target)) {
            return Err(PdfError::selection(
                "indexed page slot does not match current document",
            ));
        }
        owners.push(PageOwnedReference { page: owner, slot });
    }
    // Also count all current direct slots: an old index must not omit a newly
    // added direct occurrence on a current page within the same source bytes.
    let mut current_count = 0usize;
    for page in pages {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let Value::Dict(dictionary) = &document.parsed().object(page)?.value else {
            continue;
        };
        match dictionary.get(role.key()) {
            Some(Value::Ref(reference))
                if role == PageOwnedRole::Contents && *reference == target =>
            {
                current_count = current_count
                    .checked_add(1)
                    .ok_or_else(|| PdfError::limit("page ownership count overflows"))?
            }
            Some(Value::Array(values)) => {
                for value in values {
                    context.charge_work(1).map_err(PdfError::from_context)?;
                    if value == &Value::Ref(target) {
                        current_count = current_count
                            .checked_add(1)
                            .ok_or_else(|| PdfError::limit("page ownership count overflows"))?;
                    }
                }
            }
            _ => {}
        }
    }
    if current_count != owners.len() {
        return Err(PdfError::selection(
            "reference index omits current direct page slots",
        ));
    }
    let levels = usize::BITS - owners.len().leading_zeros();
    let sort_work = (owners.len() as u64)
        .checked_mul(u64::from(levels))
        .ok_or_else(|| PdfError::limit("page ownership sort work overflows"))?;
    context
        .charge_work(sort_work)
        .map_err(PdfError::from_context)?;
    owners.sort_unstable_by_key(|owner| (page_positions[&owner.page], owner.slot));
    context.checkpoint().map_err(PdfError::from_context)?;
    Ok(owners)
}

fn public_ref(reference: ObjectRef) -> PdfReferenceTarget {
    PdfReferenceTarget {
        number: reference.number,
        generation: reference.generation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OpenOptions, PdfEngine, PdfErrorCode};
    use binas_core::{BudgetLimits, CancellationToken, ResourceBudget, RevisionId};

    fn context() -> OperationContext {
        OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 1024 * 1024,
                max_work_units: 1024 * 1024,
            }),
            CancellationToken::new(),
        )
    }

    fn document(contents: &[u8], extra: &[u8], trailer_extra: &[u8]) -> PdfDocument {
        let mut page = b"<< /Type /Page /Parent 2 0 R ".to_vec();
        page.extend_from_slice(contents);
        page.extend_from_slice(b" >>");
        let objects: Vec<(u16, Vec<u8>)> = vec![
            (0, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
            (
                0,
                b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_vec(),
            ),
            (0, page),
            (
                0,
                b"<< /Type /Page /Parent 2 0 R /Contents 5 7 R /Annots [6 0 R] >>".to_vec(),
            ),
            (7, b"<< /Length 0 >>\nstream\n\nendstream".to_vec()),
            (0, b"<< /Type /Annot /Subtype /Text >>".to_vec()),
            (0, extra.to_vec()),
        ];
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, (generation, object)) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} {generation} obj\n", index + 1).as_bytes());
            bytes.extend_from_slice(object);
            bytes.extend_from_slice(b"\nendobj\n");
        }
        let xref = bytes.len();
        bytes.extend_from_slice(b"xref\n0 8\n0000000000 65535 f \n");
        for (offset, (generation, _)) in offsets.iter().zip(&objects) {
            bytes.extend_from_slice(format!("{offset:010} {generation:05} n \n").as_bytes());
        }
        bytes.extend_from_slice(b"trailer\n<< /Size 8 /Root 1 0 R ");
        bytes.extend_from_slice(trailer_extra);
        bytes.extend_from_slice(format!(" >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
        PdfEngine::default()
            .open_owned(bytes, OpenOptions::default())
            .unwrap()
    }

    fn run(
        doc: &PdfDocument,
        target: ObjectRef,
        role: PageOwnedRole,
    ) -> Result<Vec<PageOwnedReference>, PdfError> {
        let context = context();
        let index = PdfReferenceIndex::build(doc, &context).unwrap();
        direct_page_owners(doc, &index, index.snapshot(), target, role, &context)
    }

    #[test]
    fn shared_contents_keep_repeated_slots_and_stable_page_order() {
        let doc = document(b"/Contents [5 7 R 5 7 R]", b"<< >>", b"");
        let target = ObjectRef {
            number: 5,
            generation: 7,
        };
        assert_eq!(
            run(&doc, target, PageOwnedRole::Contents).unwrap(),
            vec![
                PageOwnedReference {
                    page: ObjectRef {
                        number: 3,
                        generation: 0
                    },
                    slot: PageOwnedSlot::ArrayIndex(0)
                },
                PageOwnedReference {
                    page: ObjectRef {
                        number: 3,
                        generation: 0
                    },
                    slot: PageOwnedSlot::ArrayIndex(1)
                },
                PageOwnedReference {
                    page: ObjectRef {
                        number: 4,
                        generation: 0
                    },
                    slot: PageOwnedSlot::Scalar
                },
            ]
        );
        assert!(
            run(
                &doc,
                ObjectRef {
                    generation: 0,
                    ..target
                },
                PageOwnedRole::Contents
            )
            .is_err()
        );
    }

    #[test]
    fn annots_require_direct_array_slots_and_exact_role() {
        let doc = document(b"/Annots [6 0 R 6 0 R]", b"<< >>", b"");
        let target = ObjectRef {
            number: 6,
            generation: 0,
        };
        assert_eq!(run(&doc, target, PageOwnedRole::Annots).unwrap().len(), 3);
        assert!(run(&doc, target, PageOwnedRole::Contents).is_err());
        let scalar = document(b"/Annots 6 0 R", b"<< >>", b"");
        assert!(run(&scalar, target, PageOwnedRole::Annots).is_err());
    }

    #[test]
    fn output_order_follows_pages_and_duplicate_page_occurrences_refuse() {
        let doc = document(b"/Contents 5 7 R", b"<< >>", b"");
        let target = ObjectRef {
            number: 5,
            generation: 7,
        };
        let mut parsed = doc.parsed().clone();
        let Value::Dict(tree) = &mut parsed
            .objects
            .get_mut(&ObjectRef {
                number: 2,
                generation: 0,
            })
            .unwrap()
            .value
        else {
            unreachable!()
        };
        tree.insert(
            b"Kids".to_vec(),
            Value::Array(vec![
                Value::Ref(ObjectRef {
                    number: 4,
                    generation: 0,
                }),
                Value::Ref(ObjectRef {
                    number: 3,
                    generation: 0,
                }),
            ]),
        );
        let reordered = doc.with_parsed(parsed.clone());
        assert_eq!(
            run(&reordered, target, PageOwnedRole::Contents)
                .unwrap()
                .iter()
                .map(|owner| owner.page.number)
                .collect::<Vec<_>>(),
            [4, 3]
        );
        let Value::Dict(tree) = &mut parsed
            .objects
            .get_mut(&ObjectRef {
                number: 2,
                generation: 0,
            })
            .unwrap()
            .value
        else {
            unreachable!()
        };
        tree.insert(
            b"Kids".to_vec(),
            Value::Array(vec![
                Value::Ref(ObjectRef {
                    number: 3,
                    generation: 0,
                }),
                Value::Ref(ObjectRef {
                    number: 3,
                    generation: 0,
                }),
            ]),
        );
        assert_eq!(
            run(&doc.with_parsed(parsed), target, PageOwnedRole::Contents)
                .unwrap_err()
                .code,
            PdfErrorCode::UnsafeRewrite
        );
    }

    #[test]
    fn any_foreign_nested_different_role_or_trailer_occurrence_refuses() {
        for (page, extra, trailer) in [
            (
                b"/Contents 5 7 R".as_slice(),
                b"<< /Opaque 5 7 R >>".as_slice(),
                b"".as_slice(),
            ),
            (
                b"/Contents 5 7 R",
                b"<< /Type /XObject /Subtype /Form /Contents 5 7 R >>",
                b"",
            ),
            (b"/Contents 5 7 R /Annots [5 7 R]", b"<< >>", b""),
            (b"/Contents [[5 7 R]]", b"<< >>", b""),
            (b"/Contents 5 7 R", b"<< >>", b"/Opaque 5 7 R"),
        ] {
            let doc = document(page, extra, trailer);
            assert_eq!(
                run(
                    &doc,
                    ObjectRef {
                        number: 5,
                        generation: 7
                    },
                    PageOwnedRole::Contents
                )
                .unwrap_err()
                .code,
                PdfErrorCode::UnsafeRewrite
            );
        }
    }

    #[test]
    fn stale_source_revision_and_changed_direct_slots_refuse() {
        let doc = document(b"/Contents 5 7 R", b"<< >>", b"");
        let context = context();
        let index = PdfReferenceIndex::build(&doc, &context).unwrap();
        let target = ObjectRef {
            number: 5,
            generation: 7,
        };
        let later = SnapshotId::new(doc.source_id(), RevisionId::new(1));
        assert_eq!(
            direct_page_owners(
                &doc,
                &index,
                later,
                target,
                PageOwnedRole::Contents,
                &context
            )
            .unwrap_err()
            .code,
            PdfErrorCode::SelectionNotFound
        );
        let foreign = document(b"/Contents [5 7 R]", b"<< >>", b"");
        assert!(
            direct_page_owners(
                &foreign,
                &index,
                index.snapshot(),
                target,
                PageOwnedRole::Contents,
                &context
            )
            .is_err()
        );
        let mut parsed = doc.parsed().clone();
        let Value::Dict(page) = &mut parsed
            .objects
            .get_mut(&ObjectRef {
                number: 3,
                generation: 0,
            })
            .unwrap()
            .value
        else {
            unreachable!()
        };
        page.insert(
            b"Contents".to_vec(),
            Value::Array(vec![Value::Ref(target), Value::Ref(target)]),
        );
        let changed = doc.with_parsed(parsed);
        assert!(
            direct_page_owners(
                &changed,
                &index,
                index.snapshot(),
                target,
                PageOwnedRole::Contents,
                &context
            )
            .is_err()
        );
    }

    #[test]
    fn cancellation_and_work_exhaustion_are_typed() {
        let doc = document(b"/Contents 5 7 R", b"<< >>", b"");
        let build = context();
        let index = PdfReferenceIndex::build(&doc, &build).unwrap();
        let cancelled = context();
        cancelled.cancellation().cancel();
        let target = ObjectRef {
            number: 5,
            generation: 7,
        };
        assert_eq!(
            direct_page_owners(
                &doc,
                &index,
                index.snapshot(),
                target,
                PageOwnedRole::Contents,
                &cancelled
            )
            .unwrap_err()
            .code,
            PdfErrorCode::Cancelled
        );
        let empty = OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 0,
                max_work_units: 0,
            }),
            CancellationToken::new(),
        );
        assert_eq!(
            direct_page_owners(
                &doc,
                &index,
                index.snapshot(),
                target,
                PageOwnedRole::Contents,
                &empty
            )
            .unwrap_err()
            .code,
            PdfErrorCode::ResourceLimit
        );
        let no_memory = OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 0,
                max_work_units: 1000,
            }),
            CancellationToken::new(),
        );
        assert_eq!(
            direct_page_owners(
                &doc,
                &index,
                index.snapshot(),
                target,
                PageOwnedRole::Contents,
                &no_memory
            )
            .unwrap_err()
            .code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(no_memory.budget().snapshot().live_bytes, 0);
        assert!(no_memory.budget().snapshot().work_units > 0);
    }
}
