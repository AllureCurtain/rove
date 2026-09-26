/**
 * One timestamp format for product surfaces: UTC, second precision, explicit
 * zone, so a screenshot from a machine in any timezone reads the same.
 */
export function formatUtcTimestamp(value: string): string {
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) {
    return value;
  }
  return new Date(timestamp)
    .toISOString()
    .replace("T", " ")
    .replace(/\.\d{3}Z$/, " UTC");
}
