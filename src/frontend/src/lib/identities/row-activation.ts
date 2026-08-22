/**
 * When a gesture on a console row means "activate this row".
 *
 * The rows of every identity listing are not `<button>`s, and cannot be: their
 * text is what an operator copies out — an address, an account id, a person id
 * — and they carry copy controls of their own, which a button may neither nest
 * nor let be selected. So each row handles the gesture itself, and has to tell
 * an activation apart from the two things that look like one: a press on a
 * control inside it, and the end of a selection dragged across its text.
 *
 * INVARIANT: one rule for every listing. A row that opens on the mouse-up of a
 * text selection makes the addresses unreadable — and the reader cannot tell
 * which of the four lists behaves which way.
 */

/** A click activates the row — unless it pressed a control or ended a selection. */
export function activatesRow(event: React.MouseEvent<HTMLElement>): boolean {
  if (event.target instanceof Element && event.target.closest("button, a")) {
    return false;
  }
  const selection = window.getSelection();
  return !selection || selection.isCollapsed;
}

/**
 * Enter and Space activate the row — but only from the row itself, never from a
 * control it contains, which has its own answer to both keys.
 */
export function activatesRowByKey(
  event: React.KeyboardEvent<HTMLElement>,
): boolean {
  if (event.key !== "Enter" && event.key !== " ") return false;
  return event.target === event.currentTarget;
}
