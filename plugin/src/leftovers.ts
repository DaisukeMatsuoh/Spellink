// Wave strokes that could not be removed (docs/spec/wave.md, W3). They are remembered, in memory
// only, so the next question asked on that page can finish the job. The page itself also carries
// them (every wave stroke has the `spellink-wave:` marker), so nothing is lost if the plugin
// restarts in between: the sweep at the start of a question finds them by the marker.
export interface Leftover {
  filePath: string;
  page: number;
  marker: string;
}

let remembered: Leftover[] = [];

export function recordLeftover(l: Leftover): void {
  if (
    !remembered.some(
      r =>
        r.filePath === l.filePath && r.page === l.page && r.marker === l.marker,
    )
  ) {
    remembered.push(l);
  }
}

export function leftoversOn(filePath: string, page: number): Leftover[] {
  return remembered.filter(r => r.filePath === filePath && r.page === page);
}

export function forgetLeftoversOn(filePath: string, page: number): void {
  remembered = remembered.filter(
    r => !(r.filePath === filePath && r.page === page),
  );
}

export function resetLeftoversForTests(): void {
  remembered = [];
}
