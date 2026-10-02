// SPDX-License-Identifier: Apache-2.0
// Shared public JS workload; observer owns the runtime-specific memory metrics.
export function workload(api, input, scenario, observe) {
  const phases = []; const outputs = [];
  let openCount = 0, inputTransferBytes = 0, outputTransferBytes = 0, error = null, errorMessage = null;
  const point = phase => phases.push({phase, ...observe()});
  function open(bytes) {
    openCount++; inputTransferBytes += bytes.length;
    return new api.BinasPdfSession(bytes, scenario === 'live-refusal' ? 1 : (scenario.startsWith('output') ? 48 : 24) * 1024 * 1024, 256 * 1024 * 1024);
  }
  function close(session) { session.close(); session.free(); }
  let session;
  point('before-input-call');
  try {
    session = open(input); point('open');
    if (scenario === 'cancel-refusal') session.cancel();
    if (scenario.startsWith('output')) {
      const refused = scenario === 'output-refusal';
      const replacement = 'A'.repeat(refused ? 8 * 1024 * 1024 - 4 : 1024 * 1024);
      const mode = refused ? 'incremental' : 'compact';
      const plan = session.prepareTextEdit('ALPHA', replacement, 0, mode, 1); point('prepare');
      if (refused) throw new Error('expected output limit refusal');
      session.acceptTextEdit(plan); plan.free(); point('accept');
      const bytes = session.export(mode); outputTransferBytes += bytes.length; outputs.push(bytes); point('export');
      if (JSON.parse(session.extractPageText(0)).spans[0].text.length !== 1024 * 1024) throw new Error('incorrect growing output text');
    } else if (scenario.endsWith('refusal')) {
      const before = session.snapshot();
      try {
        if (scenario === 'font-refusal') session.prepareTextEdit('ALPHA', 'OMEGA', 0, 'compact', 1);
        else session.extractPageText(0);
        throw new Error('expected refusal');
      } catch (failure) { if (!failure.code) throw failure; error = failure.code; }
      if (session.snapshot() !== before) throw new Error('refusal published');
      point('refused');
    } else if (scenario.startsWith('edit')) {
      const steps = [['ALPHA', 'OMEGA'], ['OMEGA', 'SIGMA'], ['SIGMA', 'DELTA'], ['DELTA', 'OMEGA']];
      for (const [index, [old, replacement]] of steps.entries()) {
        const plan = session.prepareTextEdit(old, replacement, 0, 'compact', 1); point('prepare');
        session.acceptTextEdit(plan); plan.free(); point('accept');
        if (scenario === 'edit-reopen' && index < 3) {
          const bytes = session.export('compact'); outputTransferBytes += bytes.length;
          close(session); session = open(bytes); point('reopen');
        }
      }
      const bytes = session.export('compact'); outputTransferBytes += bytes.length; outputs.push(bytes);
      if (JSON.parse(session.extractPageText(0)).spans[0].text !== 'OMEGA') throw new Error('incorrect final text');
      point('export');
    } else {
      for (let index = 0; index < (scenario === 'read-once' ? 1 : 32); index++) {
        if (JSON.parse(session.extractPageText(0)).spans[0].text !== 'ALPHA') throw new Error('incorrect read');
        if (scenario === 'read-reopen' && index < 31) { close(session); session = open(input); }
      }
      point('reads');
    }
  } catch (failure) { if (!failure.code) throw failure; error = failure.code; errorMessage = failure.message; if(session && JSON.parse(session.snapshot()).revision !== 0) throw new Error('refusal published'); point('refused'); }
  finally { if (session) { close(session); point('closed-freed'); } }
  const expected = {'live-refusal': 'resource_limit', 'input-refusal': 'resource_limit', 'decode-refusal': 'resource_limit', 'output-refusal': 'resource_limit',
    'invalid-refusal': 'invalid_syntax', 'font-refusal': 'unsupported_feature', 'cancel-refusal': 'cancelled'}[scenario] || null;
  if (error !== expected) throw new Error(`expected ${expected}, got ${error}`);
  if (scenario === 'output-refusal' && !errorMessage?.includes('output exceeds max_output_bytes')) throw new Error('incorrect output-limit refusal mechanism');
  return {case: scenario, input_bytes: input.length, error, error_message: errorMessage, open_count: openCount, input_transfer_bytes: inputTransferBytes,
    output_transfer_bytes: outputTransferBytes, output_sizes: outputs.map(bytes => bytes.length), phases, outputs};
}
