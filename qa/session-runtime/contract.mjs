// The same public JS calls run against generated Node and browser artifacts.
export function verifySessionExports(api, fixtures) {
  let assertions = 0;
  function check(condition, message) {
    assertions++;
    if (!condition) throw new Error(message);
  }
  function rejects(code, action) {
    let observed;
    try { action(); } catch (error) { observed = error?.code; }
    check(observed === code, `expected ${code}, got ${observed}`);
  }
  function same(left, right) {
    return left.length === right.length && left.every((value, index) => value === right[index]);
  }
  const live = 24 * 1024 * 1024;
  const work = 256 * 1024 * 1024;
  const completed = [];
  for (const shape of ['plain', 'flate']) {
    const input = fixtures[`raw-${shape}`];
    for (const mode of ['incremental', 'compact']) {
      const session = new api.BinasPdfSession(input, live, work);
      const other = new api.BinasPdfSession(input, live, work);
      const before = session.snapshot();
      check(before === other.snapshot(), 'same-source initial snapshot');
      const text = JSON.parse(session.extractPageText(0));
      check(text.warnings.length === 0 && text.spans[0].text === 'ALPHA', 'selected text');
      const plan = session.prepareTextEdit('ALPHA', 'OMEGA', 0, mode, 1);
      check(session.snapshot() === before, 'prepare does not publish');
      rejects('foreign_plan', () => other.acceptTextEdit(plan));
      rejects('plan_pending', () => session.prepareTextEdit('ALPHA', 'OTHER', 0, mode, 1));
      const budget = session.budget();
      session.cancel();
      rejects('cancelled', () => session.acceptTextEdit(plan));
      rejects('cancelled', () => session.extractPageText(0));
      rejects('cancelled', () => session.export(mode));
      check(session.budget() === budget && session.snapshot() === before, 'cancel rollback counters');
      session.renewCancellation();
      const receipt = JSON.parse(session.acceptTextEdit(plan));
      check(receipt.after.revision === 1 && receipt.before.revision === 0, 'revision step');
      check(receipt.operation === 'pdf.text.retained_ascii' && receipt.contract_version === 1, 'contract');
      check(receipt.mode === mode && receipt.request.old_text === 'ALPHA' && receipt.request.replacement === 'OMEGA' && receipt.request.match_index === 0, 'bound request and writer');
      check(receipt.required_checks.length === 18 && receipt.changed_objects.length === 1, 'native required/check/change equivalence');
      check(receipt.pdf_limits.max_input_bytes === 8 * 1024 * 1024 && receipt.pdf_limits.max_output_bytes === 16 * 1024 * 1024, 'effective PDF limits');
      check(receipt.context_limits.max_live_bytes === live && receipt.context_limits.max_work_units === work, 'effective logical limits');
      check(receipt.evidence.filter(entry => entry.status === 'passed').length === 18, 'required observations');
      check(receipt.evidence.filter(entry => entry.status === 'not_checked').length === 3, 'scope exclusions');
      rejects('stale_plan', () => session.acceptTextEdit(plan));
      check(JSON.parse(session.extractPageText(0)).spans[0].text === 'OMEGA', 'current selected read');
      const exported = session.export(mode);
      check(same(exported, fixtures[`p3a-${shape}-${mode}`]), `P3a bytes ${shape}/${mode}`);
      check(same(session.export(mode), exported), 'repeated deterministic export');
      check(JSON.parse(session.budget()).live_bytes === input.length, 'only retained input reservation');
      session.close(); session.close();
      check(JSON.parse(session.budget()).live_bytes === 0, 'close releases reservations');
      rejects('session_closed', () => session.acceptTextEdit(plan));
      rejects('session_closed', () => session.extractPageText(0));
      rejects('session_closed', () => session.export(mode));
      rejects('session_closed', () => session.renewCancellation());
      plan.free(); session.free(); other.close(); other.free();
      completed.push(`${shape}/${mode}`);
    }
  }
  const session = new api.BinasPdfSession(fixtures['raw-plain'], live, work);
  rejects('contract_version', () => session.prepareTextEdit('ALPHA', 'OMEGA', 0, 'compact', 99));
  rejects('unsupported_writer_mode', () => session.prepareTextEdit('ALPHA', 'OMEGA', 0, 'surgical', 1));
  const released = session.prepareTextEdit('ALPHA', 'OMEGA', 0, 'compact', 1);
  session.releasePlan(released);
  const next = session.prepareTextEdit('ALPHA', 'OMEGA', 0, 'compact', 1);
  rejects('plan_released', () => session.acceptTextEdit(released));
  session.close();
  check(JSON.parse(session.budget()).live_bytes === 0, 'close pending plan with external token');
  rejects('session_closed', () => session.acceptTextEdit(next));
  released.free(); next.free(); session.free();
  const fonted = new api.BinasPdfSession(fixtures['fonted-plain'], live, work);
  rejects('unsupported_feature', () => fonted.prepareTextEdit('ALPHA', 'OMEGA-LONG', 0, 'compact', 1));
  check(JSON.parse(fonted.snapshot()).revision === 0, 'fonted JS request refused atomically');
  fonted.close(); fonted.free();
  rejects('resource_limit', () => new api.BinasPdfSession(fixtures['raw-plain'], 1, work));
  rejects('resource_limit', () => new api.BinasPdfSession(fixtures['raw-plain'], live, 1));
  rejects('resource_limit', () => new api.BinasPdfSession(new Uint8Array(8 * 1024 * 1024 + 1), live, work));
  const limited = new api.BinasPdfSession(fixtures['raw-plain'], fixtures['raw-plain'].length, work);
  rejects('resource_limit', () => limited.prepareTextEdit('ALPHA', 'OMEGA', 0, 'compact', 1));
  rejects('resource_limit', () => limited.export('compact'));
  check(JSON.parse(limited.snapshot()).revision === 0, 'live budget failure atomic');
  limited.close(); limited.free();
  const probe = new api.BinasPdfSession(fixtures['raw-flate'], live, work);
  const probePlan = probe.prepareTextEdit('ALPHA', 'OMEGA', 0, 'compact', 1);
  const preparationWork = JSON.parse(probe.budget()).work_units;
  probe.close(); probePlan.free(); probe.free();
  for (const extra of [0, 1]) {
    const exhausted = new api.BinasPdfSession(fixtures['raw-flate'], live, preparationWork + extra);
    const before = exhausted.snapshot();
    const plan = exhausted.prepareTextEdit('ALPHA', 'OMEGA', 0, 'compact', 1);
    rejects('resource_limit', () => exhausted.acceptTextEdit(plan));
    check(exhausted.snapshot() === before, 'acceptance budget rollback');
    if (extra === 0) {
      rejects('plan_pending', () => exhausted.prepareTextEdit('ALPHA', 'OTHER', 0, 'compact', 1));
      exhausted.releasePlan(plan);
    } else {
      rejects('plan_released', () => exhausted.acceptTextEdit(plan));
    }
    check(JSON.parse(exhausted.budget()).live_bytes === fixtures['raw-flate'].length, 'failed acceptance releases output reservation');
    exhausted.close(); plan.free(); exhausted.free();
  }
  if (fixtures['decoded-too-large']) {
    const oversized = new api.BinasPdfSession(fixtures['decoded-too-large'], live, work);
    rejects('resource_limit', () => oversized.extractPageText(0));
    check(JSON.parse(oversized.snapshot()).revision === 0, '16 MiB decode refusal retains revision');
    oversized.close(); oversized.free();
  }
  return { assertions, completed, profile: 'raw P2b JS contract 1; P3a frozen byte parity', cancellation: 'pre-call cooperative only' };
}
