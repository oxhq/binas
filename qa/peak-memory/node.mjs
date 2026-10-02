// SPDX-License-Identifier: Apache-2.0
import fs from 'node:fs';
import {createRequire} from 'node:module';
import {createInterface} from 'node:readline';
import {workload} from './workload.mjs';
const [artifact, scenario, fixture, output] = process.argv.slice(2);
// Observe the generated module's instance without editing the binding or engine.
let memory;
WebAssembly.Instance = new Proxy(WebAssembly.Instance, {construct(Target, args) {
  const instance = Reflect.construct(Target, args); memory = instance.exports.memory; return instance;
}});
const api = createRequire(import.meta.url)(artifact);
if (!(memory instanceof WebAssembly.Memory)) throw new Error('WASM memory export missing');
function observe() { return {wasm_linear_bytes: memory.buffer.byteLength, ...process.memoryUsage()}; }
const lines = createInterface({input: process.stdin});
console.log(JSON.stringify({ready: true, initial: observe()}));
await new Promise(resolve => lines.once('line', resolve));
const input = fs.readFileSync(fixture);
const result = workload(api, input, scenario, observe);
if (result.outputs.length) fs.writeFileSync(output, result.outputs[0]);
delete result.outputs;
console.log(JSON.stringify({done: true, ...result, final: observe(), os_max_rss_bytes: process.resourceUsage().maxRSS * 1024}));
await new Promise(resolve => lines.once('line', resolve));
lines.close();
