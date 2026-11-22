import test from 'node:test';
import assert from 'node:assert/strict';
import {BOUNDS, merge, parseStats, quantile, render} from './usage-dashboard.mjs';

const hist = (k, i, n = 1) => {
  const h = new Array(BOUNDS[k].length + 1).fill(0);
  h[i] = n;
  return h;
};
const day = (name, over = {}) => ({day: name, sessions: 2, peak_concurrent: 1, refused: {offline: 1}, ...Object.fromEntries(Object.keys(BOUNDS).map(k => [k, hist(k, 2)])), ...over});
const doc = days => JSON.stringify({v: 1, bounds_ms: BOUNDS.ping_client_ms, bounds_s: BOUNDS.session_s, bounds_bytes: BOUNDS.session_bytes, days});

test('a quantile is the upper edge of its bucket; the last bucket and an empty one are told apart', () => {
  const b = [10, 20, 50];
  assert.equal(quantile([0, 0, 0, 0], b, 0.5), '-');
  assert.equal(quantile([5, 0, 0, 0], b, 0.99), '≤ 10');
  assert.equal(quantile([9, 0, 0, 1], b, 0.9), '≤ 10');
  assert.equal(quantile([9, 0, 0, 1], b, 0.99), '> 50');
});

test('merging adds histograms and counts, and takes the largest peak', () => {
  const m = merge([day('2026-11-22', {peak_concurrent: 3}), day('2026-11-23', {peak_concurrent: 2, refused: {offline: 2, busy: 1}})]);
  assert.equal(m.sessions, 4);
  assert.equal(m.peak_concurrent, 3);
  assert.deepEqual(m.refused, {offline: 3, busy: 1});
  assert.equal(m.forward_ms[2], 2);
});

test('a document from the network is checked: shape, bounds, lengths, counts', () => {
  assert.ok(Array.isArray(parseStats(doc([day('2026-11-22')]))));
  assert.ok(parseStats('{') instanceof Error);
  assert.ok(parseStats('{"v":2,"days":[]}') instanceof Error);
  assert.ok(parseStats(doc([day('22-11-2026')])) instanceof Error);
  assert.ok(parseStats(doc([day('2026-11-22', {sessions: -1})])) instanceof Error);
  assert.ok(parseStats(doc([day('2026-11-22', {sessions: 1.5})])) instanceof Error);
  assert.ok(parseStats(doc([day('2026-11-22', {forward_ms: [1, 2]})])) instanceof Error);
  assert.ok(parseStats(doc([day('2026-11-22', {refused: {a: 'x'}})])) instanceof Error);
  assert.ok(parseStats(doc(new Array(9).fill(day('2026-11-22')))) instanceof Error);
  assert.ok(parseStats(doc([day('2026-11-22')]).replace('[10,20,', '[11,20,')) instanceof Error);
});

test('the page says so when nothing is collected, and gives p50/p90/p99 per relay and overall', () => {
  assert.match(render({}), /まだ集計がありません/);
  const page = render({'us.relay.spellink.uk': {'2026-11-22': day('2026-11-22')}, 'jp.relay.spellink.uk': {'2026-11-22': day('2026-11-22', {ping_client_ms: hist('ping_client_ms', 9)})}});
  assert.match(page, /## us\.relay\.spellink\.uk/);
  assert.match(page, /## 全体/);
  assert.match(page, /端末⇄中継の往復 \| ms \| ≤ 50 \| ≤ 50 \| ≤ 50/);
  assert.match(page, /> 5000/);
  assert.doesNotMatch(page, /NaN|undefined/);
});

test('a relay with too few sessions shows no distribution', () => {
  const page = render({r: {'2026-11-22': day('2026-11-22', {sessions: 1})}}, 20);
  assert.match(page, /件数が少なすぎる/);
});
