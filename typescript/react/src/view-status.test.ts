import { summarizeStatuses } from './view-status';

describe('summarizeStatuses', () => {
  it('reports loading sources by name in declaration order', () => {
    const result = summarizeStatuses({
      Board: { isLoading: false, error: undefined },
      Round: { isLoading: true, error: undefined },
      Treasury: { isLoading: true, error: undefined },
    });

    expect(result).toEqual({
      isLoading: true,
      hasError: false,
      isRefreshing: false,
      loading: ['Round', 'Treasury'],
      errors: [],
      refreshing: [],
    });
  });

  it('collects errors as "Name: message"', () => {
    const result = summarizeStatuses({
      Board: { isLoading: false, error: new Error('socket closed') },
      Round: { isLoading: false, error: undefined },
    });

    expect(result.hasError).toBe(true);
    expect(result.errors).toEqual(['Board: socket closed']);
  });

  it('reports sources that are refreshing committed data', () => {
    const result = summarizeStatuses({
      Board: { isLoading: false, isRefreshing: true },
      Round: { isLoading: false, isRefreshing: false },
    });

    expect(result.isRefreshing).toBe(true);
    expect(result.refreshing).toEqual(['Board']);
  });

  it('skips conditional sources that are falsy', () => {
    const authority: string | undefined = undefined;
    const result = summarizeStatuses({
      Board: { isLoading: true, error: undefined },
      Miner: authority && { isLoading: true, error: new Error('ignored') },
    });

    expect(result.loading).toEqual(['Board']);
    expect(result.errors).toEqual([]);
  });

  it('is quiet when everything is ready', () => {
    const result = summarizeStatuses({
      Board: { isLoading: false, error: undefined },
    });

    expect(result).toEqual({
      isLoading: false,
      hasError: false,
      isRefreshing: false,
      loading: [],
      errors: [],
      refreshing: [],
    });
  });
});
