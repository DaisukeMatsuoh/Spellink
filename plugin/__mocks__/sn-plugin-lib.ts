// In-memory stand-in for the Supernote SDK, used by the unit tests.
//
// It models the rule the SDK documents: edits to the open note go to an in-memory cache
// (`pages`) and reach the file (`file`) only on saveCurrentNote(). File-level APIs read and
// write the file; a file-level write such as insertNotePage reloads the cache from the file, so
// anything not saved first is lost.
type Pt = {x: number; y: number};

class Accessor<T> {
  data: T[] = [];
  async size() {
    return this.data.length;
  }
  async get(i: number) {
    return this.data[i] ?? null;
  }
  async getRange(start: number, count: number) {
    return this.data.slice(start, start + count);
  }
  async setRange(start: number, _end: number, values: T[]) {
    this.data.splice(start, values.length, ...values);
    return true;
  }
  async add(i: number, v: T) {
    this.data.splice(i, 0, v);
    return true;
  }
}

export class Element {
  static TYPE_STROKE = 0;
  uuid = '';
  type = 0;
  numInPage = 0;
  pageNum = 0;
  thickness = 0;
  userData = '';
  stroke: {
    penType: number;
    penColor: number;
    points: Accessor<Pt>;
    pressures: Accessor<number>;
  } | null = null;
}

const copyPages = (m: Map<number, Element[]>) =>
  new Map(
    [...m].map(([k, list]) => [
      k,
      list.map(e => Object.assign(new Element(), e)),
    ]),
  );

let nextUuid = 1;
export const fakeNote = {
  /** What the app shows and the Comm APIs edit (the in-memory cache). */
  pages: new Map<number, Element[]>(),
  /** What is on disk; file-level APIs read this. */
  file: new Map<number, Element[]>(),
  deleted: [] as number[],
  currentPage: 0,
  /** The document on screen. Comm-level APIs act on this one, whichever note the plugin means. */
  currentFile: '/note.note',
  /** What an unrelated document would receive: anything written while it is open lands here. */
  /** The lasso selection and what the recogniser returns for it (null = nothing recognised). */
  lasso: [] as Element[] | null,
  ocrResult: '' as string | null,
  /** Native dialogs shown, and plugin views opened (full screen). */
  dialogs: [] as string[],
  dialogFails: false,
  panelsOpened: 0,
  strayWrites: 0,
  strayDeletes: 0,
  strayJumps: 0,
  /** Times the plugin moved the user to another page. */
  jumps: 0,
  inserted: [] as number[],
  saves: 0,
  /** Simulate a delete that reports success but removes nothing. */
  ignoreNextDelete: false,
  /** Simulate a device that does not keep Element.userData. */
  dropUserData: false,
  /** Pages in the note (the file's page count). */
  totalPages: 1,
  /** APIs that answer `success: true` with `result: false` — the SDK's way of saying "refused". */
  refuse: new Set<string>(),
  /** insertNotePage that answers `result: true` but adds no page. */
  silentInsertPage: false,
  reset() {
    this.pages = new Map();
    this.file = new Map();
    this.deleted = [];
    this.currentPage = 0;
    this.currentFile = '/note.note';
    this.lasso = [];
    this.ocrResult = '';
    this.dialogs = [];
    this.dialogFails = false;
    this.panelsOpened = 0;
    this.strayWrites = 0;
    this.strayDeletes = 0;
    this.strayJumps = 0;
    this.jumps = 0;
    this.inserted = [];
    this.saves = 0;
    this.ignoreNextDelete = false;
    this.totalPages = 1;
    this.refuse = new Set();
    this.silentInsertPage = false;
    this.dropUserData = false;
    nextUuid = 1;
  },
  /** Adds a stroke the way the user's pen would: into the cache, not yet saved. */
  addUserStroke(page: number, points: Pt[]) {
    const list = this.pages.get(page) ?? [];
    const e = new Element();
    e.uuid = `user-${nextUuid++}`;
    e.stroke = {
      penType: 10,
      penColor: 0,
      points: new Accessor<Pt>(),
      pressures: new Accessor<number>(),
    };
    e.stroke.points.data = points;
    e.numInPage = list.length + 1;
    list.push(e);
    this.pages.set(page, list);
    return e;
  },
  /** What the user sees on a page. */
  strokes(page: number) {
    return this.pages.get(page) ?? [];
  },
  /** What is saved on a page. */
  saved(page: number) {
    return this.file.get(page) ?? [];
  },
};

const ok = <T>(result: T) => ({success: true, result});

export const PluginCommAPI = {
  async createElement(type: number) {
    const e = new Element();
    e.type = type;
    e.uuid = `new-${nextUuid++}`;
    e.stroke = {
      penType: 10,
      penColor: 0,
      points: new Accessor<Pt>(),
      pressures: new Accessor<number>(),
    };
    return ok(e);
  },
  async insertPageElements(elements: Element[], page: number) {
    if (fakeNote.currentFile !== '/note.note') {
      fakeNote.strayWrites += elements.length; // would have corrupted another document
      return {
        success: false,
        error: {code: 105, message: 'No note-related app is bound!'},
      };
    }
    if (fakeNote.refuse.has('insertPageElements')) {
      return ok(false);
    }
    const list = fakeNote.pages.get(page) ?? [];
    for (const e of elements) {
      if (fakeNote.dropUserData) {
        e.userData = '';
      }
      e.numInPage = list.length + 1;
      list.push(e);
    }
    fakeNote.pages.set(page, list);
    return ok(true);
  },
  /**
   * Deletes the numbered elements one after another, renumbering after each — the strictest
   * plausible behaviour: an ascending list [5,6,7] then removes old 5, 7 and 9.
   */
  async deletePageElements(nums: number[], page: number) {
    if (fakeNote.currentFile !== '/note.note') {
      fakeNote.strayDeletes += nums.length;
      return {
        success: false,
        error: {code: 105, message: 'No note-related app is bound!'},
      };
    }
    if (fakeNote.refuse.has('deletePageElements')) {
      return ok(false);
    }
    fakeNote.deleted.push(...nums);
    if (fakeNote.ignoreNextDelete) {
      fakeNote.ignoreNextDelete = false;
      return ok(true);
    }
    const list = fakeNote.pages.get(page) ?? [];
    for (const n of nums) {
      const i = list.findIndex(e => e.numInPage === n);
      if (i >= 0) {
        list.splice(i, 1);
      }
      list.forEach((e, k) => (e.numInPage = k + 1));
    }
    fakeNote.pages.set(page, list);
    return ok(true);
  },
  recycleElement() {},
  async jumpToPage(page: number) {
    if (fakeNote.currentFile !== '/note.note') {
      fakeNote.strayJumps++;
    }
    if (fakeNote.refuse.has('jumpToPage')) {
      return ok(false);
    }
    fakeNote.currentPage = page;
    return ok(true);
  },
  async getLassoElements() {
    return ok(fakeNote.lasso);
  },
  async recognizeElements() {
    return ok(fakeNote.ocrResult);
  },
  async getLassoRect() {
    return ok({left: 100, top: 100, right: 500, bottom: 300});
  },
  async setLassoBoxState() {
    return ok(true);
  },
  async getCurrentFilePath() {
    return ok(fakeNote.currentFile);
  },
  async getCurrentPageNum() {
    return ok(fakeNote.currentPage);
  },
  async getPenInfo() {
    return ok({type: 10, color: 0, width: 200});
  },
};

export const PluginNoteAPI = {
  async saveCurrentNote() {
    if (fakeNote.refuse.has('saveCurrentNote')) {
      return ok(false); // nothing was saved
    }
    fakeNote.saves++;
    fakeNote.file = copyPages(fakeNote.pages);
    return ok(true);
  },
};

export const PluginFileAPI = {
  async getElements(page: number) {
    return ok([...(fakeNote.file.get(page) ?? [])]);
  },
  async getElementNumList(_path: string, page: number) {
    return ok((fakeNote.file.get(page) ?? []).map(e => e.numInPage));
  },
  async getNoteTotalPageNum() {
    return ok(fakeNote.totalPages);
  },
  async getPageSize() {
    return ok({width: 1404, height: 1872});
  },
  async getNotePageTemplate() {
    return ok({name: 'style_blank', md5: '0'});
  },
  /** A file-level write: pages shift in the file, then the cache is reloaded from it. */
  async insertNotePage(params: {page: number}) {
    if (fakeNote.refuse.has('insertNotePage')) {
      return ok(false);
    }
    if (fakeNote.silentInsertPage) {
      return ok(true);
    }
    fakeNote.totalPages++;
    const moved = new Map<number, Element[]>();
    for (const [i, list] of fakeNote.file) {
      moved.set(i >= params.page ? i + 1 : i, list);
    }
    fakeNote.file = moved;
    fakeNote.pages = copyPages(fakeNote.file); // unsaved edits are gone
    fakeNote.inserted.push(params.page);
    return ok(true);
  },
};

export const NativeUIUtils = {
  async showRattaDialog(tip: string) {
    if (fakeNote.dialogFails) {
      throw new Error('no activity');
    }
    fakeNote.dialogs.push(tip);
    return true;
  },
};

export const PluginManager = {
  async showPluginView() {
    fakeNote.panelsOpened++;
    return true;
  },
  async closePluginView() {
    return true;
  },
  async hasPermission() {
    return 1;
  },
  async requestPermission() {
    return 1;
  },
  registerLangListener() {},
};
