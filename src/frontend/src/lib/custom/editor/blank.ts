import type { Shape } from "./describe";

/** What a new entry of a list starts as, by the shape of an entry. */
export function blank(shape: Shape): unknown {
  switch (shape.of) {
    case "list":
      return [];
    case "record":
    case "variants":
      return {};
    case "flag":
      return false;
    case "number":
      return 0;
    case "text":
    case "longText":
    case "choice":
    case "reference":
    case "pick":
    case "typed":
      return "";
  }
}
