/// <reference types="node" />
/**
 * Reads numeric constants from Rust sources, so that a test can compare them
 * with the TypeScript copies. The parser is small and strict: a line that does
 * not match fails with a clear message. Only tests import this file.
 */
import { readFileSync } from 'node:fs';

/** The repository root (this file is in `apps/desktop/ui/src/test/`). */
const ROOT = new URL('../../../../../', import.meta.url);

function source(path: string): string {
  return readFileSync(new URL(path, ROOT), 'utf8');
}

/** Evaluates a numeric literal: `30_000`, `10_000.0`, `1.0e5`, `1 << 28`, `1.0 / 65_536.0`. */
function evaluate(expr: string, where: string): number {
  const text = expr.replace(/_/g, '').replace(/f32$|f64$|u32$|u64$|usize$/, '').trim();
  const number = /^-?[0-9]+(?:\.[0-9]+)?(?:e[+-]?[0-9]+)?$/;
  const shift = /^([0-9]+)\s*<<\s*([0-9]+)$/;
  const ratio = /^([0-9.]+)\s*\/\s*([0-9.]+)$/;
  if (number.test(text)) {
    return Number(text);
  }
  const s = shift.exec(text);
  if (s !== null) {
    return 2 ** Number(s[2]) * Number(s[1]);
  }
  const r = ratio.exec(text);
  if (r !== null) {
    return Number(r[1]) / Number(r[2]);
  }
  throw new Error(`${where}: cannot read the Rust value "${expr}".`);
}

/** The value of `[pub] const NAME: <type> = <number expression>;` in the Rust file at `path` (from the repository root). */
export function rustConst(path: string, name: string): number {
  const match = new RegExp(`^\\s*(?:pub(?:\\([a-z]+\\))? )?const ${name}: [A-Za-z0-9_]+ = ([^;]+);`, 'm').exec(source(path));
  if (match === null || match[1] === undefined) {
    throw new Error(`${path}: constant ${name} not found (or not a plain number). Update the test or the parser.`);
  }
  return evaluate(match[1], `${path} ${name}`);
}

/** The bounds of `[pub] const NAME: RangeInclusive<f32> = a..=b;` in the Rust file at `path`. */
export function rustRange(path: string, name: string): { min: number; max: number } {
  const match = new RegExp(`^\\s*(?:pub )?const ${name}: RangeInclusive<[a-z0-9]+> = ([^;]+?)\\.\\.=([^;]+);`, 'm').exec(source(path));
  if (match === null || match[1] === undefined || match[2] === undefined) {
    throw new Error(`${path}: range constant ${name} not found. Update the test or the parser.`);
  }
  return { min: evaluate(match[1], `${path} ${name}`), max: evaluate(match[2], `${path} ${name}`) };
}
