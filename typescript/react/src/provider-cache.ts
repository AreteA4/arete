export function trackConnectingPromise<T>(
  connecting: Map<string, Promise<T>>,
  cacheKey: string,
  promise: Promise<T>
): Promise<T> {
  connecting.set(cacheKey, promise);
  void promise.catch(() => {
    if (connecting.get(cacheKey) === promise) {
      connecting.delete(cacheKey);
    }
  });
  return promise;
}
