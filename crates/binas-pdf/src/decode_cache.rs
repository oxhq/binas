//! Shared decoded results with bounded retention and cumulative work accounting.
//!
//! Live accounting covers logical retained decoded bytes, not allocator spare
//! capacity, graph/key/container overhead, or decoder scratch/intermediate buffers.
//! The decoder must charge actual work through the supplied token. Cache eviction
//! does not refund work or release a payload still held by a caller.
//! Retention caps the sum of payload bytes and logical key/handle/FIFO record
//! bytes to the configured cache byte allowance. This is not an
//! allocator bound: hash-table load factor, spare capacity, Arc headers and
//! payload/Vec bookkeeping remain excluded.

use std::{
    collections::{HashMap, VecDeque},
    ops::Deref,
    sync::{Arc, Mutex, MutexGuard},
};

use binas_core::{BudgetLimits, CancellationToken, OperationContext, Reservation, ResourceBudget};
use sha2::{Digest, Sha256};

use crate::{
    PdfError,
    filters::{DecodeParams, PdfFilter},
    limits::Limits,
};

/// Identity of deterministic decoding inputs. Exact encoded bytes, ordered
/// filters, and every typed decode parameter participate; object numbers do not.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct DecodeKey([u8; 32]);

impl DecodeKey {
    pub(crate) fn new(
        encoded: &[u8],
        filters: &[PdfFilter],
        params: &[Option<DecodeParams>],
    ) -> Self {
        let mut digest = Sha256::new();
        digest.update(b"binas.pdf.decode.v1\0");
        digest.update((encoded.len() as u64).to_le_bytes());
        digest.update(encoded);
        digest.update((filters.len() as u64).to_le_bytes());
        for filter in filters {
            match filter {
                PdfFilter::FlateDecode => digest.update([0]),
                PdfFilter::ASCIIHexDecode => digest.update([1]),
                PdfFilter::ASCII85Decode => digest.update([2]),
                PdfFilter::RunLengthDecode => digest.update([3]),
                PdfFilter::LzwDecode => digest.update([4]),
                PdfFilter::Unsupported(name) => {
                    digest.update([5]);
                    digest.update((name.len() as u64).to_le_bytes());
                    digest.update(name.as_bytes());
                }
            }
        }
        digest.update((params.len() as u64).to_le_bytes());
        for value in params {
            match value {
                None => digest.update([0]),
                Some(value) => {
                    digest.update([
                        1,
                        value.predictor,
                        value.bits_per_component,
                        value.early_change,
                    ]);
                    digest.update((value.colors as u64).to_le_bytes());
                    digest.update((value.columns as u64).to_le_bytes());
                }
            }
        }
        Self(digest.finalize().into())
    }
}

#[derive(Debug)]
struct RetainedPayload {
    bytes: Vec<u8>,
    _reservation: Reservation,
}

#[derive(Clone, Debug)]
pub struct DecodedPayload(Arc<RetainedPayload>);

// One logical map entry and its FIFO entry. Architecture-dependent sizes are
// measured rather than assuming pointer widths; empty results still cost this.
const CACHE_RECORD_BYTES: usize = size_of::<(DecodeKey, DecodedPayload)>() + size_of::<DecodeKey>();

impl Deref for DecodedPayload {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.0.bytes
    }
}

impl AsRef<[u8]> for DecodedPayload {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

/// Managed logical-byte accounting. Work includes decoder byte charges, the
/// encoded input scan, and one unit per public decode attempt. Allocator and
/// spare-capacity/container overhead are excluded. `cache_record_bytes` counts
/// only the occupied logical map key/handle and FIFO key records.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DecodeMetrics {
    pub cache_entries: usize,
    pub cache_bytes: usize,
    pub cache_record_bytes: usize,
    pub live_decoded_bytes: u64,
    pub work_bytes: u64,
    pub hits: u64,
    pub misses: u64,
}

#[derive(Debug)]
struct State {
    entries: HashMap<DecodeKey, DecodedPayload>,
    fifo: VecDeque<DecodeKey>,
    cache_bytes: usize,
    hits: u64,
    misses: u64,
}

impl State {
    fn evict_oldest(&mut self) -> bool {
        let Some(key) = self.fifo.pop_front() else {
            return false;
        };
        if let Some(payload) = self.entries.remove(&key) {
            self.cache_bytes -= payload.len();
        }
        true
    }
}

/// Clones share a document's decode work and cache. Branches may share a result
/// only by exact decoder-input key; security transformations change input bytes.
/// The closure executes under the cache lock and must not reenter this state.
#[derive(Clone, Debug)]
pub(crate) struct DecodeState {
    state: Arc<Mutex<State>>,
    budget: ResourceBudget,
    max_cache_bytes: usize,
    max_cache_entries: usize,
}

impl DecodeState {
    pub(crate) fn new(limits: &Limits) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                entries: HashMap::new(),
                fifo: VecDeque::new(),
                cache_bytes: 0,
                hits: 0,
                misses: 0,
            })),
            budget: ResourceBudget::new(BudgetLimits {
                max_live_bytes: limits.max_decode_live_bytes as u64,
                max_work_units: limits.max_decode_work_bytes as u64,
            }),
            max_cache_bytes: limits.max_decode_cache_bytes,
            max_cache_entries: limits
                .max_container_items
                .min(limits.max_decode_cache_bytes / CACHE_RECORD_BYTES),
        }
    }

    pub(crate) fn decode(
        &self,
        key: DecodeKey,
        max_output: usize,
        cancellation: &CancellationToken,
        decode: impl FnOnce(&OperationContext, usize) -> Result<Vec<u8>, PdfError>,
    ) -> Result<DecodedPayload, PdfError> {
        let context = OperationContext::new(self.budget.clone(), cancellation.clone());
        context.checkpoint().map_err(context_error)?;
        let mut state = self.lock();
        context.checkpoint().map_err(context_error)?;
        if let Some(payload) = state.entries.get(&key) {
            if payload.len() > max_output {
                return Err(PdfError::limit(
                    "cached decoded stream exceeds caller output limit",
                ));
            }
            let payload = payload.clone();
            context.checkpoint().map_err(context_error)?;
            state.hits = state.hits.saturating_add(1);
            return Ok(payload);
        }
        state.misses = state.misses.saturating_add(1);
        // Remove cache references first, then give the decoder the available
        // logical allowance. A retained caller result stays charged; a small new
        // result can still fit even when the caller's configured ceiling does not.
        while (max_output as u64)
            > self.budget.limits().max_live_bytes - self.budget.snapshot().live_bytes
        {
            context.checkpoint().map_err(context_error)?;
            if !state.evict_oldest() {
                break;
            }
        }
        let available = self.budget.limits().max_live_bytes - self.budget.snapshot().live_bytes;
        let effective_output_limit =
            max_output.min(usize::try_from(available).unwrap_or(usize::MAX));
        let ceiling = context
            .reserve(effective_output_limit as u64)
            .map_err(context_error)?;
        let bytes = decode(&context, effective_output_limit)?;
        context.checkpoint().map_err(context_error)?;
        if bytes.len() > effective_output_limit {
            return Err(PdfError::limit(
                "decoder result exceeds caller output limit",
            ));
        }
        // No concurrent decode can reserve while this state lock is held, so
        // replacing a ceiling reservation with its smaller used amount is safe.
        drop(ceiling);
        let reservation = self
            .budget
            .reserve(bytes.len() as u64)
            .map_err(|error| PdfError::limit(format!("live decoded bytes: {error}")))?;
        let payload = DecodedPayload(Arc::new(RetainedPayload {
            bytes,
            _reservation: reservation,
        }));
        if self.max_cache_bytes != 0
            && self.max_cache_entries != 0
            && payload.len() <= self.max_cache_bytes - CACHE_RECORD_BYTES
        {
            while state.entries.len() >= self.max_cache_entries
                || state.cache_bytes + state.entries.len() * CACHE_RECORD_BYTES
                    > self.max_cache_bytes - CACHE_RECORD_BYTES - payload.len()
            {
                context.checkpoint().map_err(context_error)?;
                if !state.evict_oldest() {
                    break;
                }
            }
            context.checkpoint().map_err(context_error)?;
            state.cache_bytes += payload.len();
            state.fifo.push_back(key);
            state.entries.insert(key, payload.clone());
        }
        Ok(payload)
    }

    /// Charge before metadata parsing as well as inside decoder work. The public
    /// parser charges encoded input length plus one request unit even for hits
    /// and malformed filter lists.
    pub(crate) fn charge_work(
        &self,
        bytes: usize,
        cancellation: &CancellationToken,
    ) -> Result<(), PdfError> {
        OperationContext::new(self.budget.clone(), cancellation.clone())
            .charge_work(bytes as u64)
            .map_err(context_error)
    }

    pub(crate) fn clear_cache(&self) {
        let mut state = self.lock();
        state.entries.clear();
        state.fifo.clear();
        state.cache_bytes = 0;
    }

    pub(crate) fn metrics(&self) -> DecodeMetrics {
        let state = self.lock();
        let counters = self.budget.snapshot();
        DecodeMetrics {
            cache_entries: state.entries.len(),
            cache_bytes: state.cache_bytes,
            cache_record_bytes: state.entries.len() * CACHE_RECORD_BYTES,
            live_decoded_bytes: counters.live_bytes,
            work_bytes: counters.work_units,
            hits: state.hits,
            misses: state.misses,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn context_error(error: binas_core::ContextError) -> PdfError {
    PdfError::from_context(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PdfErrorCode;

    fn state(cache: usize, live: usize, work: usize) -> DecodeState {
        DecodeState::new(&Limits {
            max_decode_cache_bytes: cache,
            max_decode_live_bytes: live,
            max_decode_work_bytes: work,
            ..Limits::default()
        })
    }
    fn key(bytes: &[u8]) -> DecodeKey {
        DecodeKey::new(bytes, &[], &[])
    }

    #[test]
    fn hit_reuses_allocation_and_obeys_the_current_call_limit() {
        let state = state(CACHE_RECORD_BYTES + 16, 16, 16);
        let first = state
            .decode(key(b"input"), 8, &CancellationToken::new(), |work, _| {
                work.charge_work(5).map_err(context_error)?;
                Ok(b"decoded".to_vec())
            })
            .unwrap();
        let second = state
            .clone()
            .decode(key(b"input"), 8, &CancellationToken::new(), |_, _| {
                panic!("cache hit must not decode")
            })
            .unwrap();
        assert_eq!(first.as_ptr(), second.as_ptr());
        assert_eq!(state.metrics().work_bytes, 5);
        assert_eq!(state.metrics().hits, 1);
        assert_eq!(
            state
                .decode(key(b"input"), 6, &CancellationToken::new(), |_, _| panic!(
                    "oversize cache hit must not decode"
                ))
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(state.metrics().live_decoded_bytes, 7);
    }

    #[test]
    fn fifo_eviction_never_refunds_work_and_held_payload_remains_charged() {
        let state = state(CACHE_RECORD_BYTES + 4, 12, 9);
        let retained = state
            .decode(key(b"a"), 4, &CancellationToken::new(), |work, _| {
                work.charge_work(4).map_err(context_error)?;
                Ok(vec![1; 4])
            })
            .unwrap();
        let second = state
            .decode(key(b"b"), 4, &CancellationToken::new(), |work, _| {
                work.charge_work(4).map_err(context_error)?;
                Ok(vec![2; 4])
            })
            .unwrap();
        assert_eq!(state.metrics().cache_entries, 1);
        assert_eq!(
            state.metrics().cache_bytes + state.metrics().cache_record_bytes,
            CACHE_RECORD_BYTES + 4
        );
        assert_eq!(state.metrics().live_decoded_bytes, 8);
        state.clear_cache();
        assert_eq!(state.metrics().cache_bytes, 0);
        assert_eq!(state.metrics().cache_record_bytes, 0);
        assert_eq!(state.metrics().live_decoded_bytes, 8);
        drop(second);
        assert_eq!(state.metrics().live_decoded_bytes, 4);
        assert_eq!(
            state
                .decode(key(b"a"), 4, &CancellationToken::new(), |work, _| {
                    work.charge_work(4).map_err(context_error)?;
                    Ok(vec![1; 4])
                })
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(state.metrics().work_bytes, 8);
        assert_eq!(retained.as_ref(), &[1; 4]);
        drop(retained);
        assert_eq!(state.metrics().live_decoded_bytes, 0);
    }

    #[test]
    fn failed_results_are_not_cached_but_partial_work_remains_spent() {
        let state = state(CACHE_RECORD_BYTES + 8, 8, 8);
        assert!(
            state
                .decode(key(b"broken"), 8, &CancellationToken::new(), |work, _| {
                    work.charge_work(3).map_err(context_error)?;
                    Err(PdfError::syntax("broken", 0))
                })
                .is_err()
        );
        assert_eq!(state.metrics().work_bytes, 3);
        assert_eq!(state.metrics().cache_entries, 0);
        assert_eq!(state.metrics().live_decoded_bytes, 0);
        let result = state
            .decode(key(b"broken"), 8, &CancellationToken::new(), |work, _| {
                work.charge_work(3).map_err(context_error)?;
                Ok(vec![7])
            })
            .unwrap();
        assert_eq!(result.as_ref(), &[7]);
        assert_eq!(state.metrics().work_bytes, 6);
    }

    #[test]
    fn decoder_receives_remaining_live_allowance_before_allocation() {
        let state = state(0, 6, 100);
        let token = CancellationToken::new();
        let held = state
            .decode(key(b"one"), 4, &token, |_, limit| {
                assert_eq!(limit, 4);
                Ok(vec![1; 4])
            })
            .unwrap();
        let small = state
            .decode(key(b"two"), 100, &token, |_, limit| {
                assert_eq!(limit, 2);
                Ok(vec![2; 2])
            })
            .unwrap();
        assert_eq!(state.metrics().live_decoded_bytes, 6);
        assert!(
            state
                .decode(key(b"three"), 100, &token, |_, limit| {
                    assert_eq!(limit, 0);
                    Err(PdfError::limit("no room"))
                })
                .is_err()
        );
        drop(small);
        assert_eq!(state.metrics().live_decoded_bytes, 4);
        drop(held);
        assert!(
            state
                .decode(key(b"two"), 4, &token, |_, limit| {
                    assert_eq!(limit, 4);
                    Ok(vec![2; 4])
                })
                .is_ok()
        );
    }

    #[test]
    fn cancellation_before_hits_and_after_decode_does_not_publish_results() {
        let state = state(2 * CACHE_RECORD_BYTES + 8, 8, 100);
        let token = CancellationToken::new();
        state
            .decode(key(b"cached"), 4, &token, |_, _| Ok(vec![1; 4]))
            .unwrap();
        token.cancel();
        assert!(
            state
                .decode(key(b"cached"), 4, &token, |_, _| panic!(
                    "cancelled cache hit"
                ))
                .is_err()
        );
        let fresh = CancellationToken::new();
        assert!(
            state
                .decode(key(b"cancelled"), 4, &fresh, |context, _| {
                    context.charge_work(3).map_err(context_error)?;
                    fresh.cancel();
                    Ok(vec![2; 4])
                })
                .is_err()
        );
        assert_eq!(state.metrics().cache_entries, 1);
        assert_eq!(state.metrics().work_bytes, 3);
        assert_eq!(state.metrics().live_decoded_bytes, 4);
        assert!(state.charge_work(1, &fresh).is_err());
        state.charge_work(1, &CancellationToken::new()).unwrap();
        assert_eq!(state.metrics().work_bytes, 4);
    }

    #[test]
    fn empty_results_have_bounded_entry_count() {
        let state = DecodeState::new(&Limits {
            max_decode_cache_bytes: 2 * CACHE_RECORD_BYTES + 1,
            max_decode_live_bytes: 1,
            max_decode_work_bytes: 100,
            ..Limits::default()
        });
        for bytes in [b"a", b"b", b"c"] {
            state
                .decode(key(bytes), 0, &CancellationToken::new(), |_, _| {
                    Ok(Vec::new())
                })
                .unwrap();
        }
        assert_eq!(state.metrics().cache_entries, 2);
        assert_eq!(state.metrics().cache_bytes, 0);
        assert_eq!(state.metrics().cache_record_bytes, 2 * CACHE_RECORD_BYTES);
    }

    #[test]
    fn metadata_and_payload_share_one_retention_ceiling() {
        let state = state(CACHE_RECORD_BYTES + 3, 20, 100);
        let token = CancellationToken::new();
        let oversized = state
            .decode(key(b"oversized"), 4, &token, |_, _| Ok(vec![1; 4]))
            .unwrap();
        assert_eq!(state.metrics().cache_entries, 0);
        let retained = state
            .decode(key(b"fits"), 3, &token, |_, _| Ok(vec![2; 3]))
            .unwrap();
        assert_eq!(
            state.metrics().cache_bytes + state.metrics().cache_record_bytes,
            CACHE_RECORD_BYTES + 3
        );
        assert_eq!(state.metrics().live_decoded_bytes, 7);
        state.clear_cache();
        assert_eq!(state.metrics().cache_record_bytes, 0);
        assert_eq!(state.metrics().live_decoded_bytes, 7);
        drop((oversized, retained));
        assert_eq!(state.metrics().live_decoded_bytes, 0);
        let tiny = super::DecodeState::new(&Limits {
            max_decode_cache_bytes: CACHE_RECORD_BYTES - 1,
            ..Limits::default()
        });
        tiny.decode(key(b"empty"), 0, &token, |_, _| Ok(Vec::new()))
            .unwrap();
        assert_eq!(tiny.metrics().cache_entries, 0);
    }

    #[test]
    fn concurrent_misses_publish_one_shared_result() {
        use std::sync::{
            Barrier,
            atomic::{AtomicUsize, Ordering},
        };
        let state = state(CACHE_RECORD_BYTES + 4, 4, 100);
        let start = Arc::new(Barrier::new(4));
        let calls = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let state = state.clone();
                let start = start.clone();
                let calls = calls.clone();
                std::thread::spawn(move || {
                    start.wait();
                    state
                        .decode(key(b"same"), 4, &CancellationToken::new(), |context, _| {
                            calls.fetch_add(1, Ordering::SeqCst);
                            context.charge_work(4).map_err(context_error)?;
                            Ok(vec![1; 4])
                        })
                        .unwrap()
                })
            })
            .collect();
        let payloads: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(
            payloads
                .iter()
                .all(|payload| payload.as_ptr() == payloads[0].as_ptr())
        );
        assert_eq!(state.metrics().hits, 3);
        assert_eq!(state.metrics().work_bytes, 4);
        assert_eq!(state.metrics().live_decoded_bytes, 4);
    }

    #[test]
    fn cache_keys_cover_input_filter_order_and_all_decode_parameters() {
        let params = DecodeParams::default();
        let base = DecodeKey::new(b"x", &[PdfFilter::FlateDecode], &[Some(params)]);
        assert_ne!(
            base,
            DecodeKey::new(b"y", &[PdfFilter::FlateDecode], &[Some(params)])
        );
        assert_ne!(
            base,
            DecodeKey::new(b"x", &[PdfFilter::LzwDecode], &[Some(params)])
        );
        assert_ne!(
            base,
            DecodeKey::new(b"x", &[PdfFilter::FlateDecode], &[None])
        );
        for changed in [
            DecodeParams {
                predictor: 2,
                ..params
            },
            DecodeParams {
                colors: 3,
                ..params
            },
            DecodeParams {
                bits_per_component: 1,
                ..params
            },
            DecodeParams {
                columns: 2,
                ..params
            },
            DecodeParams {
                early_change: 0,
                ..params
            },
        ] {
            assert_ne!(
                base,
                DecodeKey::new(b"x", &[PdfFilter::FlateDecode], &[Some(changed)])
            );
        }
        assert_ne!(
            DecodeKey::new(
                b"x",
                &[PdfFilter::FlateDecode, PdfFilter::ASCII85Decode],
                &[None, None]
            ),
            DecodeKey::new(
                b"x",
                &[PdfFilter::ASCII85Decode, PdfFilter::FlateDecode],
                &[None, None]
            )
        );
        assert_ne!(
            DecodeKey::new(b"x", &[PdfFilter::Unsupported("ab".into())], &[None]),
            DecodeKey::new(
                b"x",
                &[
                    PdfFilter::Unsupported("a".into()),
                    PdfFilter::Unsupported("b".into())
                ],
                &[None, None]
            )
        );
    }
}
