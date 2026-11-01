// Plugin-side copy of bridge/internal/protocol (Go). Change both together.

export const PROTOCOL_VERSION = 1;

export type Lang = 'ja' | 'en';

export interface LayoutParams {
  x: number;
  y: number;
  width: number;
  pageHeight: number;
  bottomMargin: number;
  charSize: number;
  /** Pages the answer may use: 1 = only below the selection; more = continue on inserted pages. */
  maxPages?: number;
  /** Top of the first line on continuation pages. */
  continueTop?: number;
}

/** One pen stroke in page pixels: `p` = [x0,y0,x1,y1,…], `w` = pressure per point (0..4096). */
export interface InkStroke {
  p: number[];
  w: number[];
}

/** User preferences, stored on the bridge (the plugin cannot persist data on the device). */
export interface Settings {
  /** Open the progress panel while answering; off = just write onto the page. */
  showProgress: boolean;
}

export type ClientMessage =
  | {type: 'hello'; token: string; client: string}
  | {
      type: 'ask';
      id: string;
      question: string;
      lang: Lang;
      layout: LayoutParams;
    }
  | {type: 'cancel'; id: string}
  | {type: 'report'; data: unknown}
  | {type: 'setSettings'; settings: Settings};

export type BridgeMessage =
  | {
      type: 'welcome';
      bridge: string;
      version: string;
      protocol: number;
      settings?: Settings;
    }
  | {type: 'settings'; settings: Settings}
  | {type: 'status'; id: string; stage: 'thinking' | 'writing'}
  | {
      type: 'line';
      id: string;
      index: number;
      /** Page relative to the page the question was asked on (0, 1, …). */
      page: number;
      text: string;
      strokes: InkStroke[];
    }
  | {
      type: 'done';
      id: string;
      text: string;
      missing: string[];
      truncated: boolean;
      /** Pages the answer used, counting the starting page. */
      pages: number;
      elapsedMs: number;
    }
  | {type: 'error'; id?: string; code: string; message: string};
