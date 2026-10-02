// SPDX-License-Identifier: Apache-2.0
// Identical public raw-session contract in Node and Chromium. External oracles
// qualify the native bytes separately; these assertions establish exact parity.
export function verifyProducerSessions(api, fixtures, expected) {
  let assertions = 0;
  const check = (value, message) => { assertions++; if (!value) throw new Error(message); };
  const rejects = (code, action) => {
    let observed;
    try { action(); } catch (error) { observed = error?.code; }
    check(observed === code, `expected ${code}, observed ${observed}`);
  };
  const same = (left, right) => left.length === right.length && left.every((value, index) => value === right[index]);
  for (const mode of ['compact', 'incremental']) {
    const session = new api.BinasPdfSession(fixtures['pypdf-raw'], 24 * 1024 * 1024, 128 * 1024 * 1024);
    const before = session.snapshot();
    check(JSON.parse(session.extractPageText(0)).spans[0].text === 'ALPHA', 'original producer token');
    const plan = session.prepareTextEdit('ALPHA', 'OMEGA-LONG', 0, mode, 1);
    check(session.snapshot() === before, 'prepare must not publish');
    const receipt = JSON.parse(session.acceptTextEdit(plan));
    check(receipt.changed_objects.length === 1 && receipt.changed_objects[0].number === expected.target[0], 'declared target only');
    check(JSON.parse(session.extractPageText(0)).spans.map(span => span.text).join('|') === 'OMEGA-LONG|KEEP', 'current target and retained KEEP');
    check(JSON.parse(session.extractPageText(1)).spans.map(span => span.text).join('|') === 'UNTOUCHED PAGE', 'untouched page');
    check(same(session.export(mode), expected[mode]), `${mode} exact native independently-qualified bytes`);
    session.close(); plan.free(); session.free();
  }
  for (const name of ['pypdf-fonted', 'pypdf-shared', 'pypdf-missing-glyph', 'reportlab-truetype', 'reportlab-standard']) {
    const session = new api.BinasPdfSession(fixtures[name], 24 * 1024 * 1024, 128 * 1024 * 1024);
    const before = session.snapshot();
    check(JSON.parse(session.extractPageText(0)).spans.some(span => span.text === 'ALPHA'), `${name} selected producer token`);
    rejects('unsupported_feature', () => session.prepareTextEdit('ALPHA', 'OMEGA-LONG', 0, 'compact', 1));
    check(session.snapshot() === before, `${name} raw refusal atomic`);
    session.close(); session.free();
  }
  rejects('invalid_syntax', () => new api.BinasPdfSession(fixtures['malformed-header'], 24 * 1024 * 1024, 128 * 1024 * 1024));
  return { assertions, raw_exports: ['compact', 'incremental'], fonted_raw_refusals: 5, parse_refusals: 1,
    fonted_page_text_js_implemented: false, external_oracles_executed_by_runtime: false };
}
