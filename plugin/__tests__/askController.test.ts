import {fakeNote} from '../__mocks__/sn-plugin-lib';
import {getAskState, startAsk} from '../src/askController';

jest.mock('../src/bridgeConfig', () => ({
  loadBridges: async () => ({endpoints: [], source: 'baked'}),
}));
jest.mock('../src/bridgeClient', () => ({
  connectFirst: async () => {
    throw new Error('no bridge'); // these tests stop before a bridge is needed
  },
}));

const textStroke = () => ({type: 0} as never);
const settle = async () => {
  for (let i = 0; i < 40 && !getAskState().finished; i++) {
    await new Promise(r => setTimeout(r, 10));
  }
  await new Promise(r => setTimeout(r, 20));
};

describe('asking about something that is not text', () => {
  beforeEach(() => fakeNote.reset());

  it.each([
    ['the recogniser returns null (e.g. a music score)', null],
    ['the recogniser returns nothing', ''],
    ['the recogniser returns only spaces', '   '],
  ])(
    '%s: a short notice, not a crash and not a full-screen panel',
    async (_name, result) => {
      fakeNote.lasso = [textStroke()];
      fakeNote.ocrResult = result;
      startAsk();
      await settle();
      expect(getAskState().failed).toBe(true);
      expect(getAskState().message).not.toMatch(/trim|null/);
      expect(fakeNote.dialogs).toHaveLength(1);
      expect(fakeNote.dialogs[0]).toContain('読み取れ');
      expect(fakeNote.panelsOpened).toBe(0);
    },
  );

  it('copes with a lasso that returns no elements at all', async () => {
    fakeNote.lasso = null;
    startAsk();
    await settle();
    expect(getAskState().failed).toBe(true);
    expect(fakeNote.dialogs).toHaveLength(1);
  });

  it('falls back to the panel when the native dialog cannot be shown', async () => {
    fakeNote.lasso = [textStroke()];
    fakeNote.ocrResult = null;
    fakeNote.dialogFails = true;
    startAsk();
    await settle();
    expect(fakeNote.panelsOpened).toBe(1);
  });

  it('shows one notice per question', async () => {
    fakeNote.lasso = [textStroke()];
    fakeNote.ocrResult = null;
    startAsk();
    await settle();
    expect(fakeNote.dialogs).toHaveLength(1);
  });
});
