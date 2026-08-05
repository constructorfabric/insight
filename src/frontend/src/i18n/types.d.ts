import "i18next";

import type { defaultNS } from "./resources";

declare module "i18next" {
  interface CustomTypeOptions {
    defaultNS: typeof defaultNS;
    returnNull: false;
  }
}
