import {fakeNote} from '../__mocks__/sn-plugin-lib';
import {getAskState, startAsk} from '../src/askController';
import {t} from '../src/i18n';

jest.mock('../src/bridgeConfig', () => ({
  loadBridges: async () => ({endpoints: [], source: 'none'}),
}));
jest.mock('../src/bridgeClient', () => ({
  connectFirst: async () => {
    throw new Error('no bridge'); // these tests stop before a bridge is needed
  },
}));

const textStroke = () => ({type: 0}) as never;
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
      expect(fakeNote.dialogs[0]).toBe(t('noText'));
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

describe('pressing the button again while an answer is being written', () => {
  const {cancelAsk} = require('../src/askController');
  const {rememberSettings} = require('../src/askFlow');
  const settings = (busyNotice: boolean) => ({
    showProgress: false,
    lineTilt: true,
    thinkingAnimation: false,
    busyNotice,
  });
  /** Holds the first question inside recognition so it is still running when the button is pressed again. */
  const holdFirstQuestion = () => {
    fakeNote.lasso = [textStroke()];
    const api = require('../__mocks__/sn-plugin-lib').PluginCommAPI;
    jest
      .spyOn(api, 'recognizeElements')
      .mockImplementationOnce(() => new Promise(() => {}));
  };

  beforeEach(() => fakeNote.reset());
  afterEach(() => {
    cancelAsk();
    rememberSettings(settings(true));
  });

  it('answers the person with a short notice (the new question is not started)', async () => {
    rememberSettings(settings(true));
    holdFirstQuestion();
    startAsk();
    await new Promise(r => setTimeout(r, 30));
    startAsk(); // pressed again
    await new Promise(r => setTimeout(r, 30));
    expect(fakeNote.dialogs).toHaveLength(1);
    expect(fakeNote.dialogs[0]).toBe(t('busy'));
  });

  it('stays silent when the notice is turned off in the settings', async () => {
    rememberSettings(settings(false));
    holdFirstQuestion();
    startAsk();
    await new Promise(r => setTimeout(r, 30));
    startAsk();
    await new Promise(r => setTimeout(r, 30));
    expect(fakeNote.dialogs).toHaveLength(0);
  });
});
