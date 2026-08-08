export interface PersonCoverageProps {
  /** Sections nobody is measured in — the pool has no readings either. */
  unmeasured: string[];
  /** Sections that measure fine and hold nothing for this person. */
  inactive: string[];
}

function list(titles: string[]): string {
  if (titles.length < 3) return titles.join(" and ");
  return `${titles.slice(0, -1).join(", ")} and ${titles[titles.length - 1]}`;
}

/**
 * What this page cannot see, and why.
 *
 * Without it the screen reads as a whole picture of a person, and for someone
 * whose work leaves few traces in the connected systems that picture is mostly
 * their chat and their calendar. Naming the gap costs one line and no request
 * — the standings behind it are the ones the navigation already asked for.
 *
 * The two reasons a section is blank must not share a sentence. "No data
 * reaches us" is a claim about the instrument; "nothing recorded" is a claim
 * about the person. Saying the first where the second is true tells a reader
 * their tracker is broken when their colleagues' numbers sit on the next
 * screen; saying the second where the first is true blames a person for a
 * connector nobody has wired. The pool decides which one is honest.
 */
export function PersonCoverage({ unmeasured, inactive }: PersonCoverageProps) {
  if (unmeasured.length === 0 && inactive.length === 0) return null;
  return (
    <p className="text-xs text-muted-foreground">
      {unmeasured.length > 0 ? (
        <>
          No data reaches us for {list(unmeasured)} — this page shows what is
          measured, not everything this person does.{" "}
        </>
      ) : null}
      {inactive.length > 0 ? (
        <>Nothing recorded in {list(inactive)} this period.</>
      ) : null}
    </p>
  );
}
