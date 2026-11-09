import type { ParseKeys, TFunction } from "i18next";

export interface LocalizedText {
  key: ParseKeys;
}

export type DisplayText = string | LocalizedText;

export function resolveText(t: TFunction, text: DisplayText): string {
  return typeof text === "string" ? text : t(text.key);
}
