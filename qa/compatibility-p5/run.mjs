// SPDX-License-Identifier: Apache-2.0
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import { createRequire } from 'node:module';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { verifyProducerSessions } from './contract.mjs';

const [artifactRoot, corpusRoot, outputRoot, reportRoot] = process.argv.slice(2);
if (!reportRoot) throw new Error('usage: node run.mjs ARTIFACT_ROOT CORPUS_ROOT NATIVE_OUTPUT_ROOT REPORT_ROOT');
const manifest = JSON.parse(fs.readFileSync(path.join(corpusRoot, 'manifest.json')));
const fixtures = Object.fromEntries(manifest.fixtures.map(fixture => {
  const data = fs.readFileSync(path.join(corpusRoot, 'pdf', fixture.path));
  if (createHash('sha256').update(data).digest('hex') !== fixture.sha256) throw new Error(`fixture hash ${fixture.id}`);
  return [fixture.id, data];
}));
const expected = { target: manifest.fixtures.find(fixture => fixture.id === 'pypdf-raw').target,
  ...Object.fromEntries(['compact', 'incremental'].map(mode => [mode, fs.readFileSync(path.join(outputRoot, `pypdf-raw-${mode}.pdf`))])) };
const require = createRequire(import.meta.url);
const api = require(path.join(artifactRoot, 'pkg-node/binas_pdf_wasm.js'));
const node = verifyProducerSessions(api, fixtures, expected);
const server = http.createServer((request, response) => {
  const name = new URL(request.url, 'http://localhost').pathname;
  let data, mime = 'application/octet-stream';
  if (name === '/') {
    mime = 'text/html';
    data = `<script type="module">
      import init,* as api from '/binas_pdf_wasm.js';
      import {verifyProducerSessions} from '/contract.mjs';
      try { await init();
        const fixtures=Object.fromEntries(await Promise.all(${JSON.stringify(Object.keys(fixtures))}.map(async name=>[name,new Uint8Array(await (await fetch('/fixture/'+name)).arrayBuffer())])));
        const expected={target:${JSON.stringify(expected.target)}};
        for(const mode of ['compact','incremental']) expected[mode]=new Uint8Array(await (await fetch('/expected/'+mode)).arrayBuffer());
        window.report={ok:true,result:verifyProducerSessions(api,fixtures,expected)};
      }catch(error){window.report={ok:false,error:String(error),stack:error?.stack};}
    </script>`;
  } else if (name === '/contract.mjs') {
    mime = 'text/javascript'; data = fs.readFileSync(new URL('./contract.mjs', import.meta.url));
  } else if (name === '/binas_pdf_wasm.js' || name === '/binas_pdf_wasm_bg.wasm') {
    mime = name.endsWith('.js') ? 'text/javascript' : 'application/wasm';
    data = fs.readFileSync(path.join(artifactRoot, 'pkg-web', name.slice(1)));
  } else if (name.startsWith('/fixture/')) { data = fixtures[name.slice(9)];
  } else if (name.startsWith('/expected/')) { data = expected[name.slice(10)]; }
  if (!data) { response.writeHead(404); response.end(); return; }
  response.writeHead(200, {'Content-Type': mime}); response.end(data);
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const driverPath = 'C:/Users/garae/AppData/Local/.wasm-pack/chromedriver-a2cbe477b224ae51/chromedriver.exe';
const browserPath = 'C:/Users/garae/AppData/Local/ms-playwright/chromium-1234/chrome-win64/chrome.exe';
const port = 19753;
const driver = spawn(driverPath, [`--port=${port}`, '--allowed-ips=127.0.0.1'], {windowsHide:true, stdio:['ignore','pipe','pipe']});
let driverLog = '', sessionId;
driver.stdout.on('data', value => driverLog += value); driver.stderr.on('data', value => driverLog += value);
async function command(method, route, body) {
  const response = await fetch(`http://127.0.0.1:${port}${route}`, {method, headers:{'Content-Type':'application/json'}, body:body === undefined ? undefined : JSON.stringify(body)});
  const value = (await response.json()).value;
  if (value?.error) throw new Error(JSON.stringify(value));
  return value;
}
try {
  for (let attempt = 0;; attempt++) {
    try { await command('GET', '/status'); break; }
    catch (error) { if (attempt > 100) throw error; await new Promise(resolve => setTimeout(resolve, 100)); }
  }
  const session = await command('POST', '/session', {capabilities:{alwaysMatch:{browserName:'chrome', 'goog:chromeOptions':{binary:browserPath,args:['--headless=new','--disable-gpu']}}}});
  sessionId = session.sessionId;
  await command('POST', `/session/${sessionId}/url`, {url:`http://127.0.0.1:${server.address().port}/`});
  let report;
  for (let attempt = 0;; attempt++) {
    report = await command('POST', `/session/${sessionId}/execute/sync`, {script:'return window.report || null;',args:[]});
    if (report) break;
    if (attempt > 600) throw new Error('browser report timeout');
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  if (!report.ok) throw new Error(JSON.stringify(report));
  fs.mkdirSync(reportRoot, {recursive:true});
  const result = {node, browser:report.result, nodeVersion:process.version, browserVersion:session.capabilities.browserVersion,
    artifactRoot, corpusRoot, outputRoot, driverLog, artifactSha256:createHash('sha256').update(fs.readFileSync(path.join(artifactRoot,'pkg-web/binas_pdf_wasm_bg.wasm'))).digest('hex')};
  fs.writeFileSync(path.join(reportRoot,'js-runtime-results.json'), JSON.stringify(result,null,2));
  console.log(JSON.stringify(result));
} finally {
  if (sessionId) await command('DELETE', `/session/${sessionId}`).catch(() => {});
  driver.kill(); await new Promise(resolve => server.close(resolve));
}
