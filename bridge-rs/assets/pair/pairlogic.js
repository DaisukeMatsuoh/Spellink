// The pure parts of the pairing page: what a QR code means, which language to speak, how a reply
// is worded. No DOM here, so they can be tested on their own (plugin/__tests__/pairPage.test.ts).
(function (root, factory) {
  if (typeof module === 'object' && module.exports) {
    module.exports = factory();
  } else {
    root.pairLogic = factory();
  }
})(typeof self !== 'undefined' ? self : this, function () {
  'use strict';

  // What the device shows (plugin/src/pairing.ts): spellink://pair?v=1&i=<16 hex>&c=<8 digits>.
  // Anything else is not a Spellink code, however it looks.
  var QR = /^spellink:\/\/pair\?v=1&i=([0-9a-f]{16})&c=([0-9]{8})$/;
  var CODE = /^[0-9]{8}$/;

  /** {id, code} for a Spellink pairing QR code, or null. */
  function parseQr(text) {
    var m = typeof text === 'string' ? QR.exec(text) : null;
    return m ? {id: m[1], code: m[2]} : null;
  }

  /** The digits of what a person typed (spaces and a dash are allowed), or null unless exactly eight. */
  function parseTyped(text) {
    var digits = String(text).replace(/[\s-]/g, '');
    return CODE.test(digits) ? digits : null;
  }

  // "zh_CN", "zh-CN" and "ZH-cn" are the same language tag
  function norm(tag) {
    return String(tag || '').replace('_', '-').toLowerCase();
  }

  /** The best of `available` for the browser's `wanted` languages (exact, then language part), else "en". */
  function pickLocale(wanted, available) {
    for (var i = 0; i < wanted.length; i++) {
      var w = norm(wanted[i]);
      for (var j = 0; j < available.length; j++) {
        if (norm(available[j]) === w) return available[j];
      }
      var lang = w.split('-')[0];
      for (var k = 0; k < available.length; k++) {
        if (norm(available[k]) === lang) return available[k];
      }
    }
    return 'en';
  }

  /** Fills {name} placeholders; a missing value is left empty. */
  function format(template, vars) {
    return template.replace(/\{(\w+)\}/g, function (_, k) {
      return vars && vars[k] !== undefined ? String(vars[k]) : '';
    });
  }

  /** Which message answers an HTTP status of POST /admin/pair. */
  function messageFor(status) {
    switch (status) {
      case 200: return 'paired';
      case 400: return 'badCode';
      case 401: return 'unauthorized';
      case 403: return 'notAccepted';
      case 404: return 'nobodyWaiting';
      default: return 'failed';
    }
  }

  return {parseQr: parseQr, parseTyped: parseTyped, pickLocale: pickLocale, format: format, messageFor: messageFor};
});
