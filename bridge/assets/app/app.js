// The Spellink Bridge App page (served at /app): the bridge's status, its devices, and adding one.
// Everything the browser provides comes in through `env`, so the whole flow is tested without one.
(function (root, factory) {
  if (typeof module === 'object' && module.exports) {
    module.exports = factory();
  } else {
    root.appPage = factory();
  }
})(typeof self !== 'undefined' ? self : this, function () {
  'use strict';

  var POLL_MS = 5000;

  /** A file name that is safe on any system: the device's name made of letters, digits, dot, dash, underscore. */
  function fileName(name, id) {
    var n = String(name).replace(/[^A-Za-z0-9._-]+/g, '_').replace(/^[._-]+|[._-]+$/g, '');
    return 'Spellink-' + (n || id) + '.snplg';
  }

  /** The actions that make sense for a device in each state (ADR 0016: pairing.md, table A). */
  function actionsFor(status) {
    switch (status) {
      case 'issued': return ['save', 'revoke', 'reissue'];
      case 'waiting': return ['pair', 'revoke'];
      case 'granting': return ['revoke'];
      case 'active': return ['revoke', 'reissue'];
      case 'expired': return ['reissue', 'forget'];
      case 'revoked': return ['forget'];
      default: return [];
    }
  }

  function start(env) {
    var doc = env.document;
    var L = env.logic;
    var $ = function (id) { return doc.getElementById(id); };
    var strings = null;
    var token = '';
    var alive = true;

    function t(key, vars) {
      return strings && strings[key] ? L.format(strings[key], vars) : key;
    }

    function say(key, vars) {
      $('message').textContent = t(key, vars);
      $('message').setAttribute('data-key', key);
    }

    function api(method, path, body) {
      var init = {method: method, headers: {'Authorization': 'Bearer ' + token}};
      if (body !== undefined) {
        init.headers['Content-Type'] = 'application/json';
        init.body = JSON.stringify(body);
      }
      return env.fetch(path, init);
    }

    /** Reads a refused answer's text for the message; a body that cannot be read is just the status. */
    function whyOf(res) {
      return res.text().then(function (x) { return x.trim() || 'HTTP ' + res.status; }, function () { return 'HTTP ' + res.status; });
    }

    function line(parent, kind, text) {
      var d = doc.createElement('div');
      d.className = 'line ' + kind;
      d.textContent = text;
      parent.appendChild(d);
    }

    function renderStatus(s) {
      var box = $('status');
      box.replaceChildren();
      line(box, 'info', t('bridgeLine', {name: s.name, version: s.version}));
      line(box, s.provider.found ? 'ok' : 'bad',
        s.provider.found ? t('providerOk', {name: s.provider.name, path: s.provider.path}) : t('providerMissing', {name: s.provider.name}));
      line(box, s.relay ? 'ok' : 'info', t(s.relay ? 'relayOn' : 'relayOff'));
      if (!s.addresses.length) line(box, 'bad', t('noAddress'));
    }

    function stateText(d) {
      var key = 'state' + d.status[0].toUpperCase() + d.status.slice(1);
      var when = function (iso) {
        var date = new Date(iso);
        return isNaN(date) ? t('never') : date.toLocaleString();
      };
      return t(key, {
        until: d.expires ? when(d.expires) : '',
        last: d.lastSeen ? when(d.lastSeen) : t('never'),
      });
    }

    function button(label, onclick) {
      var b = doc.createElement('button');
      b.textContent = label;
      b.setAttribute('type', 'button');
      b.onclick = onclick;
      return b;
    }

    function renderDevices(list) {
      var box = $('devices');
      box.replaceChildren();
      if (!list.length) {
        line(box, 'info', t('noDevices'));
        return;
      }
      list.forEach(function (d) {
        var row = doc.createElement('div');
        row.className = 'device';
        var name = doc.createElement('span');
        name.className = 'name';
        name.textContent = d.name;
        var state = doc.createElement('span');
        state.className = 'state';
        state.textContent = stateText(d);
        row.appendChild(name);
        row.appendChild(state);
        actionsFor(d.status).forEach(function (a) {
          row.appendChild(button(t('action' + a[0].toUpperCase() + a.slice(1)), function () { act(a, d); }));
        });
        box.appendChild(row);
      });
    }

    /** Downloads the device's setup file. */
    function save(d, doneKey) {
      return api('GET', '/admin/devices/' + d.id + '/package').then(function (res) {
        if (!res.ok) return whyOf(res).then(function (why) { say('failed', {why: why}); return false; });
        return res.blob().then(function (blob) {
          env.download(blob, fileName(d.name, d.id));
          say(doneKey);
          return true;
        });
      });
    }

    function act(action, d) {
      if (action === 'pair') {
        env.location.href = '/pair#' + token;
        return;
      }
      if (action === 'save') {
        save(d, 'doneSave');
        return;
      }
      var confirmKey = {revoke: 'confirmRevoke', reissue: 'confirmReissue', forget: 'confirmForget'}[action];
      if (!env.confirm(t(confirmKey, {name: d.name}))) return;
      api('POST', '/admin/devices/' + d.id + '/' + action).then(function (res) {
        if (!res.ok) return whyOf(res).then(function (why) { say('failed', {why: why}); });
        // a reissued device needs its new file at once: it is what the owner does next
        var next = action === 'reissue' ? save(d, 'doneReissue') : Promise.resolve(say('done' + action[0].toUpperCase() + action.slice(1)));
        return next.then(load);
      }, function (e) { say('unreachable', {why: e && e.message ? e.message : 'network'}); });
    }

    function showSteps(device) {
      var box = $('steps');
      box.replaceChildren();
      ['step1', 'step2', 'step3'].forEach(function (k) { line(box, 'info', t(k)); });
      var a = doc.createElement('a');
      a.setAttribute('href', '/pair#' + token);
      a.textContent = t('stepLink');
      box.appendChild(a);
    }

    function add() {
      var name = String($('addName').value).trim();
      if (!name) {
        say('nameEmpty');
        return;
      }
      api('POST', '/admin/devices', {name: name}).then(function (res) {
        if (res.status !== 201) return whyOf(res).then(function (why) { say('failed', {why: why}); });
        return res.json().then(function (created) {
          return save({id: created.id, name: name}, 'added').then(function (saved) {
            if (saved) {
              $('addName').value = '';
              showSteps(created);
              say('added', {name: name});
            }
            return load();
          });
        });
      }, function (e) { say('unreachable', {why: e && e.message ? e.message : 'network'}); });
    }

    /** Reads the status and the devices and draws them. */
    function load() {
      return Promise.all([api('GET', '/admin/status'), api('GET', '/admin/devices')]).then(function (rs) {
        if (rs[0].status === 401 || rs[1].status === 401) {
          say('unauthorized');
          alive = false;
          return;
        }
        if (!rs[0].ok || !rs[1].ok) return whyOf(rs[0].ok ? rs[1] : rs[0]).then(function (why) { say('unreachable', {why: why}); });
        return Promise.all([rs[0].json(), rs[1].json()]).then(function (j) {
          renderStatus(j[0]);
          renderDevices(j[1]);
        });
      }, function (e) { say('unreachable', {why: e && e.message ? e.message : 'network'}); });
    }

    function poll() {
      env.setTimeout(function () {
        if (!alive) return;
        load().then(poll, poll);
      }, POLL_MS);
    }

    return env.fetch('/app/strings.json').then(function (r) { return r.json(); }).then(function (all) {
      var langs = env.navigator.languages && env.navigator.languages.length ? env.navigator.languages : [env.navigator.language];
      strings = all[L.pickLocale(langs, Object.keys(all))];
      doc.title = strings.title;
      $('title').textContent = strings.title;
      $('devicesTitle').textContent = strings.devicesTitle;
      $('addTitle').textContent = strings.addTitle;
      $('addButton').textContent = strings.addButton;
      $('addName').setAttribute('placeholder', strings.addPlaceholder);
      token = env.location.hash.replace(/^#/, '');
      if (!token) {
        say('noToken');
        return;
      }
      $('addButton').onclick = add;
      return load().then(poll);
    });
  }

  return {start: start, fileName: fileName, actionsFor: actionsFor};
});
