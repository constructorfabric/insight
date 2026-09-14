/**
 * Geometry assertions for stories that run in a real browser.
 *
 * A header whose title is painted under its own buttons still passes every
 * query-by-text assertion — the text is in the DOM, it is just unreadable. So
 * these read the boxes the browser actually laid out. They are meaningless in
 * jsdom, which gives every element a zero rect.
 */

/** Two boxes share at least one pixel. Touching edges do not count. */
function boxesOverlap(a: DOMRect, b: DOMRect): boolean {
  return (
    a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
  );
}

/**
 * Nothing in `elements` is drawn over `subject`.
 *
 * The message names the offender and both boxes, because a bare "expected
 * false" says nothing about which control landed on the title.
 */
export function expectNothingOverlaps(
  subject: Element,
  elements: readonly Element[],
  describe: (element: Element) => string
): void {
  const box = subject.getBoundingClientRect();
  for (const element of elements) {
    const other = element.getBoundingClientRect();
    if (!boxesOverlap(box, other)) continue;
    throw new Error(
      `${describe(element)} is drawn over the subject: ` +
        `subject x ${box.left}–${box.right} y ${box.top}–${box.bottom}, ` +
        `other x ${other.left}–${other.right} y ${other.top}–${other.bottom}`
    );
  }
}

/**
 * Every element is laid out within `container`'s left and right edges.
 *
 * The inline axis only: a card clips what runs past its side, and a list that
 * scrolls sideways is meant to exceed its box downward.
 */
export function expectHorizontallyContained(
  container: Element,
  elements: readonly Element[],
  describe: (element: Element) => string
): void {
  const SLACK_PX = 1;
  const box = container.getBoundingClientRect();
  for (const element of elements) {
    const other = element.getBoundingClientRect();
    if (
      other.left >= box.left - SLACK_PX &&
      other.right <= box.right + SLACK_PX
    ) {
      continue;
    }
    throw new Error(
      `${describe(element)} spills outside its container: ` +
        `container x ${box.left}–${box.right}, ` +
        `other x ${other.left}–${other.right}`
    );
  }
}

/**
 * Card widths the narrow/wide stories measure at.
 *
 * A phone is 390px of viewport; a card in the single-column stack loses the
 * page's own padding, which is what a component actually gets.
 */
export const CARD_PX_AT_390 = 296;

/** A card in a full-width section on a desktop viewport. */
export const CARD_PX_WIDE = 900;
