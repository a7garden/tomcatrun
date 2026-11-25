#!/usr/bin/env node
// tomcatrun 제어 창구를 부르는 명령줄 — 에이전트 스킬(SKILL.md)이 쓴다. Node 18 이상 · 의존 없음.
//
// 실행기가 켜질 때 적는 %LOCALAPPDATA%\tomcatrun\ctl.json 에서 포트 · 토큰을 읽는다.
// 버전 관리는 조회만 한다 — 창구가 쓰기를 열지 않는다 (app/src/ctl.rs).
//
//   node tomcatrun.mjs status
//   node tomcatrun.mjs restart <서버> [--no-wait] [--timeout 초]
//   node tomcatrun.mjs settle <서버>          자바 · 매퍼를 고친 뒤 — 컴파일 · 반영이 끝나기를 기다려 결과를 보인다
//   node tomcatrun.mjs log <서버> [--app] [--errors] [--lines N] [--prev]
//   node tomcatrun.mjs svn <서버> · svn-incoming · git · git-incoming · checks
//   …                                         --json 을 붙이면 받은 JSON 을 그대로 낸다

import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';

const INFO = path.join(process.env.LOCALAPPDATA || '', 'tomcatrun', 'ctl.json');
const argv = process.argv.slice(2);
const flags = new Set(argv.filter((a) => a.startsWith('--')));
const opt = (name, dflt) => {
  const i = argv.indexOf(name);
  return i >= 0 && argv[i + 1] && !argv[i + 1].startsWith('--') ? argv[i + 1] : dflt;
};
const pos = argv.filter((a, i) => !a.startsWith('--') && !(i > 0 && ['--timeout', '--lines', '--since'].includes(argv[i - 1])));
const [cmd, srvArg] = pos;
const JSON_OUT = flags.has('--json');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function fail(msg, code = 1) {
  process.stderr.write(msg + '\n');
  process.exit(code);
}

function readInfo() {
  try {
    return JSON.parse(fs.readFileSync(INFO, 'utf8'));
  } catch {
    return null; // 없거나, 실행기가 막 바꿔 다는 중
  }
}

function info() {
  const ci = readInfo();
  if (!ci) fail(`tomcatrun 실행기가 꺼져 있거나 0.32.0 보다 낮습니다 (${INFO} 없음). 켜려면 tomcatrun 을 실행하세요.`, 2);
  return ci;
}

// 실행기가 끝나며 ctl.json 을 지우지 못했을 때(강제 종료) — 그 포트를 잡은 다른 프로그램에 토큰을 보내지 않는다
function alive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (e) {
    return e.code === 'EPERM';
  }
}

async function call(method, route, query = {}) {
  const ci = info();
  if (ci.pid && !alive(ci.pid)) fail(`tomcatrun 실행기가 꺼져 있습니다. 켜려면: node tomcatrun.mjs launch`, 2);
  const qs = new URLSearchParams(query).toString();
  const url = `http://127.0.0.1:${ci.port}${route}${qs ? '?' + qs : ''}`;
  let res;
  try {
    res = await fetch(url, { method, headers: { 'X-Tomcatrun-Token': ci.token } });
  } catch {
    fail(`tomcatrun 실행기가 꺼져 있습니다 (포트 ${ci.port} 응답 없음). 켜려면: node tomcatrun.mjs launch`, 2);
  }
  const body = await res.json().catch(() => ({ error: `응답을 읽지 못했습니다 (HTTP ${res.status})` }));
  if (!res.ok) fail(body.error || `HTTP ${res.status}`);
  return body;
}

const need = () => srvArg ?? fail(`서버를 정하세요 — 번호 · id · 이름 (node tomcatrun.mjs status 로 봅니다)`);
const time = (ms) => (ms == null ? '-' : ms >= 1000 ? `${(ms / 1000).toFixed(1)}초` : `${ms}ms`);
const clock = (ms) => (ms ? new Date(ms).toTimeString().slice(0, 8) : '-');

async function server(s) {
  const st = await call('GET', '/status');
  const one = st.servers.find((x) => String(x.srv) === String(s) || x.id === s || x.name === s);
  if (!one) fail(`없는 서버: ${s} (있는 서버: ${st.servers.map((x) => `${x.srv} ${x.id} ${x.name}`).join(' · ')})`);
  return one;
}

function printServer(x) {
  const lines = [];
  let head = `[${x.srv}] ${x.id} ${x.name} — ${x.status}${x.waiting ? ' (다른 서버 기동을 기다리는 중)' : ''}`;
  if (x.status === 'running') head += ` · ${x.url} · pid ${x.pid} · 기동 ${time(x.startupMs)} · ${clock(x.startedAtMs)} 부터`;
  lines.push(head);
  lines.push(`    프로젝트 ${x.project}${x.mode ? ` · ${x.mode}` : ''}`);
  if (x.compiling) lines.push('    컴파일 중');
  if (x.reflecting) lines.push('    실행 중인 앱에 반영하는 중 (자바 교체 · Spring 빈 · 매퍼)');
  if (x.lastCompile) {
    const c = x.lastCompile;
    lines.push(`    마지막 컴파일 ${clock(c.atMs)} · ${c.ok ? '성공' : `실패 (오류 ${c.errors})`} · ${c.full ? '전체' : '바뀐 것'} ${c.files}개 · ${time(c.ms)}`);
  }
  if (x.restartNeeded?.length) {
    lines.push(`    재시작해야 반영 ${x.restartNeeded.length}개:`);
    for (const p of x.restartNeeded) lines.push(`      - ${p.path} — ${p.why}`);
  }
  if (x.depsBusy) lines.push('    의존 jar 받는 중');
  if (x.vcsBusy) lines.push('    사용자가 실행기에서 버전 관리 작업(Commit · Update 등)을 하는 중');
  return lines.join('\n');
}

async function appErrors(srv, since) {
  const r = await call('GET', '/log', { srv, src: 'app', since: String(since || 0), lines: '400' });
  return r.lines.filter((l) => l.kind === 'err' || l.kind === 'warn');
}

async function tailErrors(srv, n = 30) {
  const r = await call('GET', '/log', { srv, lines: '3000' }).catch(() => ({ lines: [] }));
  const hits = [];
  r.lines.forEach((l, i) => {
    if (/SEVERE|심각|ERROR|Exception|Caused by|Address already in use/.test(l)) hits.push(i);
  });
  const keep = new Set();
  for (const i of hits.slice(-n)) for (let k = i; k < Math.min(i + 3, r.lines.length); k++) keep.add(k);
  return [...keep].sort((a, b) => a - b).map((i) => r.lines[i]);
}

// 시작 · 재시작 — 응답의 generation 보다 큰 기동이 running(성공) 이나 stopped(실패) 에 닿을 때까지
async function waitStart(s, gen0, since, timeoutS) {
  const end = Date.now() + timeoutS * 1000;
  let stoppedSince = 0;
  let last = '';
  while (Date.now() < end) {
    const x = await server(s);
    if (x.status !== last) {
      process.stderr.write(`… ${x.status}${x.waiting ? ' (차례 기다림)' : ''}\n`);
      last = x.status;
    }
    if (x.generation > gen0 && x.status === 'running') return { ok: true, x };
    if (x.generation > gen0 && x.status === 'stopped') return { ok: false, x };
    // 새 기동이 시작도 못 하고 멈춘 채면(시작 전 점검에서 막힘) 오래 기다리지 않는다
    if (x.status === 'stopped' && x.generation === gen0) {
      stoppedSince ||= Date.now();
      if (Date.now() - stoppedSince > 8000) return { ok: false, x };
    } else stoppedSince = 0;
    await sleep(1000);
  }
  return { ok: false, timeout: true, x: await server(s) };
}

async function reportStart(s, r, gen0, since) {
  if (r.ok) {
    console.log(`완료 — ${printServer(r.x)}`);
    return;
  }
  console.log(r.timeout ? '시간 안에 다 뜨지 않았습니다.' : '기동에 실패했습니다.');
  console.log(printServer(r.x));
  const errs = await appErrors(r.x.srv, since);
  if (errs.length) {
    console.log('\n실행기 알림:');
    for (const l of errs.slice(-20)) console.log(`  [${l.kind}] ${l.text}`);
  }
  // 톰캣을 띄우기 전에 막혔으면(포트 · java · 컴파일) console.log 는 지난 기동의 것이다 — 이번 원인처럼 보이지 않게 뺀다
  if (r.x.generation > gen0) {
    const t = await tailErrors(r.x.srv);
    if (t.length) {
      console.log('\n톰캣 로그의 오류 줄 (logs/console.log):');
      for (const l of t) console.log('  ' + l);
    }
  } else console.log('\n톰캣을 띄우기 전에 막혔습니다 — 위 실행기 알림이 까닭입니다.');
  process.exitCode = 1;
}

// 자바 · 매퍼를 고친 뒤 — 저장 감시가 잡을 틈을 주고, 컴파일 · 반영이 끝나기를 기다린다.
// since 뒤에 끝난 컴파일만 「이번 결과」로 본다 — 자동 컴파일이 꺼졌거나 저장을 못 잡았으면 옛 결과를 들고 「끝났다」고 하게 된다
async function settle(s, since, timeoutS) {
  const end = Date.now() + timeoutS * 1000;
  const st = await call('GET', '/status');
  await sleep(1500);
  let x = await server(s);
  while (Date.now() < end && (x.compiling || x.reflecting)) {
    await sleep(1000);
    x = await server(s);
  }
  const busy = x.compiling || x.reflecting;
  const fresh = x.lastCompile && x.lastCompile.atMs >= since;
  if (busy) console.log(`아직 끝나지 않았습니다 (${timeoutS}초).`);
  else if (fresh) console.log('끝났습니다.');
  else {
    console.log(`이번에 컴파일한 것이 없습니다 (${clock(since)} 뒤).`);
    console.log(st.autoCompile ? '  저장한 지 오래됐으면 --since 초를 늘리거나, 손으로: node tomcatrun.mjs compile ' + x.id : '  자동 컴파일이 꺼져 있습니다. 손으로: node tomcatrun.mjs compile ' + x.id);
  }
  console.log(printServer(x));
  const errs = (await appErrors(x.srv, since)).slice(-30);
  if (errs.length) {
    console.log('\n이번에 나온 오류 · 경고:');
    for (const l of errs) console.log(`  [${l.src}/${l.kind}] ${l.text}`);
  }
  if (!busy && !fresh) process.exitCode = 1;
  if (fresh && !x.lastCompile.ok) process.exitCode = 1;
  if (x.restartNeeded?.length) console.log(`\n→ 재시작해야 들어갑니다: node tomcatrun.mjs restart ${x.id}`);
}

function printSvn(st) {
  const items = st.items || [];
  const out = [`작업복사본 ${st.url} · r${st.revMin}~r${st.revMax} · 변경 ${items.filter((i) => i.code !== '?').length}개 · 추가 안 됨 ${items.filter((i) => i.code === '?').length}개`];
  const lock = { '': '', busy: '실행기가 쓰는 중', eclipse: 'Eclipse 가 잡은 잠금', other: '다른 SVN 프로그램이 잡은 잠금', stale: '남은 잠금 (Cleanup 필요)', queue: '끊긴 작업 대기열 (Cleanup 필요)' }[st.lockKind || ''];
  if (lock) out.push(`잠금: ${lock}${st.locks?.length ? ` · ${st.locks.join(', ')}` : ''}`);
  for (const i of items) out.push(`${i.code.padEnd(2)}${i.conflict ? '충돌 ' : ''}${i.path}${i.dir ? '/' : ''}${i.changelist ? `  (${i.changelist})` : ''}`);
  return out.join('\n');
}

function printGit(st) {
  const out = [`${st.root} · ${st.branch || '(분리된 HEAD)'} @ ${st.head}${st.upstream ? ` · ${st.upstream} 보다 앞 ${st.ahead} · 뒤 ${st.behind}` : ''} · 변경 ${st.items.length}개`];
  for (const i of st.items) out.push(`${i.code.padEnd(2)}${i.path}${i.orig ? ` ← ${i.orig}` : ''}`);
  return out.join('\n');
}

function printIncoming(r) {
  const out = [];
  if (r.head != null && r.base != null) out.push(`저장소 최신 r${r.head} · 작업복사본 최저 r${r.base} · 받을 커밋 ${r.entries.length}개${r.more ? ' (더 있음)' : ''}`);
  else out.push(`${r.branch} @ ${r.head} · 받을 커밋 ${r.entries.length}개 (마지막 fetch ${clock(r.fetchedMs)} 기준)`);
  for (const e of r.entries.slice(0, 50)) {
    const id = e.rev != null ? `r${e.rev}` : e.short;
    out.push(`${id} ${e.author} ${e.dateMs ? new Date(e.dateMs).toLocaleString('ko-KR', { hour12: false }) : ''} — ${(e.msg || '').split('\n')[0]}`);
    for (const c of (e.paths || []).filter((c) => c.inside !== false).slice(0, 10)) out.push(`    ${c.action} ${c.path}`);
  }
  if (r.overlap?.length) out.push(`\n내가 고친 파일과 겹침 (받으면 합치거나 충돌): ${r.overlap.join(', ')}`);
  return out.join('\n');
}

const show = (v, text) => console.log(JSON_OUT ? JSON.stringify(v, null, 2) : text(v));
const timeout = Number(opt('--timeout', 0));

switch (cmd) {
  case 'status': {
    const st = await call('GET', '/status');
    show(st, (v) => [`tomcatrun ${v.version} · 자동 컴파일 ${v.autoCompile ? '켬' : '끔'} · 핫리로드 ${v.hotReload ? '켬' : '끔'}`, ...v.servers.map(printServer)].join('\n'));
    break;
  }
  case 'start':
  case 'restart': {
    const since = Date.now();
    const r = await call('POST', '/' + cmd, { srv: need() });
    if (flags.has('--no-wait') || JSON_OUT) show(r, () => `${cmd} 요청을 보냈습니다.`);
    else await reportStart(need(), await waitStart(need(), r.generation, since, timeout || 600), r.generation, since);
    break;
  }
  case 'stop': {
    const r = await call('POST', '/stop', { srv: need() });
    if (flags.has('--no-wait') || JSON_OUT) show(r, () => '중지 요청을 보냈습니다.');
    else {
      const end = Date.now() + (timeout || 120) * 1000;
      let x = await server(need());
      while (x.status !== 'stopped' && Date.now() < end) {
        await sleep(1000);
        x = await server(need());
      }
      console.log(x.status === 'stopped' ? `멈췄습니다 — ${x.name}` : `아직 멈추지 않았습니다 (${x.status}).`);
      if (x.status !== 'stopped') process.exitCode = 1;
    }
    break;
  }
  case 'compile': {
    const since = Date.now();
    const r = await call('POST', '/compile', { srv: need(), ...(flags.has('--full') ? { full: '1' } : {}) });
    if (flags.has('--no-wait') || JSON_OUT) show(r, () => '컴파일 요청을 보냈습니다.');
    else await settle(need(), since, timeout || 600);
    break;
  }
  case 'settle':
    await settle(need(), Date.now() - Number(opt('--since', 60)) * 1000, timeout || 120);
    break;
  case 'wait': {
    const x = await server(need());
    if (x.status === 'stopped') {
      console.log(`꺼져 있습니다 — ${x.name}. 켜려면: node tomcatrun.mjs start ${x.id}`);
      process.exitCode = 1;
      break;
    }
    // 켜는 중 · 켜짐이면 이미 generation 이 올라 있다 — 지금 기동을 기다린다
    const gen0 = x.generation - 1;
    await reportStart(need(), await waitStart(need(), gen0, Date.now(), timeout || 600), gen0, Date.now() - 600_000);
    break;
  }
  case 'log': {
    const q = { srv: need(), lines: opt('--lines', flags.has('--errors') ? '3000' : '200') };
    if (flags.has('--app')) q.src = 'app';
    if (flags.has('--prev')) q.prev = '1';
    const r = await call('GET', '/log', q);
    if (JSON_OUT) show(r, () => '');
    else if (flags.has('--app')) for (const l of r.lines.filter((l) => !flags.has('--errors') || l.kind === 'err' || l.kind === 'warn')) console.log(`${clock(l.atMs)} [${l.src}${l.kind ? '/' + l.kind : ''}] ${l.text}`);
    else if (flags.has('--errors')) for (const l of await tailErrors(q.srv, 60)) console.log(l);
    else console.log(r.lines.join('\n'));
    break;
  }
  case 'checks': {
    const r = await call('GET', '/checks', { srv: need() });
    show(r, (v) =>
      [...v.common, ...v.checks].map((c) => `${c.ok ? (c.warn ? '△' : '✓') : '✗'} ${c.label}${c.detail ? ' — ' + c.detail : ''}`).join('\n'),
    );
    break;
  }
  case 'svn':
    show(await call('GET', '/svn', { srv: need() }), printSvn);
    break;
  case 'svn-incoming':
    show(await call('GET', '/svn/incoming', { srv: need() }), printIncoming);
    break;
  case 'git':
    show(await call('GET', '/git', { srv: need() }), printGit);
    break;
  case 'git-incoming':
    show(await call('GET', '/git/incoming', { srv: need() }), printIncoming);
    break;
  case 'launch': {
    // 실행기가 끝나면 ctl.json 을 지우므로 exe 자리는 설치 기본값에서도 찾는다
    const prev = readInfo();
    if (prev?.pid && alive(prev.pid)) {
      console.log(`이미 켜져 있습니다 (${prev.version}).`);
      break;
    }
    const exe = [prev?.exe, path.join(process.env.LOCALAPPDATA || '', 'Programs', 'tomcatrun', 'tomcatrun.exe')].find((p) => p && fs.existsSync(p));
    if (!exe) fail('tomcatrun 실행 파일을 찾지 못했습니다. 사용자에게 켜 달라고 하세요.');
    spawn(exe, [], { detached: true, stdio: 'ignore' }).unref();
    const end = Date.now() + 30_000;
    while (Date.now() < end) {
      await sleep(1000);
      const now = readInfo();
      if (now && now.startedAtMs !== prev?.startedAtMs) {
        console.log(`실행기를 켰습니다 (${now.version}).`);
        process.exit(0);
      }
    }
    fail('실행기가 30초 안에 창구를 열지 않았습니다.');
    break;
  }
  default:
    fail(
      [
        '쓰는 법: node tomcatrun.mjs <명령> [서버] [--json]',
        '  status                      서버마다 상태 · 컴파일 · 재시작해야 반영되는 것',
        '  start|stop|restart <서버>   끝날 때까지 기다린다 (--no-wait · --timeout 초)',
        '  settle <서버> [--since 초]  고친 자바 · 매퍼의 컴파일 · 반영을 기다려 결과를 보인다 (기본 60초 안의 컴파일)',
        '  compile <서버> [--full]     손으로 컴파일 (보통은 저장하면 알아서 한다)',
        '  log <서버> [--app] [--errors] [--lines N] [--prev]',
        '  checks <서버>               서버 점검',
        '  svn|svn-incoming|git|git-incoming <서버>   저장소 조회 (읽기만)',
        '  launch                      꺼진 실행기를 켠다',
      ].join('\n'),
    );
}
