/**
 * The open tooltip's popup. The kit marks its positioner, popup and arrow with
 * the side it opened on; the popup is the one inside the positioner that is
 * not the decorative arrow.
 */
export function openTooltip(): Element | null {
  const marked = [...document.querySelectorAll("[data-open][data-side]")];
  return (
    marked.find(
      (element) =>
        element.parentElement?.hasAttribute("data-side") === true &&
        !element.hasAttribute("aria-hidden")
    ) ?? null
  );
}
