import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import { createRequire } from 'node:module';
import { spawn } from 'node:child_process';
import { deflateSync } from 'node:zlib';
import { verifySessionExports } from './contract.mjs';

const [artifactRoot, sourceRoot] = process.argv.slice(2);
if (!artifactRoot || !sourceRoot) throw new Error('usage: node run.mjs ARTIFACT_ROOT SOURCE_ROOT');
const fixtures = {};
for (const shape of ['plain', 'flate']) {
  fixtures[`raw-${shape}`] = fs.readFileSync(path.join(sourceRoot, `crates/binas-pdf/tests/fixtures/retained-ascii/retained-ascii-${shape}.pdf`));
  fixtures[`fonted-${shape}`] = fs.readFileSync(path.join(sourceRoot, `crates/binas-pdf/tests/fixtures/fonted-text/fonted-text-${shape}.pdf`));
  for (const mode of ['incremental', 'compact']) {
    fixtures[`p3a-${shape}-${mode}`] = fs.readFileSync(path.join(sourceRoot, `bindings/binas-pdf-wasm/tests/fixtures/sessions/p3a-${shape}-${mode}.pdf`));
  }
}
// Original synthetic fixture: tiny encoded input, stream decoded one byte over
// the named 16 MiB limit. No external library or Binas creates the input.
const payload = deflateSync(Buffer.concat([Buffer.from('BT (ALPHA) Tj ET\n'), Buffer.alloc(16 * 1024 * 1024 + 1 - 17, 32)]));
const objects = [
  '<< /Type /Catalog /Pages 2 0 R >>',
  '<< /Type /Pages /Count 1 /Kids [3 0 R] >>',
  '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R >>',
  Buffer.concat([Buffer.from(`<< /Filter /FlateDecode /Length ${payload.length} >>\nstream\n`), payload, Buffer.from('\nendstream')]),
];
const chunks = [Buffer.from('%PDF-1.4\n')], offsets = [0];
for (const [index, object] of objects.entries()) {
  offsets.push(chunks.reduce((sum, chunk) => sum + chunk.length, 0));
  chunks.push(Buffer.from(`${index + 1} 0 obj\n`), Buffer.from(object), Buffer.from('\nendobj\n'));
}
const xref = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
chunks.push(Buffer.from(`xref\n0 5\n0000000000 65535 f \n${offsets.slice(1).map(offset => `${String(offset).padStart(10, '0')} 00000 n \n`).join('')}trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`));
fixtures['decoded-too-large'] = Buffer.concat(chunks);
const require = createRequire(import.meta.url);
const nodeApi = require(path.join(artifactRoot, 'pkg-node/binas_pdf_wasm.js'));
const nodeResult = verifySessionExports(nodeApi, fixtures);
console.log(JSON.stringify({ runtime: process.version, node: nodeResult }));

const server = http.createServer((request, response) => {
  const name = new URL(request.url, 'http://localhost').pathname;
  let data; let mime = 'application/octet-stream';
  if (name === '/') {
    mime = 'text/html';
    data = `<script type="module">
      import init, * as api from '/binas_pdf_wasm.js';
      import {verifySessionExports} from '/contract.mjs';
      try { await init(); const names=${JSON.stringify(Object.keys(fixtures))};
        const fixtures=Object.fromEntries(await Promise.all(names.map(async name=>[name,new Uint8Array(await (await fetch('/fixture/'+name)).arrayBuffer())])));
        window.report={ok:true,result:verifySessionExports(api,fixtures)};
      } catch(error) {window.report={ok:false,error:String(error),stack:error?.stack};}
      </script>`;
  } else if (name === '/contract.mjs') {
    mime = 'text/javascript'; data = fs.readFileSync(new URL('./contract.mjs', import.meta.url));
  } else if (name === '/binas_pdf_wasm.js' || name === '/binas_pdf_wasm_bg.wasm') {
    mime = name.endsWith('.js') ? 'text/javascript' : 'application/wasm';
    data = fs.readFileSync(path.join(artifactRoot, 'pkg-web', name.slice(1)));
  } else if (name.startsWith('/fixture/') && fixtures[name.slice(9)]) {
    data = fixtures[name.slice(9)];
  } else { response.writeHead(404); response.end(); return; }
  response.writeHead(200, { 'Content-Type': mime }); response.end(data);
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const driverPath = 'C:/Users/garae/AppData/Local/.wasm-pack/chromedriver-a2cbe477b224ae51/chromedriver.exe';
const browserPath = 'C:/Users/garae/AppData/Local/ms-playwright/chromium-1234/chrome-win64/chrome.exe';
const driverPort = 19751;
const driver = spawn(driverPath, [`--port=${driverPort}`, '--allowed-ips=127.0.0.1'], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
let driverLog = ''; driver.stdout.on('data', data => driverLog += data); driver.stderr.on('data', data => driverLog += data);
let sessionId;
async function command(method, route, body) {
  const response = await fetch(`http://127.0.0.1:${driverPort}${route}`, { method, headers: { 'Content-Type': 'application/json' }, body: body === undefined ? undefined : JSON.stringify(body) });
  const value = (await response.json()).value;
  if (value?.error) throw new Error(JSON.stringify(value));
  return value;
}
try {
  for (let attempt = 0; ; attempt++) {
    try { await command('GET', '/status'); break; }
    catch (error) { if (attempt > 100) throw error; await new Promise(resolve => setTimeout(resolve, 100)); }
  }
  const session = await command('POST', '/session', { capabilities: { alwaysMatch: { browserName: 'chrome', 'goog:chromeOptions': { binary: browserPath, args: ['--headless=new', '--disable-gpu'] } } } });
  sessionId = session.sessionId;
  await command('POST', `/session/${sessionId}/url`, { url: `http://127.0.0.1:${server.address().port}/` });
  let report;
  for (let attempt = 0; ; attempt++) {
    report = await command('POST', `/session/${sessionId}/execute/sync`, { script: 'return window.report || null;', args: [] });
    if (report) break;
    if (attempt > 600) throw new Error('browser report timeout');
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  if (!report.ok) throw new Error(JSON.stringify(report));
  console.log(JSON.stringify({ browser: session.capabilities.browserVersion, result: report.result, driverLog }));
  fs.writeFileSync(path.join(artifactRoot, 'js-runtime-results.json'), JSON.stringify({ node: nodeResult, browser: report.result, browserVersion: session.capabilities.browserVersion, nodeVersion: process.version }, null, 2));
} finally {
  if (sessionId) await command('DELETE', `/session/${sessionId}`).catch(() => {});
  driver.kill(); await new Promise(resolve => server.close(resolve));
}
