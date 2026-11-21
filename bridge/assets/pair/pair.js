// The pairing page of the bridge (served at /pair). It reads the QR code that a device shows with
// the computer's camera and confirms the pairing at the bridge; a code can also be typed.
// All the browser's things come in through `env`, so the flow can be tested without a browser.
(function (root, factory) {
  if (typeof module === 'object' && module.exports) {
    module.exports = factory();
  } else {
    root.pairPage = factory();
  }
})(typeof self !== 'undefined' ? self : this, function () {
  'use strict';

  var SCAN_EVERY_MS = 150;
  var RETRY_AFTER_REFUSAL_MS = 3000;

  function start(env) {
    var L = env.logic;
    var doc = env.document;
    var $ = function (id) { return doc.getElementById(id); };
    var strings = null;
    var token = '';
    var busy = false;
    var finished = false;
    var stream = null;
    var lastQr = '';

    function say(key, vars) {
      var text = strings && strings[key] ? L.format(strings[key], vars) : key;
      $('status').textContent = text;
      $('status').setAttribute('data-key', key);
    }

    function stopCamera() {
      finished = true;
      if (stream) stream.getTracks().forEach(function (t) { t.stop(); });
    }

    /** Sends the code (with the device's id when it came from a QR code) and says how it went. */
    function submit(code, id) {
      if (busy || finished) return Promise.resolve();
      busy = true;
      say('sending');
      var body = id ? {id: id, code: code} : {code: code};
      return env.fetch('/admin/pair', {
        method: 'POST',
        headers: {'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json'},
        body: JSON.stringify(body),
      }).then(function (res) {
        var key = L.messageFor(res.status);
        if (res.status !== 200) {
          busy = false;
          lastQr = ''; // the same QR code may be tried again, after a pause
          say(key, {why: 'HTTP ' + res.status});
          pauseScanning();
          return;
        }
        return res.json().then(function (j) {
          stopCamera();
          busy = false;
          say('paired', {name: typeof j.name === 'string' ? j.name : ''});
        });
      }, function (err) {
        busy = false;
        say('failed', {why: err && err.message ? err.message : 'network'});
        pauseScanning();
      });
    }

    var paused = false;
    function pauseScanning() {
      paused = true;
      env.setTimeout(function () { paused = false; }, RETRY_AFTER_REFUSAL_MS);
    }

    function onDecoded(text) {
      if (text === lastQr || busy || finished) return;
      lastQr = text;
      var q = L.parseQr(text);
      if (!q) {
        say('notSpellink');
        return;
      }
      submit(q.code, q.id);
    }

    function scan(video, canvas) {
      if (finished) return;
      if (!paused && !busy && video.videoWidth > 0) {
        canvas.width = video.videoWidth;
        canvas.height = video.videoHeight;
        var ctx = canvas.getContext('2d', {willReadFrequently: true});
        ctx.drawImage(video, 0, 0);
        var found;
        if (env.BarcodeDetector) {
          found = env.BarcodeDetector.detect(canvas).then(function (r) { return r.length ? r[0].rawValue : null; });
        } else {
          var img = ctx.getImageData(0, 0, canvas.width, canvas.height);
          var r = env.jsQR(img.data, img.width, img.height);
          found = Promise.resolve(r ? r.data : null);
        }
        found.then(function (text) { if (text) onDecoded(text); }, function () {}).then(function () {
          env.setTimeout(function () { scan(video, canvas); }, SCAN_EVERY_MS);
        });
        return;
      }
      env.setTimeout(function () { scan(video, canvas); }, SCAN_EVERY_MS);
    }

    function startCamera() {
      var md = env.navigator.mediaDevices;
      if (!md || !md.getUserMedia) {
        say('cameraDenied', {why: 'no camera API'});
        return;
      }
      md.getUserMedia({video: {facingMode: 'user', width: {ideal: 1280}}}).then(function (s) {
        stream = s;
        var video = $('video');
        video.srcObject = s;
        video.play();
        say('hint');
        scan(video, doc.createElement('canvas'));
      }, function (err) {
        say('cameraDenied', {why: err && err.name ? err.name : 'unknown'});
      });
    }

    function applyStrings() {
      ['title', 'manualLabel', 'manualButton'].forEach(function (k) { $(k).textContent = strings[k]; });
      doc.title = strings.title;
    }

    function init() {
      return env.fetch('/pair/strings.json').then(function (r) { return r.json(); }).then(function (all) {
        var langs = env.navigator.languages && env.navigator.languages.length ? env.navigator.languages : [env.navigator.language];
        strings = all[L.pickLocale(langs, Object.keys(all))];
        applyStrings();
        token = env.location.hash.replace(/^#/, '');
        if (!token) {
          say('noToken');
          return;
        }
        $('manualButton').onclick = function () {
          var code = L.parseTyped($('manual').value);
          if (!code) {
            say('badCode');
            return;
          }
          submit(code, '');
        };
        startCamera();
      });
    }

    return init();
  }

  return {start: start};
});
