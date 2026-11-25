//! Leaves for the TypeScript backend (ts.sys): types, codecs and names as
//! TypeScript text, and the fixed runtimes of client.ts and hooks.ts.

use crate::vcc::types::*;

fn render(t: &Table, r: &TyRef, q: &str) -> String {
    match r {
        TyRef::Prim(p) => match &**p {
            "int" | "float" => "number".into(),
            "bool" => "boolean".into(),
            "string" => "string".into(),
            _ => "void".into(),
        },
        // `readonly readonly T[][]` doesn't parse.
        TyRef::Array(e) if matches!(**e, TyRef::Array(_)) => {
            format!("readonly ({})[]", render(t, e, q))
        }
        TyRef::Array(e) => format!("readonly {}[]", render(t, e, q)),
        TyRef::Map(m) => format!(
            "ReadonlyMap<{}, {}>",
            render(t, &m.key, q),
            render(t, &m.value, q)
        ),
        TyRef::Named(i) => format!("{q}{}", t.types[*i as usize].name),
    }
}

/// Writes a table type as TypeScript: int and float as number, bool as
/// boolean, string and void as themselves, arrays as `readonly T[]`, maps as
/// `ReadonlyMap<K, V>`, and named types by their table name after qual.
pub fn ts_render(t: Table, r: TyRef, qual: Str) -> Result<Str, String> {
    Ok(render(&t, &r, &qual).into())
}

/// Writes a vandorf type as TsRender does, through the table.
pub fn ts_type(t: Table, ty: Type, qual: Str) -> Result<Str, String> {
    let r = super::gen_leaf::get(t.clone(), ty)?;
    Ok(render(&t, &r, &qual).into())
}

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                out.push_str(&format!("\\u{:04x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Writes a string as a TypeScript string literal, escaped as JSON.
pub fn ts_quote(s: Str) -> Result<Str, String> {
    Ok(quote(&s).into())
}

/// A doc comment as JSDoc lines: one line as `/** .. */`, more as a block.
fn jsdoc(doc: &str) -> Vec<Line> {
    let doc = doc.replace("*/", "*\\/");
    let lines: Vec<&str> = doc.split('\n').collect();
    if lines.len() == 1 {
        return vec![Line::Raw(format!("/** {} */\n", lines[0]).into())];
    }
    let mut out = String::from("/**\n");
    for l in lines {
        if l.is_empty() {
            out.push_str(" *\n");
        } else {
            out.push_str(&format!(" * {l}\n"));
        }
    }
    out.push_str(" */\n");
    vec![Line::Raw(out.into())]
}

/// Writes a type's doc comment: its typedef's (or "A typedef." if it has
/// none), or "The type of the <origin>." for a shape named where it first
/// appears.
pub fn ts_doc(t: Table, n: NamedType) -> Result<List<Line>, String> {
    let doc = super::gen_leaf::origin_doc(&t, &n);
    Ok(if !doc.is_empty() {
        jsdoc(&doc)
    } else if *n.origin == *format!("typedef {}", n.name) {
        jsdoc("A typedef.")
    } else {
        jsdoc(&format!("The type of the {}.", n.origin))
    }
    .into())
}

/// Writes the doc comment of the typedef alias `name`: its typedef's, or "A
/// typedef.".
pub fn ts_alias_doc(t: Table, name: Str) -> Result<List<Line>, String> {
    let doc = super::gen_leaf::typedef_doc(&t, &name);
    Ok(jsdoc(if doc.is_empty() { "A typedef." } else { &doc }).into())
}

/// Writes a doc comment for a route's function or hook: the proc's doc, if
/// it has one, then what.
pub fn ts_route_doc(doc: Str, what: Str) -> Result<List<Line>, String> {
    let what = wrap(&what, 76);
    Ok(if doc.is_empty() {
        jsdoc(&what)
    } else {
        jsdoc(&format!("{doc}\n\n{what}"))
    }
    .into())
}

/// Breaks text into lines of at most width characters, at spaces.
fn wrap(text: &str, width: usize) -> String {
    let mut lines: Vec<String> = vec![];
    for w in text.split(' ') {
        match lines.last_mut() {
            Some(l) if l.len() + 1 + w.len() <= width => {
                l.push(' ');
                l.push_str(w);
            }
            _ => lines.push(w.to_string()),
        }
    }
    lines.join("\n")
}

/// Writes an enum's variant as a member of its union: `{ readonly tag:
/// "Tag"; readonly value: T }`, without value if it has no payload.
pub fn ts_variant(t: Table, m: RefMember) -> Result<Str, String> {
    let tag = quote(&m.name);
    Ok(match &m.r {
        TyRef::Prim(p) if &**p == "void" => format!("{{ readonly tag: {tag} }}"),
        r => format!(
            "{{ readonly tag: {tag}; readonly value: {} }}",
            render(&t, r, "")
        ),
    }
    .into())
}

fn key_codec(r: &TyRef) -> &'static str {
    match r {
        TyRef::Prim(p) if &**p == "int" => "$intKey",
        _ => "$stringKey",
    }
}

fn prim_codec(p: &str) -> &'static str {
    match p {
        "int" => "Int",
        "float" => "Float",
        "bool" => "Bool",
        "string" => "String",
        _ => "Void",
    }
}

fn codec(t: &Table, r: &TyRef, way: &str) -> String {
    match r {
        TyRef::Prim(p) => format!("${way}{}", prim_codec(p)),
        TyRef::Array(e) => format!("${way}List({})", codec(t, e, way)),
        TyRef::Map(m) => format!(
            "${way}Map({}, {})",
            key_codec(&m.key),
            codec(t, &m.value, way)
        ),
        TyRef::Named(i) => format!("${way}_{}", t.types[*i as usize].name),
    }
}

/// Writes a function encoding a value of a table type as the servers' JSON:
/// a named type's `$enc_<Name>`, the runtime's for the rest
/// (`$encList($enc_Shape)`).
pub fn ts_enc(t: Table, r: TyRef) -> Result<Str, String> {
    Ok(codec(&t, &r, "enc").into())
}

/// Writes a function decoding a value of a table type from the servers'
/// JSON, given where the value is, for errors: `$dec_<Name>`, or the
/// runtime's (`$decList($dec_Shape)`).
pub fn ts_dec(t: Table, r: TyRef) -> Result<Str, String> {
    Ok(codec(&t, &r, "dec").into())
}

/// Picks some if n isn't 0, none if it is: a codec of a shape without
/// members names nothing it doesn't read (a strict tsconfig's
/// noUnusedLocals and noUnusedParameters refuse that).
pub fn ts_pick(n: i64, none: Str, some: Str) -> Result<Str, String> {
    Ok(if n == 0 { none } else { some })
}

/// Lists the parameters a route's request carries: proc pid's, without the
/// ctx a server with a setup fills itself.
pub fn ts_route_params(prog: Program, pid: i64, setup: bool) -> Result<List<Member>, String> {
    let ps = super::sema_leaf::node_params(prog.nodes.clone(), pid)?;
    Ok(ps
        .iter()
        .filter(|m| !(setup && &*m.name == "ctx"))
        .cloned()
        .collect())
}

/// Writes the type of a route's arguments: an object of its request's
/// parameters, `{ readonly name: T; .. }`, or `Record<string, never>` for
/// none.
pub fn ts_args_type(t: Table, params: List<Member>) -> Result<Str, String> {
    if params.is_empty() {
        return Ok("Record<string, never>".into());
    }
    let fields = params
        .iter()
        .map(|m| {
            let r = super::gen_leaf::get(t.clone(), m.ty.clone())?;
            Ok(format!("readonly {}: {}", m.name, render(&t, &r, "$t.")))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(format!("{{ {} }}", fields.join("; ")).into())
}

/// Writes the parameter a route's function takes its arguments as: `args`
/// of their type, or `_args` defaulting to `{}` for none.
pub fn ts_args_param(t: Table, params: List<Member>) -> Result<Str, String> {
    let ty = ts_args_type(t, params.clone())?;
    Ok(if params.is_empty() {
        format!("_args: {ty} = {{}}")
    } else {
        format!("args: {ty}")
    }
    .into())
}

/// Names JavaScript reserves (or that a module can't declare), which a
/// function can't take.
const RESERVED: &[&str] = &[
    "arguments",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "eval",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "globalThis",
    "if",
    "implements",
    "import",
    "in",
    "Infinity",
    "instanceof",
    "interface",
    "let",
    "NaN",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

/// Names a route's client function after its proc: `Measure` -> `measure`,
/// with `_` after a name JavaScript reserves (`delete_`).
pub fn ts_func_name(proc: Str) -> Result<Str, String> {
    let mut c = proc.chars();
    let name = c.next().map_or(String::new(), |f| {
        f.to_ascii_lowercase().to_string() + c.as_str()
    });
    Ok(if RESERVED.contains(&name.as_str()) {
        format!("{name}_")
    } else {
        name
    }
    .into())
}

/// Finds two routes whose client functions would have the same name: the
/// error naming them, or none.
pub fn ts_clash(routes: List<TsRoute>) -> Result<MaybeStr, String> {
    for (i, a) in routes.iter().enumerate() {
        if let Some(b) = routes.iter().skip(i + 1).find(|b| b.name == a.name) {
            return Ok(MaybeStr::Some(
                format!(
                    "[react] names each route's client function after its proc, so http server {}'s \"{} {}\" (proc {}) and http server {}'s \"{} {}\" (proc {}) would both be {}: route each proc once",
                    a.server, a.method, a.path, a.proc, b.server, b.method, b.path, b.proc, a.name
                )
                .into(),
            ));
        }
    }
    Ok(MaybeStr::None)
}

/// Whether text mentions what (a module's `$t.`, say), so it needs that
/// import.
pub fn ts_mentions(text: Str, what: Str) -> Result<bool, String> {
    Ok(text.contains(&*what))
}

const CLIENT_RUNTIME: &str = r###"
// ---- The runtime: calls, and JSON as the servers write and read it ----
// (Exported so that a strict tsconfig's noUnusedLocals accepts what a
// program doesn't use; names starting with $ are this file's own.)

/**
 * A call the server refused or failed: its status (400 when the request
 * didn't decode, 404 for no such route, 500 when the proc failed) and the
 * server's message.
 */
export class HttpError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = "HttpError";
    this.status = status;
  }
}

/**
 * How a call is made: signal cancels it; base, if given, goes before the
 * route's path instead of [react]'s api_base.
 */
export interface CallOptions {
  readonly signal?: AbortSignal | undefined;
  readonly base?: string | undefined;
}

/**
 * Calls a route: GET and HEAD with each parameter as JSON in the query
 * string, any other method with the parameters as a JSON object body. Gives
 * the result's JSON, or fails with an HttpError.
 */
export async function $call(method: string, path: string, params: Record<string, unknown>, opts: CallOptions): Promise<unknown> {
  let url = (opts.base ?? $base) + path;
  const init: RequestInit = { method };
  if (opts.signal !== undefined) init.signal = opts.signal;
  if (method === "GET" || method === "HEAD") {
    const query = new URLSearchParams();
    for (const [k, v] of Object.entries(params)) query.append(k, JSON.stringify(v));
    const q = query.toString();
    if (q !== "") url += "?" + q;
  } else {
    init.headers = { "Content-Type": "application/json" };
    init.body = JSON.stringify(params);
  }
  const res = await globalThis.fetch(url, init);
  const text = await res.text();
  let body: unknown = undefined;
  try {
    body = JSON.parse(text);
  } catch {
    // Not JSON: an error below.
  }
  if (!res.ok) {
    const error = $isObject(body) && typeof body["error"] === "string" ? body["error"] : res.status + " " + res.statusText;
    throw new HttpError(res.status, error);
  }
  if (body === undefined) throw new HttpError(res.status, "the response isn't JSON: " + text.slice(0, 100));
  return body;
}

export function $show(v: unknown): string {
  try {
    return JSON.stringify(v) ?? String(v);
  } catch {
    return String(v);
  }
}

export function $expected(what: string, v: unknown, at: string): TypeError {
  return new TypeError("expected " + what + " at " + at + ", found " + $show(v));
}

export function $isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

// Encoding: from the types in types.ts to JSON values.

export function $encInt(x: number): number {
  if (!Number.isSafeInteger(x)) throw new TypeError("expected an int, found " + $show(x));
  return x;
}

export function $encFloat(x: number): number {
  // JSON has no NaN or infinities, and the servers read no float from null.
  if (typeof x !== "number" || !Number.isFinite(x)) throw new TypeError("expected a finite float, found " + $show(x));
  return x;
}

export function $encBool(x: boolean): boolean {
  if (typeof x !== "boolean") throw new TypeError("expected a bool, found " + $show(x));
  return x;
}

export function $encString(x: string): string {
  if (typeof x !== "string") throw new TypeError("expected a string, found " + $show(x));
  return x;
}

export function $encVoid(_: void): null {
  return null;
}

export function $encList<T>(f: (x: T) => unknown): (xs: readonly T[]) => unknown[] {
  return (xs) => xs.map((x) => f(x));
}

/** A map key's codec: maps are JSON objects, so keys are strings. cmp orders keys as vandorf does. */
export interface $Key<K> {
  enc(k: K): string;
  dec(k: string, at: string): K;
  cmp(a: K, b: K): number;
}

export const $intKey: $Key<number> = {
  enc: (k) => String($encInt(k)),
  dec: (k, at) => {
    const n = Number(k);
    if (!/^[+-]?[0-9]+$/.test(k) || !Number.isSafeInteger(n)) throw new TypeError("map key " + $show(k) + " at " + at + " is not an int");
    return n;
  },
  cmp: (a, b) => a - b,
};

export const $stringKey: $Key<string> = {
  enc: (k) => $encString(k),
  dec: (k) => k,
  // By code point, as UTF-8 bytes compare (JavaScript compares UTF-16 units).
  cmp: (a, b) => {
    const xs = Array.from(a, (c) => c.codePointAt(0) ?? 0);
    const ys = Array.from(b, (c) => c.codePointAt(0) ?? 0);
    for (let i = 0; i < xs.length && i < ys.length; i++) {
      const d = (xs[i] ?? 0) - (ys[i] ?? 0);
      if (d !== 0) return d;
    }
    return xs.length - ys.length;
  },
};

export function $encMap<K, V>(key: $Key<K>, f: (x: V) => unknown): (m: ReadonlyMap<K, V>) => Record<string, unknown> {
  return (m) => {
    // No prototype, so a key like __proto__ is a key.
    const o: Record<string, unknown> = Object.create(null);
    for (const [k, x] of m) o[key.enc(k)] = f(x);
    return o;
  };
}

// Decoding: from JSON values to the types in types.ts, failing with where
// the value is (at) and what was expected.

export function $decInt(v: unknown, at: string): number {
  if (typeof v === "number" && Number.isSafeInteger(v)) return v;
  throw $expected("an int", v, at);
}

export function $decFloat(v: unknown, at: string): number {
  if (typeof v === "number") return v;
  // The servers write NaN and the infinities as null.
  if (v === null) return NaN;
  throw $expected("a float", v, at);
}

export function $decBool(v: unknown, at: string): boolean {
  if (typeof v === "boolean") return v;
  throw $expected("a bool", v, at);
}

export function $decString(v: unknown, at: string): string {
  if (typeof v === "string") return v;
  throw $expected("a string", v, at);
}

export function $decVoid(v: unknown, at: string): void {
  if (v !== null) throw $expected("null", v, at);
}

export function $decList<T>(f: (v: unknown, at: string) => T): (v: unknown, at: string) => readonly T[] {
  return (v, at) => {
    if (!Array.isArray(v)) throw $expected("an array", v, at);
    return v.map((x: unknown, i) => f(x, at + "[" + i + "]"));
  };
}

export function $decMap<K, V>(key: $Key<K>, f: (v: unknown, at: string) => V): (v: unknown, at: string) => ReadonlyMap<K, V> {
  return (v, at) => {
    const o = $object(v, at);
    const es = Object.keys(o).map((k): [K, V] => [key.dec(k, at), f(o[k], at + "[" + JSON.stringify(k) + "]")]);
    // In key order, as vandorf's maps are (an object puts keys like "2" first).
    es.sort((a, b) => key.cmp(a[0], b[0]));
    return new Map(es);
  };
}

export function $object(v: unknown, at: string): Record<string, unknown> {
  if ($isObject(v)) return v;
  throw $expected("an object", v, at);
}

export function $field(o: Record<string, unknown>, name: string, at: string): unknown {
  if (!Object.prototype.hasOwnProperty.call(o, name)) throw new TypeError("missing field " + name + " at " + at);
  return o[name];
}

/** The payload of a variant that has none, as $variant gives it. */
export const $bare = Symbol("bare");

/** A variant as the servers write it, "Tag" or {"Tag": payload}: its tag and its payload ($bare for none). */
export function $variant(v: unknown, at: string): [string, unknown] {
  if (typeof v === "string") return [v, $bare];
  if ($isObject(v)) {
    const ks = Object.keys(v);
    const k = ks[0];
    if (ks.length === 1 && k !== undefined) return [k, v[k]];
  }
  throw $expected("a variant, \"Tag\" or {\"Tag\": payload}", v, at);
}

export function $noVariant(v: unknown, at: string, what: string): never {
  throw $expected("a variant of " + what, v, at);
}

export function $notA(x: unknown, what: string): never {
  throw new TypeError("not a " + what + ": " + $show(x));
}
"###;

/// Writes client.ts's runtime: HttpError and CallOptions, the call, and the
/// JSON codecs of primitives, arrays and maps, which the generated code
/// builds on.
pub fn ts_client_runtime(_n: i64) -> Result<Str, String> {
    Ok(CLIENT_RUNTIME.into())
}

const HOOKS_RUNTIME: &str = r###"
// ---- The runtime ----
// (Exported so that a strict tsconfig's noUnusedLocals accepts it when a
// program has no routes; names starting with $ are this file's own.)

/**
 * What a hook gives: the last run's result (kept while the next one runs)
 * or its error, whether a run is in flight, and run, which starts one,
 * cancelling the one before. run's promise gives the result, or undefined
 * if the run failed or was cancelled; it never rejects.
 */
export interface CallState<A, T> {
  readonly data: T | undefined;
  readonly error: Error | undefined;
  readonly loading: boolean;
  readonly run: (args: A) => Promise<T | undefined>;
}

/** A hook's options: base, as for the client's functions. */
export interface HookOptions {
  readonly base?: string | undefined;
}

/**
 * Runs call in React state. Given args (not undefined or null), it runs when
 * the component mounts and again whenever args change by value. A run for
 * new args cancels the one before through its AbortSignal, and the last is
 * cancelled when args become undefined or null, or the component unmounts.
 */
export function $useCall<A, T>(call: (args: A, opts: $c.CallOptions) => Promise<T>, args: A | null | undefined, opts: HookOptions | undefined): CallState<A, T> {
  const base = opts?.base;
  const [state, setState] = React.useState<{ data: T | undefined; error: Error | undefined; loading: boolean }>(() => ({
    data: undefined,
    error: undefined,
    loading: args !== undefined && args !== null,
  }));
  const live = React.useRef<AbortController | null>(null);
  const cancel = React.useCallback(() => {
    if (live.current === null) return;
    live.current.abort();
    live.current = null;
    setState((s) => ({ data: s.data, error: s.error, loading: false }));
  }, []);
  const run = React.useCallback((a: A): Promise<T | undefined> => {
    live.current?.abort();
    const ctl = new AbortController();
    live.current = ctl;
    setState((s) => ({ data: s.data, error: undefined, loading: true }));
    return call(a, { signal: ctl.signal, base }).then(
      (data) => {
        if (live.current !== ctl) return undefined;
        live.current = null;
        setState({ data, error: undefined, loading: false });
        return data;
      },
      (e: unknown) => {
        if (live.current !== ctl) return undefined;
        live.current = null;
        setState((s) => ({ data: s.data, error: e instanceof Error ? e : new Error(String(e)), loading: false }));
        return undefined;
      },
    );
  }, [call, base]);
  const key = args === undefined || args === null ? undefined : $key(args);
  // Keyed by value, so args written out again on each render don't run again.
  React.useEffect(() => {
    if (args === undefined || args === null) return undefined;
    void run(args);
    return cancel;
  }, [key, run, cancel]);
  return { data: state.data, error: state.error, loading: state.loading, run };
}

/** A value as text that's the same for equal values (maps by their entries): what hooks compare args by. */
export function $key(v: unknown): string {
  return JSON.stringify(v, (_k, x: unknown) => (x instanceof Map ? { $map: [...x.entries()] } : x)) ?? "";
}
"###;

/// Writes hooks.ts's runtime: CallState and HookOptions, and the hook every
/// route's hook runs.
pub fn ts_hooks_runtime(_n: i64) -> Result<Str, String> {
    Ok(HOOKS_RUNTIME.into())
}
