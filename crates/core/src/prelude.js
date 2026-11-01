// surimono sandbox prelude. Runs once in every new context, before any
// script, as a function of one config object. Keep it small and free of
// observable globals: it is part of every render.
(function (config) {
  'use strict';

  // First of all: switch off V8's stack-trace API, which quickjs-ng
  // implements. Through `Error.prepareStackTrace`, `CallSite.getFunction()`
  // hands a script the function of every frame on the stack, native ones
  // included, such as the real `Date` behind the shim below.
  // QuickJS reads the hook only through the accessor this replaces, so a
  // plain data property makes assignments harmless and stacks stay strings.
  Object.defineProperty(Error, 'prepareStackTrace', {
    value: undefined, writable: true, enumerable: false, configurable: true,
  });

  const apply = Reflect.apply;
  const defineProperty = Object.defineProperty;

  // Long native loops (join, fill, sort... over an array-like with a huge
  // `length`) used to need guards here. The patched QuickJS
  // (vendor/rquickjs-sys/PATCHES.md) polls for interrupts inside them, so
  // the time limit stops them like any other loop, and the built-ins stay
  // native.

  function replace(target, name, fn, length) {
    defineProperty(fn, 'length', { value: length, configurable: true });
    defineProperty(target, name, {
      value: fn,
      writable: true,
      enumerable: false,
      configurable: true,
    });
  }

  // --- Determinism -----------------------------------------------------------
  //
  // Nothing a script can read may depend on the host: Math.random is seeded,
  // the clock is fixed, local time is a fixed offset from UTC (QuickJS would
  // otherwise read the host's zone), and weak references never empty.

  const imul = Math.imul;
  const construct = Reflect.construct;
  const TypeErrorCtor = TypeError;
  const symbolKeyFor = Symbol.keyFor;
  const nowMs = config.nowMs;
  const offsetMinutes = config.utcOffsetMinutes;
  const offsetMs = offsetMinutes * 60000;

  // Math.random: xoshiro128** seeded through splitmix32, 53-bit results.
  function splitmix32(a) {
    return function () {
      a = (a + 0x9e3779b9) | 0;
      let t = a ^ (a >>> 16);
      t = imul(t, 0x21f0aaad);
      t = t ^ (t >>> 15);
      t = imul(t, 0x735a2d97);
      return (t ^ (t >>> 15)) | 0;
    };
  }
  function rotl(x, k) {
    return (x << k) | (x >>> (32 - k));
  }
  const seedLo = splitmix32(config.seedLo | 0);
  const seedHi = splitmix32(config.seedHi | 0);
  let s0 = seedLo(), s1 = seedHi(), s2 = seedLo(), s3 = seedHi();
  if ((s0 | s1 | s2 | s3) === 0) s0 = 1;
  function next32() {
    const r = imul(rotl(imul(s1, 5), 7), 9);
    const t = s1 << 9;
    s2 ^= s0;
    s3 ^= s1;
    s1 ^= s2;
    s0 ^= s3;
    s2 ^= t;
    s3 = rotl(s3, 11);
    return r >>> 0;
  }
  replace(Math, 'random', {
    random() {
      const hi = next32() >>> 5;
      const lo = next32() >>> 6;
      return (hi * 67108864 + lo) / 9007199254740992;
    },
  }.random, 0);

  // Date: fixed "now", local time = UTC + offsetMinutes.
  const RealDate = Date;
  const DP = RealDate.prototype;
  const nativeGetTime = DP.getTime;
  const nativeSetTime = DP.setTime;
  const nativeUTC = RealDate.UTC;
  const utc = {};
  for (const name of ['FullYear', 'Month', 'Date', 'Day', 'Hours', 'Minutes',
                      'Seconds', 'Milliseconds']) {
    utc['get' + name] = DP['getUTC' + name];
    if (name !== 'Day') utc['set' + name] = DP['setUTC' + name];
  }
  // One scratch Date, set to the local wall-clock instant and read with the
  // UTC accessors. Never escapes and never runs script code.
  const scratch = new RealDate(0);

  function timeOf(d) {
    return apply(nativeGetTime, d, []);
  }
  function atLocal(t) {
    apply(nativeSetTime, scratch, [t + offsetMs]);
    return scratch;
  }
  function field(local, name) {
    return apply(utc['get' + name], local, []);
  }

  for (const name of ['FullYear', 'Month', 'Date', 'Day', 'Hours', 'Minutes',
                      'Seconds', 'Milliseconds']) {
    const getter = utc['get' + name];
    replace(DP, 'get' + name, {
      ['get' + name]() {
        const t = timeOf(this);
        if (t !== t) return NaN;
        return apply(getter, atLocal(t), []);
      },
    }['get' + name], 0);
  }

  for (const name of ['FullYear', 'Month', 'Date', 'Hours', 'Minutes',
                      'Seconds', 'Milliseconds']) {
    const setter = utc['set' + name];
    const length = DP['set' + name].length;
    replace(DP, 'set' + name, {
      ['set' + name](...args) {
        const t = timeOf(this);
        // setFullYear on an invalid date starts from local midnight 1970.
        const local = t === t ? t + offsetMs : (name === 'FullYear' ? 0 : NaN);
        apply(nativeSetTime, scratch, [local]);
        apply(setter, scratch, args);
        return apply(nativeSetTime, this, [timeOf(scratch) - offsetMs]);
      },
    }['set' + name], length);
  }

  if (typeof DP.getYear === 'function') {
    replace(DP, 'getYear', {
      getYear() {
        const t = timeOf(this);
        return t !== t ? NaN : field(atLocal(t), 'FullYear') - 1900;
      },
    }.getYear, 0);
  }
  if (typeof DP.setYear === 'function') {
    const setFullYear = DP.setFullYear;
    const trunc = Math.trunc;
    replace(DP, 'setYear', {
      setYear(year) {
        timeOf(this);
        const y = +year;
        if (y !== y) return apply(nativeSetTime, this, [NaN]);
        const yi = trunc(y);
        return apply(setFullYear, this, [yi >= 0 && yi <= 99 ? 1900 + yi : y]);
      },
    }.setYear, 1);
  }

  replace(DP, 'getTimezoneOffset', {
    getTimezoneOffset() {
      const t = timeOf(this);
      return t !== t ? NaN : 0 - offsetMinutes;
    },
  }.getTimezoneOffset, 0);

  // Formatting, matching Chrome's output for en-US.
  const dayNames = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
  const monthNames = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug',
                      'Sep', 'Oct', 'Nov', 'Dec'];
  function pad(n, width) {
    let s = '' + n;
    while (s.length < width) s = '0' + s;
    return s;
  }
  // ECMA-262 DateString: sign, then at least four digits.
  function yearString(y) {
    return y < 0 ? '-' + pad(-y, 4) : pad(y, 4);
  }
  function zoneString() {
    const a = offsetMinutes < 0 ? -offsetMinutes : offsetMinutes;
    return 'GMT' + (offsetMinutes < 0 ? '-' : '+') + pad((a / 60) | 0, 2) +
      pad(a % 60, 2) + (offsetMinutes === 0 ? ' (Coordinated Universal Time)' : '');
  }
  function dateString(t) {
    const l = atLocal(t);
    return dayNames[field(l, 'Day')] + ' ' + monthNames[field(l, 'Month')] + ' ' +
      pad(field(l, 'Date'), 2) + ' ' + yearString(field(l, 'FullYear'));
  }
  function timeString(t) {
    const l = atLocal(t);
    return pad(field(l, 'Hours'), 2) + ':' + pad(field(l, 'Minutes'), 2) + ':' +
      pad(field(l, 'Seconds'), 2) + ' ' + zoneString();
  }
  function localeDate(t) {
    const l = atLocal(t);
    return (field(l, 'Month') + 1) + '/' + field(l, 'Date') + '/' + field(l, 'FullYear');
  }
  function localeTime(t) {
    const l = atLocal(t);
    const h = field(l, 'Hours');
    return ((h % 12) || 12) + ':' + pad(field(l, 'Minutes'), 2) + ':' +
      pad(field(l, 'Seconds'), 2) + ' ' + (h < 12 ? 'AM' : 'PM');
  }
  function fullString(t) {
    return t !== t ? 'Invalid Date' : dateString(t) + ' ' + timeString(t);
  }
  function formatter(name, fn) {
    replace(DP, name, {
      [name]() {
        const t = timeOf(this);
        return t !== t ? 'Invalid Date' : fn(t);
      },
    }[name], 0);
  }
  formatter('toString', fullString);
  formatter('toDateString', dateString);
  formatter('toTimeString', timeString);
  formatter('toLocaleString', (t) => localeDate(t) + ', ' + localeTime(t));
  formatter('toLocaleDateString', localeDate);
  formatter('toLocaleTimeString', localeTime);

  // --- Date.parse ----------------------------------------------------------
  //
  // QuickJS's parser reads a string without a zone as local time in the
  // host's zone, and no pattern can tell reliably from outside which strings
  // it treats as zone-less. So this is a port of quickjs-ng's own parser
  // (js_Date_parse and its helpers in quickjs.c, MIT licence, the same
  // version we link), which reports the fields, the zone and whether the
  // string was local. Local strings use the sandbox offset; the host's zone
  // never enters. A test checks it against the native parser on a UTC host.
  //
  // It reads only the string and its own constants, through functions
  // captured above, so nothing a script changes can reach into it.

  const charCodeAt = String.prototype.charCodeAt;
  const monthAbbrevs = 'JanFebMarAprMayJunJulAugSepOctNovDec';
  // quickjs.c js_tzabbr, in its order (UTC before UT, WET before WEST).
  const zoneNames = ['GMT', 'UTC', 'UT', 'Z', 'EDT', 'EST', 'CDT', 'CST', 'MDT', 'MST',
                     'PDT', 'PST', 'WET', 'WEST', 'CET', 'CEST', 'EET', 'EEST'];
  const zoneOffsets = [0, 0, 0, 0, -240, -300, -300, -360, -360, -420, -420, -480,
                       0, 60, 60, 120, 120, 180];
  // ECMA-262 MakeDay, MakeTime and MakeDate (§21.4.1.27 to .29) for
  // integer fields, without TimeClip: the caller converts local time to
  // UTC first and clips after (TimeClip(UTC(t)), §21.4.2.1 step 5), so a
  // local time past the end of the range in a zone ahead of UTC is still
  // valid. Years outside [-271821, 275760] give NaN, as in QuickJS's
  // set_date_fields: no day of theirs is a valid time value. Same operation
  // order as the spec and set_date_fields, so rounding matches too.
  const floor = Math.floor;
  const mathTrunc = Math.trunc;
  const daysBeforeMonth = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
  function makeDate(y, mon, day, h, min, sec, ms) {
    const ym = y + floor(mon / 12);
    if (!(ym >= -271821 && ym <= 275760)) return NaN;
    let mn = mon % 12;
    if (mn < 0) mn += 12;
    let days = 365 * (ym - 1970) + floor((ym - 1969) / 4) -
      floor((ym - 1901) / 100) + floor((ym - 1601) / 400) + daysBeforeMonth[mn];
    if (mn > 1 && ym % 4 === 0 && (ym % 100 !== 0 || ym % 400 === 0)) days++;
    const time = h * 3600000 + min * 60000 + sec * 1000 + ms;
    return (days + day - 1) * 86400000 + time;
  }
  // §21.4.1.31 TimeClip.
  function timeClip(t) {
    return t >= -8.64e15 && t <= 8.64e15 ? mathTrunc(t) + 0 : NaN;
  }

  let src = '';    // the string being parsed
  let srcLen = 0;  // where QuickJS's C string ends: first NUL, at most 127
  let dv = 0;      // value read by the last successful digitsAt
  const F = [0, 0, 0, 0, 0, 0, 0, 0, 0]; // year, month, day, h, m, s, ms, -, zone
  let isLocal = false;

  // The byte QuickJS sees at p: code units above 255 become 'x', except
  // U+2212 MINUS SIGN, which becomes '-'.
  function ch(p) {
    if (p >= srcLen) return 0;
    const c = apply(charCodeAt, src, [p]);
    return c > 255 ? (c === 0x2212 ? 45 : 120) : c;
  }
  function upper(c) {
    return c >= 97 && c <= 122 ? c - 32 : c;
  }
  function isDigit(c) {
    return c >= 48 && c <= 57;
  }
  // string_get_digits: returns the new position and sets dv, or -1.
  function digitsAt(p, minDigits, maxDigits) {
    const start = p;
    let v = 0;
    let c;
    while (isDigit(c = ch(p))) {
      if (v >= 100000000) return -1;
      v = v * 10 + c - 48;
      p++;
      if (p - start === maxDigits) break;
    }
    if (p - start < minDigits) return -1;
    dv = v;
    return p;
  }
  // string_get_milliseconds: sets F[6] only if digits follow the separator.
  function millisAt(p) {
    let c = ch(p);
    if (c === 46 || c === 44) {
      p++;
      const start = p;
      let mul = 100;
      let ms = 0;
      while (isDigit(c = ch(p))) {
        ms += (c - 48) * mul;
        mul = (mul / 10) | 0;
        p++;
        if (p - start === 9) break;
      }
      if (p > start) {
        F[6] = ms;
        return p;
      }
      return p - 1;
    }
    return p;
  }
  // string_get_tzoffset: returns the new position and sets F[8], or -1.
  function zoneOffsetAt(p, strict) {
    const sgn = ch(p++);
    let tz = 0;
    if (sgn === 43 || sgn === 45) {
      const start = p;
      p = digitsAt(p, 1, 0);
      if (p < 0) return -1;
      let hh = dv;
      let n = p - start;
      if (strict && n !== 2 && n !== 4) return -1;
      while (n > 4) {
        n -= 2;
        hh = (hh / 100) | 0;
      }
      let mm = 0;
      if (n > 2) {
        mm = hh % 100;
        hh = (hh / 100) | 0;
      } else if (ch(p) === 58) {
        p = digitsAt(p + 1, 2, 2);
        if (p < 0) return -1;
        mm = dv;
      }
      if (hh > 23 || mm > 59) return -1;
      tz = hh * 60 + mm;
      if (sgn !== 43) tz = -tz;
    } else if (sgn !== 90) {
      return -1;
    }
    F[8] = tz;
    return p;
  }
  // string_match, case-insensitive: returns the position after s, or -1.
  function matchAt(p, s) {
    const n = s.length;
    for (let i = 0; i < n; i++) {
      if (upper(ch(p + i)) !== upper(apply(charCodeAt, s, [i]))) return -1;
    }
    return p + n;
  }
  // string_get_month: returns the position after the abbreviation, or -1.
  function monthAt(p) {
    for (let m = 0; m < 12; m++) {
      let i = 0;
      while (upper(ch(p + i)) === upper(apply(charCodeAt, monthAbbrevs, [m * 3 + i]))) {
        if (i === 2) {
          F[1] = m + 1;
          return p + 3;
        }
        i++;
      }
    }
    return -1;
  }
  // string_skip_until: stops at NUL or any character of stop.
  function skipUntil(p, stop) {
    for (;;) {
      const c = ch(p);
      if (c === 0) return p;
      for (let i = 0; i < stop.length; i++) {
        if (apply(charCodeAt, stop, [i]) === c) return p;
      }
      p++;
    }
  }

  // js_date_parse_isostring
  function parseIso() {
    for (let i = 0; i < 9; i++) F[i] = i === 2 ? 1 : 0;
    isLocal = false;
    let p = 0;
    const sgn = ch(p);
    if (sgn === 45 || sgn === 43) {
      p = digitsAt(p + 1, 6, 6);
      if (p < 0) return false;
      F[0] = dv;
      if (sgn === 45) {
        if (dv === 0) return false;
        F[0] = -dv;
      }
    } else {
      p = digitsAt(p, 4, 4);
      if (p < 0) return false;
      F[0] = dv;
    }
    if (ch(p) === 45) {
      p = digitsAt(p + 1, 2, 2);
      if (p < 0 || dv < 1) return false;
      F[1] = dv - 1;
      if (ch(p) === 45) {
        p = digitsAt(p + 1, 2, 2);
        if (p < 0 || dv < 1) return false;
        F[2] = dv;
      }
    }
    if (ch(p) === 84) {
      isLocal = true;
      let q = digitsAt(p + 1, 2, 2);
      if (q < 0 || ch(q) !== 58) {
        F[3] = 100; // rejected by the range check, as in QuickJS
        return true;
      }
      F[3] = dv;
      q = digitsAt(q + 1, 2, 2);
      if (q < 0) {
        F[3] = 100;
        return true;
      }
      F[4] = dv;
      p = q;
      if (ch(p) === 58) {
        p = digitsAt(p + 1, 2, 2);
        if (p < 0) return false;
        F[5] = dv;
        p = millisAt(p);
      }
    }
    if (ch(p) !== 0) {
      isLocal = false;
      p = zoneOffsetAt(p, true);
      if (p < 0) return false;
    }
    return ch(p) === 0;
  }

  function twoDigitYear(v) {
    return v + (v < 100 ? 1900 : 0) + (v < 50 ? 100 : 0);
  }

  // js_date_parse_otherstring
  function parseOther() {
    F[0] = 2001;
    F[1] = 1;
    F[2] = 1;
    for (let i = 3; i < 9; i++) F[i] = 0;
    isLocal = true;
    let hasYear = false;
    let hasMon = false;
    let hasTime = false;
    let count = 0;
    let n0 = 0;
    let n1 = 0;
    let n2 = 0;
    let p = 0;
    for (;;) {
      while (ch(p) === 32) p++;
      const c = ch(p);
      if (c === 0) break;
      const start = p;
      let q;
      if (c === 43 || c === 45) {
        q = hasTime ? zoneOffsetAt(p, false) : -1;
        if (q >= 0) {
          p = q;
          isLocal = false;
        } else {
          p++;
          q = digitsAt(p, 1, 0);
          if (q >= 0) {
            p = q;
            if (c === 45) {
              if (dv === 0) return false;
              F[0] = -dv;
            } else {
              F[0] = dv;
            }
            hasYear = true;
          }
        }
      } else if ((q = digitsAt(p, 1, 0)) >= 0) {
        p = q;
        const val = dv;
        if (ch(p) === 58) {
          F[3] = val;
          p = digitsAt(p + 1, 1, 2);
          if (p < 0) return false;
          F[4] = dv;
          if (ch(p) === 58) {
            p = digitsAt(p + 1, 1, 2);
            if (p < 0) return false;
            F[5] = dv;
            p = millisAt(p);
          } else if (ch(p) !== 0 && ch(p) !== 32) {
            return false;
          }
          hasTime = true;
        } else if (p - start > 2) {
          F[0] = val;
          hasYear = true;
        } else if (val < 1 || val > 31) {
          F[0] = twoDigitYear(val);
          hasYear = true;
        } else {
          if (count === 3) return false;
          if (count === 0) n0 = val;
          else if (count === 1) n1 = val;
          else n2 = val;
          count++;
        }
      } else if ((q = monthAt(p)) >= 0) {
        hasMon = true;
        p = skipUntil(q, '0123456789 -/(');
      } else if (hasTime && (q = matchAt(p, 'PM')) >= 0) {
        p = q;
        if (F[3] !== 12) F[3] += 12;
        continue;
      } else if (hasTime && (q = matchAt(p, 'AM')) >= 0) {
        p = q;
        if (F[3] > 12) return false;
        if (F[3] === 12) F[3] -= 12;
        continue;
      } else {
        let zone = -1;
        for (let i = 0; i < zoneNames.length && zone < 0; i++) {
          q = matchAt(p, zoneNames[i]);
          if (q >= 0) zone = i;
        }
        if (zone >= 0) {
          p = q;
          F[8] = zoneOffsets[zone];
          isLocal = false;
          continue;
        }
        if (c === 40) {
          let level = 0;
          let cc;
          while ((cc = ch(p)) !== 0) {
            p++;
            if (cc === 40) level++;
            if (cc === 41) level--;
            if (level === 0) break;
          }
          if (level > 0) return false;
        } else if (c === 41) {
          return false;
        } else {
          if (hasYear || hasMon || hasTime || count > 0) return false;
          p = skipUntil(p, ' -/(');
        }
      }
      // string_skip_separators
      let s;
      while ((s = ch(p)) === 45 || s === 47 || s === 46 || s === 44) p++;
    }
    if (count + hasYear + hasMon > 3) return false;
    if (count === 0) {
      if (!hasYear) return false;
    } else if (count === 1) {
      if (hasMon) F[2] = n0;
      else F[1] = n0;
    } else if (count === 2) {
      if (hasYear) {
        F[1] = n0;
        F[2] = n1;
      } else if (hasMon) {
        F[0] = twoDigitYear(n1);
        F[2] = n0;
      } else {
        F[1] = n0;
        F[2] = n1;
      }
    } else {
      F[0] = twoDigitYear(n2);
      F[1] = n0;
      F[2] = n1;
    }
    if (F[1] < 1 || F[2] < 1) return false;
    F[1] -= 1;
    return true;
  }

  // js_Date_parse, with local time at the sandbox's offset.
  function parse(input) {
    src = `${input}`;
    const len = src.length < 127 ? src.length : 127;
    srcLen = 0;
    while (srcLen < len && apply(charCodeAt, src, [srcLen]) !== 0) srcLen++;
    const ok = parseIso() || parseOther();
    src = '';
    if (!ok || F[1] > 11 || F[2] > 31 || F[3] > 24 || F[4] > 59 || F[5] > 59 ||
        (F[3] === 24 && (F[4] | F[5] | F[6]) !== 0)) {
      return NaN;
    }
    // set_date_fields(fields, is_local): MakeDay, MakeTime, MakeDate, local
    // time or the string's zone to UTC, then TimeClip. QuickJS applies a
    // zone given in the string after clipping, so near the ends of the
    // range it returns values that are no time value (and NaN for some
    // valid ones). The result must be a time value (ECMA-262 §21.4.1.1,
    // §21.4.1.31), so here the zone goes before TimeClip; Chrome gives NaN
    // past the end too (review of #45).
    let t = makeDate(F[0], F[1], F[2], F[3], F[4], F[5], F[6]);
    if (isLocal) t -= offsetMs;
    return timeClip(t - F[8] * 60000);
  }

  // ECMA-262 §7.1.1 ToPrimitive with no hint, run here rather than in the
  // native Date constructor so a string result goes through `parse`.
  const toPrimitiveKey = Symbol.toPrimitive;
  function isObject(v) {
    return v !== null && (typeof v === 'object' || typeof v === 'function');
  }
  function toPrimitive(input) {
    const exotic = input[toPrimitiveKey];
    if (exotic !== undefined && exotic !== null) {
      if (typeof exotic !== 'function') {
        throw new TypeErrorCtor('Symbol.toPrimitive is not a function');
      }
      const r = apply(exotic, input, ['default']);
      if (!isObject(r)) return r;
      throw new TypeErrorCtor('cannot convert object to primitive value');
    }
    const valueOf = input.valueOf;
    if (typeof valueOf === 'function') {
      const r = apply(valueOf, input, []);
      if (!isObject(r)) return r;
    }
    const toString = input.toString;
    if (typeof toString === 'function') {
      const r = apply(toString, input, []);
      if (!isObject(r)) return r;
    }
    throw new TypeErrorCtor('cannot convert object to primitive value');
  }

  // §21.4.2.1 step 4: the time value for `new Date(value)`.
  function timeValueOf(value) {
    if (isObject(value)) {
      // A Date (or subclass instance): copy its time value, running no
      // script code. Anything else throws a TypeError here.
      let isDate = true;
      let t = 0;
      try {
        t = apply(nativeGetTime, value, []);
      } catch (e) {
        isDate = false;
      }
      if (isDate) return t;
      value = toPrimitive(value);
    }
    // Numbers and the like go to the native constructor, which converts
    // them without consulting the zone.
    return typeof value === 'string' ? parse(value) : value;
  }

  const DateShim = function Date(year, month, date, hours, minutes, seconds, ms) {
    if (new.target === undefined) return fullString(nowMs);
    const n = arguments.length;
    let v;
    if (n === 0) {
      v = nowMs;
    } else if (n === 1) {
      v = timeValueOf(year);
    } else {
      // §21.4.2.1 step 5: ToNumber on every argument, in order; two-digit
      // years are in the 1900s; then TimeClip(UTC(MakeDate(...))), so the
      // offset applies before clipping.
      const f = [0, 0, 1, 0, 0, 0, 0];
      for (let i = 0; i < n && i < 7; i++) f[i] = +arguments[i];
      let finite = true;
      for (let i = 0; i < 7; i++) {
        if (f[i] - f[i] === 0) f[i] = mathTrunc(f[i]);
        else finite = false;
      }
      if (f[0] >= 0 && f[0] <= 99) f[0] += 1900;
      v = finite ? timeClip(makeDate(f[0], f[1], f[2], f[3], f[4], f[5], f[6]) - offsetMs)
        : NaN;
    }
    return construct(RealDate, [v], new.target);
  };
  defineProperty(DateShim, 'prototype', {
    value: DP, writable: false, enumerable: false, configurable: false,
  });
  replace(DateShim, 'now', { now() { return nowMs; } }.now, 0);
  replace(DateShim, 'parse', { parse(s) { return parse(s); } }.parse, 1);
  replace(DateShim, 'UTC', nativeUTC, nativeUTC.length);
  defineProperty(DP, 'constructor', {
    value: DateShim, writable: true, enumerable: false, configurable: true,
  });
  defineProperty(globalThis, 'Date', {
    value: DateShim, writable: true, enumerable: false, configurable: true,
  });

  // performance: a virtual clock that stands still (timers come with the
  // phase-2 event loop).
  const performance = {};
  replace(performance, 'now', { now() { return 0; } }.now, 0);
  defineProperty(performance, 'timeOrigin', {
    value: nowMs, writable: false, enumerable: true, configurable: true,
  });
  defineProperty(globalThis, 'performance', {
    value: performance, writable: true, enumerable: true, configurable: true,
  });

  // WeakRef and FinalizationRegistry depend on when the GC runs, which
  // depends on earlier renders in the same runtime. Make them strong: deref
  // always returns the target and cleanup callbacks never run. The context
  // is thrown away after the render, so nothing is kept for long.
  function canBeHeldWeakly(v) {
    return (typeof v === 'object' && v !== null) || typeof v === 'function' ||
      (typeof v === 'symbol' && apply(symbolKeyFor, Symbol, [v]) === undefined);
  }
  class WeakRef {
    #target;
    constructor(target) {
      if (!canBeHeldWeakly(target)) {
        throw new TypeErrorCtor('WeakRef: invalid target');
      }
      this.#target = target;
    }
    deref() {
      return this.#target;
    }
  }
  class FinalizationRegistry {
    #cleanup;
    constructor(cleanup) {
      if (typeof cleanup !== 'function') {
        throw new TypeErrorCtor('FinalizationRegistry: cleanup must be callable');
      }
      this.#cleanup = cleanup;
    }
    register(target, heldValue, token) {
      this.#cleanup;
      if (!canBeHeldWeakly(target)) {
        throw new TypeErrorCtor('FinalizationRegistry.register: invalid target');
      }
      if (target === heldValue) {
        throw new TypeErrorCtor('FinalizationRegistry.register: target and held value are the same');
      }
      if (token !== undefined && !canBeHeldWeakly(token)) {
        throw new TypeErrorCtor('FinalizationRegistry.register: invalid token');
      }
    }
    unregister(token) {
      this.#cleanup;
      if (!canBeHeldWeakly(token)) {
        throw new TypeErrorCtor('FinalizationRegistry.unregister: invalid token');
      }
      return false;
    }
  }
  for (const [name, cls] of [['WeakRef', WeakRef],
                             ['FinalizationRegistry', FinalizationRegistry]]) {
    defineProperty(cls.prototype, Symbol.toStringTag, {
      value: name, writable: false, enumerable: false, configurable: true,
    });
    defineProperty(globalThis, name, {
      value: cls, writable: true, enumerable: false, configurable: true,
    });
  }

  // A silent console, so library warnings don't throw ReferenceErrors.
  const console = {};
  for (const name of ['debug', 'error', 'info', 'log', 'trace', 'warn']) {
    replace(console, name, { [name]() {} }[name], 0);
  }
  defineProperty(globalThis, 'console', {
    value: console, writable: true, enumerable: false, configurable: true,
  });
})
