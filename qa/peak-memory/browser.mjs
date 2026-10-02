// SPDX-License-Identifier: Apache-2.0
// Chromium document-per-probe WASM linear high water; no browser-process RSS claim.
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import {spawn} from 'node:child_process';
const [artifactRoot, evidence, repetitionsArg = '3'] = process.argv.slice(2);
if (!artifactRoot || !evidence) throw new Error('usage: node browser.mjs ARTIFACT_ROOT EVIDENCE [REPETITIONS]');
const cases = JSON.parse(fs.readFileSync(path.join(evidence, 'cases.json')));
const repetitions = Number(repetitionsArg);
const server = http.createServer((request, response) => {
  const url = new URL(request.url, 'http://localhost');
  let bytes, mime = 'application/octet-stream';
  if (url.pathname === '/') {
    const index = Number(url.searchParams.get('case')); const item = cases[index];
    mime = 'text/html';
    bytes = `<script type="module">
      import init, * as api from '/binas_pdf_wasm.js';
      import {workload} from '/workload.mjs';
      try {
        const exports=await init();
        const observe=()=>({wasm_linear_bytes:exports.memory.buffer.byteLength,
          js_heap_used_bytes:performance.memory?.usedJSHeapSize ?? null,
          js_heap_total_bytes:performance.memory?.totalJSHeapSize ?? null});
        const baseline=observe();
        const input=new Uint8Array(await (await fetch('/fixture?case=${index}')).arrayBuffer());
        const result=workload(api,input,${JSON.stringify(item.scenario)},observe);
        const hashes=[];
        for(const bytes of result.outputs) hashes.push(Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes))).map(b=>b.toString(16).padStart(2,'0')).join(''));
        delete result.outputs;
        window.report={ok:true,baseline,result,final:observe(),output_sha256:hashes};
      } catch(error) {window.report={ok:false,error:String(error),stack:error?.stack};}
      </script>`;
  } else if (url.pathname === '/fixture') bytes = fs.readFileSync(cases[Number(url.searchParams.get('case'))].fixture);
  else if (url.pathname === '/workload.mjs') { mime = 'text/javascript'; bytes = fs.readFileSync(new URL('./workload.mjs', import.meta.url)); }
  else if (['/binas_pdf_wasm.js', '/binas_pdf_wasm_bg.wasm'].includes(url.pathname)) {
    mime = url.pathname.endsWith('.js') ? 'text/javascript' : 'application/wasm';
    bytes = fs.readFileSync(path.join(artifactRoot, 'pkg-web', url.pathname.slice(1)));
  } else { response.writeHead(404); response.end(); return; }
  response.writeHead(200, {'Content-Type': mime}); response.end(bytes);
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const driverPath = process.env.BINAS_CHROMEDRIVER || 'C:/Users/garae/AppData/Local/.wasm-pack/chromedriver-a2cbe477b224ae51/chromedriver.exe';
const browserPath = process.env.BINAS_CHROMIUM || 'C:/Users/garae/AppData/Local/ms-playwright/chromium-1234/chrome-win64/chrome.exe';
const driverPort = 19761;
const driver = spawn(driverPath, [`--port=${driverPort}`, '--allowed-ips=127.0.0.1'], {windowsHide:true, stdio:['ignore','pipe','pipe']});
let driverLog = ''; driver.stdout.on('data', data => driverLog += data); driver.stderr.on('data', data => driverLog += data);
async function command(method, route, body) {
  const response = await fetch(`http://127.0.0.1:${driverPort}${route}`, {method, headers:{'Content-Type':'application/json'}, body:body === undefined ? undefined : JSON.stringify(body)});
  const value = (await response.json()).value;
  if (value?.error) throw new Error(JSON.stringify(value));
  return value;
}
let sessionId; const observations=[]; const failures=[]; let browserVersion;
try {
  for (let attempt=0; ; attempt++) { try {await command('GET','/status'); break;} catch(error) {if(attempt>100) throw error; await new Promise(resolve=>setTimeout(resolve,100));} }
  const session=await command('POST','/session',{capabilities:{alwaysMatch:{browserName:'chrome','goog:chromeOptions':{binary:browserPath,args:['--headless=new','--disable-gpu','--enable-precise-memory-info']}}}});
  sessionId=session.sessionId; browserVersion=session.capabilities.browserVersion;
  for (const [index,item] of cases.entries()) {
    for (let repetition=0; repetition<repetitions; repetition++) {
      await command('POST',`/session/${sessionId}/url`,{url:`http://127.0.0.1:${server.address().port}/?case=${index}&repetition=${repetition}`});
      let report;
      for(let attempt=0; ; attempt++) {
        report=await command('POST',`/session/${sessionId}/execute/sync`,{script:'return window.report || null;',args:[]});
        if(report) break;
        if(attempt>600) throw new Error('browser timeout');
        await new Promise(resolve=>setTimeout(resolve,100));
      }
      if(report.ok) observations.push({case:item.case,repetition,...report}); else failures.push({case:item.case,repetition,...report});
    }
    console.log(`chromium-wasm/${item.case}: ${observations.filter(row=>row.case===item.case).length}/${repetitions}`);
  }
  const native=JSON.parse(fs.readFileSync(path.join(evidence,'samples.json'))).observations;
  for(const row of observations.filter(row=>row.output_sha256.length)) {
    const expected=native.find(probe=>probe.runtime==='native' && probe.case===row.case)?.output_sha256;
    if(row.output_sha256[0]!==expected) failures.push({case:row.case,repetition:row.repetition,parity_expected:expected,parity_actual:row.output_sha256[0]});
  }
} finally {
  if(sessionId) await command('DELETE',`/session/${sessionId}`).catch(()=>{});
  driver.kill(); await new Promise(resolve=>server.close(resolve));
  fs.writeFileSync(path.join(evidence,'browser.json'),JSON.stringify({browserVersion,repetitions,planned_probes:cases.length*repetitions,
    successful_probes:observations.length,observations,failures,driverLog,method:'new document/module instance per probe; browser reused; JS heap phase snapshots, WASM linear capacity; Chromium RSS not observed'},null,2));
}
if(failures.length) throw new Error(`${failures.length} browser probes failed`);
