# Fuzz smoke gate

`fuzz-smoke.yml` discovers every target with `cargo fuzz list`, then builds and
runs each with AddressSanitizer for 60 seconds and a five-second per-input
timeout. It is a deterministic crash-regression smoke for the checked-out
revision.

It is not release-duration fuzzing and does not prove corpus quality,
performance or memory baselines, or hosted execution until the GitHub job runs.
