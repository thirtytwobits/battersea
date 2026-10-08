/** Nominal token compatibility, including pass-through outputs with inferred input types. */
export function tokenConnectionCompatible(
  sourceTokenType: string,
  sourceNodeInputAccepted: readonly string[],
  targetAccepted: readonly string[],
): boolean {
  if (sourceTokenType === "auto") {
    return sourceNodeInputAccepted.length === 0 ||
      sourceNodeInputAccepted.some((candidate) => targetAccepted.includes(candidate));
  }
  return targetAccepted.includes(sourceTokenType);
}
