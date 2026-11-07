// Reads the state transition table of docs/spec/ask-flow.md. The document is the source; this
// parser is strict on purpose: a cell it cannot read, a name it does not know, or a missing cell
// is an error that names the state and the event.
'use strict';

const rowsOf = lines =>
  lines
    .filter(l => l.startsWith('|') && !/^\|[-| ]+\|$/.test(l))
    .map(l =>
      l
        .replace(/^\||\|$/g, '')
        .split('|')
        .map(c => c.trim()),
    );

function sections(md) {
  const out = {};
  let name = null;
  for (const line of md.split('\n')) {
    if (line.startsWith('## ')) {
      name = line.slice(3).trim();
      out[name] = [];
    } else if (name) {
      out[name].push(line);
    }
  }
  return out;
}

function parseCell(text, state, event, known) {
  const where = `${state} × ${event}`;
  const fail = why => {
    throw new Error(`${where}: ${why} ("${text}")`);
  };
  const action = a => {
    if (!known.actions.includes(a)) fail(`unknown action ${a}`);
    return a;
  };
  const target = s => {
    if (!known.states.includes(s)) fail(`unknown state ${s}`);
    return s;
  };
  let m;
  if ((m = /^—([a-z])$/.exec(text))) {
    return {k: 'never', note: m[1]};
  }
  if ((m = /^×([a-z])$/.exec(text))) {
    return {k: 'ignore', note: m[1]};
  }
  if ((m = /^×(A\d+b?)$/.exec(text))) {
    return {k: 'ignore', action: action(m[1])};
  }
  if ((m = /^(A\d+b?)→([A-Z_]+) \[more\] \/ (A\d+b?)→([A-Z_]+) \[spent\]$/.exec(text))) {
    return {
      k: 'guard',
      more: {action: action(m[1]), next: target(m[2])},
      spent: {action: action(m[3]), next: target(m[4])},
    };
  }
  if ((m = /^(A\d+b?)(?:→([A-Z_]+))?$/.exec(text))) {
    return {k: 'do', action: action(m[1]), next: m[2] ? target(m[2]) : state};
  }
  return fail('not a cell in the documented notation');
}

function parse(md) {
  const sec = sections(md);
  const need = n => {
    if (!sec[n]) throw new Error(`section "## ${n}" not found`);
    return rowsOf(sec[n]);
  };
  const states = need('状態').slice(1).map(r => r[0]);
  const events = need('イベント').slice(1).map(r => r[0]);
  const actions = need('処理（Action）')
    .slice(1)
    .map(r => r[0]);
  const known = {states, events, actions};

  const table = {};
  let header = null;
  for (const r of need('状態遷移表')) {
    if (r[0].startsWith('状態＼イベント')) {
      header = r.slice(1);
      for (const e of header) {
        if (!events.includes(e)) throw new Error(`matrix column "${e}" is not a defined event`);
      }
      continue;
    }
    const sm = /^\*\*([A-Z_]+)\*\*$/.exec(r[0]);
    if (!sm || !header) throw new Error(`unexpected matrix row "${r[0]}"`);
    const state = sm[1];
    if (!states.includes(state)) throw new Error(`matrix row "${state}" is not a defined state`);
    table[state] = table[state] || {};
    header.forEach((e, i) => {
      if (table[state][e]) throw new Error(`${state} × ${e}: defined twice`);
      table[state][e] = parseCell(r[i + 1] ?? '', state, e, known);
    });
  }
  for (const s of states) {
    for (const e of events) {
      if (!table[s] || !table[s][e]) throw new Error(`${s} × ${e}: no cell (every cell must be filled)`);
    }
  }
  return {states, events, actions, table};
}

module.exports = {parse};
