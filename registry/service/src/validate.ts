// A port of validate(), pick() and fastest() in registry/ingest.py. The two must agree field by
// field on every report: same accept or reject, same reason, same cleaned object. The shared
// vectors in registry/vectors.json hold both to that.

import suite from "../../suite.json";
import catalog from "../../models.json";

export const MIN_SPEED = 10.0;
export const MAX_WER = 0.08;
export const TIE_WER = 0.005;
export const MAX_SPEED = 500.0;

export const SUITE_ID: string = suite.id;
export const SUITE_CLIPS: number = suite.clips;
export const SUITE_SECONDS: number = suite.seconds;
// Reports saved before Silas named its set have no suite field: they were made on the first one.
export const LEGACY_SUITE: string = suite.legacy_ids[0];
export const KNOWN_MODELS: ReadonlySet<string> = new Set(catalog.models);

const OSES = ["macos", "windows", "linux", "ios", "ipados"];
const KEY_RE = /^(macos|windows|linux|ios|ipados)\/[a-z0-9]+(?:-[a-z0-9]+)*\/[0-9]{1,4}$/;
const TEXT_RE = /^[A-Za-z0-9 ()@.,_+/:-]{1,120}$/;
const HW_RE = /^[A-Za-z0-9,._ -]{1,40}$/;
const VERSION_RE = /^[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]{1,40})?$/;

export class Reject extends Error {
  stale = false;
}

/** Valid when it was made, but for a benchmark set or model Silas no longer uses. */
export class Stale extends Reject {
  override stale = true;
}

export interface Device {
  key: string;
  os: string;
  os_version: string;
  cpu: string;
  cores: number;
  memory_gb: number;
  hw_model?: string;
}

export interface Result {
  model: string;
  speed: number;
  wer: number;
  clips: number;
  audio_seconds: number;
  compute_seconds: number;
  wait_seconds: number;
}

export interface Report {
  schema: 1;
  suite: string;
  silas: string;
  device: Device;
  results: Result[];
}

type Obj = Record<string, unknown>;

function isObj(v: unknown): v is Obj {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/** dict.get: own keys only, so nothing on Object.prototype ever answers. */
function get(o: Obj, k: string): unknown {
  return Object.hasOwn(o, k) ? o[k] : undefined;
}

export function slug(s: string): string {
  return s
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

function num(v: unknown, lo: number, hi: number, what: string, integer = false): number {
  // JSON true is a bool, never a number, as in Python.
  if (typeof v !== "number") throw new Reject(`${what} must be a number.`);
  if (!Number.isFinite(v)) throw new Reject(`${what} is out of range.`);
  if (!(lo <= v && v <= hi)) throw new Reject(`${what} is out of range.`);
  if (integer && Math.trunc(v) !== v) throw new Reject(`${what} must be a whole number.`);
  // int(10.0) is 10; float(20) is 20.0, the same number here. + 0 turns the -0 literal (an int
  // in Python, so 0) into 0.
  return integer ? Math.trunc(v) + 0 : v + 0;
}

function text(v: unknown, what: string, pattern: RegExp = TEXT_RE): string {
  if (typeof v !== "string" || !pattern.test(v)) {
    throw new Reject(`${what} has characters or a length Silas doesn't produce.`);
  }
  return v;
}

/**
 * json.loads, as ingest.extract calls it. Python's json also reads the bare tokens NaN,
 * Infinity and -Infinity as numbers; JSON.parse refuses them. Each one in a value position is
 * rewritten to an overflowing literal, which JSON.parse reads as +/-Infinity. validate() treats
 * NaN and Infinity alike (neither is a finite number, so "out of range" or "not 1" or stale), so
 * the decision and reason match Python's for every input. A token Python would refuse (inside a
 * string, after a digit, followed by a digit, "-NaN") is left alone and JSON.parse refuses it too.
 */
export function parseJson(body: string): unknown {
  let out = "";
  let last = 0;
  let inString = false;
  let prev = ""; // the last non-space character outside strings
  for (let i = 0; i < body.length; i++) {
    const c = body[i];
    if (inString) {
      if (c === "\\") i++;
      else if (c === '"') {
        inString = false;
        prev = '"';
      }
      continue;
    }
    if (c === '"') {
      inString = true;
      continue;
    }
    if (c === " " || c === "\t" || c === "\n" || c === "\r") continue;
    const valuePos = prev === "" || prev === "[" || prev === "," || prev === ":";
    if (valuePos) {
      let token = "";
      if (body.startsWith("NaN", i)) token = "NaN";
      else if (body.startsWith("Infinity", i)) token = "Infinity";
      else if (body.startsWith("-Infinity", i)) token = "-Infinity";
      const after = body[i + token.length];
      if (token && !(after !== undefined && after >= "0" && after <= "9")) {
        out += body.slice(last, i) + (token === "-Infinity" ? "-1e99999" : "1e99999");
        last = i + token.length;
        i += token.length - 1;
        prev = "9";
        continue;
      }
    }
    prev = c;
  }
  return JSON.parse(out + body.slice(last));
}

/** The JSON body as ingest.extract turns it into an object, with its reasons. */
export function readReport(body: string): Obj {
  let data: unknown;
  try {
    data = parseJson(body);
  } catch {
    throw new Reject("The report isn't valid JSON. Please submit it from Silas without editing it.");
  }
  if (!isObj(data)) throw new Reject("The report isn't a JSON object.");
  return data;
}

export function validate(data: Obj): Report {
  const schema = get(data, "schema");
  if (schema !== 1) throw new Reject("Unknown report schema.");
  const s = Object.hasOwn(data, "suite") ? data.suite : LEGACY_SUITE;
  if (s !== SUITE_ID) {
    throw new Stale("This report was made with an older benchmark set. Update Silas and run the benchmark again.");
  }
  const silas = text(get(data, "silas"), "silas", VERSION_RE);
  const d = get(data, "device");
  if (!isObj(d)) throw new Reject("The report has no device.");
  const osName = get(d, "os");
  if (typeof osName !== "string" || !OSES.includes(osName)) throw new Reject("Unknown operating system.");
  const cpu = text(get(d, "cpu"), "device.cpu");
  const cores = num(get(d, "cores"), 1, 1024, "device.cores", true);
  const key = text(get(d, "key"), "device.key", KEY_RE);
  if (key !== `${osName}/${slug(cpu)}/${cores}`) {
    throw new Reject("device.key doesn't match the device's OS, CPU and core count.");
  }
  const device: Device = {
    key,
    os: osName,
    os_version: text(get(d, "os_version"), "device.os_version"),
    cpu,
    cores,
    memory_gb: num(get(d, "memory_gb"), 0, 65536, "device.memory_gb", true),
  };
  const hw = get(d, "hw_model");
  if (hw !== undefined && hw !== null) device.hw_model = text(hw, "device.hw_model", HW_RE);

  const results = get(data, "results");
  if (!Array.isArray(results) || !(1 <= results.length && results.length <= 10)) {
    throw new Reject("The report needs between 1 and 10 model results.");
  }
  const clean: Result[] = [];
  const seen = new Set<string>();
  for (const r of results) {
    if (!isObj(r)) throw new Reject("Each result must be an object.");
    const model = get(r, "model");
    if (typeof model !== "string") throw new Reject("The report names a model Silas doesn't offer.");
    if (!KNOWN_MODELS.has(model)) throw new Stale("The report names a model Silas doesn't offer.");
    if (seen.has(model)) throw new Reject("The report lists a model twice.");
    seen.add(model);
    const res: Result = {
      model,
      speed: num(get(r, "speed"), 0.01, MAX_SPEED, `${model} speed`),
      wer: num(get(r, "wer"), 0, 1, `${model} wer`),
      clips: num(get(r, "clips"), 1, 1000, `${model} clips`, true),
      audio_seconds: num(get(r, "audio_seconds"), 1, 100_000, `${model} audio_seconds`),
      compute_seconds: num(get(r, "compute_seconds"), 0.0001, 100_000, `${model} compute_seconds`),
      // The wait after you stop talking: median compute seconds of the 5 to 10 s clips.
      wait_seconds: num(get(r, "wait_seconds"), 0.0001, 100, `${model} wait_seconds`),
    };
    // The numbers have to come from Silas's own set and agree with each other.
    if (res.clips !== SUITE_CLIPS || Math.abs(res.audio_seconds - SUITE_SECONDS) > 1.0) {
      throw new Reject(`${model} wasn't measured on Silas's benchmark set.`);
    }
    if (Math.abs(res.audio_seconds / res.compute_seconds - res.speed) > 0.02 * res.speed) {
      throw new Reject(`${model} speed doesn't match its audio and compute seconds.`);
    }
    if (res.wait_seconds > res.compute_seconds) {
      throw new Reject(`${model} wait_seconds is longer than the whole run.`);
    }
    clean.push(res);
  }
  return { schema: 1, suite: SUITE_ID, silas, device, results: clean };
}

export interface Scored {
  model: string;
  speed: number;
  wer: number;
}

/** The fastest; on an exact tie the lower model id, as in Silas. */
export function fastest(rs: Scored[]): string {
  let best = rs[0];
  for (const r of rs.slice(1)) {
    if (r.speed > best.speed || (r.speed === best.speed && r.model < best.model)) best = r;
  }
  return best.model;
}

/** Same rule as benchmark::pick in Silas. */
export function pick(results: Scored[]): string | null {
  if (results.length === 0) return null;
  const passing = results.filter((r) => r.speed >= MIN_SPEED && r.wer <= MAX_WER);
  if (passing.length > 0) {
    const best = Math.min(...passing.map((r) => r.wer));
    return fastest(passing.filter((r) => r.wer - best <= TIE_WER));
  }
  const accurate = results.filter((r) => r.wer <= MAX_WER);
  return fastest(accurate.length > 0 ? accurate : results);
}

/** statistics.median: the middle value, or the mean of the middle two. */
export function median(values: number[]): number {
  const data = [...values].sort((a, b) => a - b);
  const n = data.length;
  if (n === 0) throw new Error("no median for empty data");
  if (n % 2 === 1) return data[(n - 1) / 2];
  const i = n / 2;
  return (data[i - 1] + data[i]) / 2;
}
