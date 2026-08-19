/**
 * The five text roles a person-facing screen is allowed to use.
 *
 * Not a tidy-up. Measured on the person page before this existed: six sizes
 * across nine size-and-weight combinations, and the distinctions did not line
 * up with meaning. A section heading differed from a caption by weight alone at
 * the same 12px. One 14px step carried a control, a metric's name, a card's
 * label AND the metric's own value, so a row read flat — the number looked like
 * its label. The same thing, a metric's name, was near-black in one place and
 * grey in another. Two pill sizes, 10px and 11px, sat next to each other.
 *
 * A reader learns a screen by learning its rules. Nine combinations with no
 * rule behind them is not a hierarchy, it is decoration, and the eye has
 * nothing to hold on to.
 *
 * So: one role, one style, and every role visibly different from its
 * neighbours. Reach for the nearest role rather than inventing a size — a size
 * used once is the drift starting again.
 */

/**
 * The single large number on a card. Tabular so columns of digits line up.
 *
 * 24px rather than 30: at 30 the number was the card, and everything that
 * makes it mean something — what it counts, what it is compared against, which
 * way it moved — sat around it as small print. A figure has to be found at a
 * glance, not shouted; the size only has to beat the label above it, and it
 * does that comfortably here.
 */
export const TEXT_FIGURE = "text-2xl font-semibold tabular-nums";

/** The name of the person or page. One per screen. */
export const TEXT_TITLE = "text-lg font-semibold tracking-tight";

/**
 * The name of a thing — a metric, a card, a section. Ink, not grey.
 *
 * This is the subject of whatever it labels, and the reader scans by it. An
 * earlier pass made these grey to settle a real inconsistency (the same metric
 * name was near-black in one place and grey in another); grey was the wrong
 * half of it to keep. What is grey is context — medians, comparisons, units,
 * the pills that qualify a value. What names something is read first and gets
 * the ink to say so.
 */
export const TEXT_NAME = "text-sm font-medium text-foreground";

/** A section or card heading — a name with more weight behind it. */
export const TEXT_HEADING = "text-sm font-semibold text-foreground";

/** Values and running text — the default. */
export const TEXT_BODY = "text-sm";

/**
 * Context around a value: medians, comparisons, units, pills, captions.
 * Always muted, always this size — grey is what the reader's eye may skip, so
 * nothing that must be read belongs here.
 */
export const TEXT_LABEL = "text-xs font-medium text-muted-foreground";

/**
 * An uppercase section eyebrow — the same step as a label, spaced out. Kept
 * distinct from `TEXT_LABEL` only because the letter-spacing is load-bearing
 * at that size; it is the same size and colour on purpose.
 */
export const TEXT_EYEBROW =
  "text-xs font-medium tracking-wider text-muted-foreground uppercase";
