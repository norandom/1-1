import { describe, expect, it } from 'vitest';
import { createHeldButtons } from '../mouse-capture';

describe('createHeldButtons', () => {
  it('releases a button whose mouseup never reached the canvas at the last position', () => {
    const buttons = createHeldButtons();
    buttons.press(0, { clientX: 10, clientY: 20 });
    buttons.move({ clientX: 400, clientY: -30 });
    expect(buttons.pressed).toBe(true);
    expect(buttons.drain()).toEqual([{ button: 0, clientX: 400, clientY: -30 }]);
    expect(buttons.pressed).toBe(false);
    expect(buttons.drain()).toEqual([]);
  });

  it('does not release buttons that already came up', () => {
    const buttons = createHeldButtons();
    buttons.press(0, { clientX: 1, clientY: 1 });
    buttons.press(2, { clientX: 2, clientY: 2 });
    buttons.release(0, { clientX: 3, clientY: 3 });
    expect(buttons.drain()).toEqual([{ button: 2, clientX: 3, clientY: 3 }]);
  });

  it('keeps one release per button when it is pressed twice', () => {
    const buttons = createHeldButtons();
    buttons.press(1, { clientX: 5, clientY: 6 });
    buttons.press(1, { clientX: 7, clientY: 8 });
    expect(buttons.drain()).toEqual([{ button: 1, clientX: 7, clientY: 8 }]);
  });
});
