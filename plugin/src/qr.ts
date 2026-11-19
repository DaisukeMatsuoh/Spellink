// QR code as a boolean matrix (true = dark module). Used on the pairing screen, where the
// device shows a one-time code that the computer reads with its camera.
import qrcode from 'qrcode-generator';

/** Longest text we accept; a pairing code is far shorter, and a bigger code is hard to scan on e-ink. */
export const MAX_QR_TEXT = 200;

export type QrMatrix = boolean[][];

/** The QR matrix for `text` (error correction M). Throws on empty or too long text. */
export function qrMatrix(text: string): QrMatrix {
  if (text.length === 0 || text.length > MAX_QR_TEXT) {
    throw new Error(`qr: text length ${text.length} not in 1..${MAX_QR_TEXT}`);
  }
  const qr = qrcode(0, 'M');
  qr.addData(text);
  qr.make();
  const n = qr.getModuleCount();
  if (!Number.isInteger(n) || n < 21 || n > 177) {
    throw new Error(`qr: module count ${n} out of range`);
  }
  const rows: QrMatrix = [];
  for (let r = 0; r < n; r++) {
    const row: boolean[] = [];
    for (let c = 0; c < n; c++) {
      row.push(qr.isDark(r, c));
    }
    rows.push(row);
  }
  return rows;
}

/** Runs of dark modules per row, as [start, length] pairs; fewer views to draw than one per module. */
export function darkRuns(row: boolean[]): Array<[number, number]> {
  const runs: Array<[number, number]> = [];
  let start = -1;
  for (let i = 0; i <= row.length; i++) {
    const dark = i < row.length && row[i];
    if (dark && start < 0) {
      start = i;
    } else if (!dark && start >= 0) {
      runs.push([start, i - start]);
      start = -1;
    }
  }
  return runs;
}
