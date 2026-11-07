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
  /** Lowest y the answer may reach on the starting page (existing content below). */
  startLimit?: number;
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
  /** Apply the agent's writing slope to answers. */
  lineTilt: boolean;
  /** The agent's own habit (positive rises to the right); chosen by the bridge, read-only. */
  tiltDegrees?: number;
  /** Draw a wave on the page while Claude thinks. */
  thinkingAnimation: boolean;
  /** Letter spacing in percent of the glyph's own advance (100 = tightest). */
  fullWidthSpacing?: number;
  halfWidthSpacing?: number;
  /**
   * Read the page before answering so the answer keeps clear of existing writing. Costs a save
   * and a read of the whole page for every question: off unless turned on.
   */
  avoidOverwriting: boolean;
  /** Tell the user, with a short notice, when they ask again while an answer is being written. */
  busyNotice: boolean;
}

export type ClientMessage =
  | {type: 'hello'; token: string; client: string}
  | {
      type: 'ask';
      id: string;
      question: string;
      /** Key of the open note: the bridge hands earlier questions of the same note to Claude. */
      note: string;
      lang: Lang;
      layout: LayoutParams;
    }
  | {type: 'cancel'; id: string}
  /** After a dropped connection: send the answer's frames from number `next` on. */
  | {type: 'resume'; id: string; next: number}
  | {type: 'report'; data: unknown}
  | {type: 'setSettings'; settings: Settings}
  | {type: 'clearHistory'};

export type BridgeMessage =
  | {
      type: 'welcome';
      bridge: string;
      version: string;
      protocol: number;
      settings?: Settings;
    }
  | {type: 'settings'; settings: Settings}
  | {type: 'historyCleared'; notes: number}
  | {type: 'status'; id: string; stage: 'thinking' | 'writing' | 'alive'}
  | {
      type: 'line';
      id: string;
      index: number;
      /** Number of this frame within the answer (line, done and error frames count together). */
      seq?: number;
      /** Page relative to the page the question was asked on (0, 1, …). */
      page: number;
      text: string;
      strokes: InkStroke[];
    }
  | {
      type: 'done';
      id: string;
      seq?: number;
      text: string;
      missing: string[];
      truncated: boolean;
      /** Pages the answer used, counting the starting page. */
      pages: number;
      elapsedMs: number;
    }
  | {
      type: 'error';
      id?: string;
      seq?: number;
      code: string;
      message: string;
    };
