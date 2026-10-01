//! Typed document storage and object lookup.
//!
//! Object snapshots share one immutable base and one sparse change map. Cloning
//! a snapshot shares both maps; editing a shared snapshot copies change-map
//! metadata and the target object only. There is no chain of revision layers.
//! Values inside a changed object still clone, and source-backed streams retain
//! their complete source allocation. Atomic operations/history are separate.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, btree_map},
    iter::Peekable,
    sync::Arc,
};

use crate::{
    decode_cache::DecodeState,
    error::PdfError,
    limits::Limits,
    object::{IndirectObject, ObjectRef, Value},
    parser::XrefEntry,
};

#[derive(Clone, Debug)]
pub(crate) struct ParsedDocument {
    pub version: String,
    pub objects: ObjectStore,
    pub trailer: Value,
    /// Original parsed trailer allocation floor, unchanged across staged graph revisions.
    pub(crate) source_size: Option<usize>,
    pub xref_revisions: usize,
    pub limits: Limits,
    pub(crate) xref_entries: Arc<BTreeMap<u32, XrefEntry>>,
    pub(crate) decoded_stream_bytes: usize,
    pub(crate) decode_state: DecodeState,
    pub(crate) decode_cancellation: binas_core::CancellationToken,
    pub(crate) compressed_finished: bool,
}

type SharedObjects = BTreeMap<ObjectRef, Arc<IndirectObject>>;
type Changes = BTreeMap<ObjectRef, Option<Arc<IndirectObject>>>;

#[derive(Clone, Debug, Default)]
pub(crate) struct ObjectStore {
    base: Arc<SharedObjects>,
    changes: Arc<Changes>,
    len: usize,
}

impl From<BTreeMap<ObjectRef, IndirectObject>> for ObjectStore {
    fn from(objects: BTreeMap<ObjectRef, IndirectObject>) -> Self {
        Self {
            len: objects.len(),
            base: Arc::new(
                objects
                    .into_iter()
                    .map(|(key, value)| (key, Arc::new(value)))
                    .collect(),
            ),
            changes: Arc::default(),
        }
    }
}

impl ObjectStore {
    /// Establishes the fully parsed graph as a baseline. Parser completion uses
    /// this once; operation commits must keep their changes instead. Reuses object
    /// Arc handles and only copies map metadata if a prior snapshot shares it.
    pub(crate) fn seal_base(&mut self) {
        if self.changes.is_empty() {
            return;
        }
        let changes = Arc::unwrap_or_clone(std::mem::take(&mut self.changes));
        let base = Arc::make_mut(&mut self.base);
        for (key, value) in changes {
            match value {
                Some(object) => {
                    base.insert(key, object);
                }
                None => {
                    base.remove(&key);
                }
            }
        }
        debug_assert_eq!(base.len(), self.len);
    }

    /// Storage identity for a specific branch, not semantic equality. Rebuilt
    /// content-equivalent stores intentionally do not satisfy this stale guard.
    pub(crate) fn shares_snapshot_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.base, &other.base) && Arc::ptr_eq(&self.changes, &other.changes)
    }

    /// Full ordered delta from this store's immutable base, not just the latest
    /// operation. `None` masks a base object; new keys have no deletion record.
    pub(crate) fn changes(
        &self,
    ) -> impl ExactSizeIterator<Item = (ObjectRef, Option<&IndirectObject>)> {
        self.changes
            .iter()
            .map(|(key, object)| (*key, object.as_deref()))
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn shared(&self, key: &ObjectRef) -> Option<&Arc<IndirectObject>> {
        if self.is_empty() {
            return None;
        }
        match self.changes.get(key) {
            Some(value) => value.as_ref(),
            None => self.base.get(key),
        }
    }

    pub(crate) fn get(&self, key: &ObjectRef) -> Option<&IndirectObject> {
        self.shared(key).map(Arc::as_ref)
    }

    /// Membership in the fully parsed source graph, independent of staged deltas.
    pub(crate) fn contains_base_key(&self, key: &ObjectRef) -> bool {
        self.base.contains_key(key)
    }

    pub(crate) fn base_keys(&self) -> impl Iterator<Item = &ObjectRef> {
        self.base.keys()
    }

    pub(crate) fn contains_key(&self, key: &ObjectRef) -> bool {
        self.shared(key).is_some()
    }

    pub(crate) fn get_mut(&mut self, key: &ObjectRef) -> Option<&mut IndirectObject> {
        // Missing/tombstoned keys must not alter even sparse map metadata.
        self.shared(key)?;
        let changes = Arc::make_mut(&mut self.changes);
        let value = changes
            .entry(*key)
            .or_insert_with(|| self.base.get(key).cloned());
        value.as_mut().map(Arc::make_mut)
    }

    pub(crate) fn insert(
        &mut self,
        key: ObjectRef,
        object: IndirectObject,
    ) -> Option<Arc<IndirectObject>> {
        let previous = self.shared(&key).cloned();
        if previous.is_none() {
            self.len += 1;
        }
        Arc::make_mut(&mut self.changes).insert(key, Some(Arc::new(object)));
        previous
    }

    pub(crate) fn remove(&mut self, key: &ObjectRef) -> Option<Arc<IndirectObject>> {
        let previous = self.shared(key)?.clone();
        self.len -= 1;
        let changes = Arc::make_mut(&mut self.changes);
        if self.base.contains_key(key) {
            changes.insert(*key, None);
        } else {
            // A new key has no older base entry to mask; avoid stale tombstones.
            changes.remove(key);
        }
        Some(previous)
    }

    pub(crate) fn extend(
        &mut self,
        objects: impl IntoIterator<Item = (ObjectRef, IndirectObject)>,
    ) {
        for (key, object) in objects {
            self.insert(key, object);
        }
    }

    pub(crate) fn iter(&self) -> ObjectIter<'_> {
        ObjectIter {
            base: self.base.iter().peekable(),
            changes: self.changes.iter().peekable(),
            remaining: self.len,
        }
    }

    pub(crate) fn keys(&self) -> impl ExactSizeIterator<Item = &ObjectRef> {
        self.iter().map(|(key, _)| key)
    }

    pub(crate) fn values(&self) -> impl ExactSizeIterator<Item = &IndirectObject> {
        self.iter().map(|(_, value)| value)
    }
}

/// Merges two sorted borrowed maps without collecting a new object inventory.
pub(crate) struct ObjectIter<'a> {
    base: Peekable<btree_map::Iter<'a, ObjectRef, Arc<IndirectObject>>>,
    changes: Peekable<btree_map::Iter<'a, ObjectRef, Option<Arc<IndirectObject>>>>,
    remaining: usize,
}

impl<'a> Iterator for ObjectIter<'a> {
    type Item = (&'a ObjectRef, &'a IndirectObject);

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        loop {
            let next = match (self.base.peek(), self.changes.peek()) {
                (None, None) => return None,
                (Some(_), None) => self.base.next().map(|(key, value)| (key, value.as_ref())),
                (None, Some(_)) => self
                    .changes
                    .next()
                    .and_then(|(key, value)| value.as_deref().map(|value| (key, value))),
                (Some((base_key, _)), Some((change_key, _))) => match base_key.cmp(change_key) {
                    Ordering::Less => self.base.next().map(|(key, value)| (key, value.as_ref())),
                    Ordering::Greater => self
                        .changes
                        .next()
                        .and_then(|(key, value)| value.as_deref().map(|value| (key, value))),
                    Ordering::Equal => {
                        self.base.next();
                        self.changes
                            .next()
                            .and_then(|(key, value)| value.as_deref().map(|value| (key, value)))
                    }
                },
            };
            if next.is_some() {
                self.remaining -= 1;
                return next;
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for ObjectIter<'_> {}
impl std::iter::FusedIterator for ObjectIter<'_> {}

impl<'a> IntoIterator for &'a ObjectStore {
    type Item = (&'a ObjectRef, &'a IndirectObject);
    type IntoIter = ObjectIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl ParsedDocument {
    pub fn object(&self, reference: ObjectRef) -> Result<&IndirectObject, PdfError> {
        self.objects.get(&reference).ok_or_else(|| {
            PdfError::syntax(
                format!(
                    "missing object {} {} R",
                    reference.number, reference.generation
                ),
                0,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(number: u32) -> ObjectRef {
        ObjectRef {
            number,
            generation: 0,
        }
    }

    fn object(value: u8) -> IndirectObject {
        IndirectObject {
            value: Value::Dict(BTreeMap::from([(
                b"Bytes".to_vec(),
                Value::String(vec![value; 128]),
            )])),
            stream: Some(vec![value; 1024].into()),
            stream_offset: 0,
            offset: 0,
        }
    }

    fn store() -> ObjectStore {
        BTreeMap::from([
            (reference(2), object(2)),
            (reference(4), object(4)),
            (reference(6), object(6)),
        ])
        .into()
    }

    fn string_pointer(object: &IndirectObject) -> *const u8 {
        let Value::Dict(dictionary) = &object.value else {
            panic!("dictionary fixture")
        };
        let Value::String(bytes) = dictionary.get(b"Bytes".as_slice()).unwrap() else {
            panic!("string fixture")
        };
        bytes.as_ptr()
    }

    #[test]
    fn immutable_base_membership_survives_additions_and_tombstones() {
        let original = store();
        let mut staged = original.clone();
        let existing = *original.keys().next().unwrap();
        staged.remove(&existing);
        staged.insert(reference(99), object(9));
        assert!(staged.contains_base_key(&existing));
        assert!(!staged.contains_key(&existing));
        assert!(!staged.contains_base_key(&reference(99)));
        assert!(staged.contains_key(&reference(99)));
        assert_eq!(
            staged.base_keys().collect::<Vec<_>>(),
            original.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn snapshot_clone_shares_maps_objects_dictionaries_and_stream_payloads() {
        let original = store();
        let mut candidate = original.clone();
        assert!(Arc::ptr_eq(&original.base, &candidate.base));
        assert!(Arc::ptr_eq(&original.changes, &candidate.changes));
        for key in original.keys() {
            let before = original.get(key).unwrap();
            let after = candidate.get(key).unwrap();
            assert!(std::ptr::eq(before, after));
            assert_eq!(string_pointer(before), string_pointer(after));
            assert_eq!(
                before.stream.as_ref().unwrap().as_ptr(),
                after.stream.as_ref().unwrap().as_ptr()
            );
        }
        let target = candidate.get_mut(&reference(4)).unwrap();
        let Value::Dict(dictionary) = &mut target.value else {
            unreachable!()
        };
        dictionary.insert(b"Added".to_vec(), Value::Integer(7));
        assert!(Arc::ptr_eq(&original.base, &candidate.base));
        assert!(!Arc::ptr_eq(&original.changes, &candidate.changes));
        assert_eq!(candidate.changes.len(), 1);
        assert!(!std::ptr::eq(
            original.get(&reference(4)).unwrap(),
            candidate.get(&reference(4)).unwrap()
        ));
        assert_ne!(
            string_pointer(original.get(&reference(4)).unwrap()),
            string_pointer(candidate.get(&reference(4)).unwrap())
        );
        for key in [reference(2), reference(6)] {
            assert!(std::ptr::eq(
                original.get(&key).unwrap(),
                candidate.get(&key).unwrap()
            ));
        }
        assert_eq!(
            original
                .get(&reference(4))
                .unwrap()
                .stream
                .as_ref()
                .unwrap()
                .as_ptr(),
            candidate
                .get(&reference(4))
                .unwrap()
                .stream
                .as_ref()
                .unwrap()
                .as_ptr()
        );
        let Value::Dict(before) = &original.get(&reference(4)).unwrap().value else {
            unreachable!()
        };
        assert!(!before.contains_key(b"Added".as_slice()));
    }

    #[test]
    fn replacing_and_deleting_shared_deltas_leave_earlier_snapshots_unchanged() {
        let mut original = store();
        original.insert(reference(1), object(1));
        original.get_mut(&reference(4)).unwrap().value = Value::Integer(40);
        let mut candidate = original.clone();
        assert!(Arc::ptr_eq(&original.changes, &candidate.changes));
        candidate.get_mut(&reference(1)).unwrap().value = Value::Integer(10);
        assert!(!Arc::ptr_eq(&original.changes, &candidate.changes));
        assert!(std::ptr::eq(
            original.get(&reference(4)).unwrap(),
            candidate.get(&reference(4)).unwrap()
        ));
        candidate.remove(&reference(4)).unwrap();
        assert!(candidate.get(&reference(4)).is_none());
        assert_eq!(
            original.get(&reference(4)).unwrap().value,
            Value::Integer(40)
        );
        assert!(matches!(
            original.get(&reference(1)).unwrap().value,
            Value::Dict(_)
        ));
        assert_eq!(
            candidate.get(&reference(1)).unwrap().value,
            Value::Integer(10)
        );
        candidate.insert(reference(4), object(44));
        assert!(candidate.contains_key(&reference(4)));
        assert_eq!(original.len(), candidate.len());
    }

    #[test]
    fn sorted_logical_iteration_and_lengths_match_an_owned_map_through_changes() {
        let mut store = store();
        let mut expected = BTreeMap::from([
            (reference(2), object(2)),
            (reference(4), object(4)),
            (reference(6), object(6)),
        ]);
        for (number, insert) in [
            (1, true),
            (4, false),
            (5, true),
            (2, true),
            (4, true),
            (1, false),
            (9, false),
            (6, false),
            (0, true),
            (4, false),
            (5, false),
            (2, false),
            (0, false),
        ] {
            let key = reference(number);
            if insert {
                assert_eq!(
                    store.insert(key, object(number as u8)).is_some(),
                    expected.insert(key, object(number as u8)).is_some()
                );
            } else {
                assert_eq!(
                    store.remove(&key).is_some(),
                    expected.remove(&key).is_some()
                );
            }
            assert_eq!(store.len(), expected.len());
            assert_eq!(store.is_empty(), expected.is_empty());
            assert_eq!(
                store.keys().copied().collect::<Vec<_>>(),
                expected.keys().copied().collect::<Vec<_>>()
            );
            let mut iterator = (&store).into_iter();
            for (position, (expected_key, expected_object)) in expected.iter().enumerate() {
                assert_eq!(iterator.len(), expected.len() - position);
                let (key, object) = iterator.next().unwrap();
                assert_eq!(key, expected_key);
                assert_eq!(object.value, expected_object.value);
                assert_eq!(object.stream, expected_object.stream);
            }
            assert_eq!(iterator.len(), 0);
            assert!(iterator.next().is_none());
            assert!(iterator.next().is_none());
        }
        assert!(store.is_empty());
        assert!(store.get_mut(&reference(4)).is_none());
        // Only base tombstones persist: deleted newly inserted keys disappear.
        assert!(store.changes.keys().all(|key| store.base.contains_key(key)));
    }

    #[test]
    fn parsed_snapshots_share_xrefs_and_clone_stream_objects_only_when_mutated() {
        let document = crate::PdfEngine::default()
            .open(
                include_bytes!("../tests/fixtures/pdf/p4-xref-stream-preserve-text.pdf"),
                crate::OpenOptions::default(),
            )
            .unwrap();
        let snapshot = document.parsed().clone();
        assert!(Arc::ptr_eq(
            &snapshot.xref_entries,
            &document.parsed().xref_entries
        ));
        assert!(Arc::ptr_eq(
            &snapshot.objects.base,
            &document.parsed().objects.base
        ));
        for (key, object) in &snapshot.objects {
            assert!(std::ptr::eq(
                object,
                document.parsed().object(*key).unwrap()
            ));
        }
    }

    #[test]
    fn branch_identity_and_full_sparse_changes_are_explicit() {
        let original = store();
        let mut candidate = original.clone();
        assert!(original.shares_snapshot_with(&candidate));
        assert!(!original.shares_snapshot_with(&store()));
        candidate.insert(reference(1), object(1));
        candidate.remove(&reference(4));
        candidate.get_mut(&reference(6)).unwrap().value = Value::Integer(60);
        assert!(!original.shares_snapshot_with(&candidate));
        assert!(candidate.shares_snapshot_with(&candidate.clone()));
        let changes: Vec<_> = candidate.changes().collect();
        assert_eq!(
            changes
                .iter()
                .map(|(key, _)| key.number)
                .collect::<Vec<_>>(),
            vec![1, 4, 6]
        );
        assert!(changes[0].1.is_some());
        assert!(changes[1].1.is_none());
        assert_eq!(changes[2].1.unwrap().value, Value::Integer(60));
        assert_eq!(original.changes().len(), 0);
        let mut later = candidate.clone();
        later.insert(reference(8), object(8));
        assert_eq!(
            later
                .changes()
                .map(|(key, _)| key.number)
                .collect::<Vec<_>>(),
            vec![1, 4, 6, 8]
        );
        assert!(!candidate.shares_snapshot_with(&later));
    }

    #[test]
    fn sealing_baseline_moves_handles_without_cloning_object_values() {
        let mut original = store();
        original.insert(reference(1), object(1));
        original.remove(&reference(4));
        let mut parsed = original.clone();
        parsed.seal_base();
        assert_eq!(parsed.changes().len(), 0);
        assert_eq!(parsed.len(), original.len());
        assert!(!parsed.shares_snapshot_with(&original));
        for (key, object) in &original {
            assert!(std::ptr::eq(object, parsed.get(key).unwrap()));
        }
        assert_eq!(original.changes().len(), 2);
        let clean = parsed.clone();
        parsed.seal_base();
        assert!(clean.shares_snapshot_with(&parsed));
    }

    #[test]
    fn initial_compressed_members_belong_to_base_not_pending_changes() {
        // Original minimal PDF with its catalog inside object stream 3.
        let mut bytes = b"%PDF-1.5\n".to_vec();
        let pages_offset = bytes.len();
        bytes.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n");
        let object_offset = bytes.len();
        let member = b"1 0 << /Type /Catalog /Pages 2 0 R >>";
        bytes.extend_from_slice(
            format!(
                "3 0 obj\n<< /Type /ObjStm /N 1 /First 4 /Length {} >>\nstream\n",
                member.len()
            )
            .as_bytes(),
        );
        bytes.extend_from_slice(member);
        bytes.extend_from_slice(b"\nendstream\nendobj\n");
        let xref_offset = bytes.len();
        let mut rows = Vec::new();
        for (kind, second, third) in [
            (0u8, 0usize, 65535u16),
            (2, 3, 0),
            (1, pages_offset, 0),
            (1, object_offset, 0),
            (1, xref_offset, 0),
        ] {
            rows.push(kind);
            rows.extend_from_slice(&(second as u32).to_be_bytes());
            rows.extend_from_slice(&third.to_be_bytes());
        }
        bytes.extend_from_slice(
            format!(
                "4 0 obj\n<< /Type /XRef /Root 1 0 R /Size 5 /W [1 4 2] /Length {} >>\nstream\n",
                rows.len()
            )
            .as_bytes(),
        );
        bytes.extend(rows);
        bytes.extend_from_slice(
            format!("\nendstream\nendobj\nstartxref\n{xref_offset}\n%%EOF\n").as_bytes(),
        );
        let document = crate::PdfEngine::default()
            .open_owned(bytes, crate::OpenOptions::default())
            .unwrap();
        assert!(document.parsed().compressed_finished);
        assert!(
            document
                .parsed()
                .xref_entries
                .values()
                .any(|entry| matches!(entry, XrefEntry::Compressed { .. }))
        );
        assert_eq!(document.parsed().objects.changes().len(), 0);
        let mut edited = document.parsed().clone();
        let key = *edited.objects.keys().next().unwrap();
        edited.objects.get_mut(&key).unwrap();
        assert_eq!(edited.objects.changes().len(), 1);
        crate::parser::finish_compressed_objects(&mut edited).unwrap();
        assert_eq!(edited.objects.changes().len(), 1);
    }
}
