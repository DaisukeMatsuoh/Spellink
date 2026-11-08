import {fakeNote} from '../__mocks__/sn-plugin-lib';
import {forgetThisNote} from '../src/forget';
import {resetHostQueueForTests} from '../src/sdk';

describe('forgetting the conversation of the open note', () => {
  beforeEach(() => {
    fakeNote.reset();
    resetHostQueueForTests();
  });
  afterEach(() => jest.restoreAllMocks());

  it('sends the open note’s path as the key, in a message that cannot be mistaken for forgetting everything', async () => {
    const sent: unknown[] = [];
    expect(await forgetThisNote({send: m => sent.push(m)})).toBe('sent');
    expect(sent).toEqual([{type: 'forgetNote', note: '/note.note'}]);
  });

  it('sends nothing when there is no note to name', async () => {
    const comm = require('../__mocks__/sn-plugin-lib').PluginCommAPI;
    const sent: unknown[] = [];
    jest.spyOn(comm, 'getCurrentFilePath').mockResolvedValue({
      success: false,
      error: {code: 105, message: 'No note-related app is bound!'},
    });
    expect(await forgetThisNote({send: m => sent.push(m)})).toBe('noNote');
    jest
      .spyOn(comm, 'getCurrentFilePath')
      .mockResolvedValue({success: true, result: ''});
    expect(await forgetThisNote({send: m => sent.push(m)})).toBe('noNote');
    jest
      .spyOn(comm, 'getCurrentFilePath')
      .mockResolvedValue({success: true, result: 7});
    expect(await forgetThisNote({send: m => sent.push(m)})).toBe('noNote');
    expect(sent).toEqual([]); // never a request without a real key: that would forget every note
  });
});
