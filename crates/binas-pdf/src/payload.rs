//! Shared immutable encoded stream payloads.
//!
//! Parsed encoded streams borrow checked ranges of immutable input storage.
//! Replacements own shared allocations; snapshot clones share either form.
//! Object dictionaries, strings and decoded data still allocate. This does not
//! make the complete parser or object store zero-copy.

use binas_core::{Reservation, Source, SourceError, SourceSpan, Span};
use std::{fmt, ops::Deref, sync::Arc};

#[derive(Clone)]
enum Storage {
    Owned(Arc<OwnedPayload>),
    SourceRange { source: Source, span: Span },
}

struct OwnedPayload {
    bytes: Vec<u8>,
    // Shared with the bytes across plans and committed object snapshots.
    _reservations: Vec<Reservation>,
}

#[derive(Clone)]
pub(crate) struct StreamPayload(Storage);

impl fmt::Debug for StreamPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StreamPayload")
            .field("len", &self.len())
            .field("source_span", &self.source_span())
            .finish()
    }
}

impl PartialEq for StreamPayload {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Eq for StreamPayload {}

impl From<Vec<u8>> for StreamPayload {
    fn from(bytes: Vec<u8>) -> Self {
        Self(Storage::Owned(Arc::new(OwnedPayload {
            bytes,
            _reservations: Vec::new(),
        })))
    }
}

impl Deref for StreamPayload {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match &self.0 {
            Storage::Owned(payload) => payload.bytes.as_slice(),
            Storage::SourceRange { source, span } => span
                .slice(source.as_bytes())
                .expect("source payload range checked at construction"),
        }
    }
}

impl AsRef<[u8]> for StreamPayload {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl StreamPayload {
    /// Keeps logical byte reservations alive until the last shared payload dies.
    /// Legacy payload construction remains unaccounted; this constructor is for
    /// callers that reserved the complete encoded output before allocating it.
    pub(crate) fn from_reserved(
        bytes: Vec<u8>,
        reservations: Vec<Reservation>,
    ) -> Result<Self, crate::PdfError> {
        let reserved = reservations.iter().try_fold(0_u64, |total, entry| {
            total
                .checked_add(entry.bytes())
                .ok_or_else(|| crate::PdfError::limit("encoded payload reservations overflow"))
        })?;
        if reserved != bytes.len() as u64 {
            return Err(crate::PdfError::verification(
                "encoded payload reservations do not match its logical byte length",
            ));
        }
        Ok(Self(Storage::Owned(Arc::new(OwnedPayload {
            bytes,
            _reservations: reservations,
        }))))
    }

    /// Validates before sharing; source storage is immutable for this lifetime.
    pub(crate) fn from_source(source: &Source, span: Span) -> Result<Self, SourceError> {
        source.qualified_span(span)?;
        Ok(Self(Storage::SourceRange {
            source: source.clone(),
            span,
        }))
    }

    /// Exact encoded bytes only. An owned replacement has no inferred mapping.
    pub(crate) fn source_span(&self) -> Option<SourceSpan> {
        match &self.0 {
            Storage::Owned(_) => None,
            Storage::SourceRange { source, span } => Some(SourceSpan::new(source.id(), *span)),
        }
    }

    #[cfg(test)]
    pub(crate) fn shares_storage_with(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Storage::Owned(left), Storage::Owned(right)) => Arc::ptr_eq(left, right),
            (
                Storage::SourceRange {
                    source: left,
                    span: left_span,
                },
                Storage::SourceRange {
                    source: right,
                    span: right_span,
                },
            ) => left.shares_storage_with(right) && left_span == right_span,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::StreamPayload;
    use crate::{OpenOptions, PdfEngine, object::ObjectRef};
    use binas_core::{Source, Span};

    // Original generated fixture: two page-content streams, no external corpus.
    fn two_stream_pdf() -> Vec<u8> {
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R >>".as_slice(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            b"<< /Type /Page /Parent 2 0 R /Contents [4 0 R 5 0 R] >>",
            b"<< /Length 3 >>\nstream\none\nendstream",
            b"<< /Length 3 >>\nstream\ntwo\nendstream",
        ];
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            bytes.extend_from_slice(object);
            bytes.extend_from_slice(b"\nendobj\n");
        }
        let xref = bytes.len();
        bytes.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        bytes
    }

    #[test]
    fn ownership_conversion_and_clones_keep_payload_allocation() {
        let bytes = vec![17; 1024];
        let pointer = bytes.as_ptr();
        let payload = StreamPayload::from(bytes);
        assert_eq!(payload.as_ptr(), pointer);
        let cloned = payload.clone();
        assert!(payload.shares_storage_with(&cloned));
        assert_eq!(cloned.as_ptr(), pointer);
        let mut replacement = cloned.to_vec();
        replacement[0] = 99;
        let replacement = StreamPayload::from(replacement);
        assert!(!payload.shares_storage_with(&replacement));
        assert_eq!(payload[0], 17);
        assert_eq!(replacement[0], 99);
        assert!(replacement.source_span().is_none());
    }

    #[test]
    fn source_ranges_are_checked_and_do_not_copy_source_bytes() {
        let source = Source::from_owned(b"prefix-body-suffix".to_vec(), 18).unwrap();
        let range = Span::new(7, 11).unwrap();
        let payload = StreamPayload::from_source(&source, range).unwrap();
        assert_eq!(payload.as_ptr(), source.as_bytes()[7..].as_ptr());
        assert_eq!(payload.as_ref(), b"body");
        let provenance = payload.source_span().unwrap();
        assert_eq!(provenance.source(), source.id());
        assert_eq!(provenance.span(), range);
        assert_eq!(provenance.resolve(&source).unwrap(), b"body");
        assert!(StreamPayload::from_source(&source, Span::new(0, 100).unwrap()).is_err());
        assert_eq!(payload, StreamPayload::from(b"body".to_vec()));
        drop(source);
        assert_eq!(payload.as_ref(), b"body");
    }

    #[test]
    fn repair_xref_and_object_stream_paths_retain_original_encoded_ranges() {
        let inputs = [
            (two_stream_pdf(), OpenOptions { repair: true }),
            (
                include_bytes!("../tests/fixtures/pdf/p4-xref-stream-preserve-text.pdf").to_vec(),
                OpenOptions::default(),
            ),
            (
                include_bytes!("../tests/fixtures/pdf/p4-object-stream-preserve-text.pdf").to_vec(),
                OpenOptions::default(),
            ),
        ];
        for (bytes, options) in inputs {
            let document = PdfEngine::default().open_owned(bytes, options).unwrap();
            let mut streams = 0;
            for object in document.parsed().objects.values() {
                if let Some(payload) = &object.stream {
                    streams += 1;
                    assert_eq!(
                        payload.as_ptr(),
                        document.source()[object.stream_offset..].as_ptr()
                    );
                    assert_eq!(
                        payload.source_span().unwrap().span().len(),
                        payload.len() as u64
                    );
                }
            }
            assert!(streams > 0);
        }
    }

    #[test]
    fn parsed_and_document_clones_share_all_streams_and_replacement_is_isolated() {
        let document = PdfEngine::default()
            .open_owned(two_stream_pdf(), OpenOptions::default())
            .unwrap();
        let cloned_document = document.clone();
        let mut staged = document.parsed().clone();
        let first = ObjectRef {
            number: 4,
            generation: 0,
        };
        let second = ObjectRef {
            number: 5,
            generation: 0,
        };
        for reference in [first, second] {
            let original = document
                .parsed()
                .object(reference)
                .unwrap()
                .stream
                .as_ref()
                .unwrap();
            let cloned = cloned_document
                .parsed()
                .object(reference)
                .unwrap()
                .stream
                .as_ref()
                .unwrap();
            let staged_payload = staged.object(reference).unwrap().stream.as_ref().unwrap();
            let original_object = document.parsed().object(reference).unwrap();
            assert_eq!(
                original.as_ptr(),
                document.source()[original_object.stream_offset..].as_ptr()
            );
            assert_eq!(
                original.source_span().unwrap().span().start(),
                original_object.stream_offset as u64
            );
            assert!(original.shares_storage_with(cloned));
            assert!(original.shares_storage_with(staged_payload));
            assert_eq!(original.as_ptr(), cloned.as_ptr());
        }
        staged.objects.get_mut(&first).unwrap().stream = Some(b"new".to_vec().into());
        let candidate = document.with_parsed(staged);
        assert_eq!(
            document.parsed().object(first).unwrap().stream.as_deref(),
            Some(b"one".as_slice())
        );
        assert_eq!(
            cloned_document
                .parsed()
                .object(first)
                .unwrap()
                .stream
                .as_deref(),
            Some(b"one".as_slice())
        );
        assert_eq!(
            candidate.parsed().object(first).unwrap().stream.as_deref(),
            Some(b"new".as_slice())
        );
        assert!(
            document
                .parsed()
                .object(second)
                .unwrap()
                .stream
                .as_ref()
                .unwrap()
                .shares_storage_with(
                    candidate
                        .parsed()
                        .object(second)
                        .unwrap()
                        .stream
                        .as_ref()
                        .unwrap()
                )
        );
        assert!(
            !document
                .parsed()
                .object(first)
                .unwrap()
                .stream
                .as_ref()
                .unwrap()
                .shares_storage_with(
                    candidate
                        .parsed()
                        .object(first)
                        .unwrap()
                        .stream
                        .as_ref()
                        .unwrap()
                )
        );
    }
}
