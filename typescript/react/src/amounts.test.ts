import { formatRawToUi, safeToRawAmount, toRawAmount } from './index';

describe('amount helper exports', () => {
  it('re-exports exact raw/UI amount conversion helpers', () => {
    expect(toRawAmount({ ui: '1.25' }, 9)).toBe(1_250_000_000n);
    expect(safeToRawAmount({ ui: '1.25' }, 9)).toEqual({
      success: true,
      data: 1_250_000_000n,
    });
    expect(formatRawToUi(1_250_000_000n, 9)).toBe('1.25');
  });
});
