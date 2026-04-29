/* tslint:disable */
/* eslint-disable */

export class ScanPlan {
  free(): void;
  [Symbol.dispose](): void;
  scan_scored_box(x0: number, x1: number, y0: number, y1: number, z0: number, z1: number, max_matches: number, tol: number, max_score: number): Int32Array;
  scan_strict_box(x0: number, x1: number, y0: number, y1: number, z0: number, z1: number, max_matches: number): Int32Array;
  constructor(rel_dx: Int32Array, rel_dy: Int32Array, rel_dz: Int32Array, rel_packed: Uint16Array, rel_mask: Uint16Array, rel_drip: Uint8Array, seed_mode: number);
}

/**
 * Legacy scored scan kept for existing callers.
 */
export function scan_scored_box(rel_dx: Int32Array, rel_dy: Int32Array, rel_dz: Int32Array, rel_packed: Uint16Array, rel_mask: Uint16Array, rel_drip: Uint8Array, post1_12_any_y: boolean, x0: number, x1: number, y0: number, y1: number, z0: number, z1: number, max_matches: number, tol: number, max_score: number): Int32Array;

/**
 * Scored scan with explicit seed mode.
 *
 * Returns Int32Array [x,y,z,score, x,y,z,score, ...].
 */
export function scan_scored_box_seed(rel_dx: Int32Array, rel_dy: Int32Array, rel_dz: Int32Array, rel_packed: Uint16Array, rel_mask: Uint16Array, rel_drip: Uint8Array, seed_mode: number, x0: number, x1: number, y0: number, y1: number, z0: number, z1: number, max_matches: number, tol: number, max_score: number): Int32Array;

/**
 * Legacy strict scan kept for existing callers.
 *
 * post1_12_any_y=true maps to the 1.8+ Y-independent hash.
 * false maps to the 1.7.10-style Y-dependent vanilla hash.
 */
export function scan_strict_box(rel_dx: Int32Array, rel_dy: Int32Array, rel_dz: Int32Array, rel_packed: Uint16Array, rel_mask: Uint16Array, rel_drip: Uint8Array, post1_12_any_y: boolean, x0: number, x1: number, y0: number, y1: number, z0: number, z1: number, max_matches: number): Int32Array;

/**
 * Strict scan with explicit seed mode.
 *
 * seed_mode:
 * 0 = 1.8+ (Y ignored)
 * 1 = 1.7.10 / vanilla XOR with Y
 * 2 = b1.6-tb3 additive seed with Y
 *
 * Returns Int32Array [x,y,z, x,y,z, ...].
 */
export function scan_strict_box_seed(rel_dx: Int32Array, rel_dy: Int32Array, rel_dz: Int32Array, rel_packed: Uint16Array, rel_mask: Uint16Array, rel_drip: Uint8Array, seed_mode: number, x0: number, x1: number, y0: number, y1: number, z0: number, z1: number, max_matches: number): Int32Array;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
  readonly memory: WebAssembly.Memory;
  readonly __wbg_scanplan_free: (a: number, b: number) => void;
  readonly scan_scored_box: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number, p: number, q: number, r: number, s: number, t: number, u: number, v: number, w: number) => void;
  readonly scan_scored_box_seed: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number, p: number, q: number, r: number, s: number, t: number, u: number, v: number, w: number) => void;
  readonly scan_strict_box: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number, p: number, q: number, r: number, s: number, t: number, u: number) => void;
  readonly scan_strict_box_seed: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number, p: number, q: number, r: number, s: number, t: number, u: number) => void;
  readonly scanplan_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number) => void;
  readonly scanplan_scan_scored_box: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number) => void;
  readonly scanplan_scan_strict_box: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => void;
  readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
  readonly __wbindgen_export: (a: number, b: number) => number;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
* Instantiates the given `module`, which can either be bytes or
* a precompiled `WebAssembly.Module`.
*
* @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
*
* @returns {InitOutput}
*/
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
* If `module_or_path` is {RequestInfo} or {URL}, makes a request and
* for everything else, calls `WebAssembly.instantiate` directly.
*
* @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
*
* @returns {Promise<InitOutput>}
*/
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
