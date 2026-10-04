import { CURRENCY_OPTIONS } from '../../core/settings';

export function currencyOptionsFor(current: string) {
  if (!current || CURRENCY_OPTIONS.some((o) => o.value === current)) {
    return CURRENCY_OPTIONS;
  }
  return [...CURRENCY_OPTIONS, { value: current, label: current }];
}

export function formatMinor(minor: number, exponent: number): string {
  const negative = minor < 0;
  const digits = Math.abs(minor).toString().padStart(exponent + 1, '0');
  const text =
    exponent === 0
      ? digits
      : `${digits.slice(0, digits.length - exponent)}.${digits.slice(digits.length - exponent)}`;
  return negative ? `-${text}` : text;
}
