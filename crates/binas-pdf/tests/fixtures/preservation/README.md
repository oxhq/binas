# Mandatory preservation boundaries

These seven original generated fixtures are licensed Apache-2.0 under the repository license. `generate.py` records their construction; `manifest.json` pins their SHA-256 hashes, origin, license and expected effects. Regeneration is an explicit maintainer action. Tests always load the checked-in PDFs and fail on a missing file, unknown expectation, changed hash, duplicate entry or reduced mandatory set. No external corpus or environment flag is needed.

Metadata (including trailer Info), outlines, named destinations, page labels and Unicode-bearing tags exercise refusal of unrequested catalog/trailer loss during extraction. Explicitly requested entry removals must be listed in the operation report. The tag fixture contains UTF-16BE alternative text with accented Latin, Chinese and a supplementary-plane character; this tests the refusal boundary, not Unicode tag serialization support.

Shared and separate Type3 resource fixtures are positive controls for reversed extraction. Tests compare page text and the retained glyph program through public APIs. They do not establish visual equivalence, all-owner mutation safety, or broad Type3 compatibility. These small original fixtures are not the competitor's reproduced bug corpus or independent renderer evidence.
