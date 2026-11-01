import {answerLayout} from '../src/askFlow';

describe('answerLayout', () => {
  const nomad = {width: 1404, height: 1872};

  it('wraps within the page width and never needs horizontal scrolling', () => {
    const l = answerLayout(nomad, {left: 200, bottom: 400});
    expect(l.x + l.width).toBeLessThanOrEqual(nomad.width);
  });

  it('puts the answer below the selection', () => {
    const l = answerLayout(nomad, {left: 200, bottom: 400});
    expect(l.y).toBeGreaterThan(400);
  });

  it('falls back to the left margin when the selection is too far right', () => {
    const l = answerLayout(nomad, {left: 1300, bottom: 400});
    expect(l.width).toBeGreaterThan(nomad.width * 0.45);
  });

  it('keeps y valid even when the selection is at the very bottom', () => {
    const l = answerLayout(nomad, {left: 200, bottom: 1860});
    expect(l.y).toBeLessThanOrEqual(nomad.height);
  });

  it('allows continuing on following pages', () => {
    const l = answerLayout(nomad, {left: 200, bottom: 400});
    expect(l.maxPages).toBeGreaterThan(1);
    expect(l.continueTop).toBeDefined();
  });
});
