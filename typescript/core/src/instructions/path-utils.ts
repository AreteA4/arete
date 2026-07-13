export function getValueByPath(
  source: Record<string, unknown> | undefined,
  path: string
): unknown {
  if (!source) {
    return undefined;
  }

  if (Object.prototype.hasOwnProperty.call(source, path)) {
    return source[path];
  }

  let current: unknown = source;
  for (const segment of path.split('.')) {
    if (current === null || typeof current !== 'object') {
      return undefined;
    }
    current = (current as Record<string, unknown>)[segment];
  }

  return current;
}
