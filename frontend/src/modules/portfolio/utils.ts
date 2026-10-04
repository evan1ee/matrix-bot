import { CURRENCY_OPTIONS } from '../../core/settings';

/** Decimal places quantity is stored at — mirrors the backend's `QUANTITY_EXPONENT`. */
export const QUANTITY_EXPONENT = 8;

/**
 * Renders minor units as a decimal string. Integer maths only, same reason
 * `finance`'s `formatMinor` avoids a float: dividing into one on the way to
 * display is exactly the rounding this whole design avoids on the server.
 */
export function formatMinor(minor: number, exponent: number): string {
  const negative = minor < 0;
  const digits = Math.abs(minor).toString().padStart(exponent + 1, '0');
  const text =
    exponent === 0
      ? digits
      : `${digits.slice(0, digits.length - exponent)}.${digits.slice(digits.length - exponent)}`;
  return negative ? `-${text}` : text;
}

/** Like `formatMinor`, but trims trailing zeros — `10.00000000` shares reads as
 *  noise; a plain `10` or `1.5` is what the user actually typed. */
export function formatQuantity(minor: number): string {
  const full = formatMinor(minor, QUANTITY_EXPONENT);
  const [whole, frac] = full.split('.');
  if (!frac) return full;
  const trimmed = frac.replace(/0+$/, '');
  return trimmed ? `${whole}.${trimmed}` : whole;
}

export function formatSigned(minor: number, exponent: number): string {
  return minor < 0 ? formatMinor(minor, exponent) : `+${formatMinor(minor, exponent)}`;
}

export function plColor(minor: number): string {
  return minor < 0 ? 'text-rose-600 dark:text-rose-400' : 'text-emerald-600 dark:text-emerald-400';
}

/** Same "unlisted currency stays selectable" reasoning as `finance`'s
 *  `currencyOptionsFor` — a Matrix `!portfolio buy` can write a currency this
 *  list does not offer. */
export function currencyOptionsFor(current: string) {
  if (!current || CURRENCY_OPTIONS.some((o) => o.value === current)) {
    return CURRENCY_OPTIONS;
  }
  return [...CURRENCY_OPTIONS, { value: current, label: current }];
}
