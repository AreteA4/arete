import assert from 'node:assert/strict';
import test from 'node:test';

import { answerNumbers } from '../tasks/lib.js';

test('answer numbers accept thousands separators', () => {
  assert.deepEqual(answerNumbers('The current round is **435,570.**'), [435570]);
  assert.deepEqual(answerNumbers('round 435_570 or 435 570'), [435570, 435570]);
  assert.deepEqual(answerNumbers('round 435570, 12 miners, 3.79 SOL'), [435570]);
  assert.deepEqual(answerNumbers('1,2,3 and 999'), []);
});
