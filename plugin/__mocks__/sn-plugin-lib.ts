// In-memory stand-in for the Supernote SDK, used by the unit tests. It models one note page
// list with strokes, so code that inserts and deletes elements can be tested for real.
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
  stroke: {
    penType: number;
    penColor: number;
    points: Accessor<Pt>;
    pressures: Accessor<number>;
  } | null = null;
}

let nextUuid = 1;
export const fakeNote = {
  /** pages[page] = elements on that page, in order. */
  pages: new Map<number, Element[]>(),
  deleted: [] as number[],
  currentPage: 0,
  inserted: [] as number[],
  reset() {
    this.pages = new Map();
    this.deleted = [];
    this.currentPage = 0;
    this.inserted = [];
    nextUuid = 1;
  },
  /** Adds a stroke the way the user's pen would. */
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
  strokes(page: number) {
    return this.pages.get(page) ?? [];
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
    const list = fakeNote.pages.get(page) ?? [];
    for (const e of elements) {
      e.numInPage = list.length + 1;
      list.push(e);
    }
    fakeNote.pages.set(page, list);
    return ok(true);
  },
  async deletePageElements(nums: number[], page: number) {
    fakeNote.deleted.push(...nums);
    const kept = (fakeNote.pages.get(page) ?? []).filter(
      e => !nums.includes(e.numInPage),
    );
    kept.forEach((e, i) => (e.numInPage = i + 1));
    fakeNote.pages.set(page, kept);
    return ok(true);
  },
  recycleElement() {},
  async jumpToPage(page: number) {
    fakeNote.currentPage = page;
    return ok(true);
  },
  async getPenInfo() {
    return ok({type: 10, color: 0, width: 200});
  },
};

export const PluginFileAPI = {
  async getElements(page: number) {
    return ok([...(fakeNote.pages.get(page) ?? [])]);
  },
  async getElementNumList(_path: string, page: number) {
    return ok((fakeNote.pages.get(page) ?? []).map(e => e.numInPage));
  },
};

PluginFileAPI_extra();

function PluginFileAPI_extra() {
  Object.assign(PluginFileAPI, {
    async getNotePageTemplate() {
      return ok({name: 'style_blank', md5: '0'});
    },
    /** Inserts an empty page at `page`; later pages move down. */
    async insertNotePage(params: {page: number}) {
      const moved = new Map<number, Element[]>();
      for (const [i, list] of fakeNote.pages) {
        moved.set(i >= params.page ? i + 1 : i, list);
      }
      fakeNote.pages = moved;
      fakeNote.inserted.push(params.page);
      return ok(true);
    },
  });
}

export const PluginManager = {
  async hasPermission() {
    return 1;
  },
  async requestPermission() {
    return 1;
  },
  registerLangListener() {},
};
